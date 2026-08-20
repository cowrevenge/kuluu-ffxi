use super::*;

use kuluu_render::hud::auction::{
    self, AhNode, AhRegion, AhScreen, AuctionScreenState, AuctionSellInventory, CatalogOverlay,
    HistoryReturn, SellPick, SortChoice, YesNo,
};
use kuluu_render::hud::digit_spinner::DigitSpinner;

/// Drive `InputMode` in lock-step with the AH counter:
/// - the s2c Open push arrives as an edge-triggered `ViewerEvent` (the
///   snapshot's `open` flag stays true until zone change, so a repeat visit is
///   only visible on the event stream);
/// - zone change resets the session's auction state, closing the UI;
/// - a landed fee quote promotes Price Set to the fee confirm dialog.
pub fn auction_mode_sync_system(
    state: Res<SceneState>,
    events: Res<kuluu_render::EventLog>,
    mut cursor: Local<u64>,
    mut mode: ResMut<InputMode>,
    mut screen: ResMut<kuluu_render::hud::auction::AuctionScreenState>,
) {
    let total = events.pushed_total;
    let len = events.recent.len() as u64;
    let first_global = total - len;
    let mut opened = false;
    for i in (*cursor).max(first_global)..total {
        if matches!(
            events.recent[(i - first_global) as usize],
            kuluu_snapshot::ViewerEvent::AuctionMenuOpened
        ) {
            opened = true;
        }
    }
    *cursor = total;

    if opened && !screen.active {
        screen.open();
        *mode = InputMode::Auction;
    }

    if screen.active && !state.snapshot.auction.open {
        screen.close();
        if matches!(*mode, InputMode::Auction) {
            *mode = InputMode::World;
        }
    }

    if screen.awaiting_quote && state.snapshot.auction.fee_quote.is_some() {
        if let AhScreen::SellPrice {
            sell,
            stack,
            spinner,
        } = &screen.screen
        {
            let (sell, stack, price) = (*sell, *stack, spinner.value);
            screen.awaiting_quote = false;
            screen.screen = AhScreen::FeeConfirm {
                sell,
                stack,
                price,
                cursor: YesNo::Yes,
            };
        }
    }
}

/// The sell-list index of a pick, for restoring the cursor on back-out.
fn sell_row_index(inv: &AuctionSellInventory, sell: &SellPick) -> usize {
    inv.rows
        .iter()
        .position(|r| r.inv_slot == sell.inv_slot)
        .unwrap_or(0)
}

fn send(cmd_tx: &Sender<AgentCommand>, scene: &mut SceneState, cmd: AgentCommand) {
    if let Err(e) = cmd_tx.try_send(cmd) {
        push_system_chat_line(scene, format!("[auction] command dropped: {e}"));
    }
}

fn browse(
    screen: &mut AuctionScreenState,
    scene: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    category: u8,
) {
    send(
        cmd_tx,
        scene,
        AgentCommand::AhBrowse {
            category,
            sorts: screen.sorts.clone(),
        },
    );
}

/// Keyboard handling for the AH screens. Mode transitions in/out of
/// `InputMode::Auction` are owned by [`auction_mode_sync_system`]; Esc at the
/// root closes the UI client-side (retail keeps no server-side "closed"
/// notion).
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_auction_key(
    key: &Key,
    bindings: &Bindings,
    screen: &mut AuctionScreenState,
    scene_state: &mut SceneState,
    inv: &AuctionSellInventory,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    // Async op in flight: retail closes the panes and ignores input while the
    // dot spinner runs.
    if scene_state.snapshot.auction.busy.is_some() {
        return None;
    }

    if bindings.matches_logical(Action::NavUp, key) {
        nav_vertical(screen, scene_state, inv, true);
        return None;
    }
    if bindings.matches_logical(Action::NavDown, key) {
        nav_vertical(screen, scene_state, inv, false);
        return None;
    }
    if bindings.matches_logical(Action::NavLeft, key) {
        nav_horizontal(screen, scene_state, inv, false);
        return None;
    }
    if bindings.matches_logical(Action::NavRight, key) {
        nav_horizontal(screen, scene_state, inv, true);
        return None;
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        return cancel(screen, inv);
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        return auction_confirm(screen, scene_state, inv, cmd_tx);
    }
    None
}

