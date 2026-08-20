//! The /check window for a PC target. Retail draws a compact window — a 4x4
//! equipment grid over a View Wares entry — with the target's name and jobs in
//! the top menu bar and the focused slot's item card in a panel underneath
//! (retail capture 2026-08-04, HorizonXI).
//!
//! The grid layout is [`equipment_screen::EQUIP_GRID`] — the same one our own
//! Equipment window uses, since both windows show the same 16 SAVE_EQUIP_KIND
//! slots and must not drift apart.

use bevy::prelude::*;

use crate::equip_slot::EquipmentIndex;
use crate::hud::equipment_screen::EQUIP_GRID;
use crate::hud::item_dat_root::{ItemDatRoot, ItemIconCache};
use crate::hud::item_grid::spawn_item_cell;
use crate::hud::item_ui::{self, framed_box, text_font, theme, transparent_placeholder};
use crate::snapshot::SceneState;

#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct CheckTarget {
    pub open: bool,
    pub target_id: Option<u32>,
    /// Grid cursor, as an [`EquipmentIndex`] discriminant.
    pub slot: u8,
    /// Focus sits on View Wares rather than on the grid.
    pub on_wares: bool,
}

impl CheckTarget {
    /// Retail opens the window with the cursor already on View Wares whenever
    /// that entry is live, so a bazaar is one keypress away.
    pub fn open(&mut self, target_id: u32, wares_enabled: bool) {
        *self = Self {
            open: true,
            target_id: Some(target_id),
            slot: EquipmentIndex::Main as u8,
            on_wares: wares_enabled,
        };
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    /// Move the cursor. The grid wraps like the Equipment window; leaving the
    /// bottom row downwards lands on View Wares when that entry is live, and
    /// the button hands focus back to the row it came from.
    pub fn move_focus(&mut self, dx: i32, dy: i32, wares_enabled: bool) {
        if self.on_wares {
            if dy < 0 {
                self.on_wares = false;
            }
            return;
        }
        let bottom_row = EQUIP_GRID[EQUIP_GRID.len() - 1];
        let leaving_grid = dy > 0 && bottom_row.iter().any(|&s| s as u8 == self.slot);
        if leaving_grid && wares_enabled {
            self.on_wares = true;
            return;
        }
        self.slot = crate::hud::equipment_screen::grid_move(self.slot, dx, dy);
    }
}

/// Detail lines under the focused slot's item name.
const DETAIL_ROWS: usize = 8;
/// Stand-in while the checked entity is not (or no longer) in the scene.
const UNKNOWN_NAME: &str = "???";
const GRID_COL_PX: f32 = 176.0;
/// The item card sits under the window, as wide as retail's description panel.
const CARD_PX: f32 = 300.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckRole {
    Linkshell,
    Message,
    CellLabel(EquipmentIndex),
    DetailName,
    DetailRow(usize),
    Wares,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct CheckText(CheckRole);

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconSlot {
    Cell(EquipmentIndex),
    Detail,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct CheckIcon(IconSlot);

#[derive(Component, Clone, Copy)]
pub(crate) struct CheckCellFrame(EquipmentIndex);

#[derive(Component)]
pub(crate) struct CheckWaresButton;

#[derive(Component)]
pub struct CheckView;

pub(crate) fn spawn_check_view(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let placeholder = transparent_placeholder(&mut images);

    commands
        .spawn((
            crate::components::InGameEntity,
            CheckView,
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
            // The window itself: the 4x4 slot grid over the View Wares entry.
            let (mut n, bg, bd) = framed_box();
            n.width = Val::Px(GRID_COL_PX);
            root.spawn((n, bg, bd)).with_children(|p| {
                p.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|grid| {
                    for row in EQUIP_GRID.iter() {
                        grid.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(4.0),
                            ..default()
                        })
                        .with_children(|line| {
                            for &slot in row.iter() {
                                spawn_item_cell(
                                    line,
                                    CheckCellFrame(slot),
                                    CheckIcon(IconSlot::Cell(slot)),
                                    CheckText(CheckRole::CellLabel(slot)),
                                    slot.abbr(),
                                    placeholder.clone(),
                                );
                            }
                        });
                    }
                });
                p.spawn((
                    CheckWaresButton,
                    Node {
                        justify_content: JustifyContent::Center,
                        margin: UiRect::top(Val::Px(4.0)),
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(theme::CELL_BG),
                    BorderColor::all(theme::CELL_EDGE),
                ))
                .with_children(|b| {
                    b.spawn((
                        CheckText(CheckRole::Wares),
                        Text::new("View Wares"),
                        text_font(13.0),
                        TextColor(theme::FAINT),
                    ));
                });
            });

            // Under the window: the focused slot's item card. The target's
            // linkshell and bazaar message ride here too — 0x0C9/0x0CA carry
            // them and retail's window has no pane of its own for either.
            let (mut n, bg, bd) = framed_box();
            n.width = Val::Px(CARD_PX);
            root.spawn((n, bg, bd)).with_children(|p| {
                spawn_text(p, CheckRole::Linkshell, 13.0, theme::TEXT);
                spawn_text(p, CheckRole::Message, 13.0, theme::MUTED);
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    margin: UiRect::top(Val::Px(6.0)),
                    ..default()
                })
                .with_children(|h| {
                    h.spawn((
                        CheckIcon(IconSlot::Detail),
                        Node {
                            width: Val::Px(32.0),
                            height: Val::Px(32.0),
                            display: Display::None,
                            ..default()
                        },
                        ImageNode::new(placeholder.clone()),
                    ));
                    h.spawn((
                        CheckText(CheckRole::DetailName),
                        Text::new(""),
                        text_font(14.0),
                        TextColor(theme::TITLE),
                    ));
                });
                for i in 0..DETAIL_ROWS {
                    spawn_text(p, CheckRole::DetailRow(i), 12.0, theme::TEXT);
                }
            });
        });
}

