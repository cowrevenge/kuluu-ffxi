use std::collections::BTreeSet;

use bevy::input::gamepad::{Gamepad, GamepadConnectionEvent};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::input_focus::tab_navigation::{NavAction, TabNavigation, TabNavigationError};
use bevy::input_focus::{FocusCause, InputFocus, InputFocusVisible};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use bevy::input::gamepad::GamepadButton;
use kuluu_render::keybinds::pad::apply_stick_deadzone;
use kuluu_render::{Action, Bindings, InputMode, PadAction, PadBindings};

/// Pins gamepad-reading systems to one physical device, rather than each
/// calling `gamepads.iter().next()` independently. Steam Input can mirror one
/// physical Deck controller as two simultaneous `Gamepad` entities (see the
/// doc comment on `gamepad_launcher_nav_system`); if the launcher's and the
/// in-game systems each pick a different one of the pair, a mirrored press
/// can still read as `just_pressed` on the *other* entity on the very first
/// in-game frame after a screen transition (e.g. login's character-select
/// confirm bleeding into an in-game target-action confirm). Latching to the
/// first-ever-connected entity and holding it across screens closes that gap.
#[derive(Resource, Default)]
pub(super) struct PrimaryGamepad(Option<Entity>);

pub(super) fn track_primary_gamepad_system(
    mut primary: ResMut<PrimaryGamepad>,
    mut connections: MessageReader<GamepadConnectionEvent>,
) {
    for ev in connections.read() {
        if ev.connected() {
            if primary.0.is_none() {
                primary.0 = Some(ev.gamepad);
            }
        } else if primary.0 == Some(ev.gamepad) {
            primary.0 = None;
        }
    }
}

fn primary_gamepad<'a>(
    primary: &PrimaryGamepad,
    gamepads: &'a Query<&Gamepad>,
) -> Option<&'a Gamepad> {
    primary.0.and_then(|e| gamepads.get(e).ok())
}

/// Deadzone-processed stick state, refreshed every render frame and consumed
/// by `dispatch_movement_system` / `camera_polish_system` as true analog
/// axes — never digitized into synthetic key holds. `movement` is
/// (right, forward) in the camera/lock frame; `camera` is (yaw, pitch) with
/// `PadBindings::invert_camera_y` already applied.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct PadStickIntent {
    pub movement: Vec2,
    pub camera: Vec2,
}

/// `Action`s the pad fired this frame, consumed by `handle_input_system`
/// alongside `Bindings::just_pressed`. Direct dispatch instead of pulsing
/// synthetic `KeyCode`s into `ButtonInput` — a synthesized press has no
/// matching OS release event, so it poisoned held-key state (kuluu-obha).
#[derive(Resource, Default, Debug)]
pub struct PadPressed {
    fired: BTreeSet<Action>,
}

impl PadPressed {
    pub fn just_pressed(&self, action: Action) -> bool {
        self.fired.contains(&action)
    }
}

/// A pad-synthesized key event for `text_input_system`'s raw-event handlers,
/// carried on its own message channel so Bevy's `keyboard_input_system` never
/// sees it (a synthetic press in the global `KeyboardInput` queue also lands
/// in `ButtonInput<KeyCode>` the next frame, with no release to clear it).
#[derive(Message, Debug, Clone)]
pub struct PadKeyEvent(pub KeyboardInput);

