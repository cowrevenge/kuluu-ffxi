//! Shared item-grid cell widget: the bordered square with a centered icon and
//! a small fallback label, used by the equipment screen's 4x4 slot grid and
//! the delivery-box 2x4 grid in the dialog panel.

use bevy::prelude::*;

use crate::hud::style::text_font;
use crate::hud::style::theme;

pub(crate) const CELL_PX: f32 = 36.0;
pub(crate) const ICON_PX: f32 = 30.0;
pub(crate) const CELL_GAP_PX: f32 = 4.0;

/// Type size for a cell's slot name.
const NAME_FONT_PX: f32 = 11.0;

/// Type size for a stack count drawn over item art.
pub(crate) const BADGE_FONT_PX: f32 = 11.0;

const BADGE_PAD_PX: f32 = 2.0;
const BADGE_BORDER_PX: f32 = 1.0;
const BADGE_BG: Color = Color::srgba(0.02, 0.03, 0.06, 0.9);
const BADGE_EDGE: Color = Color::srgb(0.0, 0.0, 0.0);

/// The chip a stack count sits on, merged into the caller's placement. The art
/// under the count is whatever colour the item happens to be, so the digits
/// carry their own dark plate rather than relying on the icon being dark.
pub(crate) fn stack_badge_chip(placement: Node) -> (Node, BackgroundColor, BorderColor) {
    (
        Node {
            padding: UiRect::axes(Val::Px(BADGE_PAD_PX), Val::Px(0.0)),
            border: UiRect::all(Val::Px(BADGE_BORDER_PX)),
            ..placement
        },
        BackgroundColor(BADGE_BG),
        BorderColor::all(BADGE_EDGE),
    )
}

/// What a cell draws over its art.
pub(crate) enum CellOverlay<'a> {
    /// The slot's name, which the art covers once something is in the slot.
    Name(&'a str),
    /// The stack count, which reads on top of the art.
    StackCount,
}

/// Spawn one grid cell: a `CELL_PX` framed square containing a (hidden by
/// default) `ICON_PX` icon and its overlay. Marker components for the
/// frame / icon / overlay are supplied by the caller so each screen can drive
/// its own update systems over the shared structure.
///
/// The overlay is absolutely positioned rather than laid out beside the icon,
/// and child order decides which of the two is on top. A slot name goes
/// *under* the art, so an equipped slot reads as its gear and an empty one as
/// its name; a stack count goes over it, because a count the art hides counts
/// nothing. Observed on the horizonxi-2023 client
/// (.agents/skills/retail-observe/references/vanilla-menu-spec.md,
/// "`/check` on a player → wares + gear").
pub(crate) fn spawn_item_cell(
    p: &mut ChildSpawnerCommands,
    frame_marker: impl Bundle,
    icon_marker: impl Bundle,
    label_marker: impl Bundle,
    overlay: CellOverlay<'_>,
    placeholder: Handle<Image>,
) {
    p.spawn((
        frame_marker,
        Node {
            width: Val::Px(CELL_PX),
            height: Val::Px(CELL_PX),
            border: UiRect::all(Val::Px(1.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(theme::CELL_BG),
        BorderColor::all(theme::CELL_EDGE),
    ))
    .with_children(|c| {
        let placement = Node {
            position_type: PositionType::Absolute,
            ..default()
        };
        match overlay {
            CellOverlay::Name(text) => {
                c.spawn((
                    label_marker,
                    Text::new(text),
                    text_font(NAME_FONT_PX),
                    TextColor(theme::MUTED),
                    placement,
                ));
                spawn_cell_icon(c, icon_marker, placeholder);
            }
            CellOverlay::StackCount => {
                spawn_cell_icon(c, icon_marker, placeholder);
                let (node, bg, edge) = stack_badge_chip(placement);
                c.spawn((
                    label_marker,
                    Text::new(""),
                    text_font(BADGE_FONT_PX),
                    TextColor(theme::TEXT),
                    node,
                    bg,
                    edge,
                ));
            }
        }
    });
}

fn spawn_cell_icon(
    c: &mut ChildSpawnerCommands,
    icon_marker: impl Bundle,
    placeholder: Handle<Image>,
) {
    c.spawn((
        icon_marker,
        Node {
            width: Val::Px(ICON_PX),
            height: Val::Px(ICON_PX),
            display: Display::None,
            ..default()
        },
        ImageNode::new(placeholder),
    ));
}
