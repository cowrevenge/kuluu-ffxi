//! Sequencing the chat-service key agreement over a byte channel.
//!
//! The layers this drives are byte-exact against the static reading of
//! polcore.dll 73b1864b; this module is the order they run in, read from the
//! chat state machine (`0x100140e0` outbound, `0x10015e80` inbound). It is
//! transport-agnostic: the caller supplies a `ByteChannel`, so the same
//! sequence runs against a live socket or an in-process mock. Nothing here
//! opens a connection or names a host.
//!
//! Scope: this reaches the point where the session Blowfish key is agreed and
//! the stream cipher is on, which is the fully-observed part of the handshake.
//! Producing the lobby authCode additionally needs the profile-service
//! community transaction and its reply assembly, which are the remaining
//! reverse-engineering.

use crate::chat;
use crate::crypto::{PolBlowfish, StreamReset};
use crate::error::{Error, Result};
use crate::rng::RandomBytes;
use crate::rsa::{KeyPair, SESSION_KEY_BYTES};

/// A bidirectional byte channel. The chat service is line-oriented and its
/// stream cipher passes CR and LF through in the clear, so a reader can frame
/// on the newline whether or not the cipher is engaged.
pub trait ByteChannel {
    fn write_all(&mut self, buf: &[u8]) -> Result<()>;
    /// Read up to and including the next `\n`.
    fn read_line(&mut self) -> Result<Vec<u8>>;
}

/// numeric 300, which carries the RSA-encrypted session key.
const NUMERIC_KEY: &str = "300";
/// numeric 422 (ERR_NOMOTD), which the client treats as login complete.
const NUMERIC_REGISTERED: &str = "422";

/// The outcome of the chat key agreement: the agreed session key and the
/// enciphering context, positioned for the first profile transaction.
pub struct ChatSession {
    pub session_key: [u8; SESSION_KEY_BYTES],
    pub cipher: PolBlowfish,
}

/// Run the chat key agreement to the registered state. `account_id` and
/// `credential` fill the NICK; `mode` is the USER mode digit.
pub fn agree_session_key(
    channel: &mut dyn ByteChannel,
    rng: &mut dyn RandomBytes,
    account_id: u64,
    credential: &[u8],
    mode: u8,
) -> Result<ChatSession> {
    let key = KeyPair::generate(rng);

    // The greeting arrives in the clear; its third field is the challenge.
    let greeting = channel.read_line()?;
    let challenge = field(chat::line_body(&greeting, true), 3)
        .ok_or_else(|| Error::protocol("chat greeting has no challenge field"))?
        .to_vec();

    channel.write_all(&chat::build_user(&key, mode))?;

    // numeric 300 is still in the clear; its third field is the encrypted key.
    let key_line = channel.read_line()?;
    let key_body = chat::line_body(&key_line, true);
    if numeric(key_body) != Some(NUMERIC_KEY) {
        return Err(Error::protocol("expected numeric 300 after USER"));
    }
    let payload =
        field(key_body, 3).ok_or_else(|| Error::protocol("numeric 300 has no key field"))?;
    let session_key = key.recover_session_key(payload)?;

    // The cipher is on from here, seeded from our own modulus.
    let mut cipher = PolBlowfish::new(&session_key, key.stream_iv());
    let nick = chat::build_nick(account_id, &challenge, credential, "");
    channel.write_all(&cipher.stream(&nick, StreamReset::Boundary))?;

    // Registration lines are enciphered; each line reciphers from the IV.
    loop {
        let raw = channel.read_line()?;
        let plain = cipher.stream(&raw, StreamReset::Boundary);
        match numeric(chat::line_body(&plain, true)) {
            Some(NUMERIC_REGISTERED) => break,
            Some(n) if n.parse::<u32>().map(|v| v > 399).unwrap_or(false) => {
                return Err(Error::protocol(format!("chat registration refused: {n}")));
            }
            _ => continue,
        }
    }

    Ok(ChatSession {
        session_key,
        cipher,
    })
}

