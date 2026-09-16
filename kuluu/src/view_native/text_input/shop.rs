use super::*;

use kuluu_render::hud::shop::{
    self, ShopFocus, ShopMode, ShopRow, ShopScreenState, SHOP_NO, VENDOR_RANGE_YALMS,
};
use kuluu_render::hud::spinner::Spinner;

/// Keeps the shop window in step with the world around it. The shop has no
/// close packet (research/XiPackets client 0x0082 is deprecated and GM-only),
/// so every exit is decided here:
/// - the window opens when the stock arrives and closes when it goes away;
/// - a zone change takes the vendor with it (the retail shop table lives in
///   GC_ZONE — research/XIClient GC_ZONE::gcShop), and a zone change is not an
///   `AppPhase` transition, so no state-exit cleanup fires for us (kuluu-oe8y);
/// - walking out of the vendor's trigger range ends the conversation, the same
///   distance LSB requires to start one
///   (vendor/server/src/map/packets/c2s/0x01a_action.cpp Trigger).
pub fn shop_mode_sync_system(
    state: Res<SceneState>,
    cmd_tx: Res<CommandTx>,
    mut mode: ResMut<InputMode>,
    mut screen: ResMut<ShopScreenState>,
    mut last_zone: Local<Option<u16>>,
) {
    let zone = state.snapshot.zone_id;
    let zoned = zone.is_some() && last_zone.is_some() && *last_zone != zone;
    if zone.is_some() {
        *last_zone = zone;
    }

    let Some(shop) = state.snapshot.shop.as_ref() else {
        if screen.dismissed || matches!(*mode, InputMode::Shop) {
            screen.reset();
        }
        if matches!(*mode, InputMode::Shop) {
            *mode = InputMode::World;
        }
        return;
    };

    // Closed on this side already; the snapshot just has not caught up. Do not
    // reopen the window on top of the player.
    if screen.dismissed {
        if matches!(*mode, InputMode::Shop) {
            *mode = InputMode::World;
        }
        return;
    }

    if zoned {
        close_shop(&cmd_tx, &mut mode, &mut screen);
        return;
    }

    if !matches!(*mode, InputMode::Shop) {
        // A shop that opened while another window owned the cursor still waits
        // behind it; taking focus is only right from the world.
        if matches!(*mode, InputMode::World) {
            screen.reset();
            *mode = InputMode::Shop;
        }
        return;
    }

    if vendor_out_of_range(&state.snapshot, shop.vendor_id) {
        close_shop(&cmd_tx, &mut mode, &mut screen);
        return;
    }

    let len = shop::rows_for(screen.mode, &state.snapshot).len();
    screen.clamp(len);
}

fn close_shop(cmd_tx: &CommandTx, mode: &mut InputMode, screen: &mut ShopScreenState) {
    let _ = cmd_tx.0.try_send(AgentCommand::CloseShop);
    screen.dismiss();
    *mode = InputMode::World;
}

/// Whether the vendor has left the player's reach. A shop with no resolved
/// vendor (`vendor_id == 0`, e.g. one opened by a server-driven menu rather
/// than a Talk) has no anchor to measure against and stays open.
fn vendor_out_of_range(snap: &kuluu_snapshot::SceneSnapshot, vendor_id: u32) -> bool {
    if vendor_id == 0 {
        return false;
    }
    match snap.entities.iter().find(|e| e.id == vendor_id) {
        Some(vendor) => out_of_reach(snap.self_pos.pos, vendor.pos),
        // The vendor despawned or fell out of the entity table.
        None => true,
    }
}

fn out_of_reach(me: kuluu_snapshot::Vec3, vendor: kuluu_snapshot::Vec3) -> bool {
    let (dx, dy, dz) = (vendor.x - me.x, vendor.y - me.y, vendor.z - me.z);
    (dx * dx + dy * dy + dz * dz).sqrt() > VENDOR_RANGE_YALMS
}

/// Keyboard handling for the shop window. Cancel unwinds exactly one level per
/// press — confirm box -> list -> Buy/Sell picker -> closed — matching how the
/// retail primitives nest (`shopmain` over `shopbuy`/`shopsell`).
pub(super) fn handle_shop_key(
    key: &Key,
    bindings: &Bindings,
    screen: &mut ShopScreenState,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    if scene_state.snapshot.shop.is_none() || screen.dismissed {
        return Some(InputMode::World);
    }
    let rows = shop::rows_for(screen.mode, &scene_state.snapshot);

    match screen.focus {
        ShopFocus::Menu => handle_menu_key(key, bindings, screen, cmd_tx),
        ShopFocus::List => {
            handle_list_key(key, bindings, screen, scene_state, cmd_tx, &rows);
            None
        }
        ShopFocus::Quantity => {
            handle_quantity_key(key, bindings, screen, scene_state, cmd_tx, &rows);
            None
        }
        ShopFocus::Confirm => {
            handle_confirm_key(key, bindings, screen, scene_state, cmd_tx);
            None
        }
    }
}

