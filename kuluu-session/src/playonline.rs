//! Retail has no auth server: the PlayOnline Viewer authenticates the account
//! and hands the game a 16-byte passwd and a 64-byte authCode, which the lobby
//! validates. The viewer keeps them inside its own process, so a producer
//! running beside it writes them to a session file that this module reads.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::auth_client::{AuthSession, LobbyAuthCode, SESSION_HASH_LEN};

/// Path of a session file; unset means no PlayOnline session is available.
pub const SESSION_FILE_ENV: &str = "KULUU_POL_SESSION";

/// Where a producer writes the session when no profile names a path.
pub const SESSION_FILE_NAME: &str = "playonline-session.json";

pub fn default_session_file() -> Result<PathBuf> {
    crate::config_dir::config_file(SESSION_FILE_NAME)
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    account_id: u32,
    #[serde(with = "hex_identifer")]
    identifer: [u8; SESSION_HASH_LEN],
    auth_code: LobbyAuthCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issued_unix: Option<u64>,
}

impl SessionFile {
    fn into_session(self) -> AuthSession {
        AuthSession {
            account_id: self.account_id,
            session_hash: self.identifer,
            auth_code: self.auth_code,
        }
    }
}

/// A session and where it came from, for the launcher to show.
#[derive(Debug, Clone)]
pub struct LocatedSession {
    pub session: AuthSession,
    pub path: PathBuf,
    pub issued_unix: Option<u64>,
}

impl LocatedSession {
    /// How long ago the producer stamped the file, when it did.
    pub fn age(&self, now: SystemTime) -> Option<Duration> {
        let issued = UNIX_EPOCH + Duration::from_secs(self.issued_unix?);
        now.duration_since(issued).ok()
    }
}

mod hex_identifer {
    use super::SESSION_HASH_LEN;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8; SESSION_HASH_LEN], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<[u8; SESSION_HASH_LEN], D::Error> {
        let text = String::deserialize(d)?;
        let mut out = [0u8; SESSION_HASH_LEN];
        hex::decode_to_slice(text.trim(), &mut out).map_err(|e| {
            serde::de::Error::custom(format!(
                "identifer must be {SESSION_HASH_LEN} bytes as hex: {e}"
            ))
        })?;
        Ok(out)
    }
}

fn parse_session_file(text: &str) -> Result<SessionFile> {
    serde_json::from_str(text).context("PlayOnline session file")
}

pub fn session_from_json(text: &str) -> Result<AuthSession> {
    parse_session_file(text).map(SessionFile::into_session)
}

pub fn session_to_json(session: &AuthSession) -> String {
    session_to_json_issued(session, None)
}

pub fn session_to_json_issued(session: &AuthSession, issued_unix: Option<u64>) -> String {
    let file = SessionFile {
        account_id: session.account_id,
        identifer: session.session_hash,
        auth_code: session.auth_code,
        issued_unix,
    };
    serde_json::to_string_pretty(&file).expect("a session file has no unserialisable field")
}

pub fn session_from_file(path: &Path) -> Result<AuthSession> {
    located_from_file(path).map(|l| l.session)
}

fn located_from_file(path: &Path) -> Result<LocatedSession> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading PlayOnline session {}", path.display()))?;
    let file = parse_session_file(&text).with_context(|| path.display().to_string())?;
    Ok(LocatedSession {
        issued_unix: file.issued_unix,
        session: file.into_session(),
        path: path.to_path_buf(),
    })
}

/// `None` when `KULUU_POL_SESSION` is unset; an error when it names a file that
/// cannot be read as a session, so a typo never falls through to password auth.
pub fn session_from_env() -> Result<Option<AuthSession>> {
    match std::env::var_os(SESSION_FILE_ENV) {
        Some(path) if !path.is_empty() => session_from_file(Path::new(&path)).map(Some),
        _ => Ok(None),
    }
}

/// Finds the session for a PlayOnline profile: `KULUU_POL_SESSION` first, then
/// the path the profile names, then the default file. A named path that cannot
/// be read is an error rather than a fallthrough; only the unnamed default may
/// be absent.
pub fn locate(configured: Option<&Path>) -> Result<Option<LocatedSession>> {
    let env = std::env::var_os(SESSION_FILE_ENV);
    let default = default_session_file().ok();
    locate_in(env.as_deref(), configured, default.as_deref())
}

fn locate_in(
    env: Option<&OsStr>,
    configured: Option<&Path>,
    default: Option<&Path>,
) -> Result<Option<LocatedSession>> {
    if let Some(path) = env.filter(|p| !p.is_empty()) {
        return located_from_file(Path::new(path)).map(Some);
    }
    if let Some(path) = configured.filter(|p| !p.as_os_str().is_empty()) {
        return located_from_file(path).map(Some);
    }
    match default {
        Some(path) if path.is_file() => located_from_file(path).map(Some),
        _ => Ok(None),
    }
}