/// Up/Down: wrap top↔bottom in menus and lists; step the active digit in a
/// price spinner.
fn nav_vertical(
    screen: &mut AuctionScreenState,
    scene_state: &SceneState,
    inv: &AuctionSellInventory,
    up: bool,
) {
    let snap = &scene_state.snapshot;
    let catalog_len = auction::filtered_listings(screen, snap).len();
    let sort_len = screen
        .browse_leaf_node()
        .map(|l| auction::sort_rows(l.id).len())
        .unwrap_or(0);
    let wrap = |c: usize, len: usize| {
        if up {
            auction::wrap_up(c, len)
        } else {
            auction::wrap_down(c, len)
        }
    };
    match &mut screen.screen {
        AhScreen::Root { cursor } => *cursor = wrap(*cursor, 3),
        AhScreen::Category { path, cursor } => {
            let len = auction::menu_children(path).map(|c| c.len()).unwrap_or(0);
            *cursor = wrap(*cursor, len);
        }
        AhScreen::Catalog {
            overlay: Some(CatalogOverlay::ItemMenu { cursor }),
            ..
        } => *cursor = wrap(*cursor, 3),
        AhScreen::Catalog {
            overlay: Some(CatalogOverlay::Sort { cursor }),
            ..
        } => *cursor = wrap(*cursor, sort_len),
        AhScreen::Catalog {
            cursor,
            overlay: None,
        } => *cursor = wrap(*cursor, catalog_len),
        AhScreen::History { .. } => {}
        AhScreen::BidPrice { spinner, .. } | AhScreen::SellPrice { spinner, .. } => {
            if up {
                spinner.up();
            } else {
                spinner.down();
            }
        }
        AhScreen::SellList { cursor } => *cursor = wrap(*cursor, inv.rows.len()),
        AhScreen::SellStack { cursor, .. } => *cursor = wrap(*cursor, 2),
        AhScreen::FeeConfirm { cursor, .. }
        | AhScreen::PlaceConfirm { cursor, .. }
        | AhScreen::CancelConfirm { cursor, .. } => *cursor = cursor.toggled(),
        AhScreen::SalesStatus { cursor } => *cursor = wrap(*cursor, auction::SALES_SLOTS),
    }
}

/// Left/Right: page in item lists (Left = page up, Right = page down, no
/// wrap); move the digit column in a price spinner; toggle a Yes/No.
fn nav_horizontal(
    screen: &mut AuctionScreenState,
    scene_state: &SceneState,
    inv: &AuctionSellInventory,
    forward: bool,
) {
    use kuluu_render::hud::menu::page_cursor;
    let snap = &scene_state.snapshot;
    let catalog_len = auction::filtered_listings(screen, snap).len();
    match &mut screen.screen {
        AhScreen::Catalog {
            cursor,
            overlay: None,
        } => *cursor = page_cursor(*cursor, catalog_len, auction::CATALOG_ROWS, forward),
        AhScreen::SellList { cursor } => {
            *cursor = page_cursor(*cursor, inv.rows.len(), auction::CATALOG_ROWS, forward)
        }
        AhScreen::BidPrice { spinner, .. } | AhScreen::SellPrice { spinner, .. } => {
            if forward {
                spinner.right();
            } else {
                spinner.left();
            }
        }
        AhScreen::FeeConfirm { cursor, .. }
        | AhScreen::PlaceConfirm { cursor, .. }
        | AhScreen::CancelConfirm { cursor, .. } => *cursor = cursor.toggled(),
        _ => {}
    }
}

