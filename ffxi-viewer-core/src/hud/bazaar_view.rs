//! The wares list of a browsed bazaar (View Wares in the /check window).
//!
//! Rows come straight from the server's s2c 0x105 packets, one per priced
//! seller slot; the seller's own bazaar is the authority, so this window only
//! renders what arrived and sends purchases by slot index.

use bevy::prelude::*;
use ffxi_viewer_wire::{BazaarEntry, SceneSnapshot};

use crate::hud::delivery::current_gil;
use crate::hud::item_dat_root::{ItemDatRoot, ItemIconCache};
use crate::hud::item_ui::{framed_box, text_font, theme, transparent_placeholder};
use crate::hud::spinner::Spinner;
use crate::snapshot::SceneState;

/// Visible rows before the list scrolls.
pub const LIST_ROWS: usize = 10;

/// LSB's purchase validator caps a single buy at 99
/// (vendor/server/src/map/packets/c2s/0x106_bazaar_buy.cpp:43).
pub const MAX_BUY_QUANTITY: u32 = 99;

/// Cursor + pending-quantity state for the wares window. The row list itself
/// lives in the snapshot.
#[derive(Resource, Debug, Clone, Default)]
pub struct BazaarScreenState {
    pub cursor: usize,
    /// Active quantity picker for the focused row, once confirmed into.
    pub quantity: Option<Spinner>,
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
}

#[derive(Component)]
pub struct BazaarPanel;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BazaarRole {
    Header,
    Gil,
    Row(usize),
    Empty,
    Footer,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct BazaarText(BazaarRole);

#[derive(Component, Clone, Copy)]
pub(crate) struct BazaarRowIcon(usize);

const PANEL_WIDTH_PX: f32 = 380.0;
const ROW_ICON_PX: f32 = 20.0;

pub(crate) fn spawn_bazaar_view(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let placeholder = transparent_placeholder(&mut images);
    let (mut n, bg, bd) = framed_box();
    n.position_type = PositionType::Absolute;
    n.top = Val::Percent(22.0);
    n.left = Val::Percent(34.0);
    n.width = Val::Px(PANEL_WIDTH_PX);
    n.display = Display::None;

    commands
        .spawn((crate::components::InGameEntity, BazaarPanel, n, bg, bd))
        .with_children(|p| {
            spawn_text(p, BazaarRole::Header, 14.0, theme::TITLE);
            spawn_text(p, BazaarRole::Gil, 12.0, theme::MUTED);
            spawn_text(p, BazaarRole::Empty, 13.0, theme::MUTED);
            for i in 0..LIST_ROWS {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        BazaarRowIcon(i),
                        Node {
                            width: Val::Px(ROW_ICON_PX),
                            height: Val::Px(ROW_ICON_PX),
                            display: Display::None,
                            ..default()
                        },
                        ImageNode::new(placeholder.clone()),
                    ));
                    row.spawn((
                        BazaarText(BazaarRole::Row(i)),
                        Text::new(""),
                        text_font(13.0),
                        TextColor(theme::TEXT),
                    ));
                });
            }
            spawn_text(p, BazaarRole::Footer, 12.0, theme::MUTED);
        });
}

