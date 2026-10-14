use super::map_screen::handle_map_key;
use super::slash_apply::{apply_keybind_update, apply_menu_text_command};
use super::*;

#[derive(Debug, Clone, PartialEq)]
enum MenuDispatch {
    OpenSubmenu(MenuKind),

    /// Retail rows that "run" a command hand the runner their literal text -
    /// FFXiMain.dll retail-2026-09 RVA 0x80DC0 is the runner, and page 2's
    /// Current Time case at RVA 0x14E98C pushes its own string (.data 0x371AB8
    /// "/clock"). The same seam serves typed input later (vault §12).
    TextCommand(&'static str),

    /// myroom's Gardening/Layout rows (select ids 4 and 5 of handler FFXiMain.dll
    /// retail-2026-09 RVA 0x145460 — arms @0x145541/@0x14554A): each runs its
    /// literal through the runner (0x80DC0) and then closes myroom by name via
    /// 0x15E910, so kuluu pops the home-view level after running it (vault
    /// main_menu.md §53; the pair's literal↔id mapping was swapped in §40).
    HomeTextCommand(&'static str),

    /// A container row of the myroom panel: opens kuluu's Items window on the
    /// LSB bag named by `container` (vault main_menu.md §53 container law) —
    /// the same seam a Mog Menu storage row uses (`open_items_on_bag`).
    BrowseBag {
        container: u8,
    },

    KeybindUpdate(KeybindUpdate),

    /// View House (page-2 row 4): the retail arm opens the Mog Menu locally —
    /// FFXiMain.dll retail-2026-09 RVA 0x14E8F5 name-opens `menu    mogcont`
    /// with no packet on the wire; kuluu's built Mog Menu is the session dialog
    /// surface fed by s2c 0x02E OPENMOGMENU (vault main_menu.md §32), so this
    /// routes through the same `AgentCommand::OpenMogMenu` seam.
    OpenMogMenu,

    NotImplemented(String),
}

/// Header and action rows carry no value, so a cursor on one cycles nothing.
fn apply_settings_cycle(
    kind: MenuKind,
    cursor: usize,
    delta: i32,
    graphics: &mut kuluu_render::GraphicsSettings,
) {
    if let Some(field) =
        kuluu_render::hud::menu::settings_field_at(kind, cursor, graphics.dlss_supported)
    {
        graphics.cycle(field, delta);
    }
}

/// `flat_magic` is GraphicsSettings.magic_flat_list (Retail+ "Flat Magic List"); it
/// feeds the Root → submenu mapping so the drill and preview stay identical.
/// `flat_abilities` is the same pattern for GraphicsSettings.abilities_flat_list
/// and the Abilities row (vault main_menu.md §23).
fn resolve_menu_entry(
    kind: MenuKind,
    label: &str,
    flat_magic: bool,
    flat_abilities: bool,
) -> MenuDispatch {
    use kuluu_render::graphics_settings::MenuAction;
    use kuluu_render::hud::menu::{
        magic_category_for_label, COMM_EMOTE_LIST, COMM_FRIEND_LIST, MACRO_ROW_MACRO_RESULT,
        MACRO_ROW_SELECT_MACRO, ROOT_CURRENT_TIME, ROOT_LOG_OUT, ROOT_SHUT_DOWN, ROOT_VIEW_HOUSE,
        SYNTH_ROW_OPEN_LIST, SYNTH_ROW_PICK_MATERIAL,
    };
    // The Config family resolves through no labels at all: every window's rows are DAT
    // widgets read at runtime (bead kuluu-o9qt7.33; vault main_menu.md §43/§49/§57), and each
    // row carries its select semantics as a DynamicMenuAction — the hub's twelve jump-table
    // drills (§42), `conf5m`'s log-window arms 0x199264/0x19927A (§47), or an option-row store
    // id (§48). Those actions ride on confirm_menu_at_cursor's dynamic seam.
    match (kind, label) {
        (MenuKind::Config, l) if l == MenuAction::Controls.label() => {
            MenuDispatch::OpenSubmenu(MenuKind::Controls)
        }
        (MenuKind::Communication, l) if l == COMM_EMOTE_LIST => {
            MenuDispatch::OpenSubmenu(MenuKind::EmoteList)
        }
        // commenu's other list widget (DAT windowps330, caption token 61;
        // vault main_menu.md §37): drills the friend-list window, whose rows
        // arrive with the local store in `.23.1`.
        (MenuKind::Communication, l) if l == COMM_FRIEND_LIST => {
            MenuDispatch::OpenSubmenu(MenuKind::FriendList)
        }
        // Both power rows open the retail yes/no prompt first (Vault-grounded
        // reading at FFXiMain.dll retail-2026-09 RVA 0x160960 / 0x1609F0); the
        // request goes out only on Yes.
        (MenuKind::Root, ROOT_LOG_OUT) => {
            MenuDispatch::OpenSubmenu(MenuKind::PowerPrompt { shut_down: false })
        }
        (MenuKind::Root, ROOT_SHUT_DOWN) => {
            MenuDispatch::OpenSubmenu(MenuKind::PowerPrompt { shut_down: true })
        }

        // Page 2 row 10 runs the text command "/clock" through the runner
        // (FFXiMain.dll retail-2026-09 RVA 0x14E98C, literal .data 0x371AB8;
        // registry record /clock = id 0x3F in vault §12). Client-side: LSB
        // has no handler and never sees it on the wire.
        (MenuKind::Root, l) if l == ROOT_CURRENT_TIME => MenuDispatch::TextCommand("/clock"),

        // Page 2 row 3 runs the text command "/keyitem" through the same runner
        // (FFXiMain.dll retail-2026-09 RVA 0x14E855, literal .data 0x371B40;
        // registry record /keyitem = id 0x4C in vault §12). Client-side: LSB has
        // no handler and never sees it on the wire.
        (MenuKind::Root, "Key Items") => MenuDispatch::TextCommand("/keyitem"),

        // Page 2 row 2 runs the text command "/quest" through the same runner
        // (FFXiMain.dll retail-2026-09 RVA 0x14E889, literal .data 0x371B24;
        // registry record /quest = id 0x4D, whose table handler 0x7D240 resets
        // the quest-window filter word and name-opens menu quest00 — vault
        // main_menu.md §31). Client-side: LSB has no handler and never sees it.
        (MenuKind::Root, "Quests") => MenuDispatch::TextCommand("/quest"),

        // Page-2 row 4 name-opens `menu    mogcont` — the Mog Menu — after the
        // arm's veto predicate passes (FFXiMain.dll retail-2026-09 RVA 0x14E8F5;
        // vault main_menu.md §32). The gate itself runs at confirm time because
        // it reads snapshot state, like the Trade row's target check.
        (MenuKind::Root, ROOT_VIEW_HOUSE) => MenuDispatch::OpenMogMenu,

        // Page-1 row 11 name-opens `menu    region` with no gate (FFXiMain.dll
        // retail-2026-09 RVA 0x140691; vault main_menu.md §29): kuluu drills its
        // four-mode panel, opened at display mode 0.
        (MenuKind::Root, "Region Info") => {
            MenuDispatch::OpenSubmenu(MenuKind::RegionInfo { mode: 0 })
        }

        // Page-2 row 1 name-opens `menu    missionm` under the game-object select
        // gate (FFXiMain.dll retail-2026-09 RVA 0x14E86D; vault main_menu.md §30).
        (MenuKind::Root, "Missions") => MenuDispatch::OpenSubmenu(MenuKind::MissionList),

        // Page-2 row 5 name-opens `menu    bazaar` with no gate (FFXiMain.dll
        // retail-2026-09 RVA 0x14E8A1; vault main_menu.md §33): kuluu drills the
        // stall picker built from entity BazaarFlag rows.
        (MenuKind::Root, "Bazaar") => MenuDispatch::OpenSubmenu(MenuKind::BazaarStalls),

        // The Map screen is a bespoke pane (no generic right-pane preview via
        // root_child_kind), so it needs its own drill arm ahead of the catch-all.
        (MenuKind::Root, "Map") => MenuDispatch::OpenSubmenu(MenuKind::Map),

        // Root categories that drill into a browsable submenu share their
        // mapping with the right-pane preview (single source of truth).
        (MenuKind::Root, label) => {
            match kuluu_render::hud::menu::root_child_kind(label, flat_magic, flat_abilities) {
                Some(submenu) => MenuDispatch::OpenSubmenu(submenu),
                None => MenuDispatch::NotImplemented(label.to_string()),
            }
        }

        // The cmbmenu launcher's rows (handler FFXiMain.dll retail-2026-09 RVA 0x1E29A0; vault
        // main_menu.md §22/§28): select id 1 opens the bag, select id 2 drills kuluu's craft list
        // in place of the cmbHwin/cmbhlst window pair.
        (MenuKind::SynthLauncher, l) if l == SYNTH_ROW_PICK_MATERIAL => {
            MenuDispatch::OpenSubmenu(MenuKind::Items)
        }
        (MenuKind::SynthLauncher, l) if l == SYNTH_ROW_OPEN_LIST => {
            MenuDispatch::OpenSubmenu(MenuKind::CraftList)
        }

        // The mcrmenu launcher rows drill the §41 windows built by bead .19.1
        // (handler FFXiMain.dll retail-2026-09 RVA 0x1A64B0 items {1,2}; vault
        // main_menu.md §34/§41): Select Macro opens the store-backed mcrselec,
        // Macro Result the grounded-empty mcres20 surface.
        (MenuKind::MacrosPalette, l) if l == MACRO_ROW_SELECT_MACRO => {
            MenuDispatch::OpenSubmenu(MenuKind::MacroSelect)
        }
        (MenuKind::MacrosPalette, l) if l == MACRO_ROW_MACRO_RESULT => {
            MenuDispatch::OpenSubmenu(MenuKind::MacroResult)
        }

        // myroom's select family (handler FFXiMain.dll retail-2026-09 RVA
        // 0x145460, ids 1..7): the six container rows open their LSB bags, the
        // two text-command rows run their literals and close the panel, Change
        // Jobs name-opens `jobchang` (id3 arm @0x145517 — its predicate only
        // selects an extra helper call 0xA1860(50); BOTH paths open the window,
        // so kuluu opens it ungated). The rows whose retail arms are not yet
        // built (Delivery Box→`mogpost`, Remodel→`roomlist`, Open Mog House)
        // answer NotImplemented until their beads land (vault §53).
        (MenuKind::HomeView, _) => {
            if let Some(container) = kuluu_render::hud::menu::home_view_row_container(label) {
                MenuDispatch::BrowseBag { container }
            } else if let Some(cmd) = kuluu_render::hud::menu::home_view_text_command(label) {
                MenuDispatch::HomeTextCommand(cmd)
            } else if label == "Change Jobs" {
                MenuDispatch::OpenSubmenu(MenuKind::JobChange)
            } else {
                MenuDispatch::NotImplemented(label.to_string())
            }
        }

        // `jobchang`'s two rows (DAT 'jobc' widgets windowps273/274, catalog
        // tokens 65/66) drill their pick lists — Main Job and Support Job each
        // send one c2s 0x100 MYROOM_JOB on select (vault main_menu.md §53).
        (MenuKind::JobChange, l) if l == kuluu_render::hud::menu::JOB_CHANGE_ROWS[0] => {
            MenuDispatch::OpenSubmenu(MenuKind::JobPick { main_pick: true })
        }
        (MenuKind::JobChange, l) if l == kuluu_render::hud::menu::JOB_CHANGE_ROWS[1] => {
            MenuDispatch::OpenSubmenu(MenuKind::JobPick { main_pick: false })
        }

        // mgcmenu select: the category's school id (jump table 0x13F7CC; vault
        // main_menu.md §21) becomes the filter of the `magic` spell list.
        (MenuKind::MagicGroups, label) => match magic_category_for_label(label) {
            Some(school) => MenuDispatch::OpenSubmenu(MenuKind::MagicGroup(school)),
            None => MenuDispatch::NotImplemented(label.to_string()),
        },

        // abimenu select: the category row becomes the `ability` window's group
        // filter (FFXiMain.dll retail-2026-09 RVA 0x13FA70; vault main_menu.md §23).
        (MenuKind::AbilitiesGroups, label) => {
            match kuluu_render::hud::menu::abilities_category_index(label) {
                Some(idx) => MenuDispatch::OpenSubmenu(MenuKind::AbilitiesGroup(idx as u8)),
                None => MenuDispatch::NotImplemented(label.to_string()),
            }
        }

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

        (MenuKind::Controls, "Standard") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Preset(Preset::Standard))
        }
        (MenuKind::Controls, "Compact 1") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Preset(Preset::Compact1))
        }
        (MenuKind::Controls, "Compact 2") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Preset(Preset::Compact2))
        }
        (MenuKind::Controls, "Reset to defaults") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::Reset)
        }
        (MenuKind::Controls, "Show current bindings") => {
            MenuDispatch::KeybindUpdate(KeybindUpdate::List)
        }
        (_, other) => MenuDispatch::NotImplemented(other.to_string()),
    }
}

const EQUIP_SLOT_INDEX_MAX: u8 = (kuluu_render::equip_slot::EquipmentIndex::ALL.len() - 1) as u8;