/// D-pad moves focus between launcher UI widgets (mirrors Tab/Shift+Tab); South
/// activates the focused widget (mirrors Enter). Both ride the same
/// `bevy_input_focus`/`bevy_ui_widgets` machinery every launcher_ui screen
/// already uses for keyboard/mouse, so no per-screen changes are needed.
pub(super) fn gamepad_launcher_nav_system(
    gamepads: Query<&Gamepad>,
    primary: Res<PrimaryGamepad>,
    nav: TabNavigation,
    mut focus: ResMut<InputFocus>,
    mut visible: ResMut<InputFocusVisible>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut keyboard_writer: MessageWriter<KeyboardInput>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(gamepad) = primary_gamepad(&primary, &gamepads) else {
        return;
    };

    let nav_action = if gamepad.just_pressed(GamepadButton::DPadDown)
        || gamepad.just_pressed(GamepadButton::DPadRight)
    {
        Some(NavAction::Next)
    } else if gamepad.just_pressed(GamepadButton::DPadUp)
        || gamepad.just_pressed(GamepadButton::DPadLeft)
    {
        Some(NavAction::Previous)
    } else {
        None
    };
    if let Some(action) = nav_action {
        match nav.navigate(&focus, action) {
            Ok(next) => {
                focus.set(next, FocusCause::Navigated);
                visible.0 = true;
            }
            Err(TabNavigationError::NoTabGroupForCurrentFocus { new_focus, .. }) => {
                focus.set(new_focus, FocusCause::Navigated);
                visible.0 = true;
            }
            Err(_) => {}
        }
    }

    if gamepad.just_pressed(GamepadButton::South) {
        keyboard_writer.write(KeyboardInput {
            key_code: KeyCode::Enter,
            logical_key: Key::Enter,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
    }
}

pub(super) fn gamepad_stick_system(
    gamepads: Query<&Gamepad>,
    primary: Res<PrimaryGamepad>,
    pad_bindings: Res<PadBindings>,
    mut intent: ResMut<PadStickIntent>,
) {
    let Some(gamepad) = primary_gamepad(&primary, &gamepads) else {
        *intent = PadStickIntent::default();
        return;
    };
    let dz = pad_bindings.stick_deadzone;
    let mut camera = apply_stick_deadzone(gamepad.right_stick(), dz);
    if pad_bindings.invert_camera_y {
        camera.y = -camera.y;
    }
    *intent = PadStickIntent {
        movement: apply_stick_deadzone(gamepad.left_stick(), dz),
        camera,
    };
}

/// What one retail pad function does when its button fires: `Action`s pushed
/// into [`PadPressed`] for `handle_input_system`'s `ButtonInput`-style
/// readers, and at most one action synthesized as a raw key event for
/// `text_input_system`'s modal router. Fishing actions ride along with their
/// world equivalents because `handle_input_system` consumes the fishing set
/// first (and returns) while a cast is live, exactly like the shared
/// Enter/arrow keybinds.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PadEffect {
    pub presses: &'static [Action],
    pub key: Option<Action>,
}

pub(super) fn pad_action_effect(action: PadAction, in_world: bool) -> PadEffect {
    let none = PadEffect::default();
    match (action, in_world) {
        (PadAction::Confirm, true) => PadEffect {
            presses: &[Action::ConfirmAction, Action::FishingHook],
            key: Some(Action::ConfirmAction),
        },
        (PadAction::Confirm, false) => PadEffect {
            presses: &[],
            key: Some(Action::NavConfirm),
        },
        (PadAction::Cancel, true) => PadEffect {
            presses: &[Action::ClearTarget, Action::FishingCancel],
            key: None,
        },
        (PadAction::Cancel, false) => PadEffect {
            presses: &[],
            key: Some(Action::NavCancel),
        },
        (PadAction::MainMenu, true) => PadEffect {
            presses: &[Action::OpenMenu],
            key: None,
        },
        (PadAction::MainMenu, false) => none,
        (PadAction::ActiveWindow, _) => PadEffect {
            presses: &[Action::TogglePassiveCursor],
            key: None,
        },
        (PadAction::Autorun, true) => PadEffect {
            presses: &[Action::ToggleAutorun],
            key: None,
        },
        (PadAction::Autorun, false) => none,
        (PadAction::HealLock, true) => PadEffect {
            presses: &[Action::ToggleLockOn],
            key: None,
        },
        (PadAction::HealLock, false) => none,
        (PadAction::ViewToggle, _) => PadEffect {
            presses: &[Action::ToggleFirstPerson],
            key: None,
        },
        (PadAction::Screenshot, _) => PadEffect {
            presses: &[Action::Screenshot],
            key: None,
        },
        (PadAction::HideWindows, _) => PadEffect {
            presses: &[Action::ToggleHud],
            key: None,
        },
        (PadAction::OpenChat, true) => PadEffect {
            presses: &[],
            key: Some(Action::OpenChat),
        },
        (PadAction::OpenChat, false) => none,
        // Inert until their features exist: macro bars are kuluu-mco, the
        // logout window has no direct opener yet (kuluu-uos3.5).
        (PadAction::CtrlMacroBar | PadAction::AltMacroBar | PadAction::Logout, _) => none,
    }
}

fn bound_key(bindings: &Bindings, action: Action) -> Option<KeyCode> {
    let bind = bindings.get(action)?;
    if bind.mods != Default::default() {
        return None;
    }
    Some(bind.key)
}

fn emit_pad_key(
    writer: &mut MessageWriter<PadKeyEvent>,
    window: Entity,
    bindings: &Bindings,
    action: Action,
) {
    let Some(key_code) = bound_key(bindings, action) else {
        return;
    };
    let Some(logical_key) = kuluu_render::keybinds::logical_key_for(key_code) else {
        return;
    };
    writer.write(PadKeyEvent(KeyboardInput {
        key_code,
        logical_key,
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window,
    }));
}

/// Retail-layout digital dispatch: each configured [`PadAction`] fires its
/// [`pad_action_effect`], plus the fixed d-pad roles (field targeting in
/// `World` mode, cursor movement in menus — retail `padsin` slots 21-24).
/// Reads the same pinned device every other gamepad system does — see
/// `PrimaryGamepad`'s doc comment.
pub(super) fn gamepad_action_system(
    gamepads: Query<&Gamepad>,
    primary: Res<PrimaryGamepad>,
    bindings: Res<Bindings>,
    pad_bindings: Res<PadBindings>,
    mode: Res<InputMode>,
    trade_state: Res<kuluu_render::hud::trade::TradeState>,
    mut pad_pressed: ResMut<PadPressed>,
    mut pad_key_writer: MessageWriter<PadKeyEvent>,
    windows: Query<Entity, With<PrimaryWindow>>,
) {
    pad_pressed.fired.clear();
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(gamepad) = primary_gamepad(&primary, &gamepads) else {
        return;
    };
    // A trade window doesn't change InputMode (text_input.rs checks
    // trade_state.open ahead of the InputMode match), so without this it's
    // treated as World and D-pad/Confirm/Cancel go to combat/target actions
    // instead of navigating trade slots.
    let in_world = matches!(*mode, InputMode::World) && !trade_state.open;

    for (pad_action, button) in pad_bindings.iter() {
        if !gamepad.just_pressed(button) {
            continue;
        }
        let effect = pad_action_effect(pad_action, in_world);
        pad_pressed.fired.extend(effect.presses.iter().copied());
        if let Some(action) = effect.key {
            emit_pad_key(&mut pad_key_writer, window, &bindings, action);
        }
    }

    let dpad = [
        (GamepadButton::DPadUp, Action::NavUp, None),
        (GamepadButton::DPadDown, Action::NavDown, None),
        (
            GamepadButton::DPadLeft,
            Action::NavLeft,
            Some([Action::CycleTarget, Action::FishingReelLeft]),
        ),
        (
            GamepadButton::DPadRight,
            Action::NavRight,
            Some([Action::CycleTarget, Action::FishingReelRight]),
        ),
    ];
    for (button, nav_action, world_actions) in dpad {
        if !gamepad.just_pressed(button) {
            continue;
        }
        if in_world {
            if let Some(actions) = world_actions {
                pad_pressed.fired.extend(actions);
            }
        } else {
            emit_pad_key(&mut pad_key_writer, window, &bindings, nav_action);
        }
    }
}

/// Zone/logout transitions must not carry pad state across (the
/// bevy-lifecycle-symmetry rule kuluu-obha's stuck-key drain bug violated).
pub(super) fn drain_pad_state(mut intent: ResMut<PadStickIntent>, mut pressed: ResMut<PadPressed>) {
    *intent = PadStickIntent::default();
    pressed.fired.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_mirrors_a_bound_key_on_both_channels() {
        let world = pad_action_effect(PadAction::Confirm, true);
        assert!(world.presses.contains(&Action::ConfirmAction));
        assert!(world.presses.contains(&Action::FishingHook));
        assert_eq!(world.key, Some(Action::ConfirmAction));

        let menu = pad_action_effect(PadAction::Confirm, false);
        assert_eq!(menu.presses, &[] as &[Action]);
        assert_eq!(menu.key, Some(Action::NavConfirm));
    }

    #[test]
    fn cancel_clears_target_in_world_and_navigates_in_menus() {
        let world = pad_action_effect(PadAction::Cancel, true);
        assert!(world.presses.contains(&Action::ClearTarget));
        assert!(world.presses.contains(&Action::FishingCancel));
        assert_eq!(world.key, None);

        assert_eq!(
            pad_action_effect(PadAction::Cancel, false).key,
            Some(Action::NavCancel)
        );
    }

    #[test]
    fn retail_world_roles_dispatch_their_actions() {
        for (pad, action) in [
            (PadAction::MainMenu, Action::OpenMenu),
            (PadAction::Autorun, Action::ToggleAutorun),
            (PadAction::HealLock, Action::ToggleLockOn),
            (PadAction::ViewToggle, Action::ToggleFirstPerson),
            (PadAction::Screenshot, Action::Screenshot),
            (PadAction::HideWindows, Action::ToggleHud),
            (PadAction::ActiveWindow, Action::TogglePassiveCursor),
        ] {
            let effect = pad_action_effect(pad, true);
            assert!(
                effect.presses.contains(&action),
                "{pad:?} must press {action:?}"
            );
            assert_eq!(effect.key, None, "{pad:?}");
        }
    }

    #[test]
    fn unimplemented_functions_are_inert() {
        for pad in [
            PadAction::CtrlMacroBar,
            PadAction::AltMacroBar,
            PadAction::Logout,
        ] {
            for in_world in [true, false] {
                assert_eq!(pad_action_effect(pad, in_world), PadEffect::default());
            }
        }
    }

    #[test]
    fn open_chat_only_fires_in_world() {
        assert_eq!(
            pad_action_effect(PadAction::OpenChat, true).key,
            Some(Action::OpenChat)
        );
        assert_eq!(pad_action_effect(PadAction::OpenChat, false).key, None);
    }
}
