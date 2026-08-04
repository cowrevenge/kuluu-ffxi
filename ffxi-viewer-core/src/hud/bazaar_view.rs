//! The wares list of a browsed bazaar (View Wares in the /check window).
//!
//! Retail draws a fixed-height list of `<icon> <name> ....... <price> G` rows
//! over a Current Gil box and the focused item's description panel; the gil box
//! is replaced by the `All ◄ n/max ►` quantity picker while a purchase is being
//! sized, and the purchase itself is confirmed from a chat prompt
//! (retail capture 2026-08-04, HorizonXI).
//!
//! Rows come straight from the server's s2c 0x105 packets, one per priced
//! seller slot; the seller's own bazaar is the authority, so this window only
//! renders what arrived and sends purchases by slot index.

use bevy::prelude::*;
use ffxi_viewer_wire::{BazaarEntry, SceneSnapshot};

use crate::hud::delivery::current_gil;
use crate::hud::item_dat_root::{ItemDatRoot, ItemIconCache};
use crate::hud::item_detail;
use crate::hud::item_ui::{framed_box, text_font, theme, transparent_placeholder};
use crate::hud::spinner::Spinner;
use crate::snapshot::SceneState;

/// Rows retail keeps drawn, filled or not.
pub const LIST_ROWS: usize = 10;

/// LSB's purchase validator caps a single buy at 99
/// (vendor/server/src/map/packets/c2s/0x106_bazaar_buy.cpp validate).
pub const MAX_BUY_QUANTITY: u32 = 99;

/// Cursor + pending-purchase state for the wares window. The row list itself
/// lives in the snapshot.
#[derive(Resource, Debug, Clone, Default)]
pub struct BazaarScreenState {
    pub cursor: usize,
    /// Active quantity picker for the focused row, once confirmed into.
    pub quantity: Option<Spinner>,
    /// Sized purchase awaiting the retail "Purchase N x for Y gil?" answer.
    pub pending: Option<PendingBuy>,
}

/// A purchase the player has sized but not yet confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingBuy {
    pub index: u8,
    pub item_no: u16,
    pub quantity: u32,
    pub total_gil: u32,
}

impl BazaarScreenState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Keep the cursor inside a list the server may have shrunk under us.
    pub fn clamp(&mut self, len: usize) {
        self.cursor = self.cursor.min(len.saturating_sub(1));
        if len == 0 {
            self.quantity = None;
            self.pending = None;
        }
    }

    pub fn move_cursor(&mut self, dy: i32, len: usize) {
        if len == 0 {
            return;
        }
        let n = len as i32;
        self.cursor = (self.cursor as i32 + dy).rem_euclid(n) as usize;
    }

    /// Open the quantity picker for `entry`, or `None` for a single item (retail
    /// buys a lone item outright rather than asking for a count).
    pub fn begin_quantity(entry: &BazaarEntry) -> Option<Spinner> {
        let max = entry.quantity.min(MAX_BUY_QUANTITY);
        (max > 1).then(|| Spinner::item(max))
    }

    pub fn stage_purchase(&mut self, entry: &BazaarEntry, quantity: u32) -> PendingBuy {
        let buy = PendingBuy {
            index: entry.index,
            item_no: entry.item_no,
            quantity,
            total_gil: entry.total_price(quantity),
        };
        self.quantity = None;
        self.pending = Some(buy);
        buy
    }
}

/// The retail confirmation line, e.g.
/// `Purchase 5 pieces of hickory lumber for 124,995 gil?`.
pub fn purchase_prompt(item_name: &str, quantity: u32, total_gil: u32) -> String {
    format!(
        "Purchase {quantity} {item_name} for {} gil?",
        group_digits(total_gil)
    )
}

