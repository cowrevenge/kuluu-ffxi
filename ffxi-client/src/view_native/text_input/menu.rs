use super::map_screen::handle_map_key;
use super::slash_apply::apply_keybind_update;
use super::*;

#[derive(Debug, Clone, PartialEq)]
enum MenuDispatch {
    CommandWithToast { cmd: AgentCommand, toast: String },

    OpenSubmenu(MenuKind),

    KeybindUpdate(KeybindUpdate),

    NotImplemented(String),
}

fn apply_graphics_cycle(
    cursor: usize,
    delta: i32,
    graphics: &mut ffxi_viewer_core::GraphicsSettings,
) {
    use ffxi_viewer_core::graphics_settings::GRAPHICS_FIELDS;
    if let Some(&field) = GRAPHICS_FIELDS.get(cursor) {
        graphics.cycle(field, delta);
    }
}

fn resolve_menu_entry(kind: MenuKind, label: &str) -> MenuDispatch {
    use ffxi_viewer_core::hud::menu::{COMM_EMOTE_LIST, ROOT_LOG_OUT, ROOT_SHUT_DOWN};
    match (kind, label) {
        (MenuKind::Communication, l) if l == COMM_EMOTE_LIST => {
            MenuDispatch::OpenSubmenu(MenuKind::EmoteList)
        }
        (MenuKind::Root, ROOT_LOG_OUT) => MenuDispatch::CommandWithToast {
            cmd: AgentCommand::ReqLogout {
                kind: ReqLogoutKind::LogoutToggle,
            },
            toast: "[menu] Log Out requested (~30s; instant in Mog House). \
                    Select again or `/logout off` to cancel."
                .into(),
        },

        (MenuKind::Root, ROOT_SHUT_DOWN) => MenuDispatch::CommandWithToast {
            cmd: AgentCommand::ReqLogout {
                kind: ReqLogoutKind::ShutdownToggle,
            },
            toast: "[menu] Shut Down requested (~30s; instant in Mog House). \
                    Select again or `/shutdown off` to cancel."
                .into(),
        },

        // The Map screen is a bespoke pane (no generic right-pane preview via
        // root_child_kind), so it needs its own drill arm ahead of the catch-all.
        (MenuKind::Root, "Map") => MenuDispatch::OpenSubmenu(MenuKind::Map),

        // Root categories that drill into a browsable submenu share their
        // mapping with the right-pane preview (single source of truth).
        (MenuKind::Root, label) => match ffxi_viewer_core::hud::menu::root_child_kind(label) {
            Some(submenu) => MenuDispatch::OpenSubmenu(submenu),
            None => MenuDispatch::NotImplemented(label.to_string()),
        },

        (MenuKind::Magic, _) => {
            MenuDispatch::NotImplemented("Magic — pending Stage 2 (learned-spell decoder)".into())
        }
        (MenuKind::Abilities, _) => MenuDispatch::NotImplemented(
            "Abilities — pending Stage 2 (s2c 0x119 abil_recast)".into(),
        ),
        (MenuKind::Items, _) => {
            MenuDispatch::NotImplemented("Items — pending Stage 3 (inventory submenu)".into())
        }
        (MenuKind::Equipment, _) => MenuDispatch::NotImplemented(
            "Equipment — pending Stage 1 (s2c 0x050 equip_list)".into(),
        ),

        (MenuKind::Config, "Standard") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Preset(Preset::Standard))
        }
        (MenuKind::Config, "Compact 1") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Preset(Preset::Compact1))
        }
        (MenuKind::Config, "Compact 2") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Preset(Preset::Compact2))
        }
        (MenuKind::Config, "Reset to defaults") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Reset)
        }
        (MenuKind::Config, "Show current bindings") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::List)
        }
        (_, other) => MenuDispatch::NotImplemented(other.to_string()),
    }
}

const EQUIP_SLOT_INDEX_MAX: u8 =
    (ffxi_viewer_core::equip_slot::EquipmentIndex::ALL.len() - 1) as u8;

