//! Linkshell-name codec: the packed 6-bit encoding retail stores in a
//! linkshell item's exdata and ships in s2c 0x0C9 GENERAL `sComLinkName`.
//!
//! Port of `DecodeStringLinkshell` (vendor/server/src/common/utils.cpp:532-570)
//! over `unpackBitsLE` (utils.cpp:446), which is plain little-endian bit-field
//! extraction: the field at `bit_offset` occupies bits
//! `[bit_offset % 8, bit_offset % 8 + len)` of the LE integer starting at
//! `bit_offset / 8`.

/// Bytes of `sComLinkName` (vendor/server/src/map/packets/s2c/0x0c9_equip_inspect_general.h:47).
pub const PACKED_LEN: usize = 16;

const BITS_PER_CHAR: usize = 6;
const CHAR_MASK: u16 = (1 << BITS_PER_CHAR) - 1;

/// Encoder alphabet (utils.cpp:499-521): 1..=26 lowercase, 27..=52 uppercase,
/// 53..=62 digits. 63 is the end marker written into the trailing bits.
const CODE_LOWER_BASE: u8 = 1;
const CODE_UPPER_BASE: u8 = 27;
const CODE_DIGIT_BASE: u8 = 53;
const CODE_END: u8 = 63;

/// `std::min<size_t>(20u, ...)` — the decoder never emits more than 20
/// characters regardless of buffer size (utils.cpp:535).
const MAX_CHARS: usize = 20;

fn unpack_char(packed: &[u8], bit_offset: usize) -> u8 {
    let byte = bit_offset / 8;
    let shift = bit_offset % 8;
    let lo = packed.get(byte).copied().unwrap_or(0) as u16;
    let hi = packed.get(byte + 1).copied().unwrap_or(0) as u16;
    (((lo | (hi << 8)) >> shift) & CHAR_MASK) as u8
}

/// Decode a packed linkshell name. Returns an empty string when no linkshell
/// is equipped (the server leaves the field zeroed).
pub fn decode(packed: &[u8]) -> String {
    let len = MAX_CHARS.min(packed.len() * 8 / BITS_PER_CHAR);
    let mut out = String::with_capacity(len);
    for i in 0..len {
        let code = unpack_char(packed, i * BITS_PER_CHAR);
        match code {
            // A zero code only occurs in padding, and the encoder's partial
            // end marker can leave one bogus character in front of it — hence
            // LSB dropping the previous character here (utils.cpp:554-558).
            0 => {
                out.pop();
                break;
            }
            CODE_END => break,
            c if c < CODE_UPPER_BASE => out.push((b'a' + c - CODE_LOWER_BASE) as char),
            c if c < CODE_DIGIT_BASE => out.push((b'A' + c - CODE_UPPER_BASE) as char),
            c if c < CODE_END => out.push((b'0' + c - CODE_DIGIT_BASE) as char),
            _ => break,
        }
    }
    out
}

/// Mirror of `EncodeStringLinkshell` (utils.cpp:499-529), so tests pin our
/// decoder against the upstream encoder rather than against our own reading of
/// it. Test-only: the client never writes a linkshell name.
#[cfg(test)]
pub(crate) fn encode(name: &str) -> [u8; PACKED_LEN] {
    fn pack(buf: &mut [u8; PACKED_LEN], value: u8, bit_offset: usize, len: usize) {
        for bit in 0..len {
            if value >> bit & 1 == 1 {
                let at = bit_offset + bit;
                buf[at / 8] |= 1 << (at % 8);
            }
        }
    }
    let mut buf = [0u8; PACKED_LEN];
    let mut chars = 0usize;
    for (i, ch) in name.chars().take(MAX_CHARS).enumerate() {
        let code = match ch {
            '0'..='9' => ch as u8 - b'0' + CODE_DIGIT_BASE,
            'A'..='Z' => ch as u8 - b'A' + CODE_UPPER_BASE,
            'a'..='z' => ch as u8 - b'a' + CODE_LOWER_BASE,
            _ => 0,
        };
        pack(&mut buf, code, i * BITS_PER_CHAR, BITS_PER_CHAR);
        chars += 1;
    }
    let mut leftover = 8 - (chars * BITS_PER_CHAR) % 8;
    if leftover == 8 || leftover == 2 {
        leftover = 6;
    }
    pack(&mut buf, 0xFF, BITS_PER_CHAR * chars, leftover);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_name_length() {
        // Length mod 4 decides how many end-marker bits the encoder can fit,
        // so all four residues must survive the round trip.
        for name in ["a", "ab", "abc", "abcd", "Kuluu", "TheLinkshell1234"] {
            assert_eq!(decode(&encode(name)), name, "round trip {name}");
        }
    }

    #[test]
    fn mixed_case_and_digits_map_to_the_lsb_alphabet() {
        assert_eq!(decode(&encode("aZ09")), "aZ09");
    }

    #[test]
    fn no_linkshell_decodes_empty() {
        assert_eq!(decode(&[0u8; PACKED_LEN]), "");
    }

    #[test]
    fn decoding_stops_at_the_twenty_character_cap() {
        let long = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(decode(&encode(long)).len(), MAX_CHARS);
    }

    #[test]
    fn short_buffers_do_not_panic() {
        assert_eq!(decode(&[]), "");
        assert_eq!(decode(&[0xFF]), "");
    }
}
