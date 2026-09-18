use bevy::prelude::*;
use kuluu_snapshot::SceneSnapshot;

use crate::hud::digit_spinner::{self, DigitSpinner, SpinnerUnit};
use crate::hud::item_meta::{self, ItemDetail, ItemStatic};
use crate::hud::style::{self, theme};
use crate::snapshot::SceneState;

pub const ITEM_FLAG_RARE: u16 = 0x8000;

pub const ITEM_FLAG_EX: u16 = 0x4000;

pub const TRADE_COLS: usize = 4;
pub const TRADE_ROWS: usize = 2;
pub const TRADE_SLOTS: usize = TRADE_COLS * TRADE_ROWS;

pub const PLACED_TINT: Color = Color::srgb(0.85, 0.35, 0.10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TradeSlotItem {
    pub item_no: u16,
    pub count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeFocus {
    Gil,

    Slot(usize),

    Ok,

    Cancel,
}

impl Default for TradeFocus {
    fn default() -> Self {
        TradeFocus::Slot(0)
    }
}

#[derive(Resource, Debug, Clone, Default)]
pub struct TradeState {
    pub open: bool,

    pub target_id: u32,

    pub slots: [Option<TradeSlotItem>; TRADE_SLOTS],

    pub gil: u32,

    pub focus: TradeFocus,

    /// The shared amount picker, open while a gil amount is being sized.
    pub selector: Option<DigitSpinner>,
}

impl TradeState {
    pub fn open(target_id: u32) -> Self {
        Self {
            open: true,
            target_id,
            slots: [None; TRADE_SLOTS],
            gil: 0,
            focus: TradeFocus::default(),
            selector: None,
        }
    }

    pub fn reset(&mut self) {
        *self = TradeState::default();
    }

    pub fn placed_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    pub fn first_free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.is_none())
    }
}

pub fn is_tradeable(item_no: u16, dat: Option<&ItemStatic>, snapshot: &SceneSnapshot) -> bool {
    if snapshot.equipped.contains(&Some(item_no)) {
        return false;
    }
    if let Some(st) = dat {
        if st.flags & (ITEM_FLAG_RARE | ITEM_FLAG_EX) != 0 {
            return false;
        }
    }
    true
}

pub fn is_stackable(dat: Option<&ItemStatic>) -> bool {
    stack_size_of(dat) > 1
}

pub fn stack_size_of(dat: Option<&ItemStatic>) -> u16 {
    let _ = dat;
    1
}

#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum TradeIntent {
    Placement {
        slot: usize,
        item_no: Option<u16>,
        count: u16,
    },

    Gil {
        amount: u32,
    },

    Confirm {
        target_id: u32,
    },

    Cancel,
}

pub fn focus_up(state: &mut TradeState) {
    state.focus = match state.focus {
        TradeFocus::Gil => TradeFocus::Gil,
        TradeFocus::Slot(i) if i < TRADE_COLS => TradeFocus::Gil,
        TradeFocus::Slot(i) => TradeFocus::Slot(i - TRADE_COLS),
        TradeFocus::Ok => TradeFocus::Ok,
        TradeFocus::Cancel => TradeFocus::Ok,
    };
}

pub fn focus_down(state: &mut TradeState) {
    state.focus = match state.focus {
        TradeFocus::Gil => TradeFocus::Slot(0),
        TradeFocus::Slot(i) if i + TRADE_COLS < TRADE_SLOTS => TradeFocus::Slot(i + TRADE_COLS),
        TradeFocus::Slot(i) => TradeFocus::Slot(i),
        TradeFocus::Ok => TradeFocus::Cancel,
        TradeFocus::Cancel => TradeFocus::Cancel,
    };
}

pub fn focus_left(state: &mut TradeState) {
    state.focus = match state.focus {
        TradeFocus::Slot(i) if i % TRADE_COLS > 0 => TradeFocus::Slot(i - 1),
        TradeFocus::Ok => TradeFocus::Slot(TRADE_COLS - 1),
        TradeFocus::Cancel => TradeFocus::Slot(TRADE_SLOTS - 1),
        other => other,
    };
}

