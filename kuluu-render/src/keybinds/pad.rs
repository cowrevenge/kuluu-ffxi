use std::collections::BTreeMap;

use bevy::input::gamepad::GamepadButton;
use bevy::prelude::Resource;

/// Retail's gamepad model binds physical buttons to game FUNCTIONS (the
/// 27-slot `padsin000` registry table written by FFXiPadConfig.exe), not to
/// keys. These are the digital slots of that table; the movement/camera/menu
/// axis groups (slots 13-24) are fixed to the sticks and d-pad.
/// Slot numbers per docs.ashitaxi.com/usage/configurations/.
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum PadAction {
    /// Slot 0: toggle auto-run.
    Autorun,
    /// Slot 1: toggle CTRL macro bar. Parsed but inert until the macro
    /// system lands (kuluu-mco).
    CtrlMacroBar,
    /// Slot 2: toggle first/third person view.
    ViewToggle,
    /// Slot 3: toggle ALT macro bar. Inert until kuluu-mco; its retail
    /// button (RT) interim-binds OpenChat instead.
    AltMacroBar,
    /// Slot 4: toggle /heal with no target, lock-on with one.
    HealLock,
    /// Slot 5: cancel.
    Cancel,
    /// Slot 6: main menu.
    MainMenu,
    /// Slot 7: select / confirm.
    Confirm,
    /// Slot 8: select active window.
    ActiveWindow,
    /// Slot 9: toggle menu/window visibility.
    HideWindows,
    /// Slot 12: toggle logout window. Parsed but inert: no direct logout
    /// window exists yet (logout goes through the main menu).
    Logout,
    /// Slot 25: take screenshot.
    Screenshot,
    /// Kuluu extension (no retail slot): open the chat input line. Interim
    /// occupant of RT until AltMacroBar is real (kuluu-uos3.5).
    OpenChat,
}

/// Below this deflection a stick reads as centered. Radial, with the live
/// range renormalized to 0..1 past it. Lower than the old digital-synthesis
/// threshold (0.35): an analog axis only drifts a little phantom walk at rest
/// rather than a full held key, so it needs less margin over Deck stick drift.
pub const STICK_DEADZONE_DEFAULT: f32 = 0.20;

#[derive(Resource, Debug, Clone, PartialEq)]
pub struct PadBindings {
    map: BTreeMap<PadAction, GamepadButton>,
    pub stick_deadzone: f32,
    pub invert_camera_y: bool,
}

impl Default for PadBindings {
    fn default() -> Self {
        Self::retail()
    }
}

impl PadBindings {
    /// The canonical retail layout: `padsin000` Pattern E (the XInput preset
    /// added Nov 10 2015, identical in role to the PS2 defaults), expressed in
    /// Bevy's Xbox-logical button names. Deviations: RT carries OpenChat while
    /// AltMacroBar is inert, and Logout stays unbound (see `PadAction` docs).
    pub fn retail() -> Self {
        let map = BTreeMap::from([
            (PadAction::Confirm, GamepadButton::South),
            (PadAction::Cancel, GamepadButton::East),
            (PadAction::MainMenu, GamepadButton::West),
            (PadAction::ActiveWindow, GamepadButton::North),
            (PadAction::Autorun, GamepadButton::LeftTrigger),
            (PadAction::Screenshot, GamepadButton::RightTrigger),
            (PadAction::CtrlMacroBar, GamepadButton::LeftTrigger2),
            (PadAction::OpenChat, GamepadButton::RightTrigger2),
            (PadAction::HealLock, GamepadButton::LeftThumb),
            (PadAction::ViewToggle, GamepadButton::RightThumb),
            (PadAction::HideWindows, GamepadButton::Select),
            (PadAction::Logout, GamepadButton::Start),
        ]);
        Self {
            map,
            stick_deadzone: STICK_DEADZONE_DEFAULT,
            invert_camera_y: false,
        }
    }