pub(super) fn confirm_menu_at_cursor(
    bindings: &mut Bindings,
    stack: &mut MenuStack,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    keybinds_state: &mut KeybindsStateRes,
    graphics: &mut ffxi_viewer_core::GraphicsSettings,
    status_profile_open: &mut ffxi_viewer_core::hud::status_panel::StatusProfileOpen,
    hud_panels: &mut ffxi_viewer_core::hud::HudPanels,
    net_status: &mut ffxi_viewer_core::hud::network_status::NetStatusVisible,
    vana_clock: &ffxi_viewer_core::vana_time::VanaClock,
    vana_clock_visible: &mut ffxi_viewer_core::hud::vana_clock::VanaClockVisible,
    dynamic: &ffxi_viewer_core::hud::menu::DynamicMenu,
    target_id: Option<u32>,
    self_pos: ffxi_viewer_wire::Vec3,
) -> Option<InputMode> {
    let (kind, cursor) = {
        let level = stack.current()?;
        (level.kind, level.cursor)
    };

    if matches!(kind, MenuKind::Debug) {
        let label = ffxi_viewer_core::hud::menu::entry_label(kind, cursor, dynamic);
        toggle_debug_panel(label, hud_panels, net_status, scene_state);
        return None;
    }

    if matches!(kind, MenuKind::Root)
        && ffxi_viewer_core::hud::menu::entry_label(kind, cursor, dynamic)
            == ffxi_viewer_core::hud::menu::ROOT_CURRENT_TIME
    {
        activate_current_time(vana_clock, vana_clock_visible, scene_state);
        // Mirrors the Debug toggles: the menu stays open (provisional pending
        // retail capture, bead kuluu-y5hq retail_unknowns).
        return None;
    }

    if matches!(kind, MenuKind::Status) {
        use ffxi_viewer_core::hud::status_panel::{StatusEntryKind, STATUS_ENTRIES};
        let entry = STATUS_ENTRIES.get(cursor)?;
        match entry.kind {
            StatusEntryKind::Profile => {
                status_profile_open.0 = true;
            }
            StatusEntryKind::PlayTime => {
                let line =
                    ffxi_viewer_core::hud::status_panel::play_time_chat_line(&scene_state.snapshot);
                push_system_chat_line(scene_state, line);
            }

            StatusEntryKind::MasterLevels | StatusEntryKind::MeritPoints => {
                push_system_chat_line(
                    scene_state,
                    format!("[menu] {} — not available", entry.label),
                );
            }

            StatusEntryKind::JobLevels
            | StatusEntryKind::CombatSkill
            | StatusEntryKind::MagicSkill
            | StatusEntryKind::CraftSkill
            | StatusEntryKind::Currencies
            | StatusEntryKind::Currencies2
            | StatusEntryKind::Unity
            | StatusEntryKind::JobPoints => {
                push_system_chat_line(
                    scene_state,
                    format!("[menu] {} — not yet decoded", entry.label),
                );
            }
        }
        return None;
    }
    if matches!(kind, MenuKind::Graphics) {
        if cursor == ffxi_viewer_core::hud::menu::GRAPHICS_RESET_SLOT {
            graphics.reset_to_default();
            push_system_chat_line(scene_state, "[menu] Graphics reset to High".into());
        } else {
            apply_graphics_cycle(cursor, 1, graphics);
        }
        return None;
    }

    if matches!(kind, MenuKind::Equipment) {
        let slot = (cursor as u8).min(EQUIP_SLOT_INDEX_MAX);
        stack.push(MenuKind::EquipSlot(slot));
        return None;
    }

    if ffxi_viewer_core::hud::menu::is_dynamic(kind) {
        if let Some(action) = ffxi_viewer_core::hud::menu::entry_action(kind, cursor, dynamic) {
            use ffxi_viewer_core::hud::menu::DynamicMenuAction as A;
            if let A::OpenItemAction {
                container,
                index,
                item_no,
            } = action
            {
                stack.push(MenuKind::ItemAction {
                    container,
                    index,
                    item_no,
                });
                return None;
            }
            // Retail's key-item detail pane needs a description DAT not yet
            // identified (bead kuluu-h7x retail_unknowns); echo the name and
            // keep the list open.
            if let A::KeyItem { id } = action {
                push_system_chat_line(
                    scene_state,
                    format!(
                        "Key item: {}.",
                        ffxi_viewer_core::hud::menu::key_item_row_label(id, true)
                    ),
                );
                return None;
            }
            if let Some(sub_action) = sub_target_action_for(action) {
                if !selected_target_valid(sub_action, target_id, scene_state) {
                    // No valid target selected: retail's sub-target confirm step
                    // fires the action only after the flashing cursor is confirmed.
                    // Esc restores this menu with its cursor intact.
                    let return_to = InputMode::Menu(stack.clone());
                    return open_sub_target(sub_action, target_id, scene_state, return_to);
                }
            }
            let moved = matches!(action, A::MoveItem { .. });
            let entities = scene_state.snapshot.entities.clone();
            dispatch_dynamic_menu_action(
                action,
                target_id,
                self_pos,
                &entities,
                cmd_tx,
                scene_state,
            );
            // Retail keeps the equip list up after a gear change so the player
            // can keep swapping (or re-select to unequip), and keeps the bag
            // open after moving an item so a sort/move session flows; the
            // one-shot action menus (Magic/Abilities/item Use) close back to
            // the world.
            return if matches!(kind, MenuKind::EquipSlot(_)) {
                None
            } else if moved {
                stack.pop();
                None
            } else {
                Some(InputMode::World)
            };
        }

        push_system_chat_line(scene_state, format!("[menu] {kind:?} list is empty"));
        return None;
    }
    let label = ffxi_viewer_core::hud::menu::entry_label(kind, cursor, dynamic);
    match resolve_menu_entry(kind, label) {
        MenuDispatch::CommandWithToast { cmd, toast } => {
            if let Err(e) = cmd_tx.try_send(cmd) {
                push_system_chat_line(scene_state, format!("[menu] dispatch dropped: {e}"));
            } else {
                push_system_chat_line(scene_state, toast);
            }
            Some(InputMode::World)
        }
        MenuDispatch::OpenSubmenu(submenu) => {
            // Refresh the job-emote/chair unlock bits whenever the Emote List
            // opens (c2s 0x119 → s2c 0x11A gates the Job row).
            if submenu == MenuKind::EmoteList {
                let _ = cmd_tx.try_send(AgentCommand::RequestEmoteList);
            }
            // The Map screen opens on its command submenu; the wide-scan request
            // (0x0F4) fires only when the player selects "Wide Scan", not on open.
            // `reset_map_screen_on_open` clears the submode as the screen appears.
            stack.push(submenu);
            None
        }
        MenuDispatch::KeybindUpdate(update) => {
            let stay = matches!(update, KeybindUpdate::List);
            apply_keybind_update(update, bindings, keybinds_state, scene_state);
            if stay {
                None
            } else {
                Some(InputMode::World)
            }
        }
        MenuDispatch::NotImplemented(label) => {
            push_system_chat_line(scene_state, format!("[menu] {label} — not implemented"));
            None
        }
    }
}