/// Esc: back out one level, retail-style; `Some(World)` only when closing the
/// whole UI from the root menu.
fn cancel(screen: &mut AuctionScreenState, inv: &AuctionSellInventory) -> Option<InputMode> {
    match &screen.screen {
        AhScreen::Root { .. } => {
            screen.close();
            return Some(InputMode::World);
        }
        AhScreen::Category { path, .. } => {
            if path.is_empty() {
                screen.screen = AhScreen::Root { cursor: 0 };
            } else {
                let mut path = path.clone();
                let cursor = path.pop().unwrap_or(0);
                screen.screen = AhScreen::Category { path, cursor };
            }
        }
        AhScreen::Catalog {
            overlay: Some(CatalogOverlay::Sort { .. }),
            cursor,
        } => {
            screen.screen = AhScreen::Catalog {
                cursor: *cursor,
                overlay: Some(CatalogOverlay::ItemMenu { cursor: 2 }),
            };
        }
        AhScreen::Catalog {
            overlay: Some(CatalogOverlay::ItemMenu { .. }),
            cursor,
        } => {
            screen.screen = AhScreen::Catalog {
                cursor: *cursor,
                overlay: None,
            };
        }
        AhScreen::Catalog { overlay: None, .. } => {
            screen.screen = AhScreen::Category {
                path: screen.browse_path.clone(),
                cursor: screen.browse_leaf,
            };
        }
        AhScreen::History {
            return_to: HistoryReturn::Catalog,
        } => {
            screen.screen = AhScreen::Catalog {
                cursor: screen.catalog_cursor,
                overlay: None,
            };
        }
        AhScreen::BidPrice { .. } => {
            screen.screen = AhScreen::Catalog {
                cursor: screen.catalog_cursor,
                overlay: None,
            };
        }
        AhScreen::SellList { .. } => {
            screen.screen = AhScreen::Root { cursor: 1 };
        }
        AhScreen::SellStack { sell, .. } => {
            screen.screen = AhScreen::SellList {
                cursor: sell_row_index(inv, sell),
            };
        }
        AhScreen::SellPrice { sell, .. } => {
            screen.awaiting_quote = false;
            screen.screen = AhScreen::SellList {
                cursor: sell_row_index(inv, sell),
            };
        }
        AhScreen::FeeConfirm {
            sell, stack, price, ..
        } => {
            screen.screen = AhScreen::SellPrice {
                sell: *sell,
                stack: *stack,
                spinner: DigitSpinner::with_value(auction::PRICE_CAP, *price),
            };
        }
        AhScreen::PlaceConfirm {
            sell, stack, price, ..
        } => {
            screen.screen = AhScreen::SellPrice {
                sell: *sell,
                stack: *stack,
                spinner: DigitSpinner::with_value(auction::PRICE_CAP, *price),
            };
        }
        AhScreen::SalesStatus { .. } => {
            screen.screen = AhScreen::Root { cursor: 2 };
        }
        AhScreen::CancelConfirm { slot, .. } => {
            screen.screen = AhScreen::SalesStatus { cursor: *slot };
        }
    }
    None
}

