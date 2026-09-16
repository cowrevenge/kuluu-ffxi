use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use rustls_pki_types::ServerName;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;

use crate::auth_binary::{self, BinaryAuthError, Command as BinCommand, PayloadBuilder};
use crate::tls::TofuVerifier;
use ffxi_proto::login::login_cmd::{LOGIN_ATTEMPT, LOGIN_CHANGE_PASSWORD, LOGIN_CREATE};
use ffxi_proto::login::login_result::*;

/// The auth server answered and said no. The same inputs get the same
/// answer, so a supervisor must surface it rather than retry it as a
/// transport blip.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct AuthRejected(pub String);

pub fn is_auth_rejected(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.downcast_ref::<AuthRejected>().is_some())
}

/// vendor/server/src/login/auth_session.cpp auth_session::read_func: a
/// JSON-only `error_message` (the loader-version text) is sent instead of a
/// `result`, so it is checked first; a `result` other than the success code
/// for the request is the server's verdict on the credentials.
fn json_result(resp: &Value, request: &str) -> Result<u8> {
    if let Some(msg) = resp.get("error_message").and_then(Value::as_str) {
        return Err(AuthRejected(format!("{request}: {}", msg.trim().replace('\n', " "))).into());
    }
    let result = resp
        .get("result")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("{request} response missing `result`: {resp}"))?;
    Ok(result as u8)
}

fn describe_login_result(code: u8) -> String {
    match code {
        LOGIN_FAIL => "account status does not permit login (banned or suspended)".into(),
        LOGIN_ERROR => "invalid username or password".into(),
        LOGIN_ERROR_ALREADY_LOGGED_IN => "account already logged in".into(),
        LOGIN_ERROR_VERSION_UNSUPPORTED => "loader version not supported by this server".into(),
        LOGIN_ERROR_TRUST_TOKEN_INVALID => "trust token rejected".into(),
        LOGIN_ERROR_CREATE_DISABLED => "account creation is disabled on this server".into(),
        LOGIN_ERROR_CREATE_TAKEN => "username already taken".into(),
        LOGIN_ERROR_CREATE => "account creation failed".into(),
        LOGIN_ERROR_CHANGE_PASSWORD => "password change failed".into(),
        other => format!("server result {other:#04x}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFlavor {
    Json,

    Binary,
}

impl std::str::FromStr for AuthFlavor {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "json" | "lsb" => Ok(AuthFlavor::Json),
            "binary" | "hxi" | "horizon" => Ok(AuthFlavor::Binary),
            other => Err(format!(
                "unknown auth-flavor `{other}`; expected json|binary"
            )),
        }
    }
}

const AUTH_BUFFER_SIZE: usize = 4096;

const XILOADER_VERSION_ENV: &str = "FFXI_XILOADER_VERSION";

pub fn resolve_client_version(override_: Option<&str>) -> [u8; 3] {
    resolve_client_version_from(
        override_,
        std::env::var(XILOADER_VERSION_ENV).ok().as_deref(),
    )
}

fn resolve_client_version_from(override_: Option<&str>, env: Option<&str>) -> [u8; 3] {
    if let Some(s) = override_ {
        if let Some(v) = parse_version_triple(s) {
            return v;
        }
        tracing::warn!("--xiloader-version={s:?} invalid; falling back to env/default");
    }
    if let Some(s) = env {
        if let Some(v) = parse_version_triple(s) {
            return v;
        }
        tracing::warn!("{XILOADER_VERSION_ENV}={s:?} invalid; using default");
    }
    ffxi_proto::login::SUPPORTED_XILOADER_VERSION
}

/// The binary flavor's loader field is a fixed-width string with its own
/// default, so only an explicit override or the env var moves it off
/// auth_binary::DEFAULT_VERSION; the JSON default never leaks across.
pub fn resolve_binary_version(override_: Option<&str>) -> [u8; auth_binary::VERSION_FIELD_LEN] {
    resolve_binary_version_from(
        override_,
        std::env::var(XILOADER_VERSION_ENV).ok().as_deref(),
    )
}

fn resolve_binary_version_from(
    override_: Option<&str>,
    env: Option<&str>,
) -> [u8; auth_binary::VERSION_FIELD_LEN] {
    for (label, s) in [
        ("--xiloader-version", override_),
        (XILOADER_VERSION_ENV, env),
    ] {
        let Some(s) = s else {
            continue;
        };
        if let Some(v) = auth_binary::version_field(s) {
            return v;
        }
        tracing::warn!(
            "{label}={s:?} does not fit the binary loader's {}-byte single-digit x.y.z field; \
             ignoring it",
            auth_binary::VERSION_FIELD_LEN
        );
    }
    auth_binary::DEFAULT_VERSION
}