/// Retail writes gil with thousands separators everywhere it shows a price.
pub fn group_digits(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[derive(Component)]
pub struct BazaarPanel;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BazaarRole {
    RowName(usize),
    RowPrice(usize),
    GilLabel,
    GilValue,
    DetailName,
    DetailBody,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct BazaarText(BazaarRole);

#[derive(Component, Clone, Copy)]
pub(crate) struct BazaarRowIcon(usize);

#[derive(Component)]
pub(crate) struct BazaarDetailIcon;

const PANEL_WIDTH_PX: f32 = 340.0;
const ROW_ICON_PX: f32 = 18.0;
const PRICE_COL_PX: f32 = 110.0;
const GIL_BOX_PX: f32 = 116.0;

pub(crate) fn spawn_bazaar_view(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let placeholder = transparent_placeholder(&mut images);

    commands
        .spawn((
            crate::components::InGameEntity,
            BazaarPanel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(20.0),
                left: Val::Percent(30.0),
                row_gap: Val::Px(6.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                display: Display::None,
                ..default()
            },
        ))
        .with_children(|root| {
            let (mut n, bg, bd) = framed_box();
            n.width = Val::Px(PANEL_WIDTH_PX);
            root.spawn((n, bg, bd)).with_children(|p| {
                for i in 0..LIST_ROWS {
                    p.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(5.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            BazaarRowIcon(i),
                            Node {
                                width: Val::Px(ROW_ICON_PX),
                                height: Val::Px(ROW_ICON_PX),
                                ..default()
                            },
                            ImageNode::new(placeholder.clone()),
                            BackgroundColor(theme::CELL_BG),
                        ));
                        row.spawn((
                            BazaarText(BazaarRole::RowName(i)),
                            Text::new(""),
                            text_font(13.0),
                            TextColor(theme::TEXT),
                            Node {
                                flex_grow: 1.0,
                                ..default()
                            },
                        ));
                        row.spawn((
                            BazaarText(BazaarRole::RowPrice(i)),
                            Text::new(""),
                            text_font(13.0),
                            TextColor(theme::TEXT),
                            TextLayout {
                                justify: Justify::Right,
                                linebreak: LineBreak::NoWrap,
                                ..default()
                            },
                            Node {
                                width: Val::Px(PRICE_COL_PX),
                                ..default()
                            },
                        ));
                    });
                }
            });

            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|under| {
                let (mut n, bg, bd) = framed_box();
                n.width = Val::Px(GIL_BOX_PX);
                under.spawn((n, bg, bd)).with_children(|g| {
                    g.spawn((
                        BazaarText(BazaarRole::GilLabel),
                        Text::new(""),
                        text_font(12.0),
                        TextColor(theme::MUTED),
                    ));
                    g.spawn((
                        BazaarText(BazaarRole::GilValue),
                        Text::new(""),
                        text_font(13.0),
                        TextColor(theme::TEXT),
                    ));
                });

                let (mut n, bg, bd) = framed_box();
                n.width = Val::Px(PANEL_WIDTH_PX - GIL_BOX_PX - 6.0);
                n.flex_direction = FlexDirection::Row;
                n.column_gap = Val::Px(6.0);
                under.spawn((n, bg, bd)).with_children(|d| {
                    d.spawn((
                        BazaarDetailIcon,
                        Node {
                            width: Val::Px(32.0),
                            height: Val::Px(32.0),
                            ..default()
                        },
                        ImageNode::new(placeholder.clone()),
                    ));
                    d.spawn(Node {
                        flex_direction: FlexDirection::Column,
                        ..default()
                    })
                    .with_children(|t| {
                        t.spawn((
                            BazaarText(BazaarRole::DetailName),
                            Text::new(""),
                            text_font(13.0),
                            TextColor(theme::TITLE),
                        ));
                        t.spawn((
                            BazaarText(BazaarRole::DetailBody),
                            Text::new(""),
                            text_font(12.0),
                            TextColor(theme::TEXT),
                        ));
                    });
                });
            });
        });
}

/// First visible row, keeping the cursor on screen.
pub fn viewport_start(cursor: usize, len: usize) -> usize {
    cursor
        .saturating_sub(LIST_ROWS / 2)
        .min(len.saturating_sub(LIST_ROWS))
}