fn handle_menu_key(
    key: &Key,
    bindings: &Bindings,
    screen: &mut ShopScreenState,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    if bindings.matches_logical(Action::NavUp, key) {
        screen.move_menu_cursor(-1);
        return None;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        screen.move_menu_cursor(1);
        return None;
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        let _ = cmd_tx.try_send(AgentCommand::CloseShop);
        screen.dismiss();
        return Some(InputMode::World);
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        screen.enter_list();
    }
    None
}

fn handle_list_key(
    key: &Key,
    bindings: &Bindings,
    screen: &mut ShopScreenState,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    rows: &[ShopRow],
) {
    if bindings.matches_logical(Action::NavUp, key) {
        screen.move_cursor(-1, rows.len());
        return;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        screen.move_cursor(1, rows.len());
        return;
    }
    // Retail's item lists page ten rows on Left/Right
    // (.agents/skills/retail-observe/references/2026-09-11-items-window.md).
    if bindings.matches_logical(Action::NavLeft, key) {
        screen.page(-1, rows.len());
        return;
    }
    if bindings.matches_logical(Action::NavRight, key) {
        screen.page(1, rows.len());
        return;
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        screen.focus = ShopFocus::Menu;
        return;
    }
    if !bindings.matches_logical(Action::NavConfirm, key) {
        return;
    }
    let Some(row) = rows.get(screen.cursor).copied() else {
        return;
    };
    match begin_quantity(screen.mode, &row) {
        Some(spinner) => {
            // A sell row carries no price of its own, so the first confirm buys
            // the quote: appraise a single unit and let the picker show what
            // each one is worth before the player commits to a count.
            if matches!(screen.mode, ShopMode::Sell) {
                request_appraisal(cmd_tx, &row, 1);
            }
            screen.quantity = Some(spinner);
            screen.focus = ShopFocus::Quantity;
        }
        None => commit_quantity(screen, scene_state, cmd_tx, &row, 1),
    }
}

fn request_appraisal(cmd_tx: &Sender<AgentCommand>, row: &ShopRow, qty: u32) {
    let _ = cmd_tx.try_send(AgentCommand::ShopSellReq {
        qty,
        item_no: row.item_no,
        item_index: row.index,
    });
}

