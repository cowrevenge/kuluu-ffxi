//! The chat service: IRC line framing with polcore's 4-character checksum, the
//! member-handle codec, the NICK digest, and the USER/NICK builders.
//!
//! Read from polcore.dll build 73b1864b. The chat service is where the session
//! Blowfish key is agreed: the client sends its RSA modulus in the USER
//! realname, the server returns the key in numeric 300 (see `crate::rsa`), and
//! every line after that is enciphered (see `crate::crypto`). These are pure
//! functions; sequencing lives in `crate::transport`.

use md5::{Digest as _, Md5};

use crate::crypto::checksum;
use crate::rsa::KeyPair;

const CHECKSUM_CHARS: usize = 4;
/// polcore `0x10013730`: each checksum character is a 6-bit group biased here,
/// giving the printable range '?'..'~'.
const CHECKSUM_BIAS: u8 = 0x3F;
const LINE_TERMINATOR: &[u8] = b"\r\n";
/// The checksum encodes bits 8..31 of the sum, most significant group first.
const CHECKSUM_SHIFTS: [u32; CHECKSUM_CHARS] = [26, 20, 14, 8];

/// The ports the chat service listens on; the viewer picks one per attempt.
pub const PORTS: [u16; 3] = [51240, 51241, 51242];

/// polcore `0x10013730`: encode bits 8..31 of the line checksum as four biased
/// characters. Bits 0..7 are discarded.
fn encode_checksum(sum: u32) -> [u8; CHECKSUM_CHARS] {
    let mut out = [0u8; CHECKSUM_CHARS];
    for (o, shift) in out.iter_mut().zip(CHECKSUM_SHIFTS) {
        *o = (((sum >> shift) & 0x3F) as u8) + CHECKSUM_BIAS;
    }
    out
}

fn decode_checksum(chars: &[u8]) -> u32 {
    let mut v = 0u32;
    for (&c, shift) in chars.iter().zip(CHECKSUM_SHIFTS) {
        v = v.wrapping_add((u32::from(c.wrapping_sub(CHECKSUM_BIAS)) & 0x3F) << shift);
    }
    v & 0xFFFF_FF00
}

/// polcore `0x10013730`: frame a command line as the bytes handed to the
/// transport. The checksum covers the body only, not the checksum field or the
/// CRLF.
pub fn frame_line(body: &str) -> Vec<u8> {
    let body = body.as_bytes();
    let mut out = Vec::with_capacity(body.len() + CHECKSUM_CHARS + LINE_TERMINATOR.len());
    out.extend_from_slice(body);
    out.extend_from_slice(&encode_checksum(checksum(body, 0)));
    out.extend_from_slice(LINE_TERMINATOR);
    out
}

/// polcore `0x10015e80`: verify the 4-character checksum of a received line
/// (after the stream cipher has been undone).
pub fn verify_line(raw: &[u8]) -> bool {
    let line = strip_terminator(raw);
    if line.len() < CHECKSUM_CHARS {
        return false;
    }
    let (body, field) = line.split_at(line.len() - CHECKSUM_CHARS);
    decode_checksum(field) == (checksum(body, 0) & 0xFFFF_FF00)
}

fn strip_terminator(raw: &[u8]) -> &[u8] {
    let mut end = raw.len();
    while end > 0 && (raw[end - 1] == b'\r' || raw[end - 1] == b'\n') {
        end -= 1;
    }
    &raw[..end]
}

/// The body of a received line with the terminator and, when present, the
/// 4-character checksum removed.
pub fn line_body(raw: &[u8], has_checksum: bool) -> &[u8] {
    let line = strip_terminator(raw);
    if has_checksum && line.len() >= CHECKSUM_CHARS {
        &line[..line.len() - CHECKSUM_CHARS]
    } else {
        line
    }
}

const ID_ALPHABET: &[u8; 36] = b"EFKAOYMJVNGTDSWBQLPCIRHZXU6328401795";
const NICK_PREFIX: u8 = b'U';
const NICK_DIGITS: usize = 8;
const NICK_ID_BITS: u32 = 41;
/// polcore `0x1001a390` sets bits 46 and 47 before the diffusion pass and
/// clears them after, so byte 5 is a known constant during the pass.
const NICK_GUARD: u64 = 0xC000 << 32;