pub(crate) fn update_bazaar_view(
    state: Res<SceneState>,
    screen: Res<BazaarScreenState>,
    dat_root: Res<ItemDatRoot>,
    mut icon_cache: ResMut<ItemIconCache>,
    mut images: ResMut<Assets<Image>>,
    mut panel_q: Query<
        &mut Node,
        (
            With<BazaarPanel>,
            Without<BazaarText>,
            Without<BazaarRowIcon>,
            Without<BazaarDetailIcon>,
        ),
    >,
    mut text_q: Query<(&BazaarText, &mut Text, &mut TextColor), Without<BazaarRowIcon>>,
    mut icon_q: Query<(&BazaarRowIcon, &mut ImageNode), Without<BazaarDetailIcon>>,
    mut detail_icon_q: Query<&mut ImageNode, With<BazaarDetailIcon>>,
) {
    let Ok(mut panel) = panel_q.single_mut() else {
        return;
    };
    let snap: &SceneSnapshot = &state.snapshot;
    let Some(view) = snap.bazaar.as_ref() else {
        if panel.display != Display::None {
            panel.display = Display::None;
        }
        return;
    };
    if panel.display != Display::Flex {
        panel.display = Display::Flex;
    }

    let gil = current_gil(snap);
    let start = viewport_start(screen.cursor, view.items.len());
    let focused = view.items.get(screen.cursor);
    let table = icon_cache.table(&dat_root);
    let static_of = |item_no: u16| {
        table
            .as_ref()
            .and_then(|t| item_detail::lookup_static(t, item_no))
    };

    for (tag, mut text, mut color) in text_q.iter_mut() {
        let (want, want_color) = match tag.0 {
            BazaarRole::RowName(i) => match view.items.get(start + i) {
                Some(entry) => (
                    item_name(entry.item_no, static_of(entry.item_no).map(|s| s.name)),
                    row_color(start + i == screen.cursor, entry.total_price(1) <= gil),
                ),
                None => (String::new(), theme::TEXT),
            },
            BazaarRole::RowPrice(i) => match view.items.get(start + i) {
                Some(entry) => (
                    format!("{} G", group_digits(entry.price)),
                    row_color(start + i == screen.cursor, entry.total_price(1) <= gil),
                ),
                None => (String::new(), theme::TEXT),
            },
            // Retail swaps the Current Gil box for the quantity picker while a
            // purchase is being sized.
            BazaarRole::GilLabel => match screen.quantity.as_ref() {
                Some(spin) => (spin.label(), theme::TITLE),
                None => ("Current Gil".to_string(), theme::MUTED),
            },
            BazaarRole::GilValue => match screen.pending.as_ref() {
                Some(buy) => (
                    format!("{} G?", group_digits(buy.total_gil)),
                    if buy.total_gil <= gil {
                        theme::CURSOR
                    } else {
                        theme::DANGER
                    },
                ),
                None => (format!("{} G", group_digits(gil)), theme::TEXT),
            },
            BazaarRole::DetailName => match focused {
                Some(entry) => (
                    item_name(entry.item_no, static_of(entry.item_no).map(|s| s.name)),
                    theme::TITLE,
                ),
                None => ("Nothing for sale.".to_string(), theme::MUTED),
            },
            BazaarRole::DetailBody => match focused.and_then(|e| static_of(e.item_no)) {
                Some(s) => (s.description.clone(), theme::TEXT),
                None => (String::new(), theme::TEXT),
            },
        };
        if **text != want {
            **text = want;
        }
        if color.0 != want_color {
            color.0 = want_color;
        }
    }

    for (icon, mut image) in icon_q.iter_mut() {
        let handle = view
            .items
            .get(start + icon.0)
            .and_then(|e| icon_cache.ensure(e.item_no, &dat_root, &mut images));
        set_icon(&mut image, handle);
    }
    if let Ok(mut image) = detail_icon_q.single_mut() {
        let handle = focused.and_then(|e| icon_cache.ensure(e.item_no, &dat_root, &mut images));
        set_icon(&mut image, handle);
    }
}

/// An empty row/slot keeps its plate but shows no art, so the list holds its
/// retail height instead of collapsing.
fn set_icon(image: &mut ImageNode, handle: Option<Handle<Image>>) {
    let want_alpha = if handle.is_some() { 1.0 } else { 0.0 };
    if let Some(h) = handle {
        if image.image != h {
            image.image = h;
        }
    }
    if image.color.alpha() != want_alpha {
        image.color.set_alpha(want_alpha);
    }
}

/// Retail dims a row the player cannot afford and paints the cursor row gold.
fn row_color(cursor: bool, affordable: bool) -> Color {
    match (cursor, affordable) {
        (true, _) => theme::CURSOR,
        (false, true) => theme::TEXT,
        (false, false) => theme::FAINT,
    }
}