pub fn focus_right(state: &mut TradeState) {
    state.focus = match state.focus {
        TradeFocus::Slot(i) if i % TRADE_COLS < TRADE_COLS - 1 => TradeFocus::Slot(i + 1),

        TradeFocus::Slot(i) if i < TRADE_COLS => TradeFocus::Ok,
        TradeFocus::Slot(_) => TradeFocus::Cancel,
        other => other,
    };
}

pub fn begin_gil_entry(state: &mut TradeState, snapshot_gil: u32) {
    state.selector = Some(DigitSpinner::new(snapshot_gil));
}

pub fn gil_confirm(state: &mut TradeState) -> Option<u32> {
    let amount = state.selector.take()?.confirm();
    state.gil = amount;
    Some(amount)
}

pub fn place_item(
    state: &mut TradeState,
    slot: usize,
    item_no: u16,
    count: u16,
    dat: Option<&ItemStatic>,
    snapshot: &SceneSnapshot,
) -> bool {
    if slot >= TRADE_SLOTS || !is_tradeable(item_no, dat, snapshot) {
        return false;
    }
    state.slots[slot] = Some(TradeSlotItem {
        item_no,
        count: count.max(1),
    });
    true
}

pub fn clear_slot(state: &mut TradeState, slot: usize) {
    if slot < TRADE_SLOTS {
        state.slots[slot] = None;
    }
}

pub fn focused_tooltip(
    state: &TradeState,
    snapshot: &SceneSnapshot,
    dat_lookup: impl Fn(u16) -> Option<ItemStatic>,
) -> Option<ItemDetail> {
    let TradeFocus::Slot(i) = state.focus else {
        return None;
    };
    let item = state.slots[i]?;
    Some(item_meta::compose_item_detail(
        item.item_no,
        None,
        snapshot,
        dat_lookup(item.item_no),
    ))
}

#[derive(Component)]
pub struct TradePanel;

#[derive(Component)]
pub struct TradeTitle;

#[derive(Component, Clone, Copy)]
pub struct TradeCell {
    pub focus: TradeFocus,
}

#[derive(Component)]
pub struct TradeStatusLine;

const PANEL_WIDTH_PX: f32 = 300.0;

pub fn spawn_trade_window(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            TradePanel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(30.0),
                left: Val::Percent(40.0),
                width: Val::Px(PANEL_WIDTH_PX),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                display: Display::None,
                ..default()
            },
            ZIndex(25),
            BackgroundColor(theme::FRAME_BG),
            BorderColor::all(theme::FRAME_EDGE),
        ))
        .with_children(|p| {
            p.spawn((
                TradeTitle,
                Text::new("Trade"),
                style::text_font(15.0),
                TextColor(theme::TITLE),
            ));

            spawn_cell(p, TradeFocus::Gil, "Gil: 0");

            for row in 0..TRADE_ROWS {
                p.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|r| {
                    for col in 0..TRADE_COLS {
                        let slot = row * TRADE_COLS + col;
                        spawn_cell(r, TradeFocus::Slot(slot), "·");
                    }

                    let ctrl = if row == 0 {
                        (TradeFocus::Ok, "OK")
                    } else {
                        (TradeFocus::Cancel, "Cancel")
                    };
                    spawn_cell(r, ctrl.0, ctrl.1);
                });
            }

            p.spawn((
                TradeStatusLine,
                Text::new(""),
                style::text_font(12.0),
                TextColor(theme::MUTED),
            ));
        });
}