/// The nth whitespace-separated field of a line, skipping an `:prefix` when
/// present. Fields are 1-based to match the protocol's own numbering.
fn field(line: &[u8], n: usize) -> Option<&[u8]> {
    let start = if line.first() == Some(&b':') {
        line.iter().position(|&b| b == b' ').map(|i| i + 1)?
    } else {
        0
    };
    line[start..]
        .split(|&b| b == b' ')
        .filter(|f| !f.is_empty())
        .nth(n - 1)
}

/// The command token of a line: field 1 after any prefix.
fn numeric(line: &[u8]) -> Option<&str> {
    field(line, 1).and_then(|f| std::str::from_utf8(f).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::RandomBytes;
    use std::collections::VecDeque;

    struct SeqRng(u64);
    impl RandomBytes for SeqRng {
        fn fill(&mut self, out: &mut [u8]) {
            for b in out {
                self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
                *b = (self.0 >> 33) as u8;
            }
        }
    }

    // An in-process chat server that speaks enough of the protocol to complete
    // the key agreement: greeting, numeric 300 with a key it chooses, and 422.
    // It exercises the client's whole path; it is not the retail server.
    struct MockServer {
        server_key: [u8; 8],
        outbound: VecDeque<Vec<u8>>,
        cipher: Option<PolBlowfish>,
    }

    impl MockServer {
        fn new(server_key: [u8; 8]) -> Self {
            let mut outbound = VecDeque::new();
            outbound.push_back(chat::frame_line("greeting server CHALLENGEVALUE"));
            Self {
                server_key,
                outbound,
                cipher: None,
            }
        }

        fn on_client_line(&mut self, raw: &[u8]) {
            let body = chat::line_body(raw, true);
            if body.starts_with(b"USER ") {
                let colon = body.iter().position(|&b| b == b':').unwrap();
                let realname = &body[colon + 1..];
                let modulus_le = crate::rsa::b64_decode(realname, realname.len());
                let mut rng = SeqRng(0xFEED);
                let payload =
                    crate::rsa::encrypt_session_key(&self.server_key, &modulus_le, &mut rng);
                self.outbound
                    .push_back(chat::frame_line(&format!("300 target {payload}")));
                // The client turns the cipher on right after 300; so do we,
                // seeded from the client's modulus IV.
                let iv = crate::rsa::stream_iv_from_modulus_le(&modulus_le);
                self.cipher = Some(PolBlowfish::new(&self.server_key, iv));
            } else {
                // A NICK (enciphered) means the client is registering.
                let registered =
                    self.encipher(chat::frame_line("422 target :MOTD File is missing"));
                self.outbound.push_back(registered);
            }
        }

        fn encipher(&mut self, line: Vec<u8>) -> Vec<u8> {
            self.cipher
                .as_mut()
                .map(|c| c.stream(&line, StreamReset::Boundary))
                .unwrap_or(line)
        }
    }

    struct Pipe {
        server: MockServer,
    }

    impl ByteChannel for Pipe {
        fn write_all(&mut self, buf: &[u8]) -> Result<()> {
            self.server.on_client_line(buf);
            Ok(())
        }
        fn read_line(&mut self) -> Result<Vec<u8>> {
            self.server
                .outbound
                .pop_front()
                .ok_or_else(|| Error::protocol("mock server has nothing to send"))
        }
    }

    #[test]
    fn the_client_and_a_mock_server_agree_on_the_same_session_key() {
        let server_key = [0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6, 0x07, 0x18];
        let mut pipe = Pipe {
            server: MockServer::new(server_key),
        };
        let mut rng = SeqRng(0x1234_5678);
        let session = agree_session_key(&mut pipe, &mut rng, 0x1_2345, b"secret", 0).unwrap();
        assert_eq!(session.session_key, server_key);
    }
}