fn activate_current_time(
    vana_clock: &ffxi_viewer_core::vana_time::VanaClock,
    vana_clock_visible: &mut ffxi_viewer_core::hud::vana_clock::VanaClockVisible,
    scene_state: &mut SceneState,
) {
    vana_clock_visible.0 = !vana_clock_visible.0;
    for line in ffxi_viewer_core::hud::vana_clock::current_time_chat_lines(vana_clock) {
        push_system_chat_line(scene_state, line);
    }
}

fn toggle_debug_panel(
    label: &str,
    hud_panels: &mut ffxi_viewer_core::hud::HudPanels,
    net_status: &mut ffxi_viewer_core::hud::network_status::NetStatusVisible,
    scene_state: &mut SceneState,
) {
    use ffxi_viewer_core::hud::menu::{
        DEBUG_MESH, DEBUG_NET_STATUS, DEBUG_PERF, DEBUG_TARGET_CYCLE,
    };
    let on = match label {
        DEBUG_PERF => {
            hud_panels.perf = !hud_panels.perf;
            hud_panels.perf
        }
        DEBUG_TARGET_CYCLE => {
            hud_panels.target_cycle = !hud_panels.target_cycle;
            hud_panels.target_cycle
        }
        DEBUG_MESH => {
            hud_panels.mesh_debug = !hud_panels.mesh_debug;
            hud_panels.mesh_debug
        }
        DEBUG_NET_STATUS => {
            net_status.0 = !net_status.0;
            net_status.0
        }
        other => {
            push_system_chat_line(scene_state, format!("[menu] Debug: unknown `{other}`"));
            return;
        }
    };
    push_system_chat_line(
        scene_state,
        format!("[menu] {label}: {}", if on { "on" } else { "off" }),
    );
}