fn spawn_text(p: &mut ChildSpawnerCommands, role: BazaarRole, size: f32, color: Color) {
    p.spawn((
        BazaarText(role),
        Text::new(""),
        text_font(size),
        TextColor(color),
        Node {
            display: Display::None,
            ..default()
        },
    ));
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
        ),
    >,
    mut text_q: Query<(&BazaarText, &mut Text, &mut TextColor, &mut Node), Without<BazaarRowIcon>>,
    mut icon_q: Query<(&BazaarRowIcon, &mut Node, &mut ImageNode), Without<BazaarText>>,
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

    for (tag, mut text, mut color, mut node) in text_q.iter_mut() {
        let (want, want_color, visible) = match tag.0 {
            BazaarRole::Header => {
                let owner = if view.seller_name.is_empty() {
                    "Bazaar".to_string()
                } else {
                    format!("{}'s Bazaar", view.seller_name)
                };
                (owner, theme::TITLE, true)
            }
            BazaarRole::Gil => (format!("Gil: {gil}"), theme::MUTED, true),
            BazaarRole::Empty => (
                "Nothing for sale.".to_string(),
                theme::MUTED,
                view.items.is_empty(),
            ),
            BazaarRole::Row(i) => match view.items.get(start + i) {
                Some(entry) => {
                    let cursor = start + i == screen.cursor;
                    let qty = screen
                        .quantity
                        .as_ref()
                        .filter(|_| cursor)
                        .map(|s| s.confirm())
                        .unwrap_or(1);
                    (
                        row_label(entry, qty, cursor && screen.quantity.is_some()),
                        if cursor { theme::CURSOR } else { theme::TEXT },
                        true,
                    )
                }
                None => (String::new(), theme::TEXT, false),
            },
            BazaarRole::Footer => match view.items.get(screen.cursor) {
                Some(entry) if !view.items.is_empty() => {
                    let qty = screen.quantity.as_ref().map(|s| s.confirm()).unwrap_or(1);
                    let total = entry.total_price(qty);
                    let affordable = total <= gil;
                    (
                        format!(
                            "Total {total} gil (tax {:.2}%)",
                            f64::from(entry.tax_rate) / TAX_PERCENT_SCALE
                        ),
                        if affordable {
                            theme::MUTED
                        } else {
                            theme::DANGER
                        },
                        true,
                    )
                }
                _ => (String::new(), theme::MUTED, false),
            },
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if visible && **text != want {
            **text = want;
        }
        if color.0 != want_color {
            color.0 = want_color;
        }
    }

    for (icon, mut node, mut image) in icon_q.iter_mut() {
        let item = view.items.get(start + icon.0).map(|e| e.item_no);
        match item.and_then(|n| icon_cache.ensure(n, &dat_root, &mut images)) {
            Some(h) => {
                if image.image != h {
                    image.image = h;
                }
                if node.display != Display::Flex {
                    node.display = Display::Flex;
                }
            }
            None => {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
}

/// `tax_rate` is in hundredths of a percent (LSB basis points / 100).
const TAX_PERCENT_SCALE: f64 = 100.0;

fn row_label(entry: &BazaarEntry, quantity: u32, picking: bool) -> String {
    let name = ffxi_proto::item_names::lookup(entry.item_no)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Item #{}", entry.item_no));
    let stack = if entry.quantity > 1 {
        format!(" x{}", entry.quantity)
    } else {
        String::new()
    };
    if picking {
        format!("{name}{stack}  {} gil ea.  [buy {quantity}]", entry.price)
    } else {
        format!("{name}{stack}  {} gil ea.", entry.price)
    }
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
    fn an_empty_list_leaves_the_cursor_alone_and_drops_the_picker() {
        let mut s = BazaarScreenState {
            cursor: 0,
            quantity: Some(Spinner::item(5)),
        };
        s.move_cursor(1, 0);
        assert_eq!(s.cursor, 0);
        s.clamp(0);
        assert!(s.quantity.is_none());
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
    fn viewport_follows_the_cursor_without_running_past_the_end() {
        assert_eq!(viewport_start(0, 30), 0);
        assert_eq!(viewport_start(12, 30), 7);
        assert_eq!(viewport_start(29, 30), 20);
        assert_eq!(viewport_start(2, 4), 0, "short lists never scroll");
    }

    #[test]
    fn row_label_shows_the_stack_and_unit_price() {
        let label = row_label(&entry(3, 4096, 12, 250, 500), 1, false);
        assert!(label.contains("x12"), "{label}");
        assert!(label.contains("250 gil ea."), "{label}");
        assert!(!label.contains("buy"), "{label}");
        let picking = row_label(&entry(3, 4096, 12, 250, 500), 4, true);
        assert!(picking.contains("[buy 4]"), "{picking}");
    }
}
