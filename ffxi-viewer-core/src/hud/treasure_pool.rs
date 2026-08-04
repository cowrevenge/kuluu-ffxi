//! The treasure pool panel: what is in the 10 pool slots, who is winning each,
//! and whether this character has lotted or passed.
//!
//! Fed by s2c 0x0D2/0x0D3 (see `ffxi-client/src/session/treasure.rs`). Acting on
//! a slot goes out as `/lot <slot>` or `/pass <slot>`; the row shows the slot
//! index so the command has something to name.

use bevy::prelude::*;
use ffxi_viewer_wire::{TreasureEntry, TreasurePoolSlot};

use crate::hud::style::{self, theme};
use crate::snapshot::SceneState;

/// `TREASUREPOOL_SIZE` (vendor/server/src/map/treasure_pool.h:38) — the panel
/// pre-spawns one row per slot and hides the empty ones.
pub const POOL_ROWS: usize = 10;

const ROW_HEIGHT_PX: f32 = 16.0;
const PANEL_WIDTH_PX: f32 = 260.0;

#[derive(Component)]
pub struct TreasurePoolPanel;

#[derive(Component)]
pub struct TreasurePoolRow {
    pub index: usize,
}

#[derive(Component)]
pub struct TreasurePoolRowLabel;

#[derive(Component)]
pub struct TreasurePoolRowStatus;

pub fn spawn_treasure_pool(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            TreasurePoolPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                bottom: Val::Px(240.0),
                width: Val::Px(PANEL_WIDTH_PX),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(1.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(theme::FRAME_BG),
            BorderColor::all(theme::FRAME_EDGE),
        ))
        .with_children(|p| {
            for index in 0..POOL_ROWS {
                p.spawn((
                    TreasurePoolRow { index },
                    Node {
                        height: Val::Px(ROW_HEIGHT_PX),
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: Val::Px(6.0),
                        display: Display::None,
                        ..default()
                    },
                ))
                .with_children(|row| {
                    row.spawn((
                        TreasurePoolRowLabel,
                        Text::new(""),
                        style::text_font(12.0),
                        TextColor(theme::TEXT),
                    ));
                    row.spawn((
                        TreasurePoolRowStatus,
                        Text::new(""),
                        style::text_font(12.0),
                        TextColor(theme::MUTED),
                    ));
                });
            }
        });
}

/// `"3. Lizard Tail"` — the leading number is the slot `/lot` and `/pass` take.
pub fn row_label(s: &TreasurePoolSlot) -> String {
    if s.count > 1 {
        format!("{}. {} x{}", s.slot, s.item_name, s.count)
    } else {
        format!("{}. {}", s.slot, s.item_name)
    }
}

/// The right-hand column: this character's own action first, since that is what
/// decides whether the row still needs input, then who is currently winning.
pub fn row_status(s: &TreasurePoolSlot) -> String {
    let own = match (s.own_entry, s.own_lot) {
        (TreasureEntry::Lotted, Some(lot)) => format!("lot {lot}"),
        (TreasureEntry::Lotted, None) => "lotted".to_string(),
        (TreasureEntry::Passed, _) => "passed".to_string(),
        (TreasureEntry::None, _) => String::new(),
    };
    let winning = match (&s.winner, s.winner_lot) {
        (Some(name), lot) if !name.is_empty() => format!("{name} {lot}"),
        _ => String::new(),
    };
    match (own.is_empty(), winning.is_empty()) {
        (false, false) => format!("{own} — {winning}"),
        (false, true) => own,
        (true, false) => winning,
        (true, true) => String::new(),
    }
}

pub fn row_status_color(s: &TreasurePoolSlot) -> Color {
    match s.own_entry {
        // Nothing has been decided for this row yet, so it is the one asking
        // for input.
        TreasureEntry::None => theme::CURSOR,
        TreasureEntry::Lotted => theme::TEXT,
        TreasureEntry::Passed => theme::FAINT,
    }
}

pub fn update_treasure_pool(
    state: Res<SceneState>,
    mut panel_q: Query<(&mut Node, &Children), With<TreasurePoolPanel>>,
    mut rows: Query<(&TreasurePoolRow, &mut Node, &Children), Without<TreasurePoolPanel>>,
    mut label_q: Query<
        (&mut Text, &mut TextColor),
        (With<TreasurePoolRowLabel>, Without<TreasurePoolRowStatus>),
    >,
    mut status_q: Query<
        (&mut Text, &mut TextColor),
        (With<TreasurePoolRowStatus>, Without<TreasurePoolRowLabel>),
    >,
) {
    if !state.is_changed() {
        return;
    }
    let pool = &state.snapshot.treasure_pool;
    let Ok((mut panel_node, panel_children)) = panel_q.single_mut() else {
        return;
    };
    let want = if pool.is_empty() {
        Display::None
    } else {
        Display::Flex
    };
    if panel_node.display != want {
        panel_node.display = want;
    }

    for child in panel_children.iter() {
        let Ok((row, mut node, row_children)) = rows.get_mut(child) else {
            continue;
        };
        let slot = pool.get(row.index);
        let want = if slot.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
        let Some(slot) = slot else { continue };

        for row_child in row_children.iter() {
            if let Ok((mut text, mut color)) = label_q.get_mut(row_child) {
                let want = row_label(slot);
                if text.as_str() != want {
                    **text = want;
                }
                if color.0 != theme::TEXT {
                    color.0 = theme::TEXT;
                }
            }
            if let Ok((mut text, mut color)) = status_q.get_mut(row_child) {
                let want = row_status(slot);
                if text.as_str() != want {
                    **text = want;
                }
                let want_color = row_status_color(slot);
                if color.0 != want_color {
                    color.0 = want_color;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> TreasurePoolSlot {
        TreasurePoolSlot {
            slot: 3,
            item_id: 916,
            item_name: "Lizard Tail".into(),
            count: 1,
            dropper: "Rock Lizard".into(),
            own_entry: TreasureEntry::None,
            own_lot: None,
            winner: None,
            winner_lot: 0,
        }
    }

    #[test]
    fn label_leads_with_the_slot_the_commands_take() {
        assert_eq!(row_label(&slot()), "3. Lizard Tail");
    }

    #[test]
    fn a_stack_shows_its_count() {
        let mut s = slot();
        s.count = 3;
        assert_eq!(row_label(&s), "3. Lizard Tail x3");
    }

    #[test]
    fn an_untouched_row_has_no_status_but_stands_out() {
        let s = slot();
        assert_eq!(row_status(&s), "");
        assert_eq!(row_status_color(&s), theme::CURSOR);
    }

    #[test]
    fn own_lot_and_current_winner_both_show() {
        let mut s = slot();
        s.own_entry = TreasureEntry::Lotted;
        s.own_lot = Some(412);
        s.winner = Some("Macnugget".into());
        s.winner_lot = 856;
        assert_eq!(row_status(&s), "lot 412 — Macnugget 856");
    }

    #[test]
    fn a_passed_row_says_so_and_dims() {
        let mut s = slot();
        s.own_entry = TreasureEntry::Passed;
        assert_eq!(row_status(&s), "passed");
        assert_eq!(row_status_color(&s), theme::FAINT);
    }

    #[test]
    fn a_winner_shows_even_before_this_character_acts() {
        let mut s = slot();
        s.winner = Some("Macnugget".into());
        s.winner_lot = 856;
        assert_eq!(row_status(&s), "Macnugget 856");
    }
}