pub(super) fn handle_menu_key(
    key: &Key,
    key_code: KeyCode,
    bindings: &mut Bindings,
    stack: &mut MenuStack,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    keybinds_state: &mut KeybindsStateRes,
    graphics: &mut ffxi_viewer_core::GraphicsSettings,
    status_profile_open: &mut ffxi_viewer_core::hud::status_panel::StatusProfileOpen,
    hud_panels: &mut ffxi_viewer_core::hud::HudPanels,
    net_status: &mut ffxi_viewer_core::hud::network_status::NetStatusVisible,
    vana_clock: &ffxi_viewer_core::vana_time::VanaClock,
    vana_clock_visible: &mut ffxi_viewer_core::hud::vana_clock::VanaClockVisible,
    sort_options: &mut ffxi_viewer_core::hud::item_detail::SortOptions,
    item_menu_focus: &mut ffxi_viewer_core::hud::item_detail::ItemMenuFocus,
    item_bag: &mut ffxi_viewer_core::hud::item_screen::ItemScreenContainer,
    dynamic: &ffxi_viewer_core::hud::menu::DynamicMenu,
    target_id: Option<u32>,
    self_pos: ffxi_viewer_wire::Vec3,
    map_state: &mut ffxi_viewer_core::hud::map_screen::MapScreenState,
    map_markers: &mut ffxi_viewer_core::hud::map_screen::MapMarkers,
    map_view: &ffxi_viewer_core::hud::map_screen::MapView,
    minimap_state: &ffxi_viewer_core::minimap::MinimapState,
) -> Option<InputMode> {
    let top_kind = stack.current()?.kind;

    // The very key that opened this menu (Action::OpenMenu on "-", handled by the
    // input system chained just before this one) also arrives here on the same
    // frame; absorb it once so it doesn't immediately flip Root to page 2
    // (kuluu-bi1s.2). Clear the one-shot flag on the first menu key regardless.
    if stack.take_absorb_open_minus() && key_code == KeyCode::Minus {
        return None;
    }

    // The Map screen is a bespoke full-screen surface (full-screen map + a
    // top-right command submenu drilling into Markers/Wide Scan/Change Map),
    // with its own submode navigation, so it intercepts before generic routing.
    if top_kind == MenuKind::Map {
        return handle_map_key(
            key,
            bindings,
            stack,
            scene_state,
            cmd_tx,
            map_state,
            map_markers,
            map_view,
            minimap_state,
        );
    }
    let (kind, cursor) = {
        let level = stack.current()?;
        (level.kind, level.cursor)
    };
    let entry_count = ffxi_viewer_core::hud::menu::entry_count(kind, dynamic);

    // Menu context (not text input), so reading the raw keycode is correct.
    // "-" flips the Command menu's two pages (retail HorizonXI); single-list
    // submenus have no pages, and the Map screen handles "-" in its own path.
    if key_code == KeyCode::Minus {
        if kind == MenuKind::Root {
            if let Some(level) = stack.current_mut() {
                level.cursor = ffxi_viewer_core::hud::menu::root_other_page_cursor(level.cursor);
            }
        }
        return None;
    }

    // Root Command menu paging: Left/Right flip pages (like "-"); Up/Down wrap
    // within the current page so navigation never crosses a page boundary.
    if kind == MenuKind::Root {
        let (start, end) = ffxi_viewer_core::hud::menu::root_page_bounds(cursor);
        if bindings.matches_logical(Action::NavLeft, key)
            || bindings.matches_logical(Action::NavRight, key)
        {
            if let Some(level) = stack.current_mut() {
                level.cursor = ffxi_viewer_core::hud::menu::root_other_page_cursor(level.cursor);
            }
            return None;
        }
        if bindings.matches_logical(Action::NavUp, key) {
            let level = stack.current_mut()?;
            level.cursor = if cursor <= start { end - 1 } else { cursor - 1 };
            return None;
        }
        if bindings.matches_logical(Action::NavDown, key) {
            let level = stack.current_mut()?;
            let next = cursor + 1;
            level.cursor = if next >= end { start } else { next };
            return None;
        }
    }

    // The Items window is a stack of panes: one per accessible bag plus the
    // sort-options box. Retail's "Select active window" key (F in the compact
    // presets, Numpad + on the full keyboard) steps focus through them in
    // order, while NavLeft/NavRight page the item list a viewport at a time —
    // matching the retail client, which never repurposes left/right for pane
    // changes.
    if matches!(kind, MenuKind::Items) {
        use ffxi_viewer_core::hud::item_detail::{sort_pane_key, SortPaneKey};
        if bindings.matches_logical(Action::SelectActiveWindow, key) {
            if ffxi_viewer_core::hud::item_screen::select_active_window(
                &scene_state.snapshot,
                item_bag,
                item_menu_focus,
                sort_options,
            )
            .is_some()
            {
                if let Some(level) = stack.current_mut() {
                    level.cursor = 0;
                }
            }
            return None;
        }
        if item_menu_focus.sort_focused() {
            let pane_key = if bindings.matches_logical(Action::NavUp, key) {
                SortPaneKey::Up
            } else if bindings.matches_logical(Action::NavDown, key) {
                SortPaneKey::Down
            } else if bindings.matches_logical(Action::NavConfirm, key) {
                SortPaneKey::Confirm
            } else if bindings.matches_logical(Action::NavLeft, key)
                || bindings.matches_logical(Action::NavCancel, key)
            {
                SortPaneKey::Exit
            } else {
                // Swallow any other key so it can't leak into list navigation.
                SortPaneKey::Other
            };
            if sort_pane_key(item_menu_focus, sort_options, pane_key).is_some() {
                if let Err(e) = cmd_tx.try_send(AgentCommand::StackInventory {
                    container: ffxi_proto::map::container::LOC_INVENTORY,
                }) {
                    push_system_chat_line(scene_state, format!("sort dropped (channel): {e}"));
                }
            }
            return None;
        }
        // Retail pages the item list with left/right: one viewport per press,
        // clamped at the ends (no wrap).
        let page = if bindings.matches_logical(Action::NavLeft, key) {
            Some(false)
        } else if bindings.matches_logical(Action::NavRight, key) {
            Some(true)
        } else {
            None
        };
        if let Some(forward) = page {
            let rows = ffxi_viewer_core::hud::menu::list_page_rows(kind);
            if let Some(level) = stack.current_mut() {
                level.cursor = ffxi_viewer_core::hud::menu::page_cursor(
                    level.cursor,
                    entry_count,
                    rows,
                    forward,
                );
            }
            return None;
        }
    }

    if matches!(kind, MenuKind::Graphics) {
        if bindings.matches_logical(Action::NavLeft, key) {
            apply_graphics_cycle(cursor, -1, graphics);
            return None;
        }
        if bindings.matches_logical(Action::NavRight, key) {
            apply_graphics_cycle(cursor, 1, graphics);
            return None;
        }
    }

    // The Equipment screen is a 2D retail icon grid: arrows move between grid
    // cells (cursor stays an internal slot index), not down a linear list.
    if matches!(kind, MenuKind::Equipment) {
        let delta = if bindings.matches_logical(Action::NavLeft, key) {
            Some((-1, 0))
        } else if bindings.matches_logical(Action::NavRight, key) {
            Some((1, 0))
        } else if bindings.matches_logical(Action::NavUp, key) {
            Some((0, -1))
        } else if bindings.matches_logical(Action::NavDown, key) {
            Some((0, 1))
        } else {
            None
        };
        if let Some((dx, dy)) = delta {
            let level = stack.current_mut()?;
            level.cursor =
                ffxi_viewer_core::hud::equipment_screen::grid_move(level.cursor as u8, dx, dy)
                    as usize;
            return None;
        }
    }

    // Retail pages every other vertical list menu (the action-ring Usable list,
    // Magic/Abilities/Key Items/Emotes, the equip-slot picker) with Left/Right,
    // one visible page per press, clamped at the ends. Items handled its own
    // paging above; Graphics (value cycles) and the Equipment grid consumed
    // Left/Right in their blocks.
    if ffxi_viewer_core::hud::menu::is_dynamic(kind) {
        let page = if bindings.matches_logical(Action::NavLeft, key) {
            Some(false)
        } else if bindings.matches_logical(Action::NavRight, key) {
            Some(true)
        } else {
            None
        };
        if let Some(forward) = page {
            let rows = ffxi_viewer_core::hud::menu::list_page_rows(kind);
            let level = stack.active_level_mut()?;
            level.cursor =
                ffxi_viewer_core::hud::menu::page_cursor(level.cursor, entry_count, rows, forward);
            return None;
        }
    }

    if bindings.matches_logical(Action::NavUp, key) {
        let level = stack.active_level_mut()?;
        level.cursor = if cursor == 0 {
            entry_count.saturating_sub(1)
        } else {
            cursor - 1
        };
        return None;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        let level = stack.active_level_mut()?;
        let next = cursor + 1;
        level.cursor = if next >= entry_count { 0 } else { next };
        return None;
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        return confirm_menu_at_cursor(
            bindings,
            stack,
            scene_state,
            cmd_tx,
            keybinds_state,
            graphics,
            status_profile_open,
            hud_panels,
            net_status,
            vana_clock,
            vana_clock_visible,
            dynamic,
            target_id,
            self_pos,
        );
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        if matches!(kind, MenuKind::Status) {
            status_profile_open.0 = false;
        }
        // Cancel pops one level; from Root it closes back to the world.
        return if !stack.pop() {
            Some(InputMode::World)
        } else {
            None
        };
    }
    None
}