    pub fn button(&self, action: PadAction) -> Option<GamepadButton> {
        self.map.get(&action).copied()
    }

    pub fn set(&mut self, action: PadAction, button: Option<GamepadButton>) {
        match button {
            Some(b) => {
                self.map.insert(action, b);
            }
            None => {
                self.map.remove(&action);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (PadAction, GamepadButton)> + '_ {
        self.map.iter().map(|(a, b)| (*a, *b))
    }
}

/// Radial deadzone with the live range renormalized: full deflection still
/// reaches magnitude 1.0 and direction is preserved.
pub fn apply_stick_deadzone(v: bevy::math::Vec2, deadzone: f32) -> bevy::math::Vec2 {
    let mag = v.length();
    if mag <= deadzone {
        return bevy::math::Vec2::ZERO;
    }
    let scaled = ((mag - deadzone) / (1.0 - deadzone)).min(1.0);
    v * (scaled / mag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec2;

    #[test]
    fn retail_default_pins_pattern_e_layout() {
        let b = PadBindings::retail();
        assert_eq!(b.button(PadAction::Confirm), Some(GamepadButton::South));
        assert_eq!(b.button(PadAction::Cancel), Some(GamepadButton::East));
        assert_eq!(b.button(PadAction::MainMenu), Some(GamepadButton::West));
        assert_eq!(
            b.button(PadAction::ActiveWindow),
            Some(GamepadButton::North)
        );
        assert_eq!(
            b.button(PadAction::Autorun),
            Some(GamepadButton::LeftTrigger)
        );
        assert_eq!(
            b.button(PadAction::Screenshot),
            Some(GamepadButton::RightTrigger)
        );
        assert_eq!(
            b.button(PadAction::CtrlMacroBar),
            Some(GamepadButton::LeftTrigger2)
        );
        assert_eq!(
            b.button(PadAction::OpenChat),
            Some(GamepadButton::RightTrigger2)
        );
        assert_eq!(
            b.button(PadAction::HealLock),
            Some(GamepadButton::LeftThumb)
        );
        assert_eq!(
            b.button(PadAction::ViewToggle),
            Some(GamepadButton::RightThumb)
        );
        assert_eq!(
            b.button(PadAction::HideWindows),
            Some(GamepadButton::Select)
        );
        assert_eq!(b.button(PadAction::Logout), Some(GamepadButton::Start));
        assert_eq!(b.button(PadAction::AltMacroBar), None);
        assert_eq!(b.stick_deadzone, STICK_DEADZONE_DEFAULT);
        assert!(!b.invert_camera_y);
    }

    #[test]
    fn deadzone_zeroes_inside_and_renormalizes_outside() {
        let dz = 0.2;
        assert_eq!(apply_stick_deadzone(Vec2::new(0.1, 0.1), dz), Vec2::ZERO);
        assert_eq!(apply_stick_deadzone(Vec2::ZERO, dz), Vec2::ZERO);

        let full = apply_stick_deadzone(Vec2::new(1.0, 0.0), dz);
        assert!((full.length() - 1.0).abs() < 1e-6, "got {full:?}");

        let half = apply_stick_deadzone(Vec2::new(0.6, 0.0), dz);
        assert!((half.x - 0.5).abs() < 1e-6, "got {half:?}");
        assert_eq!(half.y, 0.0);

        let diag = apply_stick_deadzone(Vec2::new(0.5, 0.5), dz);
        assert!(
            (diag.x - diag.y).abs() < 1e-6,
            "direction preserved: {diag:?}"
        );
    }

    #[test]
    fn set_none_unbinds() {
        let mut b = PadBindings::retail();
        b.set(PadAction::OpenChat, None);
        assert_eq!(b.button(PadAction::OpenChat), None);
        b.set(PadAction::AltMacroBar, Some(GamepadButton::RightTrigger2));
        assert_eq!(
            b.button(PadAction::AltMacroBar),
            Some(GamepadButton::RightTrigger2)
        );
    }
}