/// Enter (and mouse click): activate the focused row. Shared by the key
/// handler and `mouse_nav_dispatch_system`.
pub(super) fn auction_confirm(
    screen: &mut AuctionScreenState,
    scene_state: &mut SceneState,
    inv: &AuctionSellInventory,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    let snap_listings = auction::filtered_listings(screen, &scene_state.snapshot);
    match screen.screen.clone() {
        AhScreen::Root { cursor } => match cursor {
            0 => {
                screen.screen = AhScreen::Category {
                    path: Vec::new(),
                    cursor: 0,
                };
            }
            1 => {
                screen.screen = AhScreen::SellList { cursor: 0 };
            }
            _ => {
                send(cmd_tx, scene_state, AgentCommand::AhSalesStatus);
                // Count echo is provisional: retail's line follows the
                // refreshed 7-slot stream; we echo the current snapshot.
                let count = scene_state
                    .snapshot
                    .auction
                    .sales_status
                    .iter()
                    .flatten()
                    .count();
                push_system_chat_line(scene_state, auction::chat_sales_count(count));
                screen.screen = AhScreen::SalesStatus { cursor: 0 };
            }
        },
        AhScreen::Category { path, cursor } => {
            match auction::menu_children(&path).and_then(|c| c.get(cursor)) {
                Some(AhNode::Menu { .. }) => {
                    let mut path = path.clone();
                    path.push(cursor);
                    screen.screen = AhScreen::Category { path, cursor: 0 };
                }
                Some(AhNode::Leaf(l)) => {
                    screen.sorts.clear();
                    screen.job_filter = false;
                    screen.browse_path = path.clone();
                    screen.browse_leaf = cursor;
                    let category = l.id;
                    browse(screen, scene_state, cmd_tx, category);
                    screen.screen = AhScreen::Catalog {
                        cursor: 0,
                        overlay: None,
                    };
                }
                None => {}
            }
        }
        AhScreen::Catalog {
            cursor,
            overlay: None,
        } => {
            if snap_listings.get(cursor).is_some() {
                screen.screen = AhScreen::Catalog {
                    cursor,
                    overlay: Some(CatalogOverlay::ItemMenu { cursor: 0 }),
                };
            }
        }
        AhScreen::Catalog {
            cursor,
            overlay:
                Some(CatalogOverlay::ItemMenu {
                    cursor: menu_cursor,
                }),
        } => {
            let listing = snap_listings.get(cursor).copied()?;
            screen.catalog_cursor = cursor;
            match menu_cursor {
                0 => {
                    send(
                        cmd_tx,
                        scene_state,
                        AgentCommand::AhHistory {
                            item_id: listing.item_id,
                            stack: auction::default_stack_form(&listing),
                        },
                    );
                    screen.screen = AhScreen::History {
                        return_to: HistoryReturn::Catalog,
                    };
                }
                1 => {
                    let gil = kuluu_render::hud::delivery::current_gil(&scene_state.snapshot);
                    screen.screen = AhScreen::BidPrice {
                        item_no: listing.item_id,
                        stack: auction::default_stack_form(&listing),
                        spinner: DigitSpinner::new(gil),
                    };
                }
                _ => {
                    screen.screen = AhScreen::Catalog {
                        cursor,
                        overlay: Some(CatalogOverlay::Sort { cursor: 0 }),
                    };
                }
            }
        }
        AhScreen::Catalog {
            cursor,
            overlay: Some(CatalogOverlay::Sort {
                cursor: sort_cursor,
            }),
        } => {
            let leaf = screen.browse_leaf_node()?;
            let rows = auction::sort_rows(leaf.id);
            let (_, choice, _) = rows.get(sort_cursor)?;
            match choice {
                SortChoice::Reset => {
                    screen.sorts.clear();
                    screen.job_filter = false;
                    browse(screen, scene_state, cmd_tx, leaf.id);
                }
                SortChoice::Param(p) => {
                    screen.sorts = vec![*p];
                    browse(screen, scene_state, cmd_tx, leaf.id);
                }
                SortChoice::JobFilter => {
                    screen.job_filter = !screen.job_filter;
                }
                SortChoice::RaceFilter => {
                    // Race is a client-side narrow per LSB; the snapshot does
                    // not surface the player's race yet — deferred.
                    push_system_chat_line(
                        scene_state,
                        "[auction] Race sort is not implemented yet.".to_string(),
                    );
                }
            }
            screen.screen = AhScreen::Catalog {
                cursor,
                overlay: None,
            };
        }
        AhScreen::History { .. } => {}
        AhScreen::BidPrice {
            item_no,
            stack,
            spinner,
        } => {
            if spinner.value == 0 {
                return None;
            }
            send(
                cmd_tx,
                scene_state,
                AgentCommand::AhBid {
                    item_id: item_no,
                    stack,
                    price: spinner.value,
                },
            );
            screen.screen = AhScreen::Catalog {
                cursor: screen.catalog_cursor,
                overlay: None,
            };
        }
        AhScreen::SellList { cursor } => {
            let row = inv.rows.get(cursor).copied()?;
            let sell = SellPick {
                inv_slot: row.inv_slot,
                item_no: row.item_no,
                quantity: row.quantity,
            };
            if row.quantity > 1 {
                screen.screen = AhScreen::SellStack { sell, cursor: 0 };
            } else {
                open_sell_price(screen, scene_state, cmd_tx, sell, false);
            }
        }
        AhScreen::SellStack { sell, cursor } => {
            open_sell_price(screen, scene_state, cmd_tx, sell, cursor == 1);
        }
        AhScreen::SellPrice {
            sell,
            stack,
            spinner,
        } => {
            if spinner.value == 0 {
                return None;
            }
            screen.awaiting_quote = true;
            send(
                cmd_tx,
                scene_state,
                AgentCommand::AhSell {
                    inventory_slot: sell.inv_slot,
                    stack,
                    price: spinner.value,
                },
            );
        }
        AhScreen::FeeConfirm {
            sell,
            stack,
            price,
            cursor,
        } => match cursor {
            YesNo::Yes => {
                screen.screen = AhScreen::PlaceConfirm {
                    sell,
                    stack,
                    price,
                    // Provisional default (record open question 1): Yes, like
                    // the fee confirm.
                    cursor: YesNo::Yes,
                };
            }
            YesNo::No => {
                screen.screen = AhScreen::SellPrice {
                    sell,
                    stack,
                    spinner: DigitSpinner::with_value(auction::PRICE_CAP, price),
                };
            }
        },
        AhScreen::PlaceConfirm {
            sell,
            stack,
            price,
            cursor,
        } => match cursor {
            YesNo::Yes => {
                let fee = scene_state
                    .snapshot
                    .auction
                    .fee_quote
                    .as_ref()
                    .map(|q| q.fee)
                    .unwrap_or(0);
                screen.pending_listing = Some(auction::PendingListing {
                    item_no: sell.item_no,
                    fee,
                    price,
                    stack_quantity: stack.then_some(sell.quantity),
                });
                send(cmd_tx, scene_state, AgentCommand::AhSellConfirm);
                screen.screen = AhScreen::SellList {
                    cursor: sell_row_index(inv, &sell),
                };
            }
            YesNo::No => {
                screen.screen = AhScreen::SellPrice {
                    sell,
                    stack,
                    spinner: DigitSpinner::with_value(auction::PRICE_CAP, price),
                };
            }
        },
        AhScreen::SalesStatus { cursor } => {
            if scene_state
                .snapshot
                .auction
                .sales_status
                .get(cursor)
                .and_then(|s| s.as_ref())
                .is_some()
            {
                screen.screen = AhScreen::CancelConfirm {
                    slot: cursor,
                    cursor: YesNo::Yes,
                };
            }
        }
        AhScreen::CancelConfirm { slot, cursor } => match cursor {
            YesNo::Yes => {
                send(
                    cmd_tx,
                    scene_state,
                    AgentCommand::AhCancelSale { slot: slot as u8 },
                );
                screen.screen = AhScreen::SalesStatus { cursor: slot };
            }
            YesNo::No => {
                screen.screen = AhScreen::SalesStatus { cursor: slot };
            }
        },
    }
    None
}