/// Confirm the entry under the cursor. Retail lists Use for every item and
/// refuses an unusable or cooling-down one silently: no sub-target cursor, no
/// chat line, the submenu stays put. The item drop confirm unwinds both the
/// confirm and the Item submenu back to the list on either answer.
pub(super) fn confirm_menu_at_cursor(
    bindings: &mut Bindings,
    stack: &mut MenuStack,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    keybinds_state: &mut KeybindsStateRes,
    graphics: &mut kuluu_render::GraphicsSettings,
    status_profile_open: &mut kuluu_render::hud::status_panel::StatusProfileOpen,
    hud_panels: &mut kuluu_render::hud::HudPanels,
    net_status: &mut kuluu_render::hud::network_status::NetStatusVisible,
    audio_mute: &mut kuluu_render::audio::AudioMuteState,
    vana_clock: &mut kuluu_render::vana_time::VanaClock,
    vana_clock_visible: &mut kuluu_render::hud::vana_clock::VanaClockVisible,
    dynamic: &kuluu_render::hud::menu::DynamicMenu,
    target_id: Option<u32>,
    self_pos: kuluu_snapshot::Vec3,
    command_surface: &crate::view_native::command_surface::CommandSurface,
    item_bag: &mut kuluu_render::hud::item_screen::ItemScreenContainer,
) -> Option<InputMode> {
    let (kind, cursor) = {
        let level = stack.current()?;
        (level.kind, level.cursor)
    };

    if matches!(kind, MenuKind::Debug) {
        let label = kuluu_render::hud::menu::entry_label(kind, cursor, dynamic);
        // Volume is a 0..=100 number row adjusted with Left/Right, not a toggle.
        // Pressing the confirm key on it should do nothing rather than fall
        // through to toggle_debug_panel's unknown-entry branch.
        if label == kuluu_render::hud::menu::DEBUG_VOLUME {
            return None;
        }
        if handle_retail_plus_row(label, graphics, scene_state) {
            return None;
        }
        toggle_debug_panel(
            label,
            hud_panels,
            net_status,
            audio_mute,
            vana_clock,
            self_pos,
            scene_state,
        );
        return None;
    }

    if matches!(kind, MenuKind::Status) {
        // Retail's persona answers no command events (FFXiMain.dll retail-2026-09 RVA 0x12AC60
        // is a ret-8 stub; vault main_menu.md §11.1): default is the passive panel, and kuluu's
        // actionable rows live behind the Retail+ "Status Rows" toggle.
        if !graphics.interactive_status_rows {
            return None;
        }
        use kuluu_render::hud::status_panel::{StatusEntryKind, STATUS_ENTRIES};
        let entry = STATUS_ENTRIES.get(cursor)?;
        match entry.kind {
            StatusEntryKind::Profile => {
                status_profile_open.0 = true;
            }
            StatusEntryKind::PlayTime => {
                let line =
                    kuluu_render::hud::status_panel::play_time_chat_line(&scene_state.snapshot);
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
    if matches!(
        kind,
        MenuKind::Config | MenuKind::Graphics | MenuKind::GraphicsDlss
    ) {
        use kuluu_render::graphics_settings::{MenuAction, MenuRow};
        match kuluu_render::hud::menu::settings_row_at(kind, cursor, graphics.dlss_supported) {
            Some(MenuRow::Action(MenuAction::ResetToMinimum)) => {
                graphics.reset_to_minimum();
                push_system_chat_line(scene_state, "[menu] Graphics reset to Minimum".into());
            }
            Some(MenuRow::Action(MenuAction::ResetDlssConfig)) => {
                graphics.reset_dlss_config();
                push_system_chat_line(scene_state, "[menu] DLSS config reset to defaults".into());
            }
            Some(MenuRow::Action(MenuAction::DlssConfig)) => stack.push(MenuKind::GraphicsDlss),
            Some(MenuRow::Action(MenuAction::Controls)) => stack.push(MenuKind::Controls),
            _ => apply_settings_cycle(kind, cursor, 1, graphics),
        }
        return None;
    }

    if matches!(kind, MenuKind::Equipment) {
        let slot = (cursor as u8).min(EQUIP_SLOT_INDEX_MAX);
        stack.push(MenuKind::EquipSlot(slot));
        return None;
    }

    if kuluu_render::hud::menu::is_dynamic(kind) {
        if let Some(action) = kuluu_render::hud::menu::entry_action(kind, cursor, dynamic) {
            use kuluu_render::hud::menu::DynamicMenuAction as A;
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
            // The trade window's own-slot rows (FFXiMain.dll retail-2026-09
            // handler 0x1515E0 gates every select on state==3; vault
            // main_menu.md §25): a filled slot releases via the qty-0 TRADE_LIST
            // path, an empty one opens that slot's Add-Item picker.
            if let A::TradeOwnSlot { trade_index } = action {
                let started = scene_state
                    .snapshot
                    .trade
                    .as_deref()
                    .is_some_and(|t| !t.incoming_pending && t.active);
                if !started {
                    return None;
                }
                let filled = scene_state
                    .snapshot
                    .trade
                    .as_deref()
                    .and_then(|t| t.my_slots.get(trade_index as usize).copied().flatten())
                    .is_some();
                if filled {
                    if let Err(e) = cmd_tx.try_send(AgentCommand::TradeClearSlot { trade_index }) {
                        push_system_chat_line(
                            scene_state,
                            format!("[menu] trade release dropped: {e}"),
                        );
                    }
                } else {
                    stack.push(MenuKind::TradePick { trade_index });
                }
                return None;
            }
            // An Add-Item pick stages ONE unit into the slot — retail's qty
            // keypad is [I-pending] (vault main_menu.md §25) and 1 is the
            // minimum legal ItemNum under player_trade.cpp setSlot. Like
            // retail's re-pushing views, the picker stays open for more picks.
            if let A::TradeStageFromBag {
                trade_index,
                item_no,
                qty,
                bag_slot,
            } = action
            {
                if let Err(e) = cmd_tx.try_send(AgentCommand::TradeStage {
                    trade_index,
                    item_no,
                    qty,
                    bag_slot,
                }) {
                    push_system_chat_line(scene_state, format!("[menu] trade stage dropped: {e}"));
                }
                return None;
            }
            // The region-info window answers a select by installing the display table on
            // itself — FFXiMain.dll retail-2026-09 RVA 0x140780 tail-jumps 0x1E7980 rather than
            // opening anything (vault main_menu.md §29) — so kuluu swaps the payload in place.
            if let A::RegionInfoMode { mode } = action {
                if let Some(level) = stack.current_mut() {
                    level.kind = MenuKind::RegionInfo { mode };
                }
                return None;
            }

            // The missionm select tail name-opens miss00 over the list (FFXiMain.dll
            // retail-2026-09 RVA 0x22DB40; vault main_menu.md §30) — kuluu pushes the
            // per-port detail window the same way.
            if let A::MissionOpenLog { port } = action {
                stack.push(MenuKind::MissionDetail { port });
                return None;
            }

            // The quest00 select tail name-opens quest01 over the list with the
            // row value (FFXiMain.dll retail-2026-09 RVA 0x2300EE; vault
            // main_menu.md §31) — kuluu pushes that quest's detail the same way.
            if let A::QuestOpenDetail { log, id } = action {
                stack.push(MenuKind::QuestDetail { log, id });
                return None;
            }

            // Config-family drill rows push their own window (handler jump tables:
            // hub FFXiMain.dll retail-2026-09 RVA 0x196770/.text 0x1968A4, `conf5m`
            // 0x199240; vault main_menu.md §42/§47) — the page's DAT rows arrive on
            // the next refresh of refresh_dynamic_menu_rows.
            if let A::OpenRetailConfigPage { page } = action {
                stack.push(MenuKind::RetailConfigPage(page));
                return None;
            }
            if let A::OpenRetailConfigSub { sub } = action {
                stack.push(MenuKind::RetailConfigSub(sub));
                return None;
            }
            // Leaf option rows: retail changes the value in place with no window
            // event (vault main_menu.md §46/§48), and until the option store lands
            // on bead kuluu-o9qt7.20.1 kuluu answers them with the standard line,
            // keeping its window open like retail does.
            if let A::ConfigOptionRow { .. } = action {
                let label = kuluu_render::hud::menu::entry_label(kind, cursor, dynamic);
                push_system_chat_line(scene_state, format!("[menu] {label} — not implemented"));
                return None;
            }

            // Bazaar stall picker select = “request to view a bazaar”: c2s 0x105
            // for that seller (vault main_menu.md §33), then kuluu's browse screen
            // takes over — the same seam as the /check wares path in check.rs.
            // The empty-list hint row never resolves to an entity and stays inert.
            if let A::BazaarStall { char_id } = action {
                let target_index = scene_state
                    .snapshot
                    .entities
                    .iter()
                    .find(|e| e.id == char_id && e.char_flags.bazaar)
                    .map(|e| e.act_index)?;
                if let Err(e) = cmd_tx.try_send(AgentCommand::OpenBazaar {
                    target_id: char_id,
                    target_index,
                }) {
                    push_system_chat_line(scene_state, format!("[menu] bazaar open dropped: {e}"));
                }
                return Some(InputMode::Bazaar);
            }

            // Craft-list select: the plan was resolved against the bag when the rows
            // were built, so this is that c2s 0x096 ask verbatim (vault main_menu.md
            // §28); the s2c 0x06F answer lands in snapshot.synth_result.
            if let A::SynthesizeRecipe {
                crystal_id,
                crystal_slot,
                ingredients,
            } = action
            {
                let ingredients = ingredients.into_iter().flatten().collect::<Vec<_>>();
                if let Err(e) = cmd_tx.try_send(AgentCommand::Synthesize {
                    crystal_id,
                    crystal_slot,
                    ingredients,
                }) {
                    push_system_chat_line(scene_state, format!("[menu] synth ask dropped: {e}"));
                }
                return None;
            }
            // Button row id 10 = Exit (vault main_menu.md §25): send Cancell and
            // close the window pair — kuluu pops back to Root, retail closes
            // both names through 0x15E910.
            if matches!(action, A::TradeCancelRow) {
                scene_state.snapshot.trade = None;
                if let Err(e) = cmd_tx.try_send(AgentCommand::TradeCancel) {
                    push_system_chat_line(scene_state, format!("[menu] trade cancel dropped: {e}"));
                }
                stack.pop();
                return None;
            }
            if let A::DropItem {
                container,
                index,
                item_no,
                quantity,
            } = action
            {
                stack.push(MenuKind::ItemDropConfirm {
                    container,
                    index,
                    item_no,
                    quantity,
                });
                if let Some(level) = stack.current_mut() {
                    level.cursor = kuluu_render::hud::menu::DROP_CONFIRM_DEFAULT;
                }
                return None;
            }
            // The macro family (bead kuluu-o9qt7.19.3): a store row drills its
            // line editor — retail's long-form chain from the palette/mcrselop
            // windows (FFXiMain.dll retail-2026-09 RVA 0x1A68C0/0x1A5690; vault
            // main_menu.md §34) — line rows seed the level's mcrledit buffer,
            // and the clear row commits without it. Nothing here touches the
            // wire except through CommitMacros (§41 [V-negative]).
            if let A::MacroSlotRow { block, index } = action {
                stack.push(MenuKind::MacroEditor { block, index });
                return None;
            }
            if let A::MacroEditLine { line_index } = action {
                let (block, index) = match kind {
                    MenuKind::MacroEditor { block, index } => (block, index),
                    _ => return None,
                };
                let seed = scene_state
                    .snapshot
                    .macros
                    .iter()
                    .find(|s| s.block == block && s.index == index)
                    .map(|s| kuluu_render::hud::menu::macro_slot_lines(&s.text))
                    .and_then(|ls| match line_index {
                        Some(i) => ls.get(i).map(|s| (*s).to_string()),
                        None => Some(String::new()),
                    })
                    .unwrap_or_default();
                if let Some(level) = stack.current_mut() {
                    level.macro_line_edit = Some(kuluu_render::input_mode::MacroLineBuffer {
                        block,
                        index,
                        line_no: line_index,
                        text: seed,
                    });
                }
                return None;
            }
            if let A::MacroClearSlot = action {
                let (block, index) = match kind {
                    MenuKind::MacroEditor { block, index } => (block, index),
                    _ => return None,
                };
                let slots: Vec<kuluu_snapshot::MacroSlot> = scene_state
                    .snapshot
                    .macros
                    .iter()
                    .filter(|s| !(s.block == block && s.index == index))
                    .cloned()
                    .collect();
                if let Err(e) = cmd_tx.try_send(AgentCommand::CommitMacros { slots }) {
                    push_system_chat_line(scene_state, format!("[menu] macro clear dropped: {e}"));
                }
                return None;
            }
            // Running a keytop (bead kuluu-o9qt7.19.4): the slot's command lines
            // become pending chat-pipeline steps — retail feeds a used macro one
            // line at a time into the same pipeline typed text takes (vault
            // main_menu.md §41/§56); no store write, no wire beyond what the
            // submitted lines themselves carry.
            if let A::MacroUse { block, index } = action {
                let steps = scene_state
                    .snapshot
                    .macros
                    .iter()
                    .find(|s| s.block == block && s.index == index)
                    .map(|s| {
                        kuluu_render::hud::menu::macro_slot_lines(&s.text)
                            .into_iter()
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                scene_state.queue_macro_steps(steps);
                return None;
            }
            if let A::UseItem {
                container, index, ..
            } = action
            {
                let snap = &scene_state.snapshot;
                let usable = kuluu_render::hud::item_meta::find_slot(snap, container, index)
                    .is_some_and(|slot| {
                        kuluu_render::hud::menu::item_usable_now(snap, slot)
                            && !kuluu_render::hud::item_meta::item_unusable(
                                slot,
                                kuluu_render::hud::item_meta::now_vana_ts(),
                            )
                    });
                if !usable {
                    return None;
                }
            }
            // Retail's key-item detail pane needs a description DAT not yet
            // identified (bead kuluu-h7x retail_unknowns); echo the name and
            // keep the list open.
            if let A::KeyItem { id } = action {
                push_system_chat_line(
                    scene_state,
                    format!(
                        "Key item: {}.",
                        kuluu_render::hud::menu::key_item_row_label(id, true)
                    ),
                );
                return None;
            }
            if let Some(sub_action) = sub_target_action_for(action) {
                // Retail's sub-target confirm step: the cursor always shows,
                // seeded on the current target when it passes the mask, else
                // self, else the nearest candidate (initial_candidate). Esc
                // restores this menu with its cursor intact.
                let return_to = InputMode::Menu(stack.clone());
                return open_sub_target(sub_action, target_id, scene_state, return_to);
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
                graphics.reselect_unequip,
            );
            // Retail's equip select case refreshes its windows in place instead of closing them
            // (FFXiMain.dll retail-2026-09 RVA 0x1322CE), so kuluu keeps the equip list up for
            // further swaps and keeps the bag open through a sort/move session; the
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
    let label = kuluu_render::hud::menu::entry_label(kind, cursor, dynamic);
    if let MenuKind::PowerPrompt { shut_down } = kind {
        if label == kuluu_render::hud::menu::DROP_CONFIRM_YES {
            let (cmd, toast) = if shut_down {
                (
                    AgentCommand::ReqLogout {
                        kind: ReqLogoutKind::ShutdownToggle,
                    },
                    "[menu] Shut Down requested (~30s; instant in Mog House). \
                    `/shutdown off` to cancel.",
                )
            } else {
                (
                    AgentCommand::ReqLogout {
                        kind: ReqLogoutKind::LogoutToggle,
                    },
                    "[menu] Log Out requested (~30s; instant in Mog House). \
                    `/logout off` to cancel.",
                )
            };
            if let Err(e) = cmd_tx.try_send(cmd) {
                push_system_chat_line(scene_state, format!("[menu] dispatch dropped: {e}"));
            } else {
                push_system_chat_line(scene_state, toast.to_string());
            }
        }
        // No (or any other answer) just dismisses the prompt.
        stack.pop();
        return None;
    }
    if let MenuKind::ItemDropConfirm {
        container,
        index,
        quantity,
        ..
    } = kind
    {
        if label == kuluu_render::hud::menu::DROP_CONFIRM_YES {
            if let Err(e) = cmd_tx.try_send(AgentCommand::DropItem {
                container,
                index,
                quantity,
            }) {
                push_system_chat_line(scene_state, format!("[menu] drop dropped: {e}"));
            }
        }
        stack.pop();
        stack.pop();
        return None;
    }
    match resolve_menu_entry(
        kind,
        label,
        graphics.magic_flat_list,
        graphics.abilities_flat_list,
    ) {
        MenuDispatch::OpenMogMenu => {
            // The arm's veto: in a Mog-house capital without being inside your
            // own house it ends at the error sound (FFXiMain.dll retail-2026-09
            // RVA 0x14E8F5/0xF9680; vault main_menu.md §32) — kuluu's analog is
            // this hint line, same as the Trade gate above.
            if kuluu_render::hud::menu::view_house_blocked(
                scene_state.snapshot.zone_id,
                scene_state.snapshot.self_in_mog_house(),
            ) {
                push_system_chat_line(
                    scene_state,
                    "[menu] View House — not available from here".to_string(),
                );
                return None;
            }
            if let Err(e) = cmd_tx.try_send(AgentCommand::OpenMogMenu) {
                push_system_chat_line(scene_state, format!("[menu] Mog Menu open dropped: {e}"));
            }
            None
        }
        MenuDispatch::TextCommand(cmd) => {
            apply_menu_text_command(
                cmd,
                command_surface,
                stack,
                vana_clock,
                vana_clock_visible,
                scene_state,
                cmd_tx,
                self_pos,
                target_id,
            );
            // Mirrors the Debug toggles: the menu stays open (provisional
            // pending retail capture, bead kuluu-y5hq retail_unknowns).
            None
        }
        MenuDispatch::HomeTextCommand(cmd) => {
            apply_menu_text_command(
                cmd,
                command_surface,
                stack,
                vana_clock,
                vana_clock_visible,
                scene_state,
                cmd_tx,
                self_pos,
                target_id,
            );
            // The retail arms (FFXiMain.dll retail-2026-09 RVA 0x145541 and
            // 0x14554A) close myroom by name after the runner returns; kuluu
            // pops this window's level to do the same (vault §53).
            stack.pop();
            None
        }
        MenuDispatch::BrowseBag { container } => {
            item_bag.0 = container;
            stack.push(MenuKind::Items);
            None
        }
        MenuDispatch::OpenSubmenu(submenu) => {
            // The Root Equipment row obeys the variant law (FFXiMain.dll
            // retail-2026-09 RVA 0x1404D5: Monstrosity first via helper 0x141BA0
            // into `mp_mncst`, then the puppet flag + job byte 0x12 branch into
            // `pupequip`; vault main_menu.md §39). Data presence on the folded
            // snapshot stands in for both client flags (same law as
            // equipment_drill_kind's doc).
            let submenu =
                if matches!(submenu, MenuKind::Equipment) && matches!(kind, MenuKind::Root) {
                    kuluu_render::hud::menu::equipment_drill_kind(&scene_state.snapshot)
                } else if matches!(submenu, MenuKind::Map) && matches!(kind, MenuKind::Root) {
                    // The Root Map row obeys the myroom variant law (FFXiMain.dll
                    // retail-2026-09 RVA 0x140653: name-open `myroom` when in a
                    // Mog House, else drive the mapframe family via 0x1F7810(0);
                    // vault main_menu.md §40).
                    kuluu_render::hud::menu::map_drill_kind(&scene_state.snapshot)
                } else {
                    submenu
                };
            // The Root Trade row obeys helper 0x13FE70's gate law (vault
            // main_menu.md §25): it opens only against an eligible PC target and
            // refuses — with the error-sound arm at menuwind 0x1406AD, whose
            // kuluu analog is this hint line — when a trade already exists or no
            // valid target stands selected. Success puts c2s TRADE_REQ
            // {UniqueNo, ActIndex} on the wire; our windows open right away and
            // advance only once the partner answers (player_trade.cpp).
            if matches!(submenu, MenuKind::Trade) && matches!(kind, MenuKind::Root) {
                if scene_state.snapshot.trade.is_some() {
                    push_system_chat_line(
                        scene_state,
                        "[menu] Trade — already in a trade".to_string(),
                    );
                    return None;
                }
                let partner = scene_state.snapshot.entities.iter().find(|e| {
                    Some(e.id) == target_id
                        && e.kind == kuluu_snapshot::EntityKind::Pc
                        && Some(e.id) != scene_state.snapshot.self_char_id
                });
                match partner {
                    Some(entity) => {
                        if let Err(e) = cmd_tx.try_send(AgentCommand::TradeRequest {
                            char_id: entity.id,
                            act_index: entity.act_index,
                        }) {
                            push_system_chat_line(
                                scene_state,
                                format!("[menu] trade request dropped: {e}"),
                            );
                        }
                        stack.push(submenu);
                    }
                    None => {
                        push_system_chat_line(
                            scene_state,
                            "[menu] Trade — no valid target".to_string(),
                        );
                    }
                }
                return None;
            }
            // Refresh the job-emote/chair unlock bits whenever the Emote List
            // opens (c2s 0x119 → s2c 0x11A gates the Job row).
            if submenu == MenuKind::EmoteList {
                let _ = cmd_tx.try_send(AgentCommand::RequestEmoteList);
            }
            // The power prompts open with the cursor on No, like every other
            // retail confirm.
            if matches!(submenu, MenuKind::PowerPrompt { .. }) {
                stack.push(submenu);
                if let Some(level) = stack.current_mut() {
                    level.cursor = kuluu_render::hud::menu::DROP_CONFIRM_DEFAULT;
                }
                return None;
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

/// Retail+ section rows (dev-only Debug menu). Returns true when the label is
/// one of them and it was handled here — the caller must not fall through to
/// `toggle_debug_panel`. The live toggles flip GraphicsSettings fields, so
/// `persist_graphics_on_change` writes graphics.json automatically. Mob HP
/// Under / Job Display only exist in enhanced builds (their rows are absent
/// from DEBUG_ENTRIES without their feature). The section-chrome rows
/// (separator + label) carry no state and answer with no banner. The DLSS
/// row reads N/A and its toggle is inert when this build can't run DLSS at
/// all (no dlss feature, or no RTX/Vulkan/DLLs), so the persisted gate does
/// not flip.
fn handle_retail_plus_row(
    label: &str,
    graphics: &mut kuluu_render::GraphicsSettings,
    scene_state: &mut SceneState,
) -> bool {
    #[cfg(feature = "enhanced-job-display")]
    use kuluu_render::hud::menu::RETAIL_JOB_DISPLAY;
    #[cfg(feature = "enhanced-ignore-knockback-self")]
    use kuluu_render::hud::menu::RETAIL_KNOCKBACK;
    #[cfg(feature = "enhanced-mob-hp-under")]
    use kuluu_render::hud::menu::RETAIL_MOB_HP_UNDER;
    use kuluu_render::hud::menu::{
        DEBUG_RETAIL_LABEL, DEBUG_RETAIL_SEPARATOR, RETAIL_ABILITIES_FLAT_LIST,
        RETAIL_CAMERA_LEASH, RETAIL_DLSS_MENU, RETAIL_MAGIC_FLAT_LIST, RETAIL_RESELECT_UNEQUIP,
        RETAIL_STATUS_ROWS,
    };
    match label {
        DEBUG_RETAIL_SEPARATOR | DEBUG_RETAIL_LABEL => true,
        RETAIL_STATUS_ROWS => {
            graphics.interactive_status_rows = !graphics.interactive_status_rows;
            push_system_chat_line(
                scene_state,
                format!(
                    "[menu] {label}: {}",
                    if graphics.interactive_status_rows {
                        "on"
                    } else {
                        "off"
                    }
                ),
            );
            true
        }
        // Retail's equip handler equips on every select case (FFXiMain.dll retail-2026-09 RVA
        // 0x1322B0, vault main_menu.md §20); the unequip-on-reselect shortcut is opt-in here.
        RETAIL_RESELECT_UNEQUIP => {
            graphics.reselect_unequip = !graphics.reselect_unequip;
            push_system_chat_line(
                scene_state,
                format!(
                    "[menu] {label}: {}",
                    if graphics.reselect_unequip {
                        "on"
                    } else {
                        "off"
                    }
                ),
            );
            true
        }
        // Retail's Magic row opens mgcmenu, whose handler turns a category pick into the
        // `magic` window's school filter (FFXiMain.dll retail-2026-09 RVA 0x13F750,
        // jump table 0x13F7CC; vault main_menu.md §21); kuluu's flat one-step spell list is
        // the opt-in enhancement.
        RETAIL_MAGIC_FLAT_LIST => {
            graphics.magic_flat_list = !graphics.magic_flat_list;
            push_system_chat_line(
                scene_state,
                format!(
                    "[menu] {label}: {}",
                    if graphics.magic_flat_list {
                        "on"
                    } else {
                        "off"
                    }
                ),
            );
            true
        }
        // Retail's Abilities row opens abimenu, whose rows filter the `ability`
        // list (row 6 opening `menu    mount`); kuluu's flat one-step ability list
        // is the opt-in enhancement (FFXiMain.dll retail-2026-09 RVA 0x13FA70,
        // vault main_menu.md §23).
        RETAIL_ABILITIES_FLAT_LIST => {
            graphics.abilities_flat_list = !graphics.abilities_flat_list;
            push_system_chat_line(
                scene_state,
                format!(
                    "[menu] {label}: {}",
                    if graphics.abilities_flat_list {
                        "on"
                    } else {
                        "off"
                    }
                ),
            );
            true
        }
        // Number row: the value moves with Left/Right, so confirm does
        // nothing rather than fall through to toggle_debug_panel.
        RETAIL_CAMERA_LEASH => true,
        RETAIL_DLSS_MENU => {
            if !graphics.dlss_supported {
                push_system_chat_line(
                    scene_state,
                    format!("[menu] {label}: N/A (this build can't run DLSS)"),
                );
                true
            } else {
                graphics.dlss_menu_enabled = !graphics.dlss_menu_enabled;
                push_system_chat_line(
                    scene_state,
                    format!(
                        "[menu] {label}: {}",
                        if graphics.dlss_menu_enabled {
                            "on"
                        } else {
                            "off"
                        }
                    ),
                );
                true
            }
        }
        #[cfg(feature = "enhanced-mob-hp-under")]
        RETAIL_MOB_HP_UNDER => {
            graphics.mob_hp_under = !graphics.mob_hp_under;
            push_system_chat_line(
                scene_state,
                format!(
                    "[menu] {label}: {}",
                    if graphics.mob_hp_under { "on" } else { "off" }
                ),
            );
            true
        }
        #[cfg(feature = "enhanced-job-display")]
        RETAIL_JOB_DISPLAY => {
            graphics.job_display = !graphics.job_display;
            push_system_chat_line(
                scene_state,
                format!(
                    "[menu] {label}: {}",
                    if graphics.job_display { "on" } else { "off" }
                ),
            );
            true
        }
        #[cfg(feature = "enhanced-ignore-knockback-self")]
        RETAIL_KNOCKBACK => {
            // [on] reports the feature live: knockback applied (the default).
            graphics.ignore_knockback_self = !graphics.ignore_knockback_self;
            push_system_chat_line(
                scene_state,
                format!(
                    "[menu] {label}: {}",
                    if graphics.ignore_knockback_self {
                        "off"
                    } else {
                        "on"
                    }
                ),
            );
            true
        }
        _ => false,
    }
}

/// Flip a Debug-menu toggle and report it. The weather/fog rows report the
/// feature's live state, so they invert the off flags: Weather [on] = weather
/// effects applied.
fn toggle_debug_panel(
    label: &str,
    hud_panels: &mut kuluu_render::hud::HudPanels,
    net_status: &mut kuluu_render::hud::network_status::NetStatusVisible,
    audio_mute: &mut kuluu_render::audio::AudioMuteState,
    vana_clock: &mut kuluu_render::vana_time::VanaClock,
    self_pos: kuluu_snapshot::Vec3,
    scene_state: &mut SceneState,
) {
    #[cfg(feature = "enhanced-engage-move-lock-off")]
    use kuluu_render::hud::menu::DEBUG_ENGAGE_ANIM_LOCK;
    use kuluu_render::hud::menu::{
        DEBUG_AUTO_ENTER_CS, DEBUG_ENTITY_LIST, DEBUG_FOG, DEBUG_FORCE_18, DEBUG_GRAPHICS_DEBUG,
        DEBUG_MESH, DEBUG_NAMEPLATES, DEBUG_NET_STATUS, DEBUG_NOCLIP, DEBUG_PERF,
        DEBUG_POSITION_LOG, DEBUG_PRINT_POS, DEBUG_SOUND, DEBUG_STAIR_DRAW, DEBUG_STAIR_STATUS,
        DEBUG_TARGET_CYCLE, DEBUG_UI_SETTINGS, DEBUG_WEATHER,
    };

    // Print Pos is a button, not a toggle: fire and return before the
    // on/off banner below. Prints self wire coords to the system chat.
    if label == DEBUG_PRINT_POS {
        push_system_chat_line(
            scene_state,
            format!(
                "[debug] pos: x={:.3} y={:.3} z={:.3}",
                self_pos.x, self_pos.y, self_pos.z,
            ),
        );
        return;
    }

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
        DEBUG_NOCLIP => {
            hud_panels.noclip = !hud_panels.noclip;
            hud_panels.noclip
        }
        #[cfg(feature = "enhanced-engage-move-lock-off")]
        DEBUG_ENGAGE_ANIM_LOCK => {
            hud_panels.engage_anim_lock = !hud_panels.engage_anim_lock;
            hud_panels.engage_anim_lock
        }
        DEBUG_AUTO_ENTER_CS => {
            hud_panels.auto_enter_cs = !hud_panels.auto_enter_cs;
            hud_panels.auto_enter_cs
        }
        DEBUG_WEATHER => {
            hud_panels.weather_off = !hud_panels.weather_off;
            !hud_panels.weather_off
        }
        DEBUG_FOG => {
            hud_panels.fog_off = !hud_panels.fog_off;
            !hud_panels.fog_off
        }
        DEBUG_FORCE_18 => {
            // Activation takes over another owner's hold; deactivation thaws
            // whatever the row froze.
            if vana_clock.debug_hold_active() {
                vana_clock.thaw();
                false
            } else {
                const FORCE_VANA_HOUR: u32 = 18;
                vana_clock.freeze_at_hour_minute(
                    FORCE_VANA_HOUR,
                    0,
                    kuluu_render::vana_time::ClockHoldOrigin::DebugRow,
                );
                true
            }
        }
        DEBUG_ENTITY_LIST => {
            hud_panels.entity_list = !hud_panels.entity_list;
            hud_panels.entity_list
        }
        DEBUG_STAIR_DRAW => {
            hud_panels.stair_draw = !hud_panels.stair_draw;
            hud_panels.stair_draw
        }
        DEBUG_STAIR_STATUS => {
            hud_panels.stair_debug = !hud_panels.stair_debug;
            hud_panels.stair_debug
        }
        DEBUG_GRAPHICS_DEBUG => {
            hud_panels.graphics_debug = !hud_panels.graphics_debug;
            hud_panels.graphics_debug
        }
        DEBUG_POSITION_LOG => {
            hud_panels.position_log = !hud_panels.position_log;
            hud_panels.position_log
        }
        DEBUG_NAMEPLATES => {
            hud_panels.nameplate_debug = !hud_panels.nameplate_debug;
            hud_panels.nameplate_debug
        }
        DEBUG_UI_SETTINGS => {
            hud_panels.ui_settings = !hud_panels.ui_settings;
            hud_panels.ui_settings
        }
        DEBUG_NET_STATUS => {
            net_status.0 = !net_status.0;
            net_status.0
        }
        DEBUG_SOUND => {
            // Toggle master: if either category is currently unmuted,
            // sound reads as ON, so a click MUTES both. Otherwise UNMUTE.
            let was_on = !(audio_mute.bgm && audio_mute.sfx);
            audio_mute.bgm = was_on;
            audio_mute.sfx = was_on;
            !was_on
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

/// Consume one keystroke while a macro line editor buffer is open (bead
/// kuluu-o9qt7.19.3): printable characters append, Backspace pops, Enter
/// commits the merged set through CommitMacros (retail's mcrledit commit —
/// FFXiMain.dll retail-2026-09 RVA 0x1A6180 runs its blip then stores;
/// vault main_menu.md §34/§41), Esc discards. The menu never closes here.
fn macro_line_edit_key(
    key: &Key,
    bindings: &Bindings,
    stack: &mut MenuStack,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
) {
    if bindings.matches_logical(Action::NavCancel, key) {
        if let Some(level) = stack.current_mut() {
            level.macro_line_edit = None;
        }
        return;
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        let edit = stack
            .current()
            .and_then(|l| l.macro_line_edit.as_ref())
            .map(|b| (b.block, b.index, b.line_no, b.text.clone()));
        if let Some((block, index, line_no, text)) = edit {
            let slots = kuluu_render::hud::menu::merge_macro_line(
                &scene_state.snapshot.macros,
                block,
                index,
                line_no,
                &text,
            );
            if let Err(e) = cmd_tx.try_send(AgentCommand::CommitMacros { slots }) {
                push_system_chat_line(scene_state, format!("[menu] macro commit dropped: {e}"));
            }
        }
        if let Some(level) = stack.current_mut() {
            level.macro_line_edit = None;
        }
        return;
    }
    let Some(buf) = stack.current_mut().and_then(|l| l.macro_line_edit.as_mut()) else {
        return;
    };
    if bindings.matches_logical(Action::ChatBackspace, key) {
        buf.text.pop();
        return;
    }
    match key {
        Key::Space => buf.text.push(' '),
        Key::Character(s) => {
            for c in s.chars() {
                if !c.is_control() {
                    buf.text.push(c);
                }
            }
        }
        _ => {}
    }
}

/// Route one keypress in the open menu. In the sort pane, any key the pane
/// does not own maps to SortPaneKey::Other so it can't leak into list
/// navigation. Commands > Items opens on the inventory; only the Mog Menu
/// storage rows open the window on another bag.
pub(super) fn handle_menu_key(
    key: &Key,
    key_code: KeyCode,
    bindings: &mut Bindings,
    stack: &mut MenuStack,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    keybinds_state: &mut KeybindsStateRes,
    graphics: &mut kuluu_render::GraphicsSettings,
    status_profile_open: &mut kuluu_render::hud::status_panel::StatusProfileOpen,
    hud_panels: &mut kuluu_render::hud::HudPanels,
    net_status: &mut kuluu_render::hud::network_status::NetStatusVisible,
    audio_mute: &mut kuluu_render::audio::AudioMuteState,
    vana_clock: &mut kuluu_render::vana_time::VanaClock,
    vana_clock_visible: &mut kuluu_render::hud::vana_clock::VanaClockVisible,
    sort_options: &mut kuluu_render::hud::item_detail::SortOptions,
    item_menu_focus: &mut kuluu_render::hud::item_detail::ItemMenuFocus,
    item_bag: &mut kuluu_render::hud::item_screen::ItemScreenContainer,
    item_viewport: &mut kuluu_render::hud::item_screen::ItemListViewport,
    dynamic: &kuluu_render::hud::menu::DynamicMenu,
    target_id: Option<u32>,
    self_pos: kuluu_snapshot::Vec3,
    map_state: &mut kuluu_render::hud::map_screen::MapScreenState,
    // `Mut` (not `&mut` off a call-site `ResMut` deref, which flags the
    // resource changed on every menu key): dropped untouched on non-Map paths,
    // change detection fires only when the Map screen mutates a marker
    // (kuluu-df0x).
    map_markers: Mut<kuluu_render::hud::map_screen::MapMarkers>,
    map_view: &kuluu_render::hud::map_screen::MapView,
    minimap_state: &kuluu_render::minimap::MinimapState,
    change_map_catalog: &kuluu_render::hud::map_screen::ChangeMapCatalog,
    command_surface: &crate::view_native::command_surface::CommandSurface,
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
            key_code,
            bindings,
            stack,
            scene_state,
            cmd_tx,
            map_state,
            map_markers,
            map_view,
            minimap_state,
            change_map_catalog,
        );
    }
    let (kind, cursor) = {
        let level = stack.current()?;
        (level.kind, level.cursor)
    };
    let entry_count = kuluu_render::hud::menu::entry_count(kind, dynamic, graphics.dlss_supported);

    // Menu context (not text input), so reading the raw keycode is correct.
    // "-" advances the Command menu's pages (retail HorizonXI flips two; kuluu
    // cycles three); single-list submenus have no pages, and the Map screen
    // handles "-" in its own path.
    if key_code == KeyCode::Minus {
        if kind == MenuKind::Root {
            if let Some(level) = stack.current_mut() {
                level.cursor = kuluu_render::hud::menu::root_next_page_cursor(level.cursor);
            }
        }
        return None;
    }

    // mcrledit line editor (bead kuluu-o9qt7.19.3): while the window holds an
    // edit buffer every keystroke belongs to it — retail's macro line editor
    // owns input until Enter commits or Esc discards (FFXiMain.dll
    // retail-2026-09 RVA 0x1A6180; vault main_menu.md §34/§41). Nav keys stay
    // dead while typing, which is also the `.29` rule.
    if stack.current().is_some_and(|l| l.macro_line_edit.is_some()) {
        macro_line_edit_key(key, bindings, stack, scene_state, cmd_tx);
        return None;
    }

    // Root Command menu paging: Right advances like "-", Left steps back through
    // the pages; Up/Down wrap within the current page so row navigation never
    // crosses a page boundary.
    if kind == MenuKind::Root {
        let (start, end) = kuluu_render::hud::menu::root_page_bounds(cursor);
        if bindings.matches_logical(Action::NavRight, key) {
            if let Some(level) = stack.current_mut() {
                level.cursor = kuluu_render::hud::menu::root_next_page_cursor(level.cursor);
            }
            return None;
        }
        if bindings.matches_logical(Action::NavLeft, key) {
            if let Some(level) = stack.current_mut() {
                level.cursor = kuluu_render::hud::menu::root_prev_page_cursor(level.cursor);
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
    // Options box. Retail's "Select active window" key (Numpad + on the full
    // keyboard) steps focus through them; Up/Down clamp at both ends and
    // Left/Right shift cursor and page together by a full row count
    // (.agents/skills/retail-observe/references/2026-09-11-items-window.md).
    if matches!(kind, MenuKind::Items | MenuKind::UsableItems) {
        use kuluu_render::hud::item_detail::{sort_pane_key, SortOptionId, SortPaneKey};
        use kuluu_render::hud::item_screen::{page_cursor, step_cursor};
        if kind == MenuKind::Items {
            if bindings.matches_logical(Action::SelectActiveWindow, key) {
                if kuluu_render::hud::item_screen::select_active_window(
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
                    SortPaneKey::Other
                };
                match sort_pane_key(item_menu_focus, sort_options, pane_key) {
                    Some(SortOptionId::RecycleBin) => {
                        if kuluu_render::hud::item_screen::open_recycle_bin(
                            &scene_state.snapshot,
                            item_bag,
                            item_menu_focus,
                        ) {
                            if let Some(level) = stack.current_mut() {
                                level.cursor = 0;
                            }
                        }
                    }
                    Some(_) => {
                        if let Err(e) = cmd_tx.try_send(AgentCommand::StackInventory {
                            container: ffxi_proto::map::container::LOC_INVENTORY,
                        }) {
                            push_system_chat_line(
                                scene_state,
                                format!("sort dropped (channel): {e}"),
                            );
                        }
                    }
                    None => {}
                }
                return None;
            }
        }
        let page = if bindings.matches_logical(Action::NavLeft, key) {
            Some(false)
        } else if bindings.matches_logical(Action::NavRight, key) {
            Some(true)
        } else {
            None
        };
        if let Some(forward) = page {
            item_viewport.page(forward, entry_count);
            if let Some(level) = stack.current_mut() {
                level.cursor = page_cursor(level.cursor, entry_count, forward);
            }
            return None;
        }
        let step = if bindings.matches_logical(Action::NavUp, key) {
            Some(false)
        } else if bindings.matches_logical(Action::NavDown, key) {
            Some(true)
        } else {
            None
        };
        if let Some(down) = step {
            if let Some(level) = stack.current_mut() {
                level.cursor = step_cursor(level.cursor, entry_count, down);
            }
            return None;
        }
    }

    if matches!(
        kind,
        MenuKind::Config | MenuKind::Graphics | MenuKind::GraphicsDlss
    ) {
        if bindings.matches_logical(Action::NavLeft, key) {
            apply_settings_cycle(kind, cursor, -1, graphics);
            return None;
        }
        if bindings.matches_logical(Action::NavRight, key) {
            apply_settings_cycle(kind, cursor, 1, graphics);
            return None;
        }
    }

    // Debug menu: the Volume and Camera_leash rows are numbers adjusted with
    // Left/Right. Every other Debug row is a toggle handled on the confirm
    // key, so only those two rows consume arrows here; anything else falls
    // through to normal list navigation.
    if matches!(kind, MenuKind::Debug) {
        let label = kuluu_render::hud::menu::entry_label(kind, cursor, dynamic);
        if label == kuluu_render::hud::menu::DEBUG_VOLUME {
            if bindings.matches_logical(Action::NavLeft, key) {
                audio_mute.cycle_master(-1);
                return None;
            }
            if bindings.matches_logical(Action::NavRight, key) {
                audio_mute.cycle_master(1);
                return None;
            }
        }
        if label == kuluu_render::hud::menu::RETAIL_CAMERA_LEASH {
            let dir = if bindings.matches_logical(Action::NavLeft, key) {
                -1
            } else if bindings.matches_logical(Action::NavRight, key) {
                1
            } else {
                return None;
            };
            let units = (graphics.camera_leash_yalms
                / kuluu_render::graphics_settings::CAMERA_LEASH_STEP_YALMS)
                .round() as i32
                + dir;
            graphics.camera_leash_yalms = (units.max(0) as f32
                * kuluu_render::graphics_settings::CAMERA_LEASH_STEP_YALMS)
                .min(kuluu_render::graphics_settings::CAMERA_LEASH_MAX_YALMS);
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
                kuluu_render::hud::equipment_screen::grid_move(level.cursor as u8, dx, dy) as usize;
            return None;
        }
    }

    // Retail pages every other vertical list menu (the action-ring Usable list,
    // Magic/Abilities/Key Items/Emotes, the equip-slot picker) with Left/Right,
    // one visible page per press, clamped at the ends. Items handled its own
    // paging above; Graphics (value cycles) and the Equipment grid consumed
    // Left/Right in their blocks.
    if kuluu_render::hud::menu::is_dynamic(kind) {
        let page = if bindings.matches_logical(Action::NavLeft, key) {
            Some(false)
        } else if bindings.matches_logical(Action::NavRight, key) {
            Some(true)
        } else {
            None
        };
        if let Some(forward) = page {
            let rows = kuluu_render::hud::menu::list_page_rows(kind);
            let level = stack.current_mut()?;
            level.cursor =
                kuluu_render::hud::menu::page_cursor(level.cursor, entry_count, rows, forward);
            return None;
        }
    }

    if bindings.matches_logical(Action::NavUp, key) {
        let level = stack.current_mut()?;
        let up = if cursor == 0 {
            entry_count.saturating_sub(1)
        } else {
            cursor - 1
        };
        level.cursor =
            kuluu_render::hud::menu::settle_cursor(kind, graphics.dlss_supported, up, false);
        return None;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        let level = stack.current_mut()?;
        let next = cursor + 1;
        let down = if next >= entry_count { 0 } else { next };
        level.cursor =
            kuluu_render::hud::menu::settle_cursor(kind, graphics.dlss_supported, down, true);
        return None;
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        let next = confirm_menu_at_cursor(
            bindings,
            stack,
            scene_state,
            cmd_tx,
            keybinds_state,
            graphics,
            status_profile_open,
            hud_panels,
            net_status,
            audio_mute,
            vana_clock,
            vana_clock_visible,
            dynamic,
            target_id,
            self_pos,
            command_surface,
            item_bag,
        );
        if kind == MenuKind::Root && stack.current().is_some_and(|l| l.kind == MenuKind::Items) {
            item_bag.0 = ffxi_proto::map::container::LOC_INVENTORY;
        }
        return next;
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
mod menu_key_tests {
    use super::*;
    use crate::keybinds_store::KeybindsStore;
    use bevy::ecs::world::World;
    use kuluu_render::hud::map_screen::{
        ChangeMapCatalog, MapMarkers, MapScreenState, MapSubMode, MapView,
    };
    use kuluu_render::input_mode::Pane;
    use kuluu_render::minimap::{MinimapAabb, MinimapState};

    struct Harness {
        bindings: Bindings,
        scene_state: SceneState,
        keybinds_state: KeybindsStateRes,
        graphics: kuluu_render::GraphicsSettings,
        status_profile_open: kuluu_render::hud::status_panel::StatusProfileOpen,
        hud_panels: kuluu_render::hud::HudPanels,
        net_status: kuluu_render::hud::network_status::NetStatusVisible,
        audio_mute: kuluu_render::audio::AudioMuteState,
        vana_clock: kuluu_render::vana_time::VanaClock,
        vana_clock_visible: kuluu_render::hud::vana_clock::VanaClockVisible,
        sort_options: kuluu_render::hud::item_detail::SortOptions,
        item_menu_focus: kuluu_render::hud::item_detail::ItemMenuFocus,
        item_bag: kuluu_render::hud::item_screen::ItemScreenContainer,
        item_viewport: kuluu_render::hud::item_screen::ItemListViewport,
        dynamic: kuluu_render::hud::menu::DynamicMenu,
        map_state: MapScreenState,
        map_view: MapView,
        minimap_state: MinimapState,
        change_map_catalog: ChangeMapCatalog,
        command_surface: crate::view_native::command_surface::CommandSurface,
        cmd_tx: Sender<AgentCommand>,
        _cmd_rx: tokio::sync::mpsc::Receiver<AgentCommand>,
    }

    impl Harness {
        fn new() -> Self {
            let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(8);
            Self {
                bindings: Bindings::default(),
                scene_state: SceneState::default(),
                keybinds_state: KeybindsStateRes {
                    store: KeybindsStore::new(
                        std::env::temp_dir().join("kuluu-menu-key-tests-unused.json"),
                    ),
                    persisted: Default::default(),
                },
                graphics: Default::default(),
                status_profile_open: Default::default(),
                hud_panels: Default::default(),
                net_status: Default::default(),
                audio_mute: Default::default(),
                vana_clock: kuluu_render::vana_time::VanaClock::anchored_at_hour(12.0),
                vana_clock_visible: Default::default(),
                sort_options: Default::default(),
                item_menu_focus: Default::default(),
                item_bag: Default::default(),
                item_viewport: Default::default(),
                dynamic: Default::default(),
                map_state: MapScreenState::default(),
                map_view: MapView::default(),
                minimap_state: MinimapState::default(),
                change_map_catalog: ChangeMapCatalog::default(),
                command_surface: crate::view_native::command_surface::CommandSurface::default(),
                cmd_tx,
                _cmd_rx,
            }
        }

        fn key(
            &mut self,
            key: &Key,
            key_code: KeyCode,
            stack: &mut MenuStack,
            map_markers: Mut<MapMarkers>,
        ) -> Option<InputMode> {
            handle_menu_key(
                key,
                key_code,
                &mut self.bindings,
                stack,
                &mut self.scene_state,
                &self.cmd_tx,
                &mut self.keybinds_state,
                &mut self.graphics,
                &mut self.status_profile_open,
                &mut self.hud_panels,
                &mut self.net_status,
                &mut self.audio_mute,
                &mut self.vana_clock,
                &mut self.vana_clock_visible,
                &mut self.sort_options,
                &mut self.item_menu_focus,
                &mut self.item_bag,
                &mut self.item_viewport,
                &self.dynamic,
                None,
                kuluu_snapshot::Vec3::default(),
                &mut self.map_state,
                map_markers,
                &self.map_view,
                &self.minimap_state,
                &self.change_map_catalog,
                &self.command_surface,
            )
        }
    }

    fn marker_world() -> World {
        let mut world = World::new();
        world.insert_resource(MapMarkers::default());
        world.clear_trackers();
        world
    }

    /// Row 0 of Config is the "Interface" section header, so the cursor
    /// settles onto the first selectable row below it.
    #[test]
    fn config_keys_cycle_radar_and_open_controls_without_changing_bindings() {
        use kuluu_render::graphics_settings::config_rows;
        use kuluu_render::graphics_settings::{MenuAction, MenuRow};
        use kuluu_render::hud::menu::settle_cursor;
        use kuluu_render::MinimapRadar;
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::Config);
        let minimap_slot = settle_cursor(MenuKind::Config, false, 0, true);
        stack.current_mut().unwrap().cursor = minimap_slot;
        for (key, code, expected) in [
            (Key::ArrowRight, KeyCode::ArrowRight, MinimapRadar::Enhanced),
            (Key::ArrowLeft, KeyCode::ArrowLeft, MinimapRadar::Vanilla),
            (Key::Enter, KeyCode::Enter, MinimapRadar::Enhanced),
        ] {
            harness.key(&key, code, &mut stack, world.resource_mut::<MapMarkers>());
            assert_eq!(harness.graphics.minimap_radar, expected);
            assert_eq!(stack.current().unwrap().cursor, minimap_slot);
            assert_eq!(stack.current().unwrap().kind, MenuKind::Config);
        }
        let controls_slot = config_rows()
            .iter()
            .position(|r| *r == MenuRow::Action(MenuAction::Controls))
            .expect("Controls row");
        stack.current_mut().unwrap().cursor = controls_slot;
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert_eq!(stack.current().unwrap().kind, MenuKind::Controls);
        assert_eq!(
            resolve_menu_entry(MenuKind::Controls, "Compact 1", false, false),
            MenuDispatch::KeybindUpdate(KeybindUpdate::Preset(Preset::Compact1))
        );
    }

    /// kuluu-ce6z: the cursor is read from `stack.current()`, so writes must go
    /// to `current_mut()` too. With `active_pane = Pane::Left` on a depth-2
    /// stack, `active_level_mut()` resolves to the PARENT level and a NavDown
    /// computed from the top level's state would corrupt the parent's cursor.
    #[test]
    fn nav_down_writes_the_level_it_read_even_with_left_pane_active() {
        const PARENT_CURSOR_SENTINEL: usize = 3;
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::Config);
        stack.levels[0].cursor = PARENT_CURSOR_SENTINEL;
        stack.active_pane = Pane::Left;

        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::ArrowDown, KeyCode::ArrowDown, &mut stack, markers);

        assert_eq!(stack.levels[1].cursor, 1, "top level cursor advances");
        assert_eq!(
            stack.levels[0].cursor, PARENT_CURSOR_SENTINEL,
            "parent level cursor is untouched"
        );
        assert_eq!(stack.active_pane, Pane::Left, "pane not altered by nav");
    }

    /// kuluu-df0x: navigating a non-Map menu must not flag MapMarkers changed
    /// (marker_store::sync_markers rewrites markers.json on is_changed).
    #[test]
    fn non_map_menu_key_does_not_flag_markers_changed() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();

        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::ArrowDown, KeyCode::ArrowDown, &mut stack, markers);

        assert!(
            !world.is_resource_changed::<MapMarkers>(),
            "menu navigation must not dirty MapMarkers"
        );
    }

    /// The clock is the only truth for the Force_18 row: activation freezes
    /// with a debug-row origin (taking over another owner's hold), deactivation
    /// thaws, and `VanaClock::debug_hold_active` alone drives the row.
    #[test]
    fn force_18_row_reads_and_writes_only_the_clock() {
        use kuluu_render::hud::menu::DEBUG_FORCE_18;
        use kuluu_render::vana_time::{ClockHoldOrigin, VanaClock};
        let mut hud_panels = kuluu_render::hud::HudPanels::default();
        let mut net_status = kuluu_render::hud::network_status::NetStatusVisible(false);
        let mut audio_mute = kuluu_render::audio::AudioMuteState::default();
        let mut scene_state = SceneState::default();
        let self_pos = kuluu_snapshot::Vec3::default();
        let mut clock = VanaClock::default();

        toggle_debug_panel(
            DEBUG_FORCE_18,
            &mut hud_panels,
            &mut net_status,
            &mut audio_mute,
            &mut clock,
            self_pos,
            &mut scene_state,
        );
        assert!(clock.is_frozen());
        assert_eq!(clock.hold_origin(), Some(ClockHoldOrigin::DebugRow));

        toggle_debug_panel(
            DEBUG_FORCE_18,
            &mut hud_panels,
            &mut net_status,
            &mut audio_mute,
            &mut clock,
            self_pos,
            &mut scene_state,
        );
        assert!(!clock.is_frozen());
        assert_eq!(clock.hold_origin(), None);

        // A foreign hold reads as off and is taken over; toggling then thaws.
        clock.freeze(ClockHoldOrigin::Cutscene);
        assert!(!clock.debug_hold_active());
        toggle_debug_panel(
            DEBUG_FORCE_18,
            &mut hud_panels,
            &mut net_status,
            &mut audio_mute,
            &mut clock,
            self_pos,
            &mut scene_state,
        );
        assert_eq!(clock.hold_origin(), Some(ClockHoldOrigin::DebugRow));
        toggle_debug_panel(
            DEBUG_FORCE_18,
            &mut hud_panels,
            &mut net_status,
            &mut audio_mute,
            &mut clock,
            self_pos,
            &mut scene_state,
        );
        assert!(!clock.is_frozen());
    }

    #[test]
    fn placing_a_marker_flags_markers_changed() {
        const ZONE: u16 = 231;
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::Map);
        stack.take_absorb_open_minus();

        harness.scene_state.snapshot.zone_id = Some(ZONE);
        harness.map_state.mode = MapSubMode::Markers;
        harness.map_state.map_cursor = Some(bevy::math::Vec2::splat(0.5));
        harness.map_state.marker_entry = Some("Camp".into());
        harness.map_view.visible_aabb = Some(MinimapAabb {
            min: bevy::math::Vec2::new(-100.0, -100.0),
            max: bevy::math::Vec2::new(100.0, 100.0),
        });

        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);

        assert!(
            world.is_resource_changed::<MapMarkers>(),
            "confirming a named marker must dirty MapMarkers so it persists"
        );
        let markers = world.resource::<MapMarkers>();
        assert_eq!(markers.for_zone(ZONE).len(), 1);
        assert_eq!(markers.for_zone(ZONE)[0].label, "Camp");
    }

    #[test]
    fn power_prompt_opens_on_no_then_yes_only_requests_on_yes() {
        use kuluu_render::hud::menu::DROP_CONFIRM_DEFAULT;

        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.current_mut().unwrap().cursor = menu_root_log_out_index();

        // First Enter on the Log Out row pushes the prompt, cursor on No.
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);
        {
            let level = stack.current().expect("prompt pushed");
            assert!(
                matches!(level.kind, MenuKind::PowerPrompt { shut_down: false }),
                "Log Out must open its yes/no prompt, got {:?}",
                level.kind
            );
            assert_eq!(level.cursor, DROP_CONFIRM_DEFAULT, "prompt opens on No");
        }

        // Confirming while on No answers nothing and dismisses the prompt.
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);
        assert_eq!(stack.levels.len(), 1, "No dismisses back to Root");
        assert!(
            harness._cmd_rx.try_recv().is_err(),
            "No must not send a logout request"
        );

        // Re-open and answer Yes: the toggle goes out exactly once.
        stack.current_mut().unwrap().cursor = menu_root_log_out_index();
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);
        stack.current_mut().unwrap().cursor = 0; // Yes
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);
        assert_eq!(stack.levels.len(), 1, "answer dismisses the prompt");
        assert_eq!(
            harness._cmd_rx.try_recv().expect("Yes must send"),
            AgentCommand::ReqLogout {
                kind: ReqLogoutKind::LogoutToggle,
            }
        );
    }

    fn menu_root_log_out_index() -> usize {
        use kuluu_render::hud::menu::ROOT_LOG_OUT;
        kuluu_render::hud::menu::root_entries()
            .iter()
            .position(|e| *e == ROOT_LOG_OUT)
            .unwrap()
    }

    #[test]
    fn region_info_select_installs_the_mode_in_place() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::RegionInfo { mode: 0 });
        stack.take_absorb_open_minus();
        harness.dynamic.rows = kuluu_render::hud::menu::region_info_rows(0, None);

        // Handler item id 3 installs display mode 2 (vault main_menu.md §29).
        stack.current_mut().unwrap().cursor = 2;
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);

        assert_eq!(
            stack.levels.len(),
            2,
            "install must not push another window"
        );
        match stack.current().unwrap().kind {
            MenuKind::RegionInfo { mode } => assert_eq!(mode, 2),
            other => panic!("expected the Region Info window on top, got {other:?}"),
        }
    }

    #[test]
    fn mission_row_select_pushes_the_detail_window() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::MissionList);
        stack.take_absorb_open_minus();
        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        snap.missions.logs.push(kuluu_snapshot::MissionLogEntry {
            port: 0x00D0,
            data: [1, 0, 0, 0, 0, 0, 0, 0],
        });
        harness.scene_state.snapshot = snap;
        harness.dynamic.rows =
            kuluu_render::hud::menu::mission_list_rows(&harness.scene_state.snapshot);

        // The sole row is the Nations complete-log entry (vault main_menu.md section 30).
        stack.current_mut().unwrap().cursor = 0;
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);

        match stack.current().unwrap().kind {
            MenuKind::MissionDetail { port } => assert_eq!(port, 0x00D0),
            other => panic!("expected the miss00-stand-in detail on top, got {other:?}"),
        }
        assert_eq!(
            stack.levels.len(),
            3,
            "detail pushes over the list, like retail"
        );
    }

    #[test]
    fn quest_row_select_pushes_the_detail_window() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::QuestList);
        stack.take_absorb_open_minus();
        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        // San d'Oria offer bit 0 only — the list's single row is quest (log 0, id 0).
        snap.missions.logs.push(kuluu_snapshot::MissionLogEntry {
            port: ffxi_proto::mission::QUEST_OFFER_PORTS[0],
            data: [1, 0, 0, 0, 0, 0, 0, 0],
        });
        harness.scene_state.snapshot = snap;
        harness.dynamic.rows =
            kuluu_render::hud::menu::quest_list_rows(&harness.scene_state.snapshot);

        stack.current_mut().unwrap().cursor = 0;
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);

        match stack.current().unwrap().kind {
            MenuKind::QuestDetail { log, id } => {
                assert_eq!(log, 0);
                assert_eq!(id, 0);
            }
            other => panic!("expected the quest01-stand-in detail on top, got {other:?}"),
        }
        assert_eq!(
            stack.levels.len(),
            3,
            "detail pushes over the list, like retail"
        );
    }

    fn menu_root_trade_index() -> usize {
        kuluu_render::hud::menu::root_entries()
            .iter()
            .position(|e| *e == "Trade")
            .unwrap()
    }

    /// Page-1 row 8 drills the trade window pair now — no longer a stub (vault
    /// main_menu.md §25).
    #[test]
    fn root_trade_row_resolves_to_the_window() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Trade", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::Trade)
        );
    }

    /// One Enter through the full key handler with an explicit target.
    fn drive_enter(
        harness: &mut Harness,
        stack: &mut MenuStack,
        world: &mut World,
        target_id: Option<u32>,
    ) {
        let markers = world.resource_mut::<MapMarkers>();
        handle_menu_key(
            &Key::Enter,
            KeyCode::Enter,
            &mut harness.bindings,
            stack,
            &mut harness.scene_state,
            &harness.cmd_tx,
            &mut harness.keybinds_state,
            &mut harness.graphics,
            &mut harness.status_profile_open,
            &mut harness.hud_panels,
            &mut harness.net_status,
            &mut harness.audio_mute,
            &mut harness.vana_clock,
            &mut harness.vana_clock_visible,
            &mut harness.sort_options,
            &mut harness.item_menu_focus,
            &mut harness.item_bag,
            &mut harness.item_viewport,
            &harness.dynamic,
            target_id,
            kuluu_snapshot::Vec3::default(),
            &mut harness.map_state,
            markers,
            &harness.map_view,
            &harness.minimap_state,
            &harness.change_map_catalog,
            &harness.command_surface,
        );
    }

    fn trade_pc_entity(id: u32, name: &str) -> kuluu_snapshot::Entity {
        use kuluu_snapshot::{EntityKind, Vec3};
        kuluu_snapshot::Entity {
            id,
            act_index: id as u16,
            kind: EntityKind::Pc,
            name: Some(name.to_string()),
            pos: Vec3::default(),
            heading: 0,
            hp_pct: Some(100),
            bt_target_id: 0,
            face_target: 0,
            claim_id: 0,
            speed: 0,
            speed_base: 0,
            look: None,
            animation: 0,
            animationsub: 0,
            mount: None,
            status: kuluu_snapshot::status_type::NORMAL,
            char_flags: Default::default(),
            monstrosity: false,
            name_vis: None,
        }
    }

    /// Helper 0x13FE70's gate law (vault main_menu.md §25): without an eligible
    /// target the row opens nothing and sends nothing — the hint line stands in
    /// for retail's error-sound arm at menuwind 0x1406AD. A valid PC target
    /// puts c2s TRADE_REQ {UniqueNo, ActIndex} on the wire and opens our own
    /// windows right away; self-targets and a second concurrent trade are both
    /// refused exactly like the helper's null-field gates refuse them.
    #[test]
    fn root_trade_row_gates_on_eligible_target() {
        // No target at all: Root stays, nothing on the wire.
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.current_mut().unwrap().cursor = menu_root_trade_index();
        drive_enter(&mut harness, &mut stack, &mut world, None);
        assert_eq!(stack.levels.len(), 1, "no target: the window stays shut");
        assert!(harness._cmd_rx.try_recv().is_err());

        // A valid PC target opens the window and sends TRADE_REQ with both the
        // char id and the wire act index read off the entity.
        let mut harness = Harness::new();
        let mut world = marker_world();
        harness.scene_state.snapshot.entities = vec![trade_pc_entity(42, "Gessandral")];
        let mut stack = MenuStack::root();
        stack.current_mut().unwrap().cursor = menu_root_trade_index();
        drive_enter(&mut harness, &mut stack, &mut world, Some(42));
        assert_eq!(stack.current().unwrap().kind, MenuKind::Trade);
        assert_eq!(
            harness._cmd_rx.try_recv().expect("TRADE_REQ must go out"),
            AgentCommand::TradeRequest {
                char_id: 42,
                act_index: 42,
            }
        );

        // Self-target: the helper's self-entity comparison (call 0x81550) makes
        // this ineligible — no packet, no window.
        let mut harness = Harness::new();
        let mut world = marker_world();
        harness.scene_state.snapshot.self_char_id = Some(42);
        harness.scene_state.snapshot.entities = vec![trade_pc_entity(42, "Self")];
        let mut stack = MenuStack::root();
        stack.current_mut().unwrap().cursor = menu_root_trade_index();
        drive_enter(&mut harness, &mut stack, &mut world, Some(42));
        assert_eq!(stack.levels.len(), 1, "self is not a trade partner");
        assert!(harness._cmd_rx.try_recv().is_err());

        // Already trading: helper fields +0x74/+0x6C refuse the second open.
        let mut harness = Harness::new();
        let mut world = marker_world();
        harness.scene_state.snapshot.entities = vec![trade_pc_entity(42, "Gessandral")];
        harness.scene_state.snapshot.trade = Some(Box::new(kuluu_snapshot::TradeWindow {
            partner_id: 9,
            incoming_pending: false,
            active: true,
            my_slots: Vec::new(),
            partner_slots: Vec::new(),
            my_confirmed: false,
            partner_confirmed: false,
        }));
        let mut stack = MenuStack::root();
        stack.current_mut().unwrap().cursor = menu_root_trade_index();
        drive_enter(&mut harness, &mut stack, &mut world, Some(42));
        assert_eq!(stack.levels.len(), 1, "one transaction at a time (LSB law)");
        assert!(harness._cmd_rx.try_recv().is_err());
    }

    /// kuluu-u8p1: the Change Map list the key handler indexes and the one the
    /// panel draws must be the same list, and it must only offer floors the
    /// zone's DLL records describe -- POLUtils lists a third Windurst Waters map
    /// that the loader cannot resolve. Gated on a retail install (self-skips).
    #[test]
    fn change_map_confirm_selects_a_floor_the_dll_describes() {
        const WINDURST_WATERS: u16 = 238;
        let Some(root) = ffxi_dat::archive::open_test_install() else {
            return;
        };
        let Ok(dll) = ffxi_dat::main_dll::MainDll::load(root.root()) else {
            return;
        };

        let mut harness = Harness::new();
        harness.change_map_catalog = ChangeMapCatalog::from_dll(&dll);
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::Map);
        stack.take_absorb_open_minus();

        harness.scene_state.snapshot.zone_id = Some(WINDURST_WATERS);
        harness.map_state.mode = MapSubMode::ChangeMap;

        for _ in 0..2 {
            let markers = world.resource_mut::<MapMarkers>();
            harness.key(&Key::ArrowDown, KeyCode::ArrowDown, &mut stack, markers);
        }
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(&Key::Enter, KeyCode::Enter, &mut stack, markers);

        // POLUtils counts three Windurst Waters maps and the DLL two, so under
        // the old roster the third row was this zone's phantom third floor.
        let viewed = harness.map_state.viewed.expect("a row was confirmed");
        assert_ne!(viewed, (WINDURST_WATERS, 2));
        assert_eq!(
            viewed.1, 0,
            "past the zone's floors the list moves on to other zones at map 0"
        );
        assert!(
            !dll.zone_maps(viewed.0).is_empty(),
            "and that zone has a DLL record to preview"
        );
    }

    /// kuluu-kzxp: Period/Comma zoom the full-screen map on default binds. The
    /// logical-key path never resolves non-letter printables, so the zoom gate
    /// must match on the raw keycode.
    #[test]
    fn default_zoom_keys_zoom_the_map_screen() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::Map);
        stack.take_absorb_open_minus();
        assert_eq!(harness.map_state.zoom_radius, None, "opens fit-to-zone");

        let markers = world.resource_mut::<MapMarkers>();
        harness.key(
            &Key::Character(".".into()),
            KeyCode::Period,
            &mut stack,
            markers,
        );
        let zoomed_in = harness.map_state.zoom_radius;
        assert!(zoomed_in.is_some(), "Period zooms in from fit-to-zone");

        let markers = world.resource_mut::<MapMarkers>();
        harness.key(
            &Key::Character(",".into()),
            KeyCode::Comma,
            &mut stack,
            markers,
        );
        assert_eq!(
            harness.map_state.zoom_radius, None,
            "Comma zooms back out to fit-to-zone"
        );
    }

    #[test]
    fn typing_punctuation_into_a_marker_label_does_not_zoom() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::Map);
        stack.take_absorb_open_minus();
        harness.map_state.mode = MapSubMode::Markers;
        harness.map_state.marker_entry = Some("E-8".to_string());

        let markers = world.resource_mut::<MapMarkers>();
        harness.key(
            &Key::Character(",".into()),
            KeyCode::Comma,
            &mut stack,
            markers,
        );
        let markers = world.resource_mut::<MapMarkers>();
        harness.key(
            &Key::Character(".".into()),
            KeyCode::Period,
            &mut stack,
            markers,
        );
        assert_eq!(harness.map_state.marker_entry.as_deref(), Some("E-8,."));
        assert_eq!(
            harness.map_state.zoom_radius, None,
            "zoom binds must not fire while a marker label is being typed"
        );
    }

    /// Drill law (bead kuluu-o9qt7.19.3): selecting a stored keytop row opens that
    /// slot's editor as its own level — retail chained the long-form mcr*edit
    /// windows off the palette/op-select handler (`FFXiMain.dll` retail-2026-09 RVA
    /// 0x1A68C0; vault main_menu.md §34/§41).
    #[test]
    fn macro_select_row_drills_the_line_editor() {
        use kuluu_render::hud::menu::{DynamicMenuAction, DynamicMenuRow};
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::MacroSelect);
        harness.dynamic.rows = vec![DynamicMenuRow {
            label: "B1/5: /ra".to_string(),
            action: DynamicMenuAction::MacroSlotRow { block: 0, index: 4 },
        }];
        stack.current_mut().unwrap().cursor = 0;
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert_eq!(
            stack.current().map(|l| l.kind),
            Some(MenuKind::MacroEditor { block: 0, index: 4 })
        );
    }

    /// Selecting a command-line row seeds the level's mcrledit buffer with that
    /// line (edit-and-commit law of FFXiMain.dll retail-2026-09 RVA 0x1A6180;
    /// vault main_menu.md §34/§41); selecting the add row seeds an empty append.
    #[test]
    fn editor_line_rows_seed_the_edit_buffer() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        harness.scene_state.snapshot.macros = vec![kuluu_snapshot::MacroSlot {
            block: 0,
            index: 3,
            text: "/ra".to_string(),
        }];
        let mut stack = MenuStack::root();
        stack.push(MenuKind::MacroEditor { block: 0, index: 3 });
        harness.dynamic.rows =
            kuluu_render::hud::menu::macro_editor_rows(&harness.scene_state.snapshot, 0, 3);

        stack.current_mut().unwrap().cursor = 0; // the "/ra" line row
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert_eq!(
            stack.current().map(|l| l.kind),
            Some(MenuKind::MacroEditor { block: 0, index: 3 }),
            "seeding must not leave the editor window"
        );
        let seeded = stack.current().unwrap().macro_line_edit.clone();
        assert_eq!(
            seeded,
            Some(kuluu_render::input_mode::MacroLineBuffer {
                block: 0,
                index: 3,
                line_no: Some(0),
                text: "/ra".to_string(),
            })
        );

        stack.current_mut().unwrap().macro_line_edit = None;
        stack.current_mut().unwrap().cursor = 2; // the add row (past line + use)
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert_eq!(
            stack.current().unwrap().macro_line_edit.clone(),
            Some(kuluu_render::input_mode::MacroLineBuffer {
                block: 0,
                index: 3,
                line_no: None,
                text: String::new(),
            })
        );
    }

    /// The clear row commits the store without the keytop — a blank keytop
    /// leaves no entry (vault main_menu.md §41 capacity law; bead .19.3).
    #[test]
    fn editor_clear_row_commits_the_store_without_the_slot() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        harness.scene_state.snapshot.macros = vec![
            kuluu_snapshot::MacroSlot {
                block: 0,
                index: 3,
                text: "/ra".to_string(),
            },
            kuluu_snapshot::MacroSlot {
                block: 1,
                index: 7,
                text: "/wait 1".to_string(),
            },
        ];
        let mut stack = MenuStack::root();
        stack.push(MenuKind::MacroEditor { block: 0, index: 3 });
        harness.dynamic.rows =
            kuluu_render::hud::menu::macro_editor_rows(&harness.scene_state.snapshot, 0, 3);

        // Rows are ["/ra", use, add, clear] — the cursor goes to the clear row.
        stack.current_mut().unwrap().cursor = 3;
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        let committed = match harness._cmd_rx.try_recv() {
            Ok(AgentCommand::CommitMacros { slots }) => slots,
            other => panic!("clear row must commit CommitMacros, got {other:?}"),
        };
        assert_eq!(
            committed,
            vec![kuluu_snapshot::MacroSlot {
                block: 1,
                index: 7,
                text: "/wait 1".to_string(),
            }],
            "the cleared keytop must be gone, the other untouched"
        );
    }

    /// Typing law (bead .19.3): printable keys append to the buffer, Backspace
    /// pops, Enter commits the merged set through CommitMacros and clears the
    /// buffer while the editor window stays open — retail's mcrledit blip-then-
    /// store tail (`FFXiMain.dll` retail-2026-09 handler 0x1A6180; vault
    /// main_menu.md §34/§41).
    #[test]
    fn editor_typing_commits_the_merged_slot() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        harness.scene_state.snapshot.macros = vec![kuluu_snapshot::MacroSlot {
            block: 0,
            index: 3,
            text: "/ra".to_string(),
        }];
        let mut stack = MenuStack::root();
        stack.push(MenuKind::MacroEditor { block: 0, index: 3 });
        harness.dynamic.rows =
            kuluu_render::hud::menu::macro_editor_rows(&harness.scene_state.snapshot, 0, 3);

        // Seed through the add row so the commit is an append.
        stack.current_mut().unwrap().cursor = 2;
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert!(stack.current().unwrap().macro_line_edit.is_some());

        for (key, code) in [
            (&Key::Character("/".into()), KeyCode::Slash),
            (&Key::Character("p".into()), KeyCode::KeyP),
            (&Key::Character("o".into()), KeyCode::KeyO),
            (&Key::Character("k".into()), KeyCode::KeyK),
            (&Key::Character("e".into()), KeyCode::KeyE),
        ] {
            harness.key(key, code, &mut stack, world.resource_mut::<MapMarkers>());
        }
        assert_eq!(
            stack
                .current()
                .unwrap()
                .macro_line_edit
                .as_ref()
                .unwrap()
                .text,
            "/poke"
        );

        // A typed space appends, Backspace pops it again (trailing-space edit).
        harness.key(
            &Key::Space,
            KeyCode::Space,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        harness.key(
            &Key::Backspace,
            KeyCode::Backspace,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );

        // Enter commits and clears.
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert_eq!(stack.current().unwrap().macro_line_edit, None);
        assert_eq!(
            stack.current().map(|l| l.kind),
            Some(MenuKind::MacroEditor { block: 0, index: 3 }),
            "retail's commit keeps the editor open"
        );
        let committed = match harness._cmd_rx.try_recv() {
            Ok(AgentCommand::CommitMacros { slots }) => slots,
            other => panic!("Enter must commit CommitMacros, got {other:?}"),
        };
        assert_eq!(
            committed,
            vec![kuluu_snapshot::MacroSlot {
                block: 0,
                index: 3,
                text: "/ra\n/poke".to_string(),
            }]
        );
    }

    /// While typing, navigation is dead (also the `.29` rule): arrows neither
    /// move the row cursor nor touch the buffer, and Esc discards the edit but
    /// keeps the editor window open (bead .19.3).
    #[test]
    fn editor_typing_owns_navigation_until_commit_or_escape() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        let mut stack = MenuStack::root();
        stack.push(MenuKind::MacroEditor { block: 0, index: 3 });
        let depth = stack.levels.len();
        stack.current_mut().unwrap().cursor = 1;
        stack.current_mut().unwrap().macro_line_edit =
            Some(kuluu_render::input_mode::MacroLineBuffer {
                block: 0,
                index: 3,
                line_no: Some(0),
                text: "/ra".to_string(),
            });

        for (key, code) in [
            (&Key::ArrowDown, KeyCode::ArrowDown),
            (&Key::ArrowUp, KeyCode::ArrowUp),
            (&Key::ArrowRight, KeyCode::ArrowRight),
        ] {
            harness.key(key, code, &mut stack, world.resource_mut::<MapMarkers>());
            assert_eq!(
                stack.current().unwrap().cursor,
                1,
                "arrows must not move rows while typing"
            );
            assert_eq!(
                stack
                    .current()
                    .unwrap()
                    .macro_line_edit
                    .as_ref()
                    .unwrap()
                    .text,
                "/ra",
                "arrows must not touch the buffer"
            );
        }

        harness.key(
            &Key::Escape,
            KeyCode::Escape,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert_eq!(stack.current().unwrap().macro_line_edit, None);
        assert_eq!(stack.levels.len(), depth, "Esc discards the edit only");
        assert_eq!(
            stack.current().map(|l| l.kind),
            Some(MenuKind::MacroEditor { block: 0, index: 3 })
        );
    }

    /// The use row starts a run (bead kuluu-o9qt7.19.4): confirming it queues
    /// exactly the slot's command lines, in order — and nothing else: no store
    /// commit, no window change (retail runs one line at a time through the
    /// chat pipeline; vault main_menu.md §41/§56).
    #[test]
    fn editor_use_row_queues_the_slot_lines_in_order() {
        let mut harness = Harness::new();
        let mut world = marker_world();
        harness.scene_state.snapshot.macros = vec![kuluu_snapshot::MacroSlot {
            block: 0,
            index: 3,
            text: "/ra\n/attend".to_string(),
        }];
        let mut stack = MenuStack::root();
        stack.push(MenuKind::MacroEditor { block: 0, index: 3 });
        harness.dynamic.rows =
            kuluu_render::hud::menu::macro_editor_rows(&harness.scene_state.snapshot, 0, 3);

        // Rows are ["/ra", "/attend", use, add, clear] — cursor on the use row.
        stack.current_mut().unwrap().cursor = 2;
        harness.key(
            &Key::Enter,
            KeyCode::Enter,
            &mut stack,
            world.resource_mut::<MapMarkers>(),
        );
        assert_eq!(harness.scene_state.pending_macro_len(), 2);
        assert_eq!(
            (
                harness.scene_state.pop_next_macro_step(),
                harness.scene_state.pop_next_macro_step(),
                harness.scene_state.pop_next_macro_step()
            ),
            (Some("/ra".to_string()), Some("/attend".to_string()), None)
        );
        assert_eq!(
            stack.current().map(|l| l.kind),
            Some(MenuKind::MacroEditor { block: 0, index: 3 }),
            "running must not move the editor"
        );
        assert!(
            harness._cmd_rx.try_recv().is_err(),
            "a run is not a store write"
        );
    }
}

