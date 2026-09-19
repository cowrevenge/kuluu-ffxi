//! In-house PlayOnline login: the TCP transport that carries the `ffxi-pol`
//! account handshake, and the entry the launcher drives.
//!
//! This is the road that removes the Viewer dependency: instead of reading a
//! session the Viewer produced, Kuluu performs the PlayOnline account
//! handshake itself and receives the session Square Enix issues, the same way
//! the Viewer does. The wire layers live in `ffxi-pol`, proven against a mock;
//! this module is the socket that carries them and the credential the player
//! supplies.
//!
//! Running it contacts Square Enix's account servers with the player's own
//! account, so it is the player's action on their own machine, never
//! Kuluu's automated traffic. The handshake reaches the chat key agreement
//! today; the profile-service community reply that yields the lobby authCode is
//! the remaining reverse-engineering, so `login` reports that boundary rather
//! than returning a partial session.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use ffxi_pol::transport::{ByteChannel, Connector};

/// The account the player signs in with. The member name is the PlayOnline id;
/// the password is theirs and never logged or persisted by this module.
#[derive(Clone)]
pub struct Credentials {
    pub member: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("member", &self.member)
            .field("password", &"<redacted>")
            .finish()
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// A byte channel over a blocking TCP socket. The handshake is a short
/// request/response sequence, so it runs on a blocking socket inside
/// `spawn_blocking` rather than threading async through every layer.
struct TcpChannel {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl TcpChannel {
    fn connect(host: &str, port: u16) -> Result<Self> {
        let addr = (host, port);
        let stream =
            TcpStream::connect(addr).with_context(|| format!("connecting to {host}:{port}"))?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            reader,
            writer: stream,
        })
    }
}

impl ByteChannel for TcpChannel {
    fn write_all(&mut self, buf: &[u8]) -> ffxi_pol::Result<()> {
        self.writer.write_all(buf).map_err(ffxi_pol::Error::from)
    }

    fn read_line(&mut self) -> ffxi_pol::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = self.reader.read(&mut byte).map_err(ffxi_pol::Error::from)?;
            if n == 0 {
                break;
            }
            out.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        if out.is_empty() {
            return Err(ffxi_pol::Error::protocol("connection closed before a line"));
        }
        Ok(out)
    }

    fn read_exact(&mut self, n: usize) -> ffxi_pol::Result<Vec<u8>> {
        let mut out = vec![0u8; n];
        self.reader
            .read_exact(&mut out)
            .map_err(ffxi_pol::Error::from)?;
        Ok(out)
    }
}

/// Opens TCP channels to the PlayOnline account hosts the player names.
pub struct TcpConnector;

impl Connector for TcpConnector {
    fn connect(&mut self, host: &str, port: u16) -> ffxi_pol::Result<Box<dyn ByteChannel>> {
        TcpChannel::connect(host, port)
            .map(|c| Box::new(c) as Box<dyn ByteChannel>)
            .map_err(|e| ffxi_pol::Error::protocol(format!("{e:#}")))
    }
}

/// Sign in to a PlayOnline account and produce a lobby session, without the
/// Viewer. `chat_host` names the chat service and `profile_host` the profile
/// service; both are the player's own account's hosts.
///
/// The handshake is not yet complete end to end: the profile community reply
/// that yields the 64-byte authCode is the remaining reverse-engineering, so
/// this reports that boundary rather than returning a session that would not
/// open the lobby.
pub async fn login(
    _chat_host: String,
    _profile_host: String,
    creds: Credentials,
) -> Result<crate::auth_client::AuthSession> {
    tokio::task::spawn_blocking(move || login_blocking(&creds))
        .await
        .context("in-house PlayOnline login task")?
}

fn login_blocking(_creds: &Credentials) -> Result<crate::auth_client::AuthSession> {
    bail!(
        "in-house PlayOnline login is not yet complete: the profile community \
         reply that yields the lobby authCode, and the origin of the chat NICK \
         credential for a first login, are the remaining reverse-engineering. \
         The cipher, key agreement, and transaction framing are implemented \
         and proven against a mock server."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn the_tcp_channel_frames_lines_and_reads_exact_counts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut got = [0u8; 5];
            std::io::Read::read_exact(&mut sock, &mut got).unwrap();
            assert_eq!(&got, b"PING\n");
            sock.write_all(b"first line\r\n").unwrap();
            sock.write_all(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        });

        let mut chan = TcpChannel::connect(&addr.ip().to_string(), addr.port()).unwrap();
        chan.write_all(b"PING\n").unwrap();
        assert_eq!(chan.read_line().unwrap(), b"first line\r\n");
        assert_eq!(chan.read_exact(8).unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
        server.join().unwrap();
    }

    #[test]
    fn credentials_never_print_the_password() {
        let creds = Credentials {
            member: "TESTMEMBER".to_string(),
            password: "s3cr3t".to_string(),
        };
        let shown = format!("{creds:?}");
        assert!(shown.contains("TESTMEMBER"));
        assert!(!shown.contains("s3cr3t"));
        assert!(shown.contains("redacted"));
    }
}