#[cfg(test)]
mod menu_dispatch_tests {
    use super::*;

    #[test]
    fn log_out_dispatches_reqlogout_with_toast() {
        use ffxi_viewer_core::hud::menu::ROOT_LOG_OUT;
        match resolve_menu_entry(MenuKind::Root, ROOT_LOG_OUT) {
            MenuDispatch::CommandWithToast { cmd, toast } => {
                assert_eq!(
                    cmd,
                    AgentCommand::ReqLogout {
                        kind: ReqLogoutKind::LogoutToggle,
                    }
                );
                assert!(
                    toast.to_lowercase().contains("log out"),
                    "toast should mention log out, got {toast:?}"
                );
            }
            other => panic!("expected CommandWithToast for Log Out, got {other:?}"),
        }
    }

    #[test]
    fn shut_down_dispatches_shutdown_reqlogout_with_toast() {
        use ffxi_viewer_core::hud::menu::ROOT_SHUT_DOWN;
        match resolve_menu_entry(MenuKind::Root, ROOT_SHUT_DOWN) {
            MenuDispatch::CommandWithToast { cmd, toast } => {
                assert_eq!(
                    cmd,
                    AgentCommand::ReqLogout {
                        kind: ReqLogoutKind::ShutdownToggle,
                    }
                );
                assert!(
                    toast.to_lowercase().contains("shut down"),
                    "toast should mention shut down, got {toast:?}"
                );
            }
            other => panic!("expected CommandWithToast for Shut Down, got {other:?}"),
        }
    }

