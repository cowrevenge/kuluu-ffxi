use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_world_key(
    key: &Key,
    bindings: &Bindings,
    current_target: Option<u32>,
    entities: &[ffxi_viewer_wire::Entity],
    self_pos: ffxi_viewer_wire::Vec3,
    self_id: Option<u32>,
    target_changed: bool,
    engaged: bool,
    usable_items_available: bool,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    if bindings.matches_logical(Action::OpenChat, key) {
        return Some(InputMode::Chat(ChatBuffer::empty()));
    }
    if bindings.matches_logical(Action::ConfirmAction, key) {
        return match current_target {
            Some(_) if target_changed => None,
            Some(id) => {
                let ent = entities.iter().find(|e| e.id == id);
                let is_npc = matches!(ent.map(|e| e.kind), Some(ffxi_viewer_wire::EntityKind::Npc));
                let in_range = ent.is_some_and(|e| {
                    let dx = e.pos.x - self_pos.x;
                    let dy = e.pos.y - self_pos.y;
                    let dz = e.pos.z - self_pos.z;
                    let r = ffxi_viewer_core::hud::action_model::NPC_INTERACT_YALMS;
                    dx * dx + dy * dy + dz * dz <= r * r
                });
                if is_npc {
                    if let (true, Some(ent)) = (in_range, ent) {
                        let _ = cmd_tx.try_send(AgentCommand::Action {
                            target_id: ent.id,
                            target_index: ent.act_index,
                            kind: ActionKind::Talk,
                        });
                    }
                    None
                } else {
                    open_target_action_menu(
                        current_target,
                        entities,
                        self_pos,
                        self_id,
                        engaged,
                        usable_items_available,
                    )
                }
            }
            None => None,
        };
    }
    None
}