fn spawn_cell(p: &mut ChildSpawnerCommands, focus: TradeFocus, label: &str) {
    p.spawn((
        TradeCell { focus },
        Node {
            min_width: Val::Px(40.0),
            padding: UiRect::axes(Val::Px(4.0), Val::Px(2.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BorderColor::all(theme::CELL_EDGE),
        BackgroundColor(Color::NONE),
    ))
    .with_children(|c| {
        c.spawn((
            Text::new(label.to_string()),
            style::text_font(12.0),
            TextColor(theme::TEXT),
        ));
    });
}

#[allow(clippy::type_complexity)]
pub fn update_trade_window(
    state: Res<SceneState>,
    trade: Res<TradeState>,
    mut panel_q: Query<&mut Node, With<TradePanel>>,
    mut cell_q: Query<
        (
            &TradeCell,
            &mut BorderColor,
            &mut BackgroundColor,
            &Children,
        ),
        Without<TradePanel>,
    >,
    mut text_q: Query<&mut Text, Without<TradeStatusLine>>,
    mut status_q: Query<&mut Text, With<TradeStatusLine>>,
) {
    let Ok(mut panel) = panel_q.single_mut() else {
        return;
    };

    if !trade.open {
        if panel.display != Display::None {
            panel.display = Display::None;
        }
        return;
    }
    if panel.display == Display::None {
        panel.display = Display::Flex;
    }

    for (cell, mut border, mut bg, children) in cell_q.iter_mut() {
        let focused = cell.focus == trade.focus;

        let want_border = if focused {
            theme::CURSOR
        } else {
            theme::CELL_EDGE
        };
        if border.left != want_border {
            *border = BorderColor::all(want_border);
        }

        let want_bg = match cell.focus {
            TradeFocus::Slot(i) if trade.slots[i].is_some() => PLACED_TINT,
            _ => Color::NONE,
        };
        if bg.0 != want_bg {
            bg.0 = want_bg;
        }

        let want_text = cell_text(&trade, cell.focus);
        if let Some(child) = children.first() {
            if let Ok(mut text) = text_q.get_mut(*child) {
                if **text != want_text {
                    **text = want_text;
                }
            }
        }
    }

    if let Ok(mut status) = status_q.single_mut() {
        let want = status_text(&trade, &state.snapshot);
        if **status != want {
            **status = want;
        }
    }
}

fn cell_text(trade: &TradeState, focus: TradeFocus) -> String {
    match focus {
        TradeFocus::Gil => format!("Gil: {}", trade.gil),
        TradeFocus::Ok => "OK".to_string(),
        TradeFocus::Cancel => "Cancel".to_string(),
        TradeFocus::Slot(i) => match trade.slots[i] {
            Some(item) if item.count > 1 => format!("#{} x{}", item.item_no, item.count),
            Some(item) => format!("#{}", item.item_no),
            None => "·".to_string(),
        },
    }
}

fn status_text(trade: &TradeState, _snapshot: &SceneSnapshot) -> String {
    match &trade.selector {
        Some(spinner) => digit_spinner::slots()
            .map(|slot| digit_spinner::slot_style(spinner, slot, SpinnerUnit::Gil).0)
            .collect(),
        None => match trade.focus {
            TradeFocus::Slot(i) => match trade.slots[i] {
                Some(item) => format!("#{}", item.item_no),
                None => "Empty slot".to_string(),
            },
            TradeFocus::Gil => "Enter to edit gil".to_string(),
            TradeFocus::Ok => "Confirm trade".to_string(),
            TradeFocus::Cancel => "Cancel trade".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> SceneSnapshot {
        SceneSnapshot::default()
    }

    fn rare_ex(flags: u16) -> ItemStatic {
        ItemStatic {
            flags,
            ..Default::default()
        }
    }

    /// Gil rides the same picker as every other amount: walk left to the widest
    /// place the purse allows and step it up until the clamp bites, and the
    /// whole purse is selected.
    #[test]
    fn stepping_the_top_place_commits_the_whole_purse() {
        let mut s = TradeState::open(42);
        s.gil = 777;
        begin_gil_entry(&mut s, 5000);

        let spinner = s.selector.as_mut().expect("picker open");
        for _ in 0..=digit_spinner::PRICE_DIGITS {
            spinner.left();
        }
        for _ in 0..10 {
            spinner.up();
        }
        assert!(spinner.is_all());

        assert_eq!(gil_confirm(&mut s), Some(5000));
        assert_eq!(s.gil, 5000);
        assert!(s.selector.is_none());
    }

    /// Stepping a digit past the purse clamps to it rather than overcommitting.
    #[test]
    fn stepping_a_digit_clamps_to_the_purse() {
        let mut s = TradeState::open(1);
        begin_gil_entry(&mut s, 1500);

        let spinner = s.selector.as_mut().expect("picker open");
        for _ in 0..3 {
            spinner.left();
        }
        for _ in 0..9 {
            spinner.up();
        }

        assert_eq!(gil_confirm(&mut s), Some(1500));
    }

    #[test]
    fn rare_and_ex_items_are_not_tradeable() {
        let sn = snap();
        assert!(!is_tradeable(1, Some(&rare_ex(ITEM_FLAG_RARE)), &sn));
        assert!(!is_tradeable(2, Some(&rare_ex(ITEM_FLAG_EX)), &sn));
        assert!(is_tradeable(3, Some(&rare_ex(0)), &sn));
    }

    #[test]
    fn equipped_items_are_not_tradeable() {
        let mut sn = snap();
        sn.equipped[0] = Some(555);
        assert!(!is_tradeable(555, Some(&rare_ex(0)), &sn));
        assert!(is_tradeable(556, Some(&rare_ex(0)), &sn));
    }

    #[test]
    fn no_dat_allows_unequipped_items() {
        let sn = snap();

        assert!(is_tradeable(999, None, &sn));
    }

    #[test]
    fn place_rejects_untradeable() {
        let mut s = TradeState::open(1);
        let sn = snap();
        assert!(!place_item(
            &mut s,
            0,
            1,
            1,
            Some(&rare_ex(ITEM_FLAG_RARE)),
            &sn
        ));
        assert!(s.slots[0].is_none());
        assert!(place_item(&mut s, 0, 1, 1, Some(&rare_ex(0)), &sn));
        assert_eq!(s.slots[0].unwrap().item_no, 1);
    }

    #[test]
    fn up_from_top_grid_row_lands_on_gil() {
        let mut s = TradeState::open(1);
        s.focus = TradeFocus::Slot(2);
        focus_up(&mut s);
        assert_eq!(s.focus, TradeFocus::Gil);
    }

    #[test]
    fn up_within_grid_moves_one_row() {
        let mut s = TradeState::open(1);
        s.focus = TradeFocus::Slot(5);
        focus_up(&mut s);
        assert_eq!(s.focus, TradeFocus::Slot(1));
    }

    #[test]
    fn gil_down_returns_to_grid() {
        let mut s = TradeState::open(1);
        s.focus = TradeFocus::Gil;
        focus_down(&mut s);
        assert_eq!(s.focus, TradeFocus::Slot(0));
    }

    #[test]
    fn right_from_grid_edge_enters_control_column() {
        let mut s = TradeState::open(1);
        s.focus = TradeFocus::Slot(TRADE_COLS - 1);
        focus_right(&mut s);
        assert_eq!(s.focus, TradeFocus::Ok);
        s.focus = TradeFocus::Slot(TRADE_SLOTS - 1);
        focus_right(&mut s);
        assert_eq!(s.focus, TradeFocus::Cancel);
    }

    #[test]
    fn ok_down_moves_to_cancel() {
        let mut s = TradeState::open(1);
        s.focus = TradeFocus::Ok;
        focus_down(&mut s);
        assert_eq!(s.focus, TradeFocus::Cancel);
    }

    #[test]
    fn reset_clears_everything() {
        let mut s = TradeState::open(7);
        s.gil = 100;
        s.slots[0] = Some(TradeSlotItem {
            item_no: 1,
            count: 1,
        });
        s.reset();
        assert!(!s.open);
        assert_eq!(s.placed_count(), 0);
        assert_eq!(s.gil, 0);
    }
}