/// Where the launcher tells the player to put a session when none was found.
pub fn expected_session_path(configured: Option<&Path>) -> PathBuf {
    configured
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| default_session_file().ok())
        .unwrap_or_else(|| PathBuf::from(SESSION_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_client::LOBBY_AUTH_CODE_LEN;

    fn fixture() -> AuthSession {
        let mut code = [0u8; LOBBY_AUTH_CODE_LEN];
        for (i, b) in code.iter_mut().enumerate() {
            *b = i as u8;
        }
        AuthSession {
            account_id: 4242,
            session_hash: [0xA5; SESSION_HASH_LEN],
            auth_code: LobbyAuthCode(code),
        }
    }

    #[test]
    fn a_session_file_round_trips_through_hex_fields() {
        let session = fixture();
        let text = session_to_json(&session);
        assert!(text.contains(&hex::encode(session.session_hash)), "{text}");
        assert!(text.contains(&hex::encode(session.auth_code.0)), "{text}");
        let back = session_from_json(&text).unwrap();
        assert_eq!(back.account_id, session.account_id);
        assert_eq!(back.session_hash, session.session_hash);
        assert_eq!(back.auth_code, session.auth_code);
    }

    #[test]
    fn a_short_auth_code_is_rejected_not_padded() {
        let text = format!(
            r#"{{"account_id":1,"identifer":"{}","auth_code":"{}"}}"#,
            hex::encode([0u8; SESSION_HASH_LEN]),
            hex::encode([0u8; LOBBY_AUTH_CODE_LEN - 1])
        );
        let err = format!("{:#}", session_from_json(&text).unwrap_err());
        assert!(err.contains("authCode"), "{err}");
    }

    #[test]
    fn the_auth_code_never_reaches_debug_output() {
        let session = fixture();
        let shown = format!("{session:?}");
        assert!(shown.contains("LobbyAuthCode(set)"), "{shown}");
        assert!(
            !shown.contains(&hex::encode(session.auth_code.0)),
            "{shown}"
        );
        assert_eq!(format!("{:?}", LobbyAuthCode::NONE), "LobbyAuthCode(none)");
    }

    #[test]
    fn an_lsb_session_is_not_mistaken_for_a_playonline_one() {
        let lsb = AuthSession {
            account_id: 7,
            session_hash: [1; SESSION_HASH_LEN],
            auth_code: LobbyAuthCode::NONE,
        };
        assert!(!lsb.is_playonline());
        assert!(fixture().is_playonline());
    }

    #[test]
    fn locate_prefers_the_env_then_the_profile_then_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join("env.json");
        let profile_file = dir.path().join("profile.json");
        let default_file = dir.path().join("default.json");
        let mut env_session = fixture();
        env_session.account_id = 1;
        let mut profile_session = fixture();
        profile_session.account_id = 2;
        let mut default_session = fixture();
        default_session.account_id = 3;
        std::fs::write(&env_file, session_to_json(&env_session)).unwrap();
        std::fs::write(&profile_file, session_to_json(&profile_session)).unwrap();
        std::fs::write(
            &default_file,
            session_to_json_issued(&default_session, Some(1_000)),
        )
        .unwrap();

        let by_env = locate_in(
            Some(env_file.as_os_str()),
            Some(&profile_file),
            Some(&default_file),
        )
        .unwrap()
        .unwrap();
        assert_eq!(by_env.session.account_id, 1);
        assert_eq!(by_env.path, env_file);

        let by_profile = locate_in(
            Some(OsStr::new("")),
            Some(&profile_file),
            Some(&default_file),
        )
        .unwrap()
        .unwrap();
        assert_eq!(by_profile.session.account_id, 2);

        let by_default = locate_in(None, Some(Path::new("")), Some(&default_file))
            .unwrap()
            .unwrap();
        assert_eq!(by_default.session.account_id, 3);
        assert_eq!(by_default.issued_unix, Some(1_000));
        assert_eq!(
            by_default.age(UNIX_EPOCH + Duration::from_secs(1_060)),
            Some(Duration::from_secs(60))
        );
        assert_eq!(by_env.age(UNIX_EPOCH), None);

        let absent_default = dir.path().join("missing.json");
        assert!(locate_in(None, None, Some(&absent_default))
            .unwrap()
            .is_none());
        assert!(locate_in(None, None, None).unwrap().is_none());
    }

    #[test]
    fn a_named_session_that_cannot_be_read_is_an_error_not_a_fallthrough() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.json");
        std::fs::write(&good, session_to_json(&fixture())).unwrap();
        let missing = dir.path().join("missing.json");

        let err = format!(
            "{:#}",
            locate_in(Some(missing.as_os_str()), Some(&good), Some(&good)).unwrap_err()
        );
        assert!(err.contains("missing.json"), "{err}");
        let err = format!(
            "{:#}",
            locate_in(None, Some(&missing), Some(&good)).unwrap_err()
        );
        assert!(err.contains("missing.json"), "{err}");
        assert_eq!(
            expected_session_path(Some(&missing)),
            missing,
            "the player is told the path the profile names"
        );
        assert!(expected_session_path(None).ends_with(SESSION_FILE_NAME));
    }

    #[test]
    fn session_from_file_names_the_path_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pol-session.json");
        std::fs::write(&path, "{").unwrap();
        let err = format!("{:#}", session_from_file(&path).unwrap_err());
        assert!(err.contains("pol-session.json"), "{err}");

        std::fs::write(&path, session_to_json(&fixture())).unwrap();
        assert_eq!(session_from_file(&path).unwrap().account_id, 4242);
    }
}