    #[test]
    fn unwired_root_entries_stay_not_implemented() {
        for label in ["Party", "Search", "Macros"] {
            assert_eq!(
                resolve_menu_entry(MenuKind::Root, label),
                MenuDispatch::NotImplemented(label.into()),
                "{label} should still be a stub"
            );
        }
    }

    /// The right-pane preview (`menu::root_child_kind`) and the drill dispatch
    /// share one Root → submenu mapping; pin that they can't drift apart.
    #[test]
    fn root_drill_matches_preview_child_kind() {
        use ffxi_viewer_core::hud::menu::{self, ROOT_LOG_OUT, ROOT_SHUT_DOWN};
        for &label in menu::root_entries() {
            // Log Out / Shut Down fire commands, not a browsable submenu.
            if label == ROOT_LOG_OUT || label == ROOT_SHUT_DOWN {
                continue;
            }
            match (
                resolve_menu_entry(MenuKind::Root, label),
                menu::root_child_kind(label),
            ) {
                (MenuDispatch::OpenSubmenu(dispatched), Some(preview)) => {
                    assert_eq!(dispatched, preview, "{label} drill vs preview drift");
                }
                // A drill with no right-pane preview is only legal when it opens
                // a bespoke full-screen menu (e.g. Map), which renders its own
                // panes instead of the generic preview.
                (MenuDispatch::OpenSubmenu(dispatched), None) => {
                    assert!(
                        menu::renders_bespoke_screen(dispatched),
                        "{label}: preview-less drill into non-bespoke {dispatched:?}"
                    );
                }
                (MenuDispatch::NotImplemented(_), None) => {}
                (dispatch, preview) => {
                    panic!("{label}: dispatch {dispatch:?} disagrees with preview {preview:?}")
                }
            }
        }
    }