/// Enter the Price Set screen for a sale: prefetch the item's sale history for
/// the table and open the capped digit spinner.
fn open_sell_price(
    screen: &mut AuctionScreenState,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    sell: SellPick,
    stack: bool,
) {
    send(
        cmd_tx,
        scene_state,
        AgentCommand::AhHistory {
            item_id: sell.item_no,
            stack,
        },
    );
    screen.screen = AhScreen::SellPrice {
        sell,
        stack,
        spinner: DigitSpinner::new(auction::PRICE_CAP),
    };
}

/// Mouse click on an AH row: land the cursor there, then confirm.
pub(super) fn auction_click(
    region: AhRegion,
    slot: usize,
    screen: &mut AuctionScreenState,
    scene_state: &mut SceneState,
    inv: &AuctionSellInventory,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    if scene_state.snapshot.auction.busy.is_some() {
        return None;
    }
    auction::apply_hover(screen, &scene_state.snapshot, inv.rows.len(), region, slot);
    auction_confirm(screen, scene_state, inv, cmd_tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuluu_render::hud::auction::SellRow;
    use tokio::sync::mpsc;

    fn fixture() -> (
        AuctionScreenState,
        SceneState,
        AuctionSellInventory,
        Sender<AgentCommand>,
        mpsc::Receiver<AgentCommand>,
    ) {
        let (tx, rx) = mpsc::channel(8);
        let mut screen = AuctionScreenState::default();
        screen.open();
        let inv = AuctionSellInventory {
            rows: vec![
                SellRow {
                    inv_slot: 1,
                    item_no: 4570,
                    quantity: 12,
                },
                SellRow {
                    inv_slot: 2,
                    item_no: 5000,
                    quantity: 1,
                },
            ],
        };
        (screen, SceneState::default(), inv, tx, rx)
    }

    #[test]
    fn bid_drills_categories_and_esc_restores_the_cursor() {
        let (mut screen, mut scene, inv, tx, mut rx) = fixture();

        assert!(auction_confirm(&mut screen, &mut scene, &inv, &tx).is_none());
        assert!(matches!(
            screen.screen,
            AhScreen::Category { ref path, cursor: 0 } if path.is_empty()
        ));

        // Drill into Weapons (index 0), then its Ammo&Misc. submenu (index 14).
        assert!(auction_confirm(&mut screen, &mut scene, &inv, &tx).is_none());
        if let AhScreen::Category { path, cursor } = &mut screen.screen {
            assert_eq!(path, &vec![0]);
            *cursor = 14;
        } else {
            panic!("expected weapons menu");
        }
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(matches!(
            screen.screen,
            AhScreen::Category { ref path, cursor: 0 } if path == &vec![0, 14]
        ));

        // Leaf: Fishing Gear (index 1, id 47) fires AhBrowse and opens the catalog.
        if let AhScreen::Category { cursor, .. } = &mut screen.screen {
            *cursor = 1;
        }
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentCommand::AhBrowse { category: 47, ref sorts }) if sorts.is_empty()
        ));
        assert!(matches!(
            screen.screen,
            AhScreen::Catalog {
                cursor: 0,
                overlay: None
            }
        ));

        // Esc restores Ammo&Misc. with Fishing Gear highlighted, then walks up.
        assert!(cancel(&mut screen, &inv).is_none());
        assert!(matches!(
            screen.screen,
            AhScreen::Category { ref path, cursor: 1 } if path == &vec![0, 14]
        ));
        cancel(&mut screen, &inv);
        assert!(matches!(
            screen.screen,
            AhScreen::Category { ref path, cursor: 14 } if path == &vec![0]
        ));
        cancel(&mut screen, &inv);
        assert!(matches!(
            screen.screen,
            AhScreen::Category { ref path, cursor: 0 } if path.is_empty()
        ));
        cancel(&mut screen, &inv);
        assert!(matches!(screen.screen, AhScreen::Root { cursor: 0 }));
        assert!(matches!(cancel(&mut screen, &inv), Some(InputMode::World)));
        assert!(!screen.active, "Esc from the root closes the UI");
    }

    #[test]
    fn sell_stack_flow_quotes_confirms_and_records_the_echo() {
        let (mut screen, mut scene, inv, tx, mut rx) = fixture();
        screen.screen = AhScreen::SellList { cursor: 0 };

        // Stackable row opens the single/stack chooser.
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(matches!(
            screen.screen,
            AhScreen::SellStack { cursor: 0, .. }
        ));

        // Stack choice prefetches history and opens Price Set.
        if let AhScreen::SellStack { cursor, .. } = &mut screen.screen {
            *cursor = 1;
        }
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentCommand::AhHistory {
                item_id: 4570,
                stack: true
            })
        ));
        let AhScreen::SellPrice { spinner, .. } = &mut screen.screen else {
            panic!("expected price set");
        };
        spinner.up(); // 1 gil — enough to confirm

        // Confirm sends the quote request and arms the fee-confirm promotion.
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(screen.awaiting_quote);
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentCommand::AhSell {
                inventory_slot: 1,
                stack: true,
                price: 1
            })
        ));

        // Quote lands (as the mode-sync system would apply it).
        scene.snapshot.auction.fee_quote = Some(kuluu_snapshot::AhFeeQuote {
            fee: 9,
            inventory_slot: 1,
            item_no: 4570,
            stack: true,
            asking_price: 1,
        });
        screen.awaiting_quote = false;
        screen.screen = AhScreen::FeeConfirm {
            sell: SellPick {
                inv_slot: 1,
                item_no: 4570,
                quantity: 12,
            },
            stack: true,
            price: 1,
            cursor: YesNo::Yes,
        };

        // Fee Yes → placement confirm (default Yes, provisional); Yes → LotIn.
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(matches!(
            screen.screen,
            AhScreen::PlaceConfirm {
                cursor: YesNo::Yes,
                ..
            }
        ));
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(matches!(rx.try_recv(), Ok(AgentCommand::AhSellConfirm)));
        let pending = screen.pending_listing.expect("echo payload recorded");
        assert_eq!(pending.fee, 9);
        assert_eq!(pending.stack_quantity, Some(12));
        assert!(matches!(screen.screen, AhScreen::SellList { cursor: 0 }));
    }

    #[test]
    fn bid_price_confirm_sends_the_bid_and_returns_to_the_catalog() {
        let (mut screen, mut scene, inv, tx, mut rx) = fixture();
        screen.catalog_cursor = 3;
        screen.screen = AhScreen::BidPrice {
            item_no: 17440,
            stack: false,
            spinner: DigitSpinner::with_value(80_121, 490),
        };
        auction_confirm(&mut screen, &mut scene, &inv, &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentCommand::AhBid {
                item_id: 17440,
                stack: false,
                price: 490
            })
        ));
        assert!(matches!(
            screen.screen,
            AhScreen::Catalog {
                cursor: 3,
                overlay: None
            }
        ));
    }
}
