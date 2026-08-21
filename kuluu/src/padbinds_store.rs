use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bevy::input::gamepad::GamepadButton;
use kuluu_render::{PadAction, PadBindings};
use serde::{Deserialize, Serialize};

/// Sparse overrides layered on the retail default layout, so the shipped
/// defaults can evolve without being frozen into saved files. `None` in
/// `overrides` unbinds the action (retail padsin's `-1`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedPadBinds {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<PadAction, Option<GamepadButton>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stick_deadzone: Option<f32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert_camera_y: Option<bool>,
}

impl PersistedPadBinds {
    pub fn into_bindings(self) -> PadBindings {
        let mut bindings = PadBindings::retail();
        for (action, button) in self.overrides {
            bindings.set(action, button);
        }
        if let Some(dz) = self.stick_deadzone {
            bindings.stick_deadzone = dz.clamp(0.0, 0.9);
        }
        if let Some(inv) = self.invert_camera_y {
            bindings.invert_camera_y = inv;
        }
        bindings
    }
}

#[derive(Debug, Clone)]
pub struct PadBindsStore {
    path: PathBuf,
}

impl PadBindsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_path() -> Result<PathBuf> {
        kuluu_session::config_dir::config_file("gamepad.json")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<PersistedPadBinds>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let pb: PersistedPadBinds = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parse {}", self.path.display()))?;
                Ok(Some(pb))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", self.path.display())),
        }
    }

    pub fn save(&self, pb: &PersistedPadBinds) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(pb).context("serialize pad bindings")?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), self.path.display()))?;
        Ok(())
    }
}

pub fn load_or_default() -> PadBindings {
    let path = match PadBindsStore::default_path() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "padbinds: no config dir; using retail defaults");
            return PadBindings::retail();
        }
    };
    let store = PadBindsStore::new(path);
    match store.load() {
        Ok(Some(pb)) => pb.into_bindings(),
        Ok(None) => PadBindings::retail(),
        Err(e) => {
            tracing::warn!(
                path = %store.path().display(),
                error = %e,
                "padbinds: parse failed; falling back to retail defaults",
            );
            PadBindings::retail()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!(
            "ffxi-padbinds-store-{}-{stamp}.json",
            std::process::id()
        ));
        p
    }

    #[test]
    fn default_path_uses_player_facing_dir() {
        let path = PadBindsStore::default_path().unwrap();
        assert!(
            path.ends_with("kuluu/gamepad.json"),
            "got {}",
            path.display()
        );
    }

    #[test]
    fn save_and_load_roundtrip_with_overrides() {
        let store = PadBindsStore::new(tmp_path());
        let mut overrides = BTreeMap::new();
        overrides.insert(PadAction::OpenChat, Some(GamepadButton::Start));
        overrides.insert(PadAction::Screenshot, None);
        let pb = PersistedPadBinds {
            overrides,
            stick_deadzone: Some(0.3),
            invert_camera_y: Some(true),
        };
        store.save(&pb).unwrap();

        let loaded = store.load().unwrap().expect("present after save");
        assert_eq!(loaded, pb);
        std::fs::remove_file(store.path()).ok();
    }

    #[test]
    fn into_bindings_layers_on_retail_defaults() {
        let mut overrides = BTreeMap::new();
        overrides.insert(PadAction::OpenChat, Some(GamepadButton::Start));
        overrides.insert(PadAction::Screenshot, None);
        let pb = PersistedPadBinds {
            overrides,
            stick_deadzone: Some(0.3),
            invert_camera_y: None,
        };
        let b = pb.into_bindings();

        assert_eq!(b.button(PadAction::OpenChat), Some(GamepadButton::Start));
        assert_eq!(b.button(PadAction::Screenshot), None);
        assert_eq!(b.button(PadAction::Confirm), Some(GamepadButton::South));
        assert_eq!(b.stick_deadzone, 0.3);
        assert!(!b.invert_camera_y);
    }

    #[test]
    fn empty_file_shape_omits_all_fields() {
        let pb = PersistedPadBinds::default();
        let json = serde_json::to_string(&pb).unwrap();
        assert_eq!(json, "{}", "got: {json}");
    }
}