#[cfg(test)]
mod menu_dispatch_tests {
    use super::*;

    #[test]
    fn log_out_row_opens_the_yes_no_prompt() {
        use kuluu_render::hud::menu::ROOT_LOG_OUT;
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, ROOT_LOG_OUT, false, false),
            MenuDispatch::OpenSubmenu(MenuKind::PowerPrompt { shut_down: false })
        );
    }

    #[test]
    fn shut_down_row_opens_the_yes_no_prompt() {
        use kuluu_render::hud::menu::ROOT_SHUT_DOWN;
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, ROOT_SHUT_DOWN, false, false),
            MenuDispatch::OpenSubmenu(MenuKind::PowerPrompt { shut_down: true })
        );
    }

    /// The Search row drills into the searchma/scresult surface unconditionally,
    /// like retail's menuwind arm 0x140626 opens `searchma` with no gate (vault
    /// main_menu.md §26).
    #[test]
    fn search_row_opens_the_search_window() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Search", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::Search)
        );

        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Linkshell", false, false),
            MenuDispatch::OpenSubmenu(kuluu_render::MenuKind::Linkshell),
            "page-1 row 10 drills to the slot window (retail always opens menu    link5)"
        );
    }

    /// The Synthesis row drills the cmbmenu launcher ungated, exactly like the
    /// menuwind arm FFXiMain.dll retail-2026-09 RVA 0x1405D2 name-opens `menu
    /// cmbmenu` with no gate (vault main_menu.md §28), and the launcher's two rows
    /// go where select ids 1/2 of handler 0x1E29A0 take them (§22).
    #[test]
    fn synthesis_row_opens_the_cmbmenu_launcher() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Synthesis", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::SynthLauncher)
        );
        use kuluu_render::hud::menu::{SYNTH_ROW_OPEN_LIST, SYNTH_ROW_PICK_MATERIAL};
        assert_eq!(
            resolve_menu_entry(
                MenuKind::SynthLauncher,
                SYNTH_ROW_PICK_MATERIAL,
                false,
                false
            ),
            MenuDispatch::OpenSubmenu(MenuKind::Items)
        );
        assert_eq!(
            resolve_menu_entry(MenuKind::SynthLauncher, SYNTH_ROW_OPEN_LIST, false, false),
            MenuDispatch::OpenSubmenu(MenuKind::CraftList)
        );
    }

    #[test]
    fn missions_row_opens_the_battery_list() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Missions", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::MissionList)
        );
    }

    #[test]
    fn region_info_row_drills_the_four_mode_window() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Region Info", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::RegionInfo { mode: 0 })
        );
    }

    #[test]
    fn edit_macros_row_drills_the_mcrmenu_launcher() {
        // Ungated page-2 arm FFXiMain.dll retail-2026-09 RVA 0x14E8BD name-opens
        // `menu    mcrmenu` (vault main_menu.md §34); its two rows drill the
        // §41 windows bead .19.1 built.
        use kuluu_render::hud::menu::{MACRO_ROW_MACRO_RESULT, MACRO_ROW_SELECT_MACRO};
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Edit Macros", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::MacrosPalette)
        );
        assert_eq!(
            resolve_menu_entry(
                MenuKind::MacrosPalette,
                MACRO_ROW_SELECT_MACRO,
                false,
                false
            ),
            MenuDispatch::OpenSubmenu(MenuKind::MacroSelect)
        );
        assert_eq!(
            resolve_menu_entry(
                MenuKind::MacrosPalette,
                MACRO_ROW_MACRO_RESULT,
                false,
                false
            ),
            MenuDispatch::OpenSubmenu(MenuKind::MacroResult)
        );
    }

    /// The Map row drills `myroom` lawfully (§40) and the fourteen home-view
    /// rows answer through §53's corrected select family (handler
    /// FFXiMain.dll retail-2026-09 RVA 0x145460 jump table .rdata 0x1455BC):
    /// container rows
    /// open their LSB bags (id1's `menu    storage`/`storage2` arms and the
    /// locker-family widgets), Gardening/Layout hand their literals to the
    /// runner, Change Jobs drills `jobchang`; only the still-unbuilt retail
    /// arms (Delivery Box→`mogpost`, Remodel→`roomlist`, the Mog House door)
    /// stay on NotImplemented until their beads.
    #[test]
    fn home_view_rows_resolve_through_the_select_family() {
        use ffxi_proto::map::container as c;
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Map", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::Map)
        );
        for (label, want) in [
            (
                "M.Ward.",
                MenuDispatch::BrowseBag {
                    container: c::LOC_WARDROBE,
                },
            ),
            (
                "Mog Safe",
                MenuDispatch::BrowseBag {
                    container: c::LOC_MOGSAFE,
                },
            ),
            (
                "Storage",
                MenuDispatch::BrowseBag {
                    container: c::LOC_STORAGE,
                },
            ),
            (
                "Mog Locker",
                MenuDispatch::BrowseBag {
                    container: c::LOC_MOGLOCKER,
                },
            ),
            (
                "Satchel",
                MenuDispatch::BrowseBag {
                    container: c::LOC_MOGSATCHEL,
                },
            ),
            (
                "Mog Case",
                MenuDispatch::BrowseBag {
                    container: c::LOC_MOGCASE,
                },
            ),
            (
                "Mog Sack",
                MenuDispatch::BrowseBag {
                    container: c::LOC_MOGSACK,
                },
            ),
            (
                "Safe 2",
                MenuDispatch::BrowseBag {
                    container: c::LOC_MOGSAFE2,
                },
            ),
            ("Gardening", MenuDispatch::HomeTextCommand("/garden")),
            ("Layout", MenuDispatch::HomeTextCommand("/layout")),
            (
                "Change Jobs",
                MenuDispatch::OpenSubmenu(MenuKind::JobChange),
            ),
            (
                "Open Mog House",
                MenuDispatch::NotImplemented("Open Mog House".to_string()),
            ),
            (
                "Delivery Box",
                MenuDispatch::NotImplemented("Delivery Box".to_string()),
            ),
            (
                "Remodel",
                MenuDispatch::NotImplemented("Remodel".to_string()),
            ),
        ] {
            assert_eq!(
                resolve_menu_entry(MenuKind::HomeView, label, false, false),
                want,
                "{label}"
            );
        }
    }

    /// `jobchang`'s two DAT widgets (catalog tokens 65 "Main Job" / 66
    /// "Support Job", vault main_menu.md §53) drill their roster-built pick
    /// lists; each side commits its own c2s 0x100 index.
    #[test]
    fn job_change_rows_drill_their_pick_sides() {
        assert_eq!(
            resolve_menu_entry(MenuKind::JobChange, "Main Job", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::JobPick { main_pick: true })
        );
        assert_eq!(
            resolve_menu_entry(MenuKind::JobChange, "Support Job", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::JobPick { main_pick: false })
        );
    }

    /// Boilerplate-free confirm rig for child-window rows: runs the shared
    /// keyboard/mouse seam on `stack` with its own scratch writers.
    fn confirm_row(
        stack: &mut MenuStack,
        scene_state: &mut SceneState,
        cmd_tx: &Sender<AgentCommand>,
        item_bag: &mut kuluu_render::hud::item_screen::ItemScreenContainer,
    ) -> Option<InputMode> {
        let dynamic = kuluu_render::hud::menu::DynamicMenu::default();
        confirm_row_dynamic(stack, scene_state, cmd_tx, &dynamic, item_bag)
    }

    /// Same seam with caller-built dynamic rows — used for the Config family,
    /// whose rows come from a synthetic DAT table in these tests.
    fn confirm_row_dynamic(
        stack: &mut MenuStack,
        scene_state: &mut SceneState,
        cmd_tx: &Sender<AgentCommand>,
        dynamic: &kuluu_render::hud::menu::DynamicMenu,
        item_bag: &mut kuluu_render::hud::item_screen::ItemScreenContainer,
    ) -> Option<InputMode> {
        use crate::keybinds_store::KeybindsStore;
        let mut bindings = Bindings::default();
        let mut keybinds_state = KeybindsStateRes {
            store: KeybindsStore::new(std::env::temp_dir().join("kuluu-myroom-seam-tests.json")),
            persisted: Default::default(),
        };
        let mut graphics = kuluu_render::GraphicsSettings::default();
        let mut status_profile_open = Default::default();
        let mut hud_panels = Default::default();
        let mut net_status = Default::default();
        let mut audio_mute = Default::default();
        let mut vana_clock = kuluu_render::vana_time::VanaClock::anchored_at_hour(12.0);
        let mut vana_clock_visible = Default::default();
        let surface = crate::view_native::command_surface::CommandSurface::default();
        confirm_menu_at_cursor(
            &mut bindings,
            stack,
            scene_state,
            cmd_tx,
            &mut keybinds_state,
            &mut graphics,
            &mut status_profile_open,
            &mut hud_panels,
            &mut net_status,
            &mut audio_mute,
            &mut vana_clock,
            &mut vana_clock_visible,
            dynamic,
            None,
            kuluu_snapshot::Vec3::default(),
            &surface,
            item_bag,
        )
    }

    fn stack_home_view_with_cursor(stack: &mut MenuStack, label: &str) {
        use kuluu_render::hud::menu::HOME_VIEW_ROWS;
        stack.push(MenuKind::HomeView);
        let row = HOME_VIEW_ROWS
            .iter()
            .position(|e| *e == label)
            .expect("home-view row");
        stack.current_mut().unwrap().cursor = row;
    }

    /// Confirm-time law: the Storage row (id1's `menu    storage` arm, holder
    /// method FFXiMain.dll retail-2026-09 RVA 0x145C70(0)) opens kuluu's Items
    /// window on the LSB storage bag — the same seam a Mog Menu storage row uses.
    #[test]
    fn storage_row_confirm_opens_items_window_on_the_storage_bag() {
        use ffxi_proto::map::container as c;
        let mut stack = MenuStack::root();
        stack_home_view_with_cursor(&mut stack, "Storage");
        let mut scene_state = SceneState::default();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let mut item_bag = kuluu_render::hud::item_screen::ItemScreenContainer::default();
        confirm_row(&mut stack, &mut scene_state, &cmd_tx, &mut item_bag);
        assert_eq!(stack.current().map(|l| l.kind), Some(MenuKind::Items));
        assert_eq!(item_bag.0, c::LOC_STORAGE);
        // The home view stays open below, like the retail window pair.
        let kinds: Vec<MenuKind> = stack.levels.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![MenuKind::Root, MenuKind::HomeView, MenuKind::Items]
        );
    }

    /// Confirm-time law: the Gardening row runs its literal through the runner
    /// and then closes myroom by name (arms 0x145541/0x15E910 tail; §53) — kuluu
    /// pops the home-view level, landing back on Root.
    #[test]
    fn gardening_row_runs_its_literal_and_closes_the_panel() {
        let mut stack = MenuStack::root();
        stack_home_view_with_cursor(&mut stack, "Gardening");
        let mut scene_state = SceneState::default();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let mut item_bag = kuluu_render::hud::item_screen::ItemScreenContainer::default();
        confirm_row(&mut stack, &mut scene_state, &cmd_tx, &mut item_bag);
        assert_eq!(stack.current().map(|l| l.kind), Some(MenuKind::Root));
    }

    /// A job pick commits exactly one c2s 0x100 MYROOM_JOB side: the picked id
    /// rides its own index, the other side stays zero ("no change" — LSB skips
    /// validating zero indices per vendor/server/src/map/packets/c2s/
    /// 0x100_myroom_job.cpp; vault main_menu.md §53).
    #[test]
    fn home_job_picks_send_their_side_of_change_job() {
        use kuluu_render::hud::menu::DynamicMenuAction;
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let mut scene_state = SceneState::default();
        dispatch_dynamic_menu_action(
            DynamicMenuAction::HomeJobRow {
                job_id: 6,
                as_main: true,
            },
            None,
            kuluu_snapshot::Vec3::default(),
            &[],
            &cmd_tx,
            &mut scene_state,
            false,
        );
        assert_eq!(
            cmd_rx.try_recv().expect("main pick must send"),
            AgentCommand::ChangeJob {
                main_job: Some(6),
                sub_job: None
            }
        );
        dispatch_dynamic_menu_action(
            DynamicMenuAction::HomeJobRow {
                job_id: 3,
                as_main: false,
            },
            None,
            kuluu_snapshot::Vec3::default(),
            &[],
            &cmd_tx,
            &mut scene_state,
            false,
        );
        assert_eq!(
            cmd_rx.try_recv().expect("sub pick must send"),
            AgentCommand::ChangeJob {
                main_job: None,
                sub_job: Some(3)
            }
        );
    }

    #[test]
    fn config_row_drills_the_retail_hub() {
        // Ungated page-2 arm FFXiMain.dll retail-2026-09 RVA 0x14E8D9
        // name-opens `menu    configwi` (vault main_menu.md §35). The hub's own
        // rows are DAT widgets read at runtime (bead kuluu-o9qt7.33); their
        // drills are pinned in confirm-time by config_hub_confirm_pushes_the_page_windows.
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Config", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::RetailConfig)
        );
    }

    /// Confirm-time law for the DAT-driven Config family (bead kuluu-o9qt7.33):
    /// hub row i pushes its own page window — the twelve jump-table branches of
    /// FFXiMain.dll retail-2026-09 RVA 0x196770 (.text 0x1968A4; vault main_menu.md §42)
    /// — while a thirteenth widget falls to the handler's default tail and answers as an
    /// option row instead (vault main_menu.md §57).
    #[test]
    fn config_hub_confirm_pushes_the_page_windows() {
        use kuluu_render::hud::menu::{
            config_window_rows, ConfigDatRows, DynamicMenu, RETAIL_CONFIG_PAGE_WINDOWS,
        };
        let mut table = ConfigDatRows {
            loaded: true,
            ..Default::default()
        };
        table.by_root.insert(
            "configwi".to_string(),
            (0..13u32).map(|i| (200 + i, format!("hub{i}"))).collect(),
        );
        let dynamic = DynamicMenu {
            rows: config_window_rows("configwi", &table),
        };
        for page in 0..RETAIL_CONFIG_PAGE_WINDOWS.len() {
            let mut stack = MenuStack::root();
            stack.push(MenuKind::RetailConfig);
            stack.current_mut().unwrap().cursor = page;
            let mut scene_state = SceneState::default();
            let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
            let mut item_bag = kuluu_render::hud::item_screen::ItemScreenContainer::default();
            confirm_row_dynamic(
                &mut stack,
                &mut scene_state,
                &cmd_tx,
                &dynamic,
                &mut item_bag,
            );
            assert_eq!(
                stack.current().map(|l| l.kind),
                Some(MenuKind::RetailConfigPage(page as u8)),
                "hub row {page} must push its own page window"
            );
        }
        // The widget past the twelve branches (GAMEPAD Config launcher) has no drill:
        // it stays put and answers with the option line instead.
        let mut stack = MenuStack::root();
        stack.push(MenuKind::RetailConfig);
        stack.current_mut().unwrap().cursor = 12;
        let mut scene_state = SceneState::default();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let mut item_bag = kuluu_render::hud::item_screen::ItemScreenContainer::default();
        confirm_row_dynamic(
            &mut stack,
            &mut scene_state,
            &cmd_tx,
            &dynamic,
            &mut item_bag,
        );
        assert_eq!(
            stack.current().map(|l| l.kind),
            Some(MenuKind::RetailConfig)
        );
        let toast = &scene_state.local_toasts.last().expect("answer line").text;
        assert!(
            toast.contains("hub12") && toast.contains("not implemented"),
            "{toast}"
        );
    }

    /// `conf5m`'s select law (handler FFXiMain.dll retail-2026-09 RVA 0x199240,
    /// arms 0x199264/0x19927A; vault main_menu.md §47): its first widget name-opens
    /// `conf5w2` (sub 2), the second `conf5w1`, and a third falls to the arm recorded as opening `conf5win`
    /// (sub 0). A leaf option row answers with the standard line WITHOUT closing
    /// its window — retail's leaf selects carry no window event, and the value writes ride on
    /// c2s SAVE_CONF owned by bead kuluu-o9qt7.20.1.
    #[test]
    fn config_launcher_and_option_rows_follow_their_arms() {
        use kuluu_render::hud::menu::{config_window_rows, ConfigDatRows, DynamicMenu};
        let mut table = ConfigDatRows {
            loaded: true,
            ..Default::default()
        };
        table.by_root.insert(
            "conf5m".to_string(),
            vec![
                (901, "shared".to_string()),
                (902, "individual".to_string()),
                (903, "other".to_string()),
            ],
        );
        let launcher = DynamicMenu {
            rows: config_window_rows("conf5m", &table),
        };
        for (cursor, sub) in [(0usize, 2u8), (1, 1), (2, 0)] {
            let mut stack = MenuStack::root();
            stack.push(MenuKind::RetailConfigPage(3));
            stack.current_mut().unwrap().cursor = cursor;
            let mut scene_state = SceneState::default();
            let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
            let mut item_bag = kuluu_render::hud::item_screen::ItemScreenContainer::default();
            confirm_row_dynamic(
                &mut stack,
                &mut scene_state,
                &cmd_tx,
                &launcher,
                &mut item_bag,
            );
            assert_eq!(
                stack.current().map(|l| l.kind),
                Some(MenuKind::RetailConfigSub(sub)),
                "conf5m widget {cursor} must name-open sub {sub}"
            );
        }
        table.by_root.insert(
            "conf7".to_string(),
            vec![(500, "o1".to_string()), (501, "o2".to_string())],
        );
        let page = DynamicMenu {
            rows: config_window_rows("conf7", &table),
        };
        let mut stack = MenuStack::root();
        stack.push(MenuKind::RetailConfigPage(7));
        stack.current_mut().unwrap().cursor = 1;
        let mut scene_state = SceneState::default();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let mut item_bag = kuluu_render::hud::item_screen::ItemScreenContainer::default();
        confirm_row_dynamic(&mut stack, &mut scene_state, &cmd_tx, &page, &mut item_bag);
        assert_eq!(
            stack.current().map(|l| l.kind),
            Some(MenuKind::RetailConfigPage(7)),
            "option-row selects must keep the page window open"
        );
        let toast = &scene_state.local_toasts.last().expect("answer line").text;
        assert!(
            toast.contains("o2") && toast.contains("not implemented"),
            "{toast}"
        );
    }

    #[test]
    fn help_desk_row_drills_the_faqsub_panel() {
        // Page-2 arm FFXiMain.dll retail-2026-09 RVA 0x14E9C0 -> trampoline
        // 0x2174C0 name-opens `menu    faqsub` (vault main_menu.md §36); the
        // three destination rows answer until `.21.1` builds them.
        use kuluu_render::hud::menu::{FAQ_ROW_CONFIG, FAQ_ROW_MENTOR, FAQ_ROW_QA};
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Help Desk", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::HelpDeskPanel)
        );
        for label in [FAQ_ROW_QA, FAQ_ROW_MENTOR, FAQ_ROW_CONFIG] {
            assert!(matches!(
                resolve_menu_entry(MenuKind::HelpDeskPanel, label, false, false),
                MenuDispatch::NotImplemented(_)
            ));
        }
    }

    #[test]
    fn communication_row_drills_both_commen_list_widgets() {
        // Page-2 arm FFXiMain.dll retail-2026-09 RVA 0x14E970 name-opens
        // `menu    commenu` (vault main_menu.md §37); the DAT order is
        // windowps330 Friend List then windowps928 Emote List, and each row
        // drills its own list widget.
        use kuluu_render::hud::menu::{COMM_EMOTE_LIST, COMM_FRIEND_LIST};
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Communication", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::Communication)
        );
        assert_eq!(
            resolve_menu_entry(MenuKind::Communication, COMM_FRIEND_LIST, false, false),
            MenuDispatch::OpenSubmenu(MenuKind::FriendList)
        );
        assert_eq!(
            resolve_menu_entry(MenuKind::Communication, COMM_EMOTE_LIST, false, false),
            MenuDispatch::OpenSubmenu(MenuKind::EmoteList)
        );
    }

    #[test]
    fn unwired_root_entries_stay_not_implemented() {
        // Party is built (opens the prtyN window per vault main_menu.md §24), so
        // are Search (§26), Region Info (§29), Missions (§30) and View House —
        // the Mog Menu row (§32); everything else still answers
        // "Not supported yet".
        // Bazaar is built now (drills the stall picker, vault main_menu.md §33),
        // and so are Edit Macros (§34 launcher), Config (§35 hub) and Help Desk
        // (§36 panel): every Root entry resolves to a drill or a command —
        // nothing unwired. Assert it by scanning the whole root list.
        for label in kuluu_render::hud::menu::entries_for(MenuKind::Root, false) {
            assert!(
                !matches!(
                    resolve_menu_entry(MenuKind::Root, label, false, false),
                    MenuDispatch::NotImplemented(_)
                ),
                "{label} must resolve"
            );
        }
    }

    #[test]
    fn bazaar_row_drills_the_stall_picker() {
        // Ungated page-2 arm 0x14E8A1 name-open of `menu    bazaar` (vault
        // main_menu.md §33).
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Bazaar", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::BazaarStalls)
        );
    }

    #[test]
    fn view_house_row_opens_the_mog_menu() {
        // Retail arm FFXiMain.dll retail-2026-09 RVA 0x14E8F5 name-opens
        // `menu    mogcont` (vault main_menu.md §32); kuluu routes the same
        // open through AgentCommand::OpenMogMenu at confirm time.
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "View House", false, false),
            MenuDispatch::OpenMogMenu
        );
    }

    /// The right-pane preview (`menu::root_child_kind`) and the drill dispatch
    /// share one Root → submenu mapping; pin that they can't drift apart.
    #[test]
    fn root_drill_matches_preview_child_kind() {
        use kuluu_render::hud::menu::{self, ROOT_CURRENT_TIME, ROOT_LOG_OUT, ROOT_SHUT_DOWN};
        for &label in menu::root_entries() {
            // Log Out / Shut Down open the yes/no prompt and Current Time, Key
            // Items and Quests run text commands - none of them has a right-pane
            // preview (vault main_menu.md §31 for the quest arm).
            // View House dispatches OpenMogMenu (dialog surface, no submenu),
            // like the power/text rows it takes no right-pane preview.
            if matches!(
                label,
                ROOT_LOG_OUT
                    | ROOT_SHUT_DOWN
                    | ROOT_CURRENT_TIME
                    | "Key Items"
                    | "Quests"
                    | "View House"
            ) {
                continue;
            }
            match (
                resolve_menu_entry(MenuKind::Root, label, false, false),
                menu::root_child_kind(label, false, false),
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
        use super::super::slash_apply::activate_current_time;
        use kuluu_render::hud::vana_clock::{
            VanaClockVisible, EARTH_TIME_LINE_PREFIX, VANA_TIME_LINE_PREFIX,
        };
        let clock = kuluu_render::vana_time::VanaClock::anchored_at_hour(12.0);
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

    /// Retail's Current Time row hands the runner its literal string
    /// (FFXiMain.dll retail-2026-09 RVA 0x14E98C, .data 0x371AB8); kuluu must
    /// route through the same text-command seam, not a bespoke intercept.
    #[test]
    fn current_time_row_runs_the_retail_clock_literal() {
        use kuluu_render::hud::menu::ROOT_CURRENT_TIME;
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, ROOT_CURRENT_TIME, false, false),
            MenuDispatch::TextCommand("/clock"),
        );
    }

    /// The runner seam (vault main_menu.md §12, bead kuluu-o9qt7.28): row literals go through
    /// the same `parse_slash` typed input uses - "/clock" reaches the widget toggle + time lines;
    /// an unknown word answers with the parser's own unknown-command line (the shim-specific
    /// refusal wording retired with the unification).
    #[test]
    fn text_command_runner_routes_clock_and_refuses_unknown() {
        use kuluu_render::hud::vana_clock::VANA_TIME_LINE_PREFIX;
        use kuluu_render::vana_time::VanaClock;

        let clock = VanaClock::anchored_at_hour(12.0);
        let mut visible = kuluu_render::hud::vana_clock::VanaClockVisible::default();
        let mut scene_state = SceneState::default();
        let mut stack = MenuStack::root();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let surface = crate::view_native::command_surface::CommandSurface::default();
        seam_call(
            "/clock",
            &surface,
            &mut stack,
            &clock,
            &mut visible,
            &mut scene_state,
            &cmd_tx,
        );
        // The widget's default is shown, so the retail toggle hides it.
        assert!(!visible.0, "the widget flipped off from its shown default");
        assert!(
            scene_state
                .local_toasts
                .iter()
                .any(|l| l.text.starts_with(VANA_TIME_LINE_PREFIX)),
            "{}",
            scene_state.local_toasts.len()
        );
        seam_call(
            "/nope",
            &surface,
            &mut stack,
            &clock,
            &mut visible,
            &mut scene_state,
            &cmd_tx,
        );
        assert!(scene_state
            .local_toasts
            .iter()
            .any(|l| { l.text.contains("unknown command") && l.text.contains("/nope") }));
    }

    /// The seam's arguments ride the snapshot and shared primitives (bead kuluu-o9qt7.28), so a
    /// test can drive it without any Bevy scaffolding.
    #[allow(clippy::too_many_arguments)]
    fn seam_call(
        cmd: &str,
        surface: &crate::view_native::command_surface::CommandSurface,
        stack: &mut MenuStack,
        clock: &kuluu_render::vana_time::VanaClock,
        visible: &mut kuluu_render::hud::vana_clock::VanaClockVisible,
        scene_state: &mut SceneState,
        cmd_tx: &Sender<AgentCommand>,
    ) {
        apply_menu_text_command(
            cmd,
            surface,
            stack,
            clock,
            visible,
            scene_state,
            cmd_tx,
            kuluu_snapshot::Vec3::default(),
            None,
        );
    }

    /// Retail's Key Items row runs "/keyitem" (FFXiMain.dll retail-2026-09 RVA
    /// 0x14E855, .data 0x371B40; vault §12 id 0x4C): the dispatch hands over the
    /// literal and the runner opens the key-item list window.
    #[test]
    fn key_items_row_runs_the_retail_keyitem_literal() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Key Items", false, false),
            MenuDispatch::TextCommand("/keyitem"),
        );

        let clock = kuluu_render::vana_time::VanaClock::anchored_at_hour(12.0);
        let mut visible = kuluu_render::hud::vana_clock::VanaClockVisible::default();
        let mut scene_state = SceneState::default();
        let mut stack = MenuStack::root();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let surface = crate::view_native::command_surface::CommandSurface::default();
        seam_call(
            "/keyitem",
            &surface,
            &mut stack,
            &clock,
            &mut visible,
            &mut scene_state,
            &cmd_tx,
        );
        assert_eq!(stack.current().map(|l| l.kind), Some(MenuKind::KeyItems));
    }

    /// Retail's Quests row runs "/quest" (FFXiMain.dll retail-2026-09 RVA
    /// 0x14E889, .data 0x371B24; vault main_menu.md §31 id-table entry 0x4D →
    /// handler RVA 0x7D240): the dispatch hands over the literal and the runner opens the
    /// quest00-stand-in list.
    #[test]
    fn quests_row_runs_the_retail_quest_literal() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Quests", false, false),
            MenuDispatch::TextCommand("/quest"),
        );

        let clock = kuluu_render::vana_time::VanaClock::anchored_at_hour(12.0);
        let mut visible = kuluu_render::hud::vana_clock::VanaClockVisible::default();
        let mut scene_state = SceneState::default();
        let mut stack = MenuStack::root();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let surface = crate::view_native::command_surface::CommandSurface::default();
        seam_call(
            "/quest",
            &surface,
            &mut stack,
            &clock,
            &mut visible,
            &mut scene_state,
            &cmd_tx,
        );
        assert_eq!(stack.current().map(|l| l.kind), Some(MenuKind::QuestList));
    }

    /// The unification (bead kuluu-o9qt7.28): the seam runs `parse_slash`, so a retail alias from
    /// the install's command table reaches its handler too, and both spellings of the clock parse
    /// to one outcome - state the same primitive applies (`set_current_time`).
    #[test]
    fn menu_text_command_shares_the_typed_parser_and_aliases() {
        use ffxi_dat::main_dll::{ClientCommand, CommandTable};

        let surface = crate::view_native::command_surface::CommandSurface::new(
            CommandTable::from_entries(vec![
                ClientCommand {
                    name: "clock".to_owned(),
                    id: 0x3f,
                    flags: 0,
                },
                ClientCommand {
                    name: "cl".to_owned(),
                    id: 0x3f,
                    flags: 0,
                },
            ]),
        );

        let clock = kuluu_render::vana_time::VanaClock::anchored_at_hour(12.0);
        let mut visible = kuluu_render::hud::vana_clock::VanaClockVisible::default();
        let mut scene_state = SceneState::default();
        let mut stack = MenuStack::root();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        seam_call(
            "/cl",
            &surface,
            &mut stack,
            &clock,
            &mut visible,
            &mut scene_state,
            &cmd_tx,
        );
        // Shown by default: flipping off proves the alias landed on the clock handler.
        assert!(
            !visible.0,
            "the alias reached the clock handler through parse_slash"
        );

        // Both spellings produce exactly one typed outcome off the same snapshot args.
        let gate = kuluu_render::fishing_spot::gate_from_snapshot(&scene_state.snapshot);
        for spelling in ["/clock", "/cl"] {
            let out = parse_slash(
                spelling,
                &surface,
                &scene_state.snapshot.entities,
                kuluu_snapshot::Vec3::default(),
                None,
                scene_state.snapshot.zone_id,
                scene_state.snapshot.self_char_id,
                &scene_state.snapshot.party,
                gate,
                None,
                scene_state.snapshot.self_pet_targid,
            );
            assert!(
                matches!(out, SlashOutcome::SetVanaClock(None)),
                "{spelling}: the seam's source of truth is one outcome, got {out:?}"
            );
        }
    }

    /// End to end (bead kuluu-o9qt7.28): confirming the Root Current Time row - the seam both
    /// the keyboard and mouse paths share - lands the typed `/clock` state: widget flips, both
    /// time lines print, menu stays open on the same level.
    #[test]
    fn current_time_row_confirm_matches_typed_clock() {
        use crate::keybinds_store::KeybindsStore;
        use kuluu_render::hud::menu::ROOT_CURRENT_TIME;

        let mut bindings = Bindings::default();
        // Root at cursor 0, like InputMode::Menu(MenuStack::root()).
        let mut stack = MenuStack::root();
        let row = kuluu_render::hud::menu::root_entries()
            .iter()
            .position(|e| *e == ROOT_CURRENT_TIME)
            .expect("Current Time is a root row");
        stack.current_mut().unwrap().cursor = row;
        let mut scene_state = SceneState::default();
        let (cmd_tx, _rx) = tokio::sync::mpsc::channel::<AgentCommand>(4);
        let mut keybinds_state = KeybindsStateRes {
            store: KeybindsStore::new(std::env::temp_dir().join("kuluu-clock-seam-tests.json")),
            persisted: Default::default(),
        };
        let mut graphics = kuluu_render::GraphicsSettings::default();
        let mut status_profile_open = Default::default();
        let mut hud_panels = Default::default();
        let mut net_status = Default::default();
        let mut audio_mute = Default::default();
        let mut vana_clock = kuluu_render::vana_time::VanaClock::anchored_at_hour(12.0);
        let mut vana_clock_visible = Default::default();
        let dynamic = kuluu_render::hud::menu::DynamicMenu::default();
        let surface = crate::view_native::command_surface::CommandSurface::default();

        let mut item_bag = kuluu_render::hud::item_screen::ItemScreenContainer::default();
        let next = confirm_menu_at_cursor(
            &mut bindings,
            &mut stack,
            &mut scene_state,
            &cmd_tx,
            &mut keybinds_state,
            &mut graphics,
            &mut status_profile_open,
            &mut hud_panels,
            &mut net_status,
            &mut audio_mute,
            &mut vana_clock,
            &mut vana_clock_visible,
            &dynamic,
            None,
            kuluu_snapshot::Vec3::default(),
            &surface,
            &mut item_bag,
        );

        assert!(next.is_none(), "the menu stays open");
        assert_eq!(stack.current().map(|l| l.kind), Some(MenuKind::Root));
        // Shown by default: flipping off is the toggle landing (bead kuluu-o9qt7.28).
        assert!(!vana_clock_visible.0, "widget flipped like typed /clock");
        use kuluu_render::hud::vana_clock::{EARTH_TIME_LINE_PREFIX, VANA_TIME_LINE_PREFIX};
        let lines: Vec<&str> = scene_state
            .local_toasts
            .iter()
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(lines.len(), 2, "both retail time lines print: {lines:?}");
        assert!(
            lines[0].starts_with(VANA_TIME_LINE_PREFIX)
                && lines[1].starts_with(EARTH_TIME_LINE_PREFIX),
            "{lines:?}"
        );
    }

    /// Retail persona answers no command events (FFXiMain.dll retail-2026-09 RVA 0x12AC60), so the
    /// Status panel is passive by default; kuluu's actionable rows sit behind the "Status Rows"
    /// Retail+ toggle, flipped via the shared Debug-row handler.
    #[test]
    fn status_rows_passive_by_default_with_retail_plus_toggle() {
        use kuluu_render::GraphicsSettings;

        let mut graphics = GraphicsSettings::default();
        assert!(
            !graphics.interactive_status_rows,
            "retail default: persona rows are display-only"
        );
        let mut scene_state = SceneState::default();
        assert!(handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_STATUS_ROWS,
            &mut graphics,
            &mut scene_state,
        ));
        assert!(graphics.interactive_status_rows, "toggle flips on");
        handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_STATUS_ROWS,
            &mut graphics,
            &mut scene_state,
        );
        assert!(!graphics.interactive_status_rows, "flips back off");
    }

    /// Retail's equipment window equips on every select case - no toggle-off branch in the
    /// command-event jump table (FFXiMain.dll retail-2026-09 RVA 0x1322B0 / 0x132608, vault
    /// main_menu.md §20) - so reselect-unequip is off by default and flips only through the
    /// Debug/Retail+ "Reselect Unequip" row.
    #[test]
    fn reselect_unequip_off_by_default_with_retail_plus_toggle() {
        use kuluu_render::GraphicsSettings;

        let mut graphics = GraphicsSettings::default();
        assert!(
            !graphics.reselect_unequip,
            "retail default: re-selecting the equipped item just re-equips"
        );
        let mut scene_state = SceneState::default();
        assert!(handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_RESELECT_UNEQUIP,
            &mut graphics,
            &mut scene_state,
        ));
        assert!(graphics.reselect_unequip, "toggle flips on");
        handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_RESELECT_UNEQUIP,
            &mut graphics,
            &mut scene_state,
        );
        assert!(!graphics.reselect_unequip, "flips back off");
    }

    /// Retail's Magic row is the two-window flow (vault main_menu.md §21): Root → mgcmenu picker →
    /// school-filtered `magic` list. The flat one-step list only exists with the Debug/Retail+
    /// "Flat Magic List" toggle, which is off by default and flips through that row.
    #[test]
    fn magic_drills_the_retail_picker_and_the_flat_toggle_moves_it() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Magic", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::MagicGroups)
        );
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Magic", true, false),
            MenuDispatch::OpenSubmenu(MenuKind::Magic)
        );

        let mut graphics = kuluu_render::GraphicsSettings::default();
        assert!(
            !graphics.magic_flat_list,
            "retail default: Magic goes through mgcmenu"
        );
        let mut scene_state = SceneState::default();
        assert!(handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_MAGIC_FLAT_LIST,
            &mut graphics,
            &mut scene_state,
        ));
        assert!(graphics.magic_flat_list, "toggle flips on");
        handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_MAGIC_FLAT_LIST,
            &mut graphics,
            &mut scene_state,
        );
        assert!(!graphics.magic_flat_list, "flips back off");
    }

    /// Picking a mgcmenu category drills to that school's spell list on the §21 numbers
    /// (display row 3 is Songs = 5, row 5 Summoning = 3).
    #[test]
    fn mgcmenu_category_select_opens_the_matching_spell_list() {
        for (label, school) in [
            ("White Magic", 1u8),
            ("Black Magic", 2),
            ("Songs", 5),
            ("Ninjutsu", 4),
            ("Summoning", 3),
            ("Blue Magic", 6),
            ("Geomancy", 7),
            ("Trust Magic", 8),
        ] {
            assert_eq!(
                resolve_menu_entry(MenuKind::MagicGroups, label, false, false),
                MenuDispatch::OpenSubmenu(MenuKind::MagicGroup(school)),
                "{label} should open school {school}",
            );
        }
    }

    /// The Abilities row follows the same two-window law (vault main_menu.md §23):
    /// Root → abimenu picker → group-filtered list. Kuluu's flat one-step ability
    /// list survives behind the Debug/Retail+ "Flat Abilities List" toggle, off by
    /// default like its magic twin.
    #[test]
    fn abilities_drill_the_retail_picker_and_the_flat_toggle_moves_it() {
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Abilities", false, false),
            MenuDispatch::OpenSubmenu(MenuKind::AbilitiesGroups)
        );
        assert_eq!(
            resolve_menu_entry(MenuKind::Root, "Abilities", false, true),
            MenuDispatch::OpenSubmenu(MenuKind::Abilities)
        );

        let mut graphics = kuluu_render::GraphicsSettings::default();
        assert!(
            !graphics.abilities_flat_list,
            "retail default: Abilities goes through abimenu"
        );
        let mut scene_state = SceneState::default();
        assert!(handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_ABILITIES_FLAT_LIST,
            &mut graphics,
            &mut scene_state,
        ));
        assert!(graphics.abilities_flat_list, "toggle flips on");
        handle_retail_plus_row(
            kuluu_render::hud::menu::RETAIL_ABILITIES_FLAT_LIST,
            &mut graphics,
            &mut scene_state,
        );
        assert!(!graphics.abilities_flat_list, "flips back off");
    }

    /// Picking an abimenu category drills to that group's ability list; the picker
    /// rows pair positionally with `AbilityGroup::ALL` (the [I-pending] grounding
    /// recorded on bead kuluu-o9qt7.7 and vault main_menu.md §23).
    #[test]
    fn abimenu_category_select_opens_the_matching_ability_list() {
        use kuluu_render::hud::action_model::AbilityGroup;
        assert_eq!(
            kuluu_render::hud::menu::ABILITIES_CATEGORY_LABELS.len(),
            AbilityGroup::ALL.len()
        );
        for (idx, label) in kuluu_render::hud::menu::ABILITIES_CATEGORY_LABELS
            .iter()
            .enumerate()
        {
            assert_eq!(
                resolve_menu_entry(MenuKind::AbilitiesGroups, label, false, false),
                MenuDispatch::OpenSubmenu(MenuKind::AbilitiesGroup(idx as u8)),
                "{label} should open group {idx}",
            );
        }
    }

    /// Send-panel shape: choice 0 = recipient row (above the grid), choices
    /// 1..=8 = the 2x4 slot grid, choice 9 = Cancel (below the grid).
    fn send_panel_grid() -> kuluu_snapshot::DialogGrid {
        kuluu_snapshot::DialogGrid {
            cols: 4,
            rows: 2,
            cells: (0..8u32)
                .map(|i| kuluu_snapshot::DialogGridCell {
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
    fn every_menu_action_takes_the_sub_target_confirm() {
        use kuluu_render::hud::menu::DynamicMenuAction as A;
        use kuluu_render::input_mode::SubTargetAction as S;
        // Boost (39) and Mighty Strikes (16) are validTarget SELF and still prompt.
        assert_eq!(
            sub_target_action_for(A::JobAbility { ability_id: 39 }),
            Some(S::Ability(39))
        );
        assert_eq!(
            sub_target_action_for(A::JobAbility { ability_id: 16 }),
            Some(S::Ability(16))
        );
        assert_eq!(
            sub_target_action_for(A::JobAbility { ability_id: 35 }),
            Some(S::Ability(35))
        );
        assert_eq!(
            sub_target_action_for(A::CastSpell { spell_id: 1 }),
            Some(S::Spell(1))
        );
        assert_eq!(sub_target_action_for(A::Dismount), None);
    }
}