fn open_target_action_menu(
    current_target: Option<u32>,
    entities: &[ffxi_viewer_wire::Entity],
    self_pos: ffxi_viewer_wire::Vec3,
    self_id: Option<u32>,
    engaged: bool,
    usable_items_available: bool,
) -> Option<InputMode> {
    use ffxi_viewer_core::hud::action_model;
    let ctx = action_model::context_for_target(
        current_target,
        entities,
        self_pos,
        self_id,
        engaged,
        usable_items_available,
    );
    if action_model::build_target_action_entries(&ctx, &ffxi_viewer_core::hud::overlay::RETAIL)
        .is_empty()
    {
        return None;
    }
    Some(InputMode::TargetAction(
        ffxi_viewer_core::input_mode::TargetActionState::open(ctx),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_target_action_key(
    key: &Key,
    bindings: &Bindings,
    state: &mut ffxi_viewer_core::input_mode::TargetActionState,
    scene_state: &mut SceneState,
    current_target: Option<u32>,
    entities: &[ffxi_viewer_wire::Entity],
    cmd_tx: &Sender<AgentCommand>,
    check_target: &mut ffxi_viewer_core::hud::check_view::CheckTarget,
    trade_state: &mut ffxi_viewer_core::hud::trade::TradeState,
    trade_intent: &mut MessageWriter<ffxi_viewer_core::hud::trade::TradeIntent>,
    select_target: &mut SelectTargetMode,
) -> Option<InputMode> {
    use ffxi_viewer_core::hud::action_model::{ActionEntryKind, TargetActionId};
    use ffxi_viewer_core::input_mode::SubAction;

    if trade_state.open {
        return handle_trade_key(key, bindings, trade_state, trade_intent, scene_state);
    }

    if let Some(SubAction::AbilitiesGroup(group)) = state.sub.as_ref().and_then(|s| s.current()) {
        return handle_abilities_group_key(
            key,
            bindings,
            state,
            group,
            scene_state,
            current_target,
            entities,
            cmd_tx,
        );
    }

    let entries = ffxi_viewer_core::hud::overlay::RETAIL.resolve_target_actions(&state.ctx);
    let count = entries.len();
    if count == 0 {
        return Some(InputMode::World);
    }
    if state.cursor >= count {
        state.cursor = count - 1;
    }

    if bindings.matches_logical(Action::NavUp, key) {
        state.cursor = if state.cursor == 0 {
            count - 1
        } else {
            state.cursor - 1
        };
        return None;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        let next = state.cursor + 1;
        state.cursor = if next >= count { 0 } else { next };
        return None;
    }
    if bindings.matches_logical(Action::NavRight, key) {
        if let Some(entry) = entries.get(state.cursor) {
            if let ActionEntryKind::Select { modes, .. } = &entry.kind {
                if !modes.is_empty() {
                    match entry.id {
                        TargetActionId::Chat => {
                            state.chat_mode_idx = (state.chat_mode_idx + 1) % modes.len();
                        }
                        TargetActionId::Abilities => {
                            state.abilities_group_idx =
                                (state.abilities_group_idx + 1) % modes.len();
                        }
                        _ => {}
                    }
                }
            }
        }
        return None;
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        return confirm_target_action_at_cursor(
            state,
            &entries,
            scene_state,
            current_target,
            entities,
            cmd_tx,
            check_target,
            trade_state,
            select_target,
        );
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        if check_target.open {
            check_target.open = false;
            check_target.target_id = None;
        }
        return Some(InputMode::World);
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn confirm_target_action_at_cursor(
    state: &mut ffxi_viewer_core::input_mode::TargetActionState,
    entries: &[ffxi_viewer_core::hud::action_model::ActionEntry],
    scene_state: &mut SceneState,
    current_target: Option<u32>,
    entities: &[ffxi_viewer_wire::Entity],
    cmd_tx: &Sender<AgentCommand>,
    check_target: &mut ffxi_viewer_core::hud::check_view::CheckTarget,
    trade_state: &mut ffxi_viewer_core::hud::trade::TradeState,
    select_target: &mut SelectTargetMode,
) -> Option<InputMode> {
    use ffxi_viewer_core::hud::action_model::TargetActionId;

    let Some(entry) = entries.get(state.cursor) else {
        return Some(InputMode::World);
    };
    if !entry.enabled {
        if let Some(hint) = &entry.hint {
            push_system_chat_line(scene_state, format!("[menu] {hint}"));
        }
        return None;
    }

    let target_ent = current_target.and_then(|id| entities.iter().find(|e| e.id == id));
    match entry.id {
        TargetActionId::Attack => {
            match target_ent {
                Some(e) => {
                    if let Err(err) = cmd_tx.try_send(AgentCommand::Engage { target_id: e.id }) {
                        push_system_chat_line(
                            scene_state,
                            format!("[menu] Attack dispatch dropped: {err}"),
                        );
                    }
                }
                None => push_system_chat_line(scene_state, "[menu] Attack: no target".to_string()),
            }
            Some(InputMode::World)
        }
        TargetActionId::SwitchTarget => {
            select_target.active = true;
            select_target.prev = current_target;
            push_system_chat_line(
                scene_state,
                "[menu] Switch Target — Tab to cycle, Enter to confirm, Esc to cancel".to_string(),
            );
            Some(InputMode::World)
        }
        TargetActionId::Disengage => {
            if let Err(err) = cmd_tx.try_send(AgentCommand::Cancel) {
                push_system_chat_line(
                    scene_state,
                    format!("[menu] Disengage dispatch dropped: {err}"),
                );
            }
            Some(InputMode::World)
        }
        TargetActionId::Chat => Some(InputMode::Chat(chat_buffer_for_mode(
            state.chat_mode_idx,
            target_ent,
        ))),
        TargetActionId::Magic => Some(open_submenu(MenuKind::Magic)),
        TargetActionId::Abilities => {
            use ffxi_viewer_core::hud::action_model::AbilityGroup;
            use ffxi_viewer_core::input_mode::{SubAction, SubActionStack};
            let group = AbilityGroup::ALL[state.abilities_group_idx % AbilityGroup::ALL.len()];
            state.sub = Some(SubActionStack::with(SubAction::AbilitiesGroup(group)));
            None
        }
        TargetActionId::Items => Some(open_submenu(MenuKind::UsableItems)),
        TargetActionId::Check => {
            use ffxi_viewer_core::hud::action_model::TargetKindLite;
            match target_ent {
                Some(e) => {
                    let cmd = AgentCommand::CheckTarget {
                        target_id: e.id,
                        target_index: e.act_index,
                        kind: CheckKind::Check,
                    };
                    if let Err(err) = cmd_tx.try_send(cmd) {
                        push_system_chat_line(
                            scene_state,
                            format!("[menu] Check dispatch dropped: {err}"),
                        );
                    }

                    let is_pc = matches!(
                        state.ctx.target_kind,
                        TargetKindLite::Pc | TargetKindLite::SelfPc
                    );
                    if is_pc {
                        check_target.open = true;
                        check_target.target_id = Some(e.id);
                        None
                    } else {
                        Some(InputMode::World)
                    }
                }
                None => {
                    push_system_chat_line(scene_state, "[menu] Check: no target".into());
                    Some(InputMode::World)
                }
            }
        }
        TargetActionId::Trade => match target_ent {
            Some(e) => {
                *trade_state = ffxi_viewer_core::hud::trade::TradeState::open(e.id);
                None
            }
            None => {
                push_system_chat_line(scene_state, "[menu] Trade: no target".into());
                Some(InputMode::World)
            }
        },
        TargetActionId::Trust => {
            push_system_chat_line(scene_state, "[menu] Trust — not implemented yet".into());
            Some(InputMode::World)
        }
        TargetActionId::Open => {
            match target_ent {
                Some(e) => {
                    // Doors are TYPE_NPC server-side (look.size == 0x02) and
                    // trigger through the same Talk action_id as any other
                    // NPC — vendor/server/src/map/packets/c2s/0x01a_action.cpp:198,213.
                    // The server's own door script drives the yes/no confirm
                    // and zone change; nothing door-specific is needed here.
                    let cmd = AgentCommand::Action {
                        target_id: e.id,
                        target_index: e.act_index,
                        kind: ActionKind::Talk,
                    };
                    if let Err(err) = cmd_tx.try_send(cmd) {
                        push_system_chat_line(
                            scene_state,
                            format!("[menu] Open dispatch dropped: {err}"),
                        );
                    }
                }
                None => push_system_chat_line(scene_state, "[menu] Open: no target".to_string()),
            }
            Some(InputMode::World)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_abilities_group_key(
    key: &Key,
    bindings: &Bindings,
    state: &mut ffxi_viewer_core::input_mode::TargetActionState,
    group: ffxi_viewer_core::hud::action_model::AbilityGroup,
    scene_state: &mut SceneState,
    current_target: Option<u32>,
    entities: &[ffxi_viewer_wire::Entity],
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    let rows = ffxi_viewer_core::hud::menu::ability_group_rows(&scene_state.snapshot, group);
    let count = rows.len();

    let sub = state.sub.as_mut()?;
    if count > 0 && sub.cursor >= count {
        sub.cursor = count - 1;
    }

    if bindings.matches_logical(Action::NavUp, key) {
        if count > 0 {
            sub.cursor = if sub.cursor == 0 {
                count - 1
            } else {
                sub.cursor - 1
            };
        }
        return None;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        if count > 0 {
            let next = sub.cursor + 1;
            sub.cursor = if next >= count { 0 } else { next };
        }
        return None;
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        if let Some(row) = rows.get(sub.cursor) {
            let action = row.action;
            if let Some(sub_action) = sub_target_action_for(action) {
                if !selected_target_valid(sub_action, current_target, scene_state) {
                    // No valid target selected: retail's flashing sub-target
                    // cursor asks "on whom?" first. Esc returns here with the
                    // menu cursor preserved.
                    let return_to = InputMode::TargetAction(state.clone());
                    return open_sub_target(sub_action, current_target, scene_state, return_to);
                }
            }
            let self_pos = scene_state.snapshot.self_pos.pos;
            dispatch_dynamic_menu_action(
                action,
                current_target,
                self_pos,
                entities,
                cmd_tx,
                scene_state,
            );
            return Some(InputMode::World);
        }

        return None;
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        if !sub.pop() {
            state.sub = None;
        }
        return None;
    }
    None
}

fn open_submenu(kind: MenuKind) -> InputMode {
    let mut stack = MenuStack::root();
    stack.push(kind);
    InputMode::Menu(stack)
}