fn spawn_text(p: &mut ChildSpawnerCommands, role: CheckRole, size: f32, color: Color) {
    p.spawn((
        CheckText(role),
        Text::new(""),
        text_font(size),
        TextColor(color),
        Node {
            display: Display::None,
            ..default()
        },
    ));
}

/// Everything the window renders, resolved once per update from the snapshot.
struct CheckModel<'a> {
    check: Option<&'a kuluu_snapshot::CheckResult>,
    message: Option<&'a str>,
    /// The target's linkshell pearl colour, for tinting their linkshell name.
    linkshell_color: Option<Color>,
    wares_enabled: bool,
}

/// The checked PC's name as the scene knows it; the 0x0C9/0x0CA answers carry
/// no name the window can trust on their own.
pub fn target_name(snap: &kuluu_snapshot::SceneSnapshot, target_id: u32) -> String {
    snap.entities
        .iter()
        .find(|e| e.id == target_id)
        .and_then(|e| e.name.clone())
        .unwrap_or_else(|| UNKNOWN_NAME.to_string())
}

/// Whether View Wares is live for `target_id`. The target's own bazaar flag is
/// the retail gate: LSB sets `Flags1.BazaarFlag` from `PChar->hasBazaar()`, i.e.
/// from having any priced inventory slot
/// (vendor/server/src/map/packets/char_update.cpp:318). Shared with the input
/// layer so the rendered state and the key that fires cannot disagree.
pub fn wares_enabled(snap: &kuluu_snapshot::SceneSnapshot, target_id: u32) -> bool {
    snap.entities
        .iter()
        .any(|e| e.id == target_id && e.char_flags.bazaar)
}

