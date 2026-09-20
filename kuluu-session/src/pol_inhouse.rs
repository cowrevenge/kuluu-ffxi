//! In-house PlayOnline login: the TCP transport that carries the `ffxi-pol`
//! account handshake, and the entry `AuthClient` drives for the PlayOnline
//! auth flavor.
//!
//! Kuluu performs the PlayOnline account handshake itself and receives the
//! session Square Enix issues, the same way the Viewer does; no Viewer process
//! and no session file are involved. The wire layers live in `ffxi-pol`,
//! proven against a mock; this module is the socket that carries them and the
//! credential the player supplies.
//!
//! Running it contacts Square Enix's account servers with the player's own
//! account, so it is the player's action on their own machine, never Kuluu's
//! automated traffic. Nothing here is exercised by any automated run: the
//! wire layers are proven against an in-process mock, and only a player
//! signing in on their own machine reaches a real host.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rand::SeedableRng as _;

use ffxi_pol::transport::{ByteChannel, Connector};

/// The account the player signs in with. A PlayOnline account carries two
/// identities and the handshake uses both: the PlayOnline pair authenticates
/// the connection and the Square Enix pair authenticates the member. Neither
/// password is logged or persisted by this module.
#[derive(Clone, Default)]
pub struct Credentials {
    /// Eight characters, four capitals then four digits.
    pub playonline_id: String,
    pub playonline_password: String,
    pub square_enix_id: String,
    pub square_enix_password: String,
    /// The six characters of a security token, for an account that uses one.
    pub otp: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("playonline_id", &self.playonline_id)
            .field("playonline_password", &"<redacted>")
            .field("square_enix_id", &self.square_enix_id)
            .field("square_enix_password", &"<redacted>")
            .field("otp", &self.otp.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Credentials {
    fn account(&self) -> Result<ffxi_pol::transport::Account> {
        let playonline_id = self
            .playonline_id
            .as_bytes()
            .try_into()
            .map_err(|_| anyhow!("a PlayOnline ID is exactly eight characters"))?;
        let otp = match &self.otp {
            None => None,
            Some(text) => Some(
                text.as_bytes()
                    .try_into()
                    .map_err(|_| anyhow!("a one-time password is exactly six characters"))?,
            ),
        };
        Ok(ffxi_pol::transport::Account {
            playonline_id,
            playonline_password: self.playonline_password.clone(),
            square_enix_id: self.square_enix_id.clone(),
            square_enix_password: self.square_enix_password.clone(),
            otp,
        })
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
/// Viewer. The chat host is the one the Viewer's own member records default
/// to and the profile host is the one the chat service routes the account to,
/// so the caller names neither.
///
/// The account id stays zero: it is a LandSandBoat auth-server row, and the
/// PlayOnline stack has no equivalent to read one from.
pub async fn login(creds: Credentials) -> Result<crate::auth_client::AuthSession> {
    tokio::task::spawn_blocking(move || login_blocking(&creds))
        .await
        .context("in-house PlayOnline login task")?
}

fn login_blocking(creds: &Credentials) -> Result<crate::auth_client::AuthSession> {
    let account = creds.account()?;
    let outcome = ffxi_pol::transport::login(
        &mut TcpConnector,
        &mut rand::rngs::StdRng::try_from_rng(&mut rand::rngs::SysRng)
            .map_err(|e| anyhow!("seeding the key-agreement generator: {e}"))?,
        &account,
        ffxi_pol::hosts::CHAT_HOST,
        &ffxi_pol::authcode::CommunityRequest::initial(),
    )
    .map_err(|e| anyhow!("PlayOnline account handshake: {e}"))?;

    tracing::info!(
        profile_host = %outcome.profile_host,
        world_index = outcome.selection.world_index,
        "in-house PlayOnline login completed"
    );

    Ok(crate::auth_client::AuthSession {
        account_id: 0,
        session_hash: outcome.session.value,
        auth_code: crate::auth_client::LobbyAuthCode(outcome.session.auth_code),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn creds() -> Credentials {
        Credentials {
            playonline_id: "YBCA9726".to_string(),
            playonline_password: "polsecret".to_string(),
            square_enix_id: "TESTMEMBER".to_string(),
            square_enix_password: "sqexsecret".to_string(),
            otp: Some("123456".to_string()),
        }
    }

    #[test]
    fn credentials_never_print_a_password() {
        let shown = format!("{:?}", creds());
        assert!(shown.contains("YBCA9726"));
        assert!(shown.contains("TESTMEMBER"));
        for secret in ["polsecret", "sqexsecret", "123456"] {
            assert!(!shown.contains(secret), "{secret} leaked into {shown}");
        }
    }

    #[test]
    fn an_account_carries_both_identities_and_rejects_wrong_widths() {
        let account = creds().account().unwrap();
        assert_eq!(&account.playonline_id, b"YBCA9726");
        assert_eq!(account.square_enix_id, "TESTMEMBER");
        assert_eq!(account.otp, Some(*b"123456"));

        let mut short = creds();
        short.playonline_id = "YBCA972".to_string();
        assert!(short.account().is_err());

        let mut bad_otp = creds();
        bad_otp.otp = Some("12345".to_string());
        assert!(bad_otp.account().is_err());
    }
}