fn parse_version_triple(s: &str) -> Option<[u8; 3]> {
    let mut out = [0u8; 3];
    let mut count = 0;
    for (i, part) in s.trim().split('.').enumerate().take(3) {
        out[i] = part.parse::<u8>().ok()?;
        count += 1;
    }
    if count == 3 {
        Some(out)
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub account_id: u32,

    pub session_hash: [u8; 16],
}

pub struct AuthClient {
    pub host: String,
    pub port: u16,
    pub verifier: Arc<TofuVerifier>,
    pub config: Arc<rustls::ClientConfig>,
    pub flavor: AuthFlavor,

    pub version: [u8; 3],
    pub binary_version: [u8; auth_binary::VERSION_FIELD_LEN],

    binary_builder: std::sync::OnceLock<Result<PayloadBuilder, BinaryAuthError>>,
}

impl AuthClient {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self::with_flavor(host, port, AuthFlavor::Json)
    }

    pub fn with_flavor(host: impl Into<String>, port: u16, flavor: AuthFlavor) -> Self {
        Self::with_flavor_and_version(host, port, flavor, None)
    }

    pub fn with_flavor_and_version(
        host: impl Into<String>,
        port: u16,
        flavor: AuthFlavor,
        version_override: Option<&str>,
    ) -> Self {
        let verifier = TofuVerifier::new();
        let config = crate::tls::make_client_config(verifier.clone());
        Self {
            host: host.into(),
            port,
            verifier,
            config,
            flavor,
            version: resolve_client_version(version_override),
            binary_version: resolve_binary_version(version_override),
            binary_builder: std::sync::OnceLock::new(),
        }
    }

    fn binary_builder(&self) -> Result<&PayloadBuilder> {
        let res = self
            .binary_builder
            .get_or_init(|| PayloadBuilder::with_version(self.binary_version));
        match res {
            Ok(b) => Ok(b),
            Err(e) => Err(anyhow!("binary auth builder unavailable: {e}")),
        }
    }

    pub async fn ensure_account(&self, username: &str, password: &str) -> Result<()> {
        if self.flavor == AuthFlavor::Binary {
            return self.ensure_account_binary(username, password).await;
        }
        let payload = json!({
            "command": LOGIN_CREATE,
            "username": username,
            "password": password,
            "version": self.version,
        });
        let resp = self.exchange(&payload).await?;
        match json_result(&resp, "LOGIN_CREATE")? {
            LOGIN_SUCCESS_CREATE | LOGIN_ERROR_CREATE_TAKEN => Ok(()),
            other => {
                Err(AuthRejected(format!("LOGIN_CREATE: {}", describe_login_result(other))).into())
            }
        }
    }

    pub async fn change_password(
        &self,
        username: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<()> {
        if self.flavor == AuthFlavor::Binary {
            bail!("change_password unsupported in binary auth flavor");
        }
        let payload = json!({
            "command": LOGIN_CHANGE_PASSWORD,
            "username": username,
            "password": current_password,
            "new_password": new_password,
            "version": self.version,
        });
        let resp = self.exchange(&payload).await?;
        match json_result(&resp, "LOGIN_CHANGE_PASSWORD")? {
            LOGIN_SUCCESS_CHANGE_PASSWORD => Ok(()),
            other => Err(AuthRejected(format!(
                "LOGIN_CHANGE_PASSWORD: {}",
                describe_login_result(other)
            ))
            .into()),
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthSession> {
        if self.flavor == AuthFlavor::Binary {
            return self.login_binary(username, password).await;
        }
        let payload = json!({
            "command": LOGIN_ATTEMPT,
            "username": username,
            "password": password,
            "version": self.version,
        });
        let resp = self.exchange(&payload).await?;
        let result = json_result(&resp, "LOGIN_ATTEMPT")?;
        if result != LOGIN_SUCCESS {
            return Err(
                AuthRejected(format!("LOGIN_ATTEMPT: {}", describe_login_result(result))).into(),
            );
        }

        let account_id =
            resp.get("account_id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow!("missing account_id in {resp}"))? as u32;

        let hash_arr = resp
            .get("session_hash")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("missing session_hash array in {resp}"))?;
        if hash_arr.len() != 16 {
            bail!(
                "session_hash has {} elements, expected 16: {resp}",
                hash_arr.len()
            );
        }
        let mut session_hash = [0u8; 16];
        for (i, v) in hash_arr.iter().enumerate() {
            session_hash[i] =
                v.as_u64()
                    .ok_or_else(|| anyhow!("session_hash[{i}] not u8: {v}"))? as u8;
        }

        Ok(AuthSession {
            account_id,
            session_hash,
        })
    }

    async fn exchange(&self, payload: &Value) -> Result<Value> {
        let connector = TlsConnector::from(self.config.clone());
        let server_name = ServerName::try_from(self.host.clone())
            .map_err(|_| anyhow!("invalid server name: {}", self.host))?;
        let tcp = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .with_context(|| format!("TCP connect to {}:{}", self.host, self.port))?;
        let mut tls = connector.connect(server_name, tcp).await?;

        let body = payload.to_string();
        tls.write_all(body.as_bytes()).await?;

        tls.flush().await?;

        let mut buf = vec![0u8; AUTH_BUFFER_SIZE];
        let mut total = 0usize;

        loop {
            match tls.read(&mut buf[total..]).await {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if let Ok(v) = serde_json::from_slice::<Value>(&buf[..total]) {
                        return Ok(v);
                    }
                    if total == buf.len() {
                        bail!("auth response exceeded {AUTH_BUFFER_SIZE} bytes");
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
        }

        let trimmed = trim_trailing_zero(&buf[..total]);
        serde_json::from_slice(trimmed)
            .with_context(|| format!("auth server reply was not valid JSON: {trimmed:?}"))
    }

    async fn login_binary(&self, username: &str, password: &str) -> Result<AuthSession> {
        let builder = self.binary_builder()?;
        let payload = builder
            .build(username, password, BinCommand::Login)
            .map_err(|e| anyhow!("build binary login payload: {e}"))?;
        let reply = self.exchange_binary(&payload).await?;
        let (account_id, session_hash) = auth_binary::parse_response(&reply, builder.version)
            .map_err(|e| match e {
                BinaryAuthError::LoginFailed
                | BinaryAuthError::AlreadyLoggedIn
                | BinaryAuthError::VersionMismatch { .. } => {
                    anyhow::Error::from(AuthRejected(format!("binary login: {e}")))
                }
                other => anyhow::Error::from(other),
            })
            .with_context(|| {
                format!(
                    "binary login (server={}:{}, user={username})",
                    self.host, self.port
                )
            })?;
        Ok(AuthSession {
            account_id,
            session_hash,
        })
    }

    async fn ensure_account_binary(&self, username: &str, password: &str) -> Result<()> {
        let builder = self.binary_builder()?;
        let payload = builder
            .build(username, password, BinCommand::Create)
            .map_err(|e| anyhow!("build binary create payload: {e}"))?;
        let reply = self.exchange_binary(&payload).await?;
        match auth_binary::parse_create_response(&reply) {
            Ok(()) => Ok(()),
            Err(auth_binary::BinaryAuthError::CreateTaken) => Ok(()),
            Err(e) => Err(anyhow!("binary create: {e}")),
        }
    }

    async fn exchange_binary(&self, payload: &[u8; auth_binary::PAYLOAD_LEN]) -> Result<Vec<u8>> {
        let connector = TlsConnector::from(self.config.clone());
        let server_name = ServerName::try_from(self.host.clone())
            .map_err(|_| anyhow!("invalid server name: {}", self.host))?;
        let tcp = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .with_context(|| format!("TCP connect to {}:{}", self.host, self.port))?;
        let mut tls = connector.connect(server_name, tcp).await?;
        if std::env::var_os("FFXI_AUTH_TRACE").is_some() {
            eprintln!("[auth-trace] TX {} bytes:", payload.len());
            eprintln!("{}", hex_dump(payload));
        }
        tls.write_all(payload).await?;
        tls.flush().await?;

        let mut buf = vec![0u8; auth_binary::RESPONSE_LEN];
        let mut total = 0;
        while total < buf.len() {
            match tls.read(&mut buf[total..]).await {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
        }
        buf.truncate(total);
        if std::env::var_os("FFXI_AUTH_TRACE").is_some() {
            eprintln!("[auth-trace] RX {} bytes:", buf.len());
            eprintln!("{}", hex_dump(&buf));
        }
        Ok(buf)
    }
}

fn hex_dump(b: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in b.chunks(16).enumerate() {
        out.push_str(&format!("  {:04x}  ", i * 16));
        for (j, byte) in chunk.iter().enumerate() {
            out.push_str(&format!("{:02x} ", byte));
            if j == 7 {
                out.push(' ');
            }
        }
        for _ in chunk.len()..16 {
            out.push_str("   ");
        }
        out.push_str(" |");
        for &byte in chunk {
            out.push(if (0x20..0x7f).contains(&byte) {
                byte as char
            } else {
                '.'
            });
        }
        out.push_str("|\n");
    }
    out
}

fn trim_trailing_zero(b: &[u8]) -> &[u8] {
    let end = b.iter().rposition(|&c| c != 0).map_or(0, |i| i + 1);
    &b[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    // vendor/server/src/login/auth_session.h SupportedXiloaderVersion
    const LSB_SUPPORTED_XILOADER_VERSION: [u8; 3] = [2, 1, 0];

    #[test]
    fn default_version_matches_lsb_supported_xiloader() {
        assert_eq!(
            resolve_client_version_from(None, None),
            LSB_SUPPORTED_XILOADER_VERSION
        );
        assert_eq!(
            ffxi_proto::login::SUPPORTED_XILOADER_VERSION,
            LSB_SUPPORTED_XILOADER_VERSION
        );
    }

    #[test]
    fn error_message_is_a_rejection_even_without_a_result() {
        let resp = json!({"error_message": "Your xiloader is too old.\nPlease update to version '2.1.x'."});
        let err = json_result(&resp, "LOGIN_ATTEMPT").unwrap_err();
        assert!(is_auth_rejected(&err));
        assert_eq!(
            err.to_string(),
            "LOGIN_ATTEMPT: Your xiloader is too old. Please update to version '2.1.x'."
        );
    }

    #[test]
    fn missing_result_is_not_a_rejection() {
        let err = json_result(&json!({"junk": 1}), "LOGIN_ATTEMPT").unwrap_err();
        assert!(!is_auth_rejected(&err));
    }

    #[test]
    fn result_codes_describe_the_server_verdict() {
        assert_eq!(
            json_result(&json!({"result": 1}), "x").unwrap(),
            LOGIN_SUCCESS
        );
        assert_eq!(
            describe_login_result(LOGIN_ERROR),
            "invalid username or password"
        );
        assert_eq!(describe_login_result(0x7F), "server result 0x7f");
    }

    #[test]
    fn rejection_survives_a_context_chain() {
        let err = anyhow::Error::from(AuthRejected("no".into())).context("auth login");
        assert!(is_auth_rejected(&err));
        assert!(!is_auth_rejected(&anyhow!("connection reset")));
    }

    #[test]
    fn binary_version_defaults_to_the_loader_field_not_the_json_triple() {
        assert_eq!(
            resolve_binary_version_from(None, None),
            auth_binary::DEFAULT_VERSION
        );
        assert_eq!(resolve_binary_version_from(Some("2.1.0"), None), *b"2.1.0");
        assert_eq!(resolve_binary_version_from(None, Some("3.0.1")), *b"3.0.1");
        assert_eq!(
            resolve_binary_version_from(Some("10.0.0"), Some("3.0.1")),
            *b"3.0.1"
        );
        assert_eq!(
            resolve_binary_version_from(Some("bad"), Some("worse")),
            auth_binary::DEFAULT_VERSION
        );
    }

    #[test]
    fn binary_client_threads_the_override_into_its_builder() {
        let c =
            AuthClient::with_flavor_and_version("127.0.0.1", 1, AuthFlavor::Binary, Some("2.1.0"));
        assert_eq!(c.binary_version, *b"2.1.0");
        if let Ok(b) = c.binary_builder() {
            assert_eq!(b.version, *b"2.1.0");
        }
    }

    #[test]
    fn override_wins_over_env() {
        assert_eq!(
            resolve_client_version_from(Some("3.4.5"), Some("9.9.9")),
            [3, 4, 5]
        );
    }

    #[test]
    fn env_wins_over_default() {
        assert_eq!(resolve_client_version_from(None, Some("2.9.1")), [2, 9, 1]);
    }

    #[test]
    fn invalid_override_falls_back_to_env() {
        assert_eq!(
            resolve_client_version_from(Some("not-a-version"), Some("2.1.5")),
            [2, 1, 5]
        );
    }

    #[test]
    fn invalid_override_and_env_fall_back_to_default() {
        assert_eq!(
            resolve_client_version_from(Some("x"), Some("1.2")),
            LSB_SUPPORTED_XILOADER_VERSION
        );
    }
}