pub fn item_name(item_no: u16, dat_name: Option<String>) -> String {
    dat_name
        .filter(|n| !n.is_empty())
        .or_else(|| ffxi_proto::item_names::lookup(item_no).map(str::to_string))
        .unwrap_or_else(|| format!("Item #{item_no}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: u8, item_no: u16, quantity: u32, price: u32, tax_rate: u16) -> BazaarEntry {
        BazaarEntry {
            index,
            item_no,
            quantity,
            price,
            tax_rate,
        }
    }

    #[test]
    fn total_price_applies_the_zone_tax() {
        // 5% tax = 500 hundredths of a percent (LSB tax/10000 basis points).
        let e = entry(1, 4096, 12, 1000, 500);
        assert_eq!(e.total_price(1), 1050);
        assert_eq!(e.total_price(3), 3150);
        // A tax-free zone charges the asking price exactly.
        assert_eq!(entry(1, 4096, 12, 1000, 0).total_price(4), 4000);
    }

    #[test]
    fn prices_group_into_thousands_like_retail() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(24_999), "24,999");
        assert_eq!(group_digits(14_000_000), "14,000,000");
        assert_eq!(group_digits(1_389_292), "1,389,292");
    }

    #[test]
    fn purchase_prompt_matches_the_retail_wording() {
        assert_eq!(
            purchase_prompt("pieces of hickory lumber", 5, 124_995),
            "Purchase 5 pieces of hickory lumber for 124,995 gil?"
        );
    }

    #[test]
    fn unaffordable_rows_dim_and_the_cursor_row_stays_gold() {
        assert_eq!(row_color(false, true), theme::TEXT);
        assert_eq!(row_color(false, false), theme::FAINT);
        assert_eq!(row_color(true, false), theme::CURSOR);
    }

    #[test]
    fn cursor_wraps_and_clamps_to_a_shrinking_list() {
        let mut s = BazaarScreenState::default();
        s.move_cursor(-1, 3);
        assert_eq!(s.cursor, 2, "up from the top wraps to the end");
        s.move_cursor(1, 3);
        assert_eq!(s.cursor, 0);
        s.cursor = 2;
        s.clamp(1);
        assert_eq!(s.cursor, 0, "a sold-out list pulls the cursor back");
    }

    #[test]
    fn an_empty_list_leaves_the_cursor_alone_and_drops_the_pending_buy() {
        let mut s = BazaarScreenState {
            cursor: 0,
            quantity: Some(Spinner::item(5)),
            pending: Some(PendingBuy {
                index: 1,
                item_no: 4096,
                quantity: 2,
                total_gil: 10,
            }),
        };
        s.move_cursor(1, 0);
        assert_eq!(s.cursor, 0);
        s.clamp(0);
        assert!(s.quantity.is_none());
        assert!(s.pending.is_none());
    }

    #[test]
    fn quantity_picker_only_opens_for_a_real_stack() {
        assert!(BazaarScreenState::begin_quantity(&entry(1, 4096, 1, 100, 0)).is_none());
        let spin = BazaarScreenState::begin_quantity(&entry(1, 4096, 12, 100, 0)).expect("stack");
        assert_eq!(spin.max, 12);
        assert_eq!(spin.confirm(), 1, "defaults to one like retail");
    }

    #[test]
    fn quantity_picker_respects_the_server_cap() {
        let spin = BazaarScreenState::begin_quantity(&entry(1, 4096, 250, 100, 0)).expect("stack");
        assert_eq!(spin.max, MAX_BUY_QUANTITY);
    }

    #[test]
    fn staging_a_purchase_prices_it_and_closes_the_picker() {
        let mut s = BazaarScreenState {
            quantity: Some(Spinner::item(5)),
            ..Default::default()
        };
        let buy = s.stage_purchase(&entry(3, 4096, 5, 1000, 500), 4);
        assert_eq!((buy.index, buy.quantity, buy.total_gil), (3, 4, 4200));
        assert!(s.quantity.is_none(), "picker closes behind the prompt");
        assert_eq!(s.pending, Some(buy));
    }

    #[test]
    fn viewport_follows_the_cursor_without_running_past_the_end() {
        assert_eq!(viewport_start(0, 30), 0);
        assert_eq!(viewport_start(12, 30), 7);
        assert_eq!(viewport_start(29, 30), 20);
        assert_eq!(viewport_start(2, 4), 0, "short lists never scroll");
    }
}
