use std::cmp::Ordering;

include!(concat!(env!("OUT_DIR"), "/xiloader_version_table.rs"));
include!(concat!(env!("OUT_DIR"), "/login_settings_table.rs"));

pub const IXFF_TERMINATOR: u32 = u32::from_le_bytes(*b"IXFF");

pub const LOGIN_AUTH_PORT: u16 = 54231;
pub const LOGIN_DATA_PORT: u16 = 54230;
pub const LOGIN_VIEW_PORT: u16 = 54001;

// vendor/server/src/login/view_session.cpp view_session::read_func case 0x26:
// the lobby compares only the first six characters of the client's patch
// stamp against login.CLIENT_VER, replacing the rest with a fixed suffix.
pub const CLIENT_VER_ERA_LEN: usize = 6;
pub const CLIENT_VER_KEY_SUFFIX: &str = "xx_x";

/// login.VER_LOCK as view_session::read_func case 0x26 switches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerLock {
    Off,
    Exact,
    AtLeast,
}

impl VerLock {
    pub fn from_setting(ver_lock: u8) -> Self {
        match ver_lock {
            1 => VerLock::Exact,
            2 => VerLock::AtLeast,
            _ => VerLock::Off,
        }
    }
}

pub fn client_ver_key(stamp: &str) -> String {
    let mut key: String = stamp.chars().take(CLIENT_VER_ERA_LEN).collect();
    key.push_str(CLIENT_VER_KEY_SUFFIX);
    key
}

/// A stamp too short to carry an era orders below every server pin: the lobby
/// would byte-compare whatever sits in its buffer, and "older than anything"
/// is the reading that keeps the mismatch visible.
pub fn compare_client_ver_era(client: &str, expected: &str) -> Ordering {
    if client.chars().count() < CLIENT_VER_ERA_LEN {
        return Ordering::Less;
    }
    client_ver_key(client).cmp(&client_ver_key(expected))
}

pub fn lobby_accepts_client_ver(client: &str, expected: &str, lock: VerLock) -> bool {
    let era = compare_client_ver_era(client, expected);
    match lock {
        VerLock::Off => true,
        VerLock::Exact => era == Ordering::Equal,
        VerLock::AtLeast => era != Ordering::Less,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // vendor/server/settings/default/login.lua CLIENT_VER / VER_LOCK; a vendor
    // pin bump updates both literals alongside the scrape.
    const LSB_PINNED_CLIENT_VER: &str = "30260904_1";
    const LSB_PINNED_VER_LOCK: u8 = 2;

    #[test]
    fn scraped_login_settings_match_the_pinned_lsb_tree() {
        assert_eq!(LSB_CLIENT_VER, LSB_PINNED_CLIENT_VER);
        assert_eq!(LSB_DEFAULT_VER_LOCK, LSB_PINNED_VER_LOCK);
    }

    #[test]
    fn ver_lock_setting_maps_like_view_session() {
        assert_eq!(VerLock::from_setting(0), VerLock::Off);
        assert_eq!(VerLock::from_setting(1), VerLock::Exact);
        assert_eq!(VerLock::from_setting(2), VerLock::AtLeast);
        assert_eq!(VerLock::from_setting(3), VerLock::Off);
        assert_eq!(VerLock::from_setting(u8::MAX), VerLock::Off);
        assert_eq!(
            VerLock::from_setting(LSB_DEFAULT_VER_LOCK),
            VerLock::AtLeast
        );
    }

    #[test]
    fn client_ver_key_keeps_only_the_era() {
        assert_eq!(client_ver_key("30260904_1"), "302609xx_x");
        assert_eq!(client_ver_key("30260203_0"), "302602xx_x");
    }

    #[test]
    fn era_comparison_ignores_day_and_sequence() {
        assert_eq!(
            compare_client_ver_era("30260203_1", "30260203_0"),
            Ordering::Equal
        );
        assert_eq!(
            compare_client_ver_era("30260228_9", "30260203_0"),
            Ordering::Equal
        );
        assert_eq!(
            compare_client_ver_era("30260904_1", "30260203_0"),
            Ordering::Greater
        );
        assert_eq!(
            compare_client_ver_era("30230905_0", "30260203_0"),
            Ordering::Less
        );
    }

    #[test]
    fn short_or_empty_stamp_compares_less() {
        assert_eq!(compare_client_ver_era("", "30260203_0"), Ordering::Less);
        assert_eq!(
            compare_client_ver_era("30260", "30260203_0"),
            Ordering::Less
        );
        assert_eq!(
            compare_client_ver_era("302602", "30260203_0"),
            Ordering::Equal
        );
    }

    #[test]
    fn lock_off_accepts_every_era() {
        assert!(lobby_accepts_client_ver(
            "30230905_0",
            LSB_CLIENT_VER,
            VerLock::Off
        ));
        assert!(lobby_accepts_client_ver(
            "30260203_0",
            LSB_CLIENT_VER,
            VerLock::Off
        ));
        assert!(lobby_accepts_client_ver(
            "30260904_1",
            LSB_CLIENT_VER,
            VerLock::Off
        ));
        assert!(lobby_accepts_client_ver("", LSB_CLIENT_VER, VerLock::Off));
    }

    #[test]
    fn lock_exact_accepts_only_the_same_era() {
        assert!(!lobby_accepts_client_ver(
            "30230905_0",
            LSB_CLIENT_VER,
            VerLock::Exact
        ));
        assert!(!lobby_accepts_client_ver(
            "30260203_0",
            LSB_CLIENT_VER,
            VerLock::Exact
        ));
        assert!(lobby_accepts_client_ver(
            "30260904_1",
            LSB_CLIENT_VER,
            VerLock::Exact
        ));
        assert!(lobby_accepts_client_ver(
            "30260928_2",
            LSB_CLIENT_VER,
            VerLock::Exact
        ));
        assert!(!lobby_accepts_client_ver(
            "30261001_0",
            LSB_CLIENT_VER,
            VerLock::Exact
        ));
        assert!(!lobby_accepts_client_ver(
            "",
            LSB_CLIENT_VER,
            VerLock::Exact
        ));
    }

    #[test]
    fn lock_at_least_rejects_only_older_eras() {
        assert!(!lobby_accepts_client_ver(
            "30230905_0",
            LSB_CLIENT_VER,
            VerLock::AtLeast
        ));
        assert!(!lobby_accepts_client_ver(
            "30260203_0",
            LSB_CLIENT_VER,
            VerLock::AtLeast
        ));
        assert!(lobby_accepts_client_ver(
            "30260904_1",
            LSB_CLIENT_VER,
            VerLock::AtLeast
        ));
        assert!(lobby_accepts_client_ver(
            "30261001_0",
            LSB_CLIENT_VER,
            VerLock::AtLeast
        ));
        assert!(!lobby_accepts_client_ver(
            "",
            LSB_CLIENT_VER,
            VerLock::AtLeast
        ));
    }
}
