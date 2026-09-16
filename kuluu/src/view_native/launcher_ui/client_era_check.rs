use std::cmp::Ordering;

use bevy::prelude::*;
use ffxi_dat::client_profile::ClientProfile;
use ffxi_proto::login::{compare_client_ver_era, lobby_accepts_client_ver, VerLock};

use crate::ffxi_client;
use crate::launcher_store::{self, ServerProfile};

use super::server_version_check::active_server_profile;
use super::{LauncherState, ServerInfo, ServerSelectForm};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum EraVerdict {
    /// No install resolved, or no server entry to compare against.
    #[default]
    Unchecked,
    Ok,
    /// The lobby would admit the install, but it is not the era the server
    /// was built against, or it is not the entry's preferred install.
    Warn,
    /// vendor/server/src/login/view_session.cpp view_session::read_func
    /// case 0x26 would reject this patch stamp.
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveInstall {
    pub name: String,
    pub patch_version: Option<String>,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClientEraStatus {
    pub expected: String,
    pub lock: Option<VerLock>,
    pub install: Option<ActiveInstall>,
    pub preferred_client: Option<String>,
    pub verdict: EraVerdict,
}

impl ClientEraStatus {
    pub fn blocks_login(&self) -> bool {
        self.verdict == EraVerdict::Refused
    }

    pub fn install_stamp(&self) -> &str {
        self.install
            .as_ref()
            .and_then(|i| i.patch_version.as_deref())
            .unwrap_or("unknown")
    }

    pub fn install_name(&self) -> &str {
        self.install
            .as_ref()
            .map(|i| i.name.as_str())
            .unwrap_or("none")
    }

    pub fn preferred_mismatch(&self) -> Option<&str> {
        let preferred = self.preferred_client.as_deref()?;
        let active = self.install.as_ref()?;
        (active.name != preferred).then_some(preferred)
    }
}

pub(crate) fn classify(profile: &ServerProfile, install: Option<ActiveInstall>) -> ClientEraStatus {
    let expected = profile.expected_client_ver().to_string();
    let lock = profile.ver_lock();
    let preferred_client = profile.preferred_client.clone();
    let mut status = ClientEraStatus {
        expected,
        lock: Some(lock),
        install,
        preferred_client,
        verdict: EraVerdict::Unchecked,
    };
    let Some(install) = status.install.as_ref() else {
        return status;
    };
    status.verdict = match install.patch_version.as_deref() {
        None => EraVerdict::Warn,
        Some(stamp) if !lobby_accepts_client_ver(stamp, &status.expected, lock) => {
            EraVerdict::Refused
        }
        Some(stamp) if compare_client_ver_era(stamp, &status.expected) != Ordering::Equal => {
            EraVerdict::Warn
        }
        Some(_) => EraVerdict::Ok,
    };
    if status.verdict == EraVerdict::Ok && status.preferred_mismatch().is_some() {
        status.verdict = EraVerdict::Warn;
    }
    status
}

/// The install every `DatRoot::from_env_or_default` in this process will
/// load, named the way the install screen lists it; the loaded root's probe
/// is reused when it is the same directory, else the tree is probed here.
fn active_install(
    settings: &launcher_store::Settings,
    loaded: Option<&ffxi_dat::DatRoot>,
) -> Option<ActiveInstall> {
    let located = ffxi_client::resolve(settings).ok()?;
    let name = ffxi_client::installs()
        .into_iter()
        .find(|i| ffxi_client::same_dir(&i.path, &located.path))
        .map(|i| i.name)
        .unwrap_or_else(|| located.path.display().to_string());
    let patch_version = match loaded {
        Some(root) if ffxi_client::same_dir(root.root(), &located.path) => {
            root.profile().patch_version.clone()
        }
        _ => ClientProfile::probe(&located.path).patch_version,
    };
    Some(ActiveInstall {
        name,
        patch_version,
    })
}

fn evaluate_on_enter(
    form: Res<ServerSelectForm>,
    info: Res<ServerInfo>,
    loaded: Option<Res<crate::view_native::DatRootRes>>,
    mut status: ResMut<ClientEraStatus>,
) {
    let next = match active_server_profile(&form, &info) {
        Some(profile) => {
            let settings = launcher_store::load().settings;
            let loaded = loaded.as_ref().and_then(|r| r.0.as_deref());
            classify(&profile, active_install(&settings, loaded))
        }
        None => ClientEraStatus::default(),
    };
    if *status != next {
        if next.verdict != EraVerdict::Ok {
            tracing::warn!(
                verdict = ?next.verdict,
                expected = %next.expected,
                lock = ?next.lock,
                install = next.install_name(),
                install_patch = next.install_stamp(),
                "client era check against the selected server entry"
            );
        }
        *status = next;
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<ClientEraStatus>().add_systems(
        OnEnter(LauncherState::Login),
        evaluate_on_enter.before(super::login::spawn_login_ui),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher_store::AuthFlavorKind;
    use ffxi_proto::login::{LSB_CLIENT_VER, LSB_DEFAULT_VER_LOCK};

    fn profile(client_ver: Option<&str>, ver_lock: Option<u8>) -> ServerProfile {
        ServerProfile {
            name: "local".into(),
            host: "127.0.0.1".into(),
            auth_port: 54231,
            data_port: 54230,
            view_port: 54001,
            flavor: AuthFlavorKind::Json,
            xiloader_version: None,
            version_check_url: None,
            client_ver: client_ver.map(str::to_string),
            ver_lock,
            preferred_client: None,
        }
    }

    fn install(name: &str, stamp: Option<&str>) -> Option<ActiveInstall> {
        Some(ActiveInstall {
            name: name.into(),
            patch_version: stamp.map(str::to_string),
        })
    }

    #[test]
    fn no_install_is_unchecked_but_still_reports_the_expectation() {
        let s = classify(&profile(None, None), None);
        assert_eq!(s.verdict, EraVerdict::Unchecked);
        assert_eq!(s.expected, LSB_CLIENT_VER);
        assert_eq!(s.lock, Some(VerLock::from_setting(LSB_DEFAULT_VER_LOCK)));
    }

    #[test]
    fn same_era_is_ok() {
        let s = classify(
            &profile(Some("30260901_0"), Some(2)),
            install("retail", Some("30260904_1")),
        );
        assert_eq!(s.verdict, EraVerdict::Ok);
    }

    #[test]
    fn older_era_under_at_least_is_refused() {
        let s = classify(
            &profile(Some("30260203_0"), Some(2)),
            install("hxi", Some("30230905_0")),
        );
        assert_eq!(s.verdict, EraVerdict::Refused);
        assert!(s.blocks_login());
    }

    #[test]
    fn newer_era_under_at_least_warns() {
        let s = classify(
            &profile(Some("30230518_0"), Some(2)),
            install("retail", Some("30260904_1")),
        );
        assert_eq!(s.verdict, EraVerdict::Warn);
        assert!(!s.blocks_login());
    }

    #[test]
    fn newer_era_under_exact_is_refused() {
        let s = classify(
            &profile(Some("30230518_0"), Some(1)),
            install("retail", Some("30260904_1")),
        );
        assert_eq!(s.verdict, EraVerdict::Refused);
    }

    #[test]
    fn lock_off_never_blocks_but_flags_the_drift() {
        let s = classify(
            &profile(Some("30260203_0"), Some(0)),
            install("hxi", Some("30230905_0")),
        );
        assert_eq!(s.verdict, EraVerdict::Warn);
    }

    #[test]
    fn unreadable_patch_stamp_warns_rather_than_blocks() {
        let s = classify(&profile(None, None), install("odd", None));
        assert_eq!(s.verdict, EraVerdict::Warn);
        assert_eq!(s.install_stamp(), "unknown");
    }

    #[test]
    fn preferred_install_mismatch_warns_on_an_otherwise_ok_era() {
        let mut p = profile(Some("30260901_0"), Some(2));
        p.preferred_client = Some("retail".into());
        let s = classify(&p, install("retail-copy", Some("30260904_1")));
        assert_eq!(s.verdict, EraVerdict::Warn);
        assert_eq!(s.preferred_mismatch(), Some("retail"));
        let s = classify(&p, install("retail", Some("30260904_1")));
        assert_eq!(s.verdict, EraVerdict::Ok);
        assert_eq!(s.preferred_mismatch(), None);
    }
}