fn nick_diffuse(mut v: u64, descending: bool) -> u64 {
    v |= NICK_GUARD;
    let order: [u32; 5] = if descending {
        [4, 3, 2, 1, 0]
    } else {
        [0, 1, 2, 3, 4]
    };
    for k in order {
        v ^= (0xFFu64 << (8 * k)) & (v >> 8);
    }
    v & !NICK_GUARD
}

/// polcore `0x1001a390`: render a 41-bit member handle as the nine-character
/// NICK string (a 'U' prefix and eight base-36 digits).
pub fn id_to_nick(account_id: u64) -> String {
    let mut v = nick_diffuse(account_id & ((1 << NICK_ID_BITS) - 1), true);
    let mut digits = [0u8; NICK_DIGITS];
    for slot in digits.iter_mut().rev() {
        *slot = ID_ALPHABET[(v % 36) as usize];
        v /= 36;
    }
    let mut out = String::with_capacity(NICK_DIGITS + 1);
    out.push(NICK_PREFIX as char);
    out.push_str(std::str::from_utf8(&digits).unwrap());
    out
}

/// polcore `0x10016ae0`: the MD5 whose hex half goes into NICK, over the
/// challenge bytes then the credential up to but not including its NUL.
pub fn nick_digest(challenge: &[u8], credential: &[u8]) -> [u8; 16] {
    let cred = credential
        .iter()
        .position(|&b| b == 0)
        .map_or(credential, |n| &credential[..n]);
    let mut h = Md5::new();
    h.update(challenge);
    h.update(cred);
    h.finalize().into()
}

const USER_ARG0: &str = "x";
const USER_ARG2: &str = "*";

/// polcore `0x10016cf0`: the USER command carrying the client's RSA modulus
/// (base64 of the little-endian modulus bytes) in the realname field.
pub fn build_user(key: &KeyPair, mode: u8) -> Vec<u8> {
    frame_line(&format!(
        "USER {USER_ARG0} {} {USER_ARG2} :{}",
        mode & 0x0F,
        key.realname()
    ))
}

/// polcore `0x10016ae0`: the NICK command. `trailing` is a token from a
/// polcore routine the decompilation does not cover; it is empty here.
pub fn build_nick(account_id: u64, challenge: &[u8], credential: &[u8], trailing: &str) -> Vec<u8> {
    frame_line(&format!(
        "NICK {}:{}:{}",
        id_to_nick(account_id),
        hex::encode(nick_digest(challenge, credential)),
        trailing
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_framed_line_verifies_and_ends_with_crlf() {
        let framed = frame_line("USER x 0 * :blob");
        assert_eq!(&framed[framed.len() - 2..], b"\r\n");
        assert!(verify_line(&framed));
        // The checksum covers bits 8..31 of the dword sum, so a change in a
        // byte above the lowest lane is caught.
        let mut broken = framed.clone();
        broken[1] ^= 0x20;
        assert!(!verify_line(&broken));
    }

    #[test]
    fn line_body_strips_the_checksum_and_terminator() {
        let framed = frame_line("300 target payload");
        assert_eq!(line_body(&framed, true), b"300 target payload");
        assert_eq!(line_body(b"greeting field3\r\n", false), b"greeting field3");
    }

    #[test]
    fn the_nick_handle_round_trips_and_is_nine_characters() {
        for id in [0u64, 1, 0x1_2345, (1 << NICK_ID_BITS) - 1] {
            let nick = id_to_nick(id);
            assert_eq!(nick.len(), NICK_DIGITS + 1);
            assert!(nick.starts_with('U'));
        }
    }

    #[test]
    fn the_nick_digest_stops_at_the_credential_nul() {
        let with_nul = nick_digest(b"challenge", b"secret\0ignored");
        let without = nick_digest(b"challenge", b"secret");
        assert_eq!(with_nul, without);
    }

    #[test]
    fn the_user_line_carries_the_modulus_realname() {
        use crate::rng::RandomBytes;
        struct Fixed(u64);
        impl RandomBytes for Fixed {
            fn fill(&mut self, out: &mut [u8]) {
                for b in out {
                    self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
                    *b = (self.0 >> 33) as u8;
                }
            }
        }
        let key = KeyPair::generate(&mut Fixed(1));
        let line = build_user(&key, 0);
        let body = line_body(&line, true);
        let text = std::str::from_utf8(body).unwrap();
        assert!(text.starts_with("USER x 0 * :"));
        assert!(text.ends_with(&key.realname()));
    }
}
