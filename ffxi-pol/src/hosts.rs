//! The PlayOnline host names the Viewer and the game carry, read from the
//! binaries of a retail install. The profile host lives with its transaction
//! layer in `crate::profile::host`.

/// The FFXI lobby server the game resolves in its default connection mode.
/// FFXiMain.dll of the retail-2026-09 row (sha256 f2245d1c9d06e02c): the
/// string at VA 0x10362044, resolved from VA 0x100ed84d when the
/// connection-mode global is 0; the other modes are development paths.
pub const LOBBY_HOST: &str = "ffxi00.pol.com";

/// app.dll 7ba99828: the member chat host template `pc%03d%s.pol.com`; the
/// Viewer's application layer fills the index and the suffix.
pub fn chat_host(index: u8, suffix: &str) -> String {
    format!("pc{index:03}{suffix}.pol.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chat_host_pads_the_index_to_three_digits() {
        assert_eq!(chat_host(0, ""), "pc000.pol.com");
        assert_eq!(chat_host(7, "x"), "pc007x.pol.com");
    }
}