fn handle_quantity_key(
    key: &Key,
    bindings: &Bindings,
    screen: &mut ShopScreenState,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    rows: &[ShopRow],
) {
    let Some(spinner) = screen.quantity.as_mut() else {
        screen.focus = ShopFocus::List;
        return;
    };
    if bindings.matches_logical(Action::NavConfirm, key) {
        let quantity = spinner.confirm();
        match rows.get(screen.cursor).copied() {
            Some(row) => commit_quantity(screen, scene_state, cmd_tx, &row, quantity),
            None => {
                screen.quantity = None;
                screen.focus = ShopFocus::List;
            }
        }
        return;
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        if matches!(screen.mode, ShopMode::Sell) {
            let _ = cmd_tx.try_send(AgentCommand::ShopSellCancel);
        }
        screen.quantity = None;
        screen.focus = ShopFocus::List;
        return;
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
}

fn handle_confirm_key(
    key: &Key,
    bindings: &Bindings,
    screen: &mut ShopScreenState,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
) {
    if bindings.matches_logical(Action::NavCancel, key) {
        if matches!(screen.mode, ShopMode::Sell) {
            let _ = cmd_tx.try_send(AgentCommand::ShopSellCancel);
        }
        screen.pending_buy = None;
        screen.focus = ShopFocus::List;
        return;
    }
    if !bindings.matches_logical(Action::NavConfirm, key) {
        return;
    }
    match screen.mode {
        ShopMode::Buy => {
            let Some(buy) = screen.pending_buy.take() else {
                screen.focus = ShopFocus::List;
                return;
            };
            let _ = cmd_tx.try_send(AgentCommand::ShopBuy {
                shop_no: SHOP_NO,
                shop_index: buy.shop_index,
                qty: buy.quantity,
            });
            screen.focus = ShopFocus::List;
        }
        ShopMode::Sell => {
            // The appraisal is what the player is answering; until it lands
            // there is no price to say yes to.
            if shop_pending_sale(scene_state).is_none() {
                return;
            }
            let _ = cmd_tx.try_send(AgentCommand::ShopSellConfirm);
            screen.focus = ShopFocus::List;
        }
    }
}

fn shop_pending_sale(scene_state: &SceneState) -> Option<&kuluu_snapshot::ShopSale> {
    scene_state.snapshot.shop.as_ref()?.pending_sale.as_ref()
}

/// The quantity picker for a row, or `None` when there is only one to move —
/// retail commits a lone item outright rather than asking for a count. Buy
/// stacks are bounded by the item's stack size (LSB clamps anything larger:
/// vendor/server/src/map/packets/c2s/0x083_shop_buy.cpp process); sell stacks
/// by what the player is holding.
fn begin_quantity(mode: ShopMode, row: &ShopRow) -> Option<Spinner> {
    let max = match mode {
        ShopMode::Buy => ffxi_vocab::item_flags::stack_size(row.item_no) as u32,
        ShopMode::Sell => row.quantity,
    };
    (max > 1).then(|| Spinner::item(max))
}

/// Take the sized amount into the priced step. A buy prices itself from the
/// listed unit price; a sell has to ask the server, so it sends the 0x084
/// appraisal and waits for the 0x03D answer to fill the prompt in.
fn commit_quantity(
    screen: &mut ShopScreenState,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    row: &ShopRow,
    quantity: u32,
) {
    match screen.mode {
        ShopMode::Buy => {
            let buy = screen.stage_buy(row, quantity);
            let name = kuluu_render::hud::bazaar_view::item_name(buy.item_no, None);
            push_system_chat_line(
                scene_state,
                shop::purchase_prompt(&name, buy.quantity, buy.total_gil),
            );
        }
        ShopMode::Sell => {
            screen.quantity = None;
            screen.focus = ShopFocus::Confirm;
            // Re-price at the chosen count: the quote shown while sizing was
            // for one unit, and the confirm prompt states the whole sale.
            request_appraisal(cmd_tx, row, quantity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuluu_snapshot::{SceneSnapshot, Vec3};

    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    #[test]
    fn the_vendor_leaving_reach_ends_the_conversation() {
        let me = at(0.0, 0.0, 0.0);
        assert!(!out_of_reach(me, at(VENDOR_RANGE_YALMS - 0.5, 0.0, 0.0)));
        assert!(out_of_reach(me, at(VENDOR_RANGE_YALMS + 0.5, 0.0, 0.0)));
    }

    #[test]
    fn range_is_measured_in_three_dimensions() {
        let me = at(0.0, 0.0, 0.0);
        assert!(
            out_of_reach(me, at(0.0, VENDOR_RANGE_YALMS + 1.0, 0.0)),
            "a vendor a floor away is out of reach"
        );
        assert!(
            out_of_reach(me, at(4.0, 0.0, 5.0)),
            "the diagonal is what counts, not either axis"
        );
    }

    #[test]
    fn a_vendor_missing_from_the_zone_closes_the_window() {
        let snap = SceneSnapshot::default();
        assert!(vendor_out_of_range(&snap, 7), "vendor is no longer spawned");
    }

    #[test]
    fn a_shop_with_no_resolved_vendor_is_never_range_closed() {
        let snap = SceneSnapshot::default();
        assert!(!vendor_out_of_range(&snap, 0));
    }

    /// Closing is a client decision that takes a round trip to reach the
    /// snapshot. Until it lands, the sync system must leave the still-present
    /// stock alone instead of reopening the window on top of the player — that
    /// oscillation strobed the window and its help bar every frame.
    #[test]
    fn a_dismissed_shop_is_not_reopened_while_the_stock_lingers() {
        let mut app = App::new();
        app.init_resource::<InputMode>()
            .init_resource::<ShopScreenState>()
            .insert_resource(CommandTx(tokio::sync::mpsc::channel(8).0))
            .insert_resource(SceneState {
                snapshot: SceneSnapshot {
                    zone_id: Some(245),
                    shop: Some(kuluu_snapshot::ShopState {
                        opened: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            });
        app.add_systems(Update, shop_mode_sync_system);

        app.update();
        assert!(
            matches!(*app.world().resource::<InputMode>(), InputMode::Shop),
            "a live shop takes the cursor"
        );

        app.world_mut().resource_mut::<ShopScreenState>().dismiss();
        *app.world_mut().resource_mut::<InputMode>() = InputMode::World;

        for _ in 0..5 {
            app.update();
            assert!(
                matches!(*app.world().resource::<InputMode>(), InputMode::World),
                "the window must stay closed until the snapshot catches up"
            );
        }

        // The session finally clears the stock: the latch lifts.
        app.world_mut().resource_mut::<SceneState>().snapshot.shop = None;
        app.update();
        assert!(!app.world().resource::<ShopScreenState>().dismissed);
    }
}
