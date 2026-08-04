use super::*;

use ffxi_viewer_core::hud::bazaar_view::BazaarScreenState;
use ffxi_viewer_core::hud::check_view::{self, CheckTarget};

/// Keeps the Check/Bazaar windows in step with the server:
/// - leaving `Bazaar` when the browsed bazaar disappears (s2c 0x107, or its last
///   row selling out). Only the `Some → None` edge closes it, so the mode
///   survives the gap between our c2s 0x105 and the first row arriving.
/// - keeping the row cursor inside a list the server shrinks under us.
/// - closing both windows on a zone change, since the checked PC and their
///   bazaar are entities of the zone we just left. A zone change is not an
///   `AppPhase` transition, so no state-exit cleanup fires for us (kuluu-oe8y).
pub fn bazaar_mode_sync_system(
    state: Res<SceneState>,
    mut mode: ResMut<InputMode>,
    mut screen: ResMut<BazaarScreenState>,
    mut check: ResMut<CheckTarget>,
    mut was_open: Local<bool>,
    mut last_zone: Local<Option<u16>>,
) {
    let zone = state.snapshot.zone_id;
    if zone.is_some() && *last_zone != zone {
        let zoned = last_zone.is_some();
        *last_zone = zone;
        if zoned {
            screen.reset();
            check.close();
            if matches!(*mode, InputMode::Check | InputMode::Bazaar) {
                *mode = InputMode::World;
            }
        }
    }

    let open = state.snapshot.bazaar.is_some();
    if matches!(*mode, InputMode::Bazaar) {
        if *was_open && !open {
            screen.reset();
            *mode = InputMode::Check;
        } else if let Some(view) = state.snapshot.bazaar.as_ref() {
            screen.clamp(view.items.len());
        }
    }
    *was_open = open;
}

/// Keyboard handling for the /check window: the equipment grid cursor plus the
/// View Wares entry.
pub(super) fn handle_check_key(
    key: &Key,
    bindings: &Bindings,
    check: &mut CheckTarget,
    screen: &mut BazaarScreenState,
    scene_state: &SceneState,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    let Some(target_id) = check.target_id else {
        check.close();
        return Some(InputMode::World);
    };
    let snap = &scene_state.snapshot;
    let wares_enabled = check_view::wares_enabled(snap, target_id);

    for (action, dx, dy) in [
        (Action::NavUp, 0, -1),
        (Action::NavDown, 0, 1),
        (Action::NavLeft, -1, 0),
        (Action::NavRight, 1, 0),
    ] {
        if bindings.matches_logical(action, key) {
            check.move_focus(dx, dy, wares_enabled);
            return None;
        }
    }

    if bindings.matches_logical(Action::NavCancel, key) {
        check.close();
        return Some(InputMode::World);
    }

    if !bindings.matches_logical(Action::NavConfirm, key) {
        return None;
    }
    if !check.on_wares || !wares_enabled {
        return None;
    }
    let target_index = snap
        .entities
        .iter()
        .find(|e| e.id == target_id)
        .map(|e| e.act_index)?;
    let _ = cmd_tx.try_send(AgentCommand::OpenBazaar {
        target_id,
        target_index,
    });
    screen.reset();
    Some(InputMode::Bazaar)
}

/// Keyboard handling for the browsed bazaar's wares list.
pub(super) fn handle_bazaar_key(
    key: &Key,
    bindings: &Bindings,
    screen: &mut BazaarScreenState,
    scene_state: &SceneState,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    let Some(view) = scene_state.snapshot.bazaar.as_ref() else {
        return Some(InputMode::Check);
    };

    // Quantity picker for the focused stack.
    if let Some(spinner) = screen.quantity.as_mut() {
        if bindings.matches_logical(Action::NavConfirm, key) {
            let quantity = spinner.confirm();
            screen.quantity = None;
            if let Some(entry) = view.items.get(screen.cursor) {
                let _ = cmd_tx.try_send(AgentCommand::BuyBazaarItem {
                    index: entry.index,
                    quantity,
                });
            }
            return None;
        }
        if bindings.matches_logical(Action::NavCancel, key) {
            screen.quantity = None;
            return None;
        }
        if bindings.matches_logical(Action::NavUp, key) {
            spinner.up();
        } else if bindings.matches_logical(Action::NavDown, key) {
            spinner.down();
        } else if bindings.matches_logical(Action::NavRight, key) {
            spinner.jump_up();
        } else if bindings.matches_logical(Action::NavLeft, key) {
            spinner.jump_down();
        } else if matches!(key, Key::Tab) {
            spinner.set_all();
        }
        return None;
    }

    if bindings.matches_logical(Action::NavUp, key) {
        screen.move_cursor(-1, view.items.len());
        return None;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        screen.move_cursor(1, view.items.len());
        return None;
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        let _ = cmd_tx.try_send(AgentCommand::CloseBazaar);
        screen.reset();
        return Some(InputMode::Check);
    }
    if !bindings.matches_logical(Action::NavConfirm, key) {
        return None;
    }
    let entry = view.items.get(screen.cursor)?;
    match BazaarScreenState::begin_quantity(entry) {
        Some(spinner) => screen.quantity = Some(spinner),
        None => {
            let _ = cmd_tx.try_send(AgentCommand::BuyBazaarItem {
                index: entry.index,
                quantity: 1,
            });
        }
    }
    None
}