    #[test]
    fn current_time_toggles_widget_and_prints_both_time_lines() {
        use ffxi_viewer_core::hud::vana_clock::{
            VanaClockVisible, EARTH_TIME_LINE_PREFIX, VANA_TIME_LINE_PREFIX,
        };
        let clock = ffxi_viewer_core::vana_time::VanaClock::anchored_at_hour(12.0);
        let mut visible = VanaClockVisible::default();
        let mut scene_state = SceneState::default();

        activate_current_time(&clock, &mut visible, &mut scene_state);
        assert!(
            !visible.0,
            "default-visible widget hides on first activation"
        );
        let lines: Vec<&str> = scene_state
            .local_toasts
            .iter()
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with(VANA_TIME_LINE_PREFIX), "{lines:?}");
        assert!(lines[1].starts_with(EARTH_TIME_LINE_PREFIX), "{lines:?}");

        activate_current_time(&clock, &mut visible, &mut scene_state);
        assert!(visible.0, "second activation shows the widget again");
    }

    #[test]
    fn current_time_never_reaches_resolve_wired() {
        // confirm_menu_at_cursor intercepts ROOT_CURRENT_TIME (a
        // resource-touching entry) before resolve_menu_entry; this pins the
        // fallback so a lost wiring degrades to a visible "not implemented"
        // chat line rather than silently dispatching something else.
        use ffxi_viewer_core::hud::menu::ROOT_CURRENT_TIME;
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, ROOT_CURRENT_TIME),
            MenuDispatch::NotImplemented(ROOT_CURRENT_TIME.into()),
        );
    }

    /// Send-panel shape: choice 0 = recipient row (above the grid), choices
    /// 1..=8 = the 2x4 slot grid, choice 9 = Cancel (below the grid).
    fn send_panel_grid() -> ffxi_viewer_wire::DialogGrid {
        ffxi_viewer_wire::DialogGrid {
            cols: 4,
            rows: 2,
            cells: (0..8u32)
                .map(|i| ffxi_viewer_wire::DialogGridCell {
                    choice: Some(i + 1),
                    ..Default::default()
                })
                .collect(),
        }
    }

    #[test]
    fn grid_nav_walks_cells_spatially() {
        let g = send_panel_grid();
        // Right along the top row; clamped at the edge.
        assert_eq!(grid_nav_choice(&g, 9, 1, 1, 0), 2);
        assert_eq!(grid_nav_choice(&g, 9, 4, 1, 0), 4);
        // Down keeps the column; up returns.
        assert_eq!(grid_nav_choice(&g, 9, 2, 0, 1), 6);
        assert_eq!(grid_nav_choice(&g, 9, 6, 0, -1), 2);
    }

    #[test]
    fn grid_nav_bridges_flat_rows_above_and_below() {
        let g = send_panel_grid();
        // Up off the top row lands on the recipient row (choice 0)…
        assert_eq!(grid_nav_choice(&g, 9, 3, 0, -1), 0);
        // …and down from it re-enters the grid.
        assert_eq!(grid_nav_choice(&g, 9, 0, 0, 1), 1);
        // Down off the bottom row lands on Cancel (choice 9)…
        assert_eq!(grid_nav_choice(&g, 9, 7, 0, 1), 9);
        // …and up from Cancel re-enters the grid's bottom row.
        assert_eq!(grid_nav_choice(&g, 9, 9, 0, -1), 5);
        // Left/right on flat rows do nothing.
        assert_eq!(grid_nav_choice(&g, 9, 0, 1, 0), 0);
        assert_eq!(grid_nav_choice(&g, 9, 9, -1, 0), 9);
    }

    #[test]
    fn grid_nav_skips_inert_cells() {
        // Incoming-box shape: only slots 0 and 6 occupied, no flat rows
        // besides the trailing Cancel (choice 2).
        let mut g = send_panel_grid();
        for (i, cell) in g.cells.iter_mut().enumerate() {
            cell.choice = match i {
                0 => Some(0),
                6 => Some(1),
                _ => None,
            };
        }
        // Down from (0,0) reaches (2,1) — nearest selectable on the next row.
        assert_eq!(grid_nav_choice(&g, 2, 0, 0, 1), 1);
        // Right from (0,0) has no selectable neighbor on that row.
        assert_eq!(grid_nav_choice(&g, 2, 0, 1, 0), 0);
        // Down off the bottom row hits Cancel; up from Cancel returns.
        assert_eq!(grid_nav_choice(&g, 2, 1, 0, 1), 2);
        assert_eq!(grid_nav_choice(&g, 2, 2, 0, -1), 1);
    }

    #[test]
    fn self_only_actions_skip_sub_target() {
        use ffxi_viewer_core::hud::menu::DynamicMenuAction as A;
        use ffxi_viewer_core::input_mode::SubTargetAction as S;
        // Boost (ability 39, validTarget SELF) casts on <me> — no <st> prompt.
        assert_eq!(
            sub_target_action_for(A::JobAbility { ability_id: 39 }),
            None
        );
        // Provoke (ability 35, ENEMY) still opens the sub-target cursor.
        assert_eq!(
            sub_target_action_for(A::JobAbility { ability_id: 35 }),
            Some(S::Ability(35))
        );
        // Cure (spell 1, PARTY) still prompts.
        assert_eq!(
            sub_target_action_for(A::CastSpell { spell_id: 1 }),
            Some(S::Spell(1))
        );
    }
}