fn model<'a>(snap: &'a kuluu_snapshot::SceneSnapshot, target_id: u32) -> CheckModel<'a> {
    let entity = snap.entities.iter().find(|e| e.id == target_id);
    let name = target_name(snap, target_id);
    let linkshell_color = entity.filter(|e| e.char_flags.linkshell).map(|e| {
        let [r, g, b] = e.char_flags.linkshell_color;
        Color::srgb_u8(r, g, b)
    });
    // 0x0CA carries only the target's name, so it is only theirs if it matches.
    let message = snap
        .check_message
        .as_ref()
        .filter(|m| m.name == name)
        .map(|m| m.message.trim())
        .filter(|m| !m.is_empty());
    CheckModel {
        check: snap.check.as_ref().filter(|c| c.target_id == target_id),
        message,
        linkshell_color,
        wares_enabled: wares_enabled(snap, target_id),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_check_view(
    target: Res<CheckTarget>,
    state: Res<SceneState>,
    dat_root: Res<ItemDatRoot>,
    mut icon_cache: ResMut<ItemIconCache>,
    mut images: ResMut<Assets<Image>>,
    mut view_q: Query<
        &mut Node,
        (
            With<CheckView>,
            Without<CheckText>,
            Without<CheckIcon>,
            Without<CheckWaresButton>,
        ),
    >,
    mut text_q: Query<
        (&CheckText, &mut Text, &mut TextColor, &mut Node),
        (Without<CheckView>, Without<CheckIcon>),
    >,
    mut icon_q: Query<(&CheckIcon, &mut Node, &mut ImageNode), Without<CheckView>>,
    mut cell_q: Query<(&CheckCellFrame, &mut BorderColor, &mut BackgroundColor)>,
    mut wares_q: Query<
        (&mut BorderColor, &mut BackgroundColor),
        (With<CheckWaresButton>, Without<CheckCellFrame>),
    >,
) {
    let Ok(mut view_node) = view_q.single_mut() else {
        return;
    };
    let Some(target_id) = target.target_id.filter(|_| target.open) else {
        if view_node.display != Display::None {
            view_node.display = Display::None;
        }
        return;
    };
    if view_node.display != Display::Flex {
        view_node.display = Display::Flex;
    }

    let snap = &state.snapshot;
    let m = model(snap, target_id);
    let focused_slot = EquipmentIndex::from_index(target.slot).unwrap_or(EquipmentIndex::Main);
    let equipped = |slot: EquipmentIndex| -> Option<u16> {
        m.check
            .and_then(|c| c.equipped.get(slot as usize).copied().flatten())
    };
    let focused_item = equipped(focused_slot);
    let (detail_name, detail_rows) =
        item_ui::focus_detail(focused_item, None, snap, &dat_root, &mut icon_cache);

    for (tag, mut text, mut color, mut node) in text_q.iter_mut() {
        let (want, want_color, visible) = role_value(
            tag.0,
            &m,
            focused_slot,
            target.on_wares,
            &detail_name,
            &detail_rows,
        );
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
        let item = match icon.0 {
            IconSlot::Cell(slot) => equipped(slot),
            IconSlot::Detail => focused_item,
        };
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

    for (cell, mut border, mut bg) in cell_q.iter_mut() {
        let focused = cell.0 == focused_slot && !target.on_wares;
        set_frame(&mut border, &mut bg, focused, true);
    }

    if let Ok((mut border, mut bg)) = wares_q.single_mut() {
        set_frame(&mut border, &mut bg, target.on_wares, m.wares_enabled);
    }
}

fn set_frame(border: &mut BorderColor, bg: &mut BackgroundColor, focused: bool, enabled: bool) {
    let want_border = match (focused, enabled) {
        (true, _) => theme::CURSOR,
        (false, true) => theme::CELL_EDGE,
        (false, false) => theme::FAINT,
    };
    if border.left != want_border {
        *border = BorderColor::all(want_border);
    }
    let want_bg = if focused {
        theme::CURSOR_BG
    } else {
        theme::CELL_BG
    };
    if bg.0 != want_bg {
        bg.0 = want_bg;
    }
}

fn role_value(
    role: CheckRole,
    m: &CheckModel,
    focused_slot: EquipmentIndex,
    on_wares: bool,
    detail_name: &str,
    detail_rows: &[String],
) -> (String, Color, bool) {
    match role {
        CheckRole::Linkshell => match m.check.map(|c| c.linkshell.as_str()).unwrap_or("") {
            "" => (String::new(), theme::TEXT, false),
            ls => (
                format!("Linkshell: {ls}"),
                m.linkshell_color.unwrap_or(theme::TEXT),
                true,
            ),
        },
        CheckRole::Message => match m.message {
            Some(msg) => (msg.to_string(), theme::MUTED, true),
            None => (String::new(), theme::MUTED, false),
        },
        CheckRole::CellLabel(slot) => {
            let filled = m
                .check
                .and_then(|c| c.equipped.get(slot as usize).copied().flatten())
                .is_some();
            let color = if slot == focused_slot && !on_wares {
                theme::CURSOR
            } else if filled {
                // Retail keeps the label under the icon, dimmed so the icon reads.
                theme::FAINT
            } else {
                theme::MUTED
            };
            (slot.abbr().to_string(), color, true)
        }
        CheckRole::DetailName => {
            let empty = format!("{}: —", focused_slot.name());
            let text = if detail_name == item_ui::NO_ITEM_PROMPT {
                empty
            } else {
                detail_name.to_string()
            };
            (text, theme::TITLE, true)
        }
        CheckRole::DetailRow(i) => match detail_rows.get(i) {
            Some(line) => (line.clone(), theme::TEXT, true),
            None => (String::new(), theme::TEXT, false),
        },
        CheckRole::Wares => {
            let color = match (on_wares, m.wares_enabled) {
                (true, _) => theme::CURSOR,
                (false, true) => theme::TEXT,
                (false, false) => theme::FAINT,
            };
            ("View Wares".to_string(), color, true)
        }
    }
}

/// `Lv.75 Black Mage / Lv.37 White Mage` — retail levels both jobs and spaces
/// the separator (retail capture 2026-08-04, HorizonXI).
pub fn job_ribbon(check: Option<&kuluu_snapshot::CheckResult>) -> String {
    let job_name = |id: u8| ffxi_vocab::job_names::lookup(u16::from(id)).unwrap_or("Adventurer");
    match check {
        Some(c) if c.main_job != 0 => {
            let main = format!("Lv.{} {}", c.main_job_lv, job_name(c.main_job));
            match c.sub_job {
                0 => main,
                sub => format!("{main} / Lv.{} {}", c.sub_job_lv, job_name(sub)),
            }
        }
        // Zeroed jobs are what /anon looks like on the wire
        // (0x0c9_equip_inspect_general.cpp gates the whole block on isAnon).
        _ => "Lv.? —".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> CheckTarget {
        let mut t = CheckTarget::default();
        t.open(1, false);
        t
    }

    #[test]
    fn grid_navigation_wraps_like_the_equipment_window() {
        let mut t = target();
        t.move_focus(1, 0, false);
        assert_eq!(t.slot, EquipmentIndex::Sub as u8);
        t.move_focus(-1, 0, false);
        assert_eq!(t.slot, EquipmentIndex::Main as u8);
        t.move_focus(0, -1, false);
        assert_eq!(t.slot, EquipmentIndex::Back as u8, "up from the top wraps");
    }

    #[test]
    fn wares_is_reachable_below_the_last_row_only_when_enabled() {
        let mut t = target();
        t.slot = EquipmentIndex::Back as u8;
        t.move_focus(0, 1, false);
        assert!(!t.on_wares, "a bazaar-less target keeps the wrap");
        assert_eq!(t.slot, EquipmentIndex::Main as u8);

        let mut t = target();
        t.slot = EquipmentIndex::Feet as u8;
        t.move_focus(0, 1, true);
        assert!(t.on_wares);
    }

    #[test]
    fn wares_hands_focus_back_to_the_slot_it_came_from() {
        let mut t = target();
        t.slot = EquipmentIndex::Waist as u8;
        t.move_focus(0, 1, true);
        assert!(t.on_wares);
        t.move_focus(1, 0, true);
        assert!(t.on_wares, "sideways does not leave the button");
        t.move_focus(0, -1, true);
        assert!(!t.on_wares);
        assert_eq!(t.slot, EquipmentIndex::Waist as u8);
    }

    #[test]
    fn a_bazaar_target_opens_with_view_wares_already_focused() {
        let mut t = CheckTarget::default();
        t.open(1, true);
        assert!(t.on_wares, "retail parks the cursor on View Wares");
        let mut t = CheckTarget::default();
        t.open(1, false);
        assert!(!t.on_wares, "with no bazaar the grid keeps the cursor");
        assert_eq!(t.slot, EquipmentIndex::Main as u8);
    }

    // Scraped LSB job ids (ffxi_vocab::job_names).
    const BLACK_MAGE: u8 = 4;
    const WHITE_MAGE: u8 = 3;

    #[test]
    fn job_ribbon_levels_both_jobs_like_retail() {
        let mut c = kuluu_snapshot::CheckResult {
            target_id: 1,
            equipped: [None; 16],
            main_job: BLACK_MAGE,
            sub_job: WHITE_MAGE,
            main_job_lv: 75,
            sub_job_lv: 37,
            master_lv: 0,
            linkshell: String::new(),
        };
        assert_eq!(job_ribbon(Some(&c)), "Lv.75 Black Mage / Lv.37 White Mage");
        c.sub_job = 0;
        assert_eq!(job_ribbon(Some(&c)), "Lv.75 Black Mage");
    }

    #[test]
    fn close_clears_the_target_so_the_window_hides() {
        let mut t = target();
        t.on_wares = true;
        t.close();
        assert!(!t.open);
        assert_eq!(t.target_id, None);
        assert!(!t.on_wares);
    }

    #[test]
    fn anonymous_targets_render_the_unknown_job_ribbon() {
        let anon = kuluu_snapshot::CheckResult {
            target_id: 1,
            equipped: [None; 16],
            main_job: 0,
            sub_job: 0,
            main_job_lv: 0,
            sub_job_lv: 0,
            master_lv: 0,
            linkshell: String::new(),
        };
        assert_eq!(job_ribbon(Some(&anon)), "Lv.? —");
        assert_eq!(job_ribbon(None), "Lv.? —");
    }
}
