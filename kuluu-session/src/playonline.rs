//! Retail has no auth server: the PlayOnline Viewer authenticates the account
//! and hands the game an identifer and authCode, which the lobby validates in
//! the 0x26. Until the viewer handoff is traced, the session comes from a file.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::auth_client::{AuthSession, LobbyAuthCode, SESSION_HASH_LEN};

/// Path of a session file; unset means no PlayOnline session is available.
pub const SESSION_FILE_ENV: &str = "KULUU_POL_SESSION";

#[derive(Serialize, Deserialize)]
struct SessionFile {
    account_id: u32,
    #[serde(with = "hex_identifer")]
    identifer: [u8; SESSION_HASH_LEN],
    auth_code: LobbyAuthCode,
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

pub fn session_from_json(text: &str) -> Result<AuthSession> {
    let file: SessionFile = serde_json::from_str(text).context("PlayOnline session file")?;
    Ok(AuthSession {
        account_id: file.account_id,
        session_hash: file.identifer,
        auth_code: file.auth_code,
    })
}

pub fn session_to_json(session: &AuthSession) -> String {
    let file = SessionFile {
        account_id: session.account_id,
        identifer: session.session_hash,
        auth_code: session.auth_code,
    };
    serde_json::to_string_pretty(&file).expect("a session file has no unserialisable field")
}

pub fn session_from_file(path: &Path) -> Result<AuthSession> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading PlayOnline session {}", path.display()))?;
    session_from_json(&text).with_context(|| path.display().to_string())
}

/// `None` when `KULUU_POL_SESSION` is unset; an error when it names a file that
/// cannot be read as a session, so a typo never falls through to password auth.
pub fn session_from_env() -> Result<Option<AuthSession>> {
    match std::env::var_os(SESSION_FILE_ENV) {
        Some(path) if !path.is_empty() => session_from_file(Path::new(&path)).map(Some),
        _ => Ok(None),
    }
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
