//! XIUI-style party/alliance frame — the single party/self HUD.
//!
//! Replaces the old self_hud panel: self is row 0 of Party A (XIUI behavior),
//! so there is exactly ONE panel drawing player/party state. self_hud's spawn
//! and update are unwired in hud/mod.rs; this module owns the ROSTER column
//! slot.
//!
//! Step 1 (P0) of the party-frame spec: three windows grouped by `party_no`
//! (A=0, B=1, C=2), L1 "compact vertical" geometry for Party A, L2 "super
//! compact" for alliance B/C. Per row: job-abbrev text (icon later), HP bar
//! with the name overlaid on its top edge, MP bar below, TP text, HP color
//! ramp, leader dots, out-of-zone black-block, target highlight, click-to-
//! target. Buffs / casts / distance / debug panel are later steps.
//!
//! Data: SceneSnapshot.party (already populated from GROUP_LIST 0x0DD /
//! GROUP_ATTR 0x0DF). No protocol work needed for this step.

use bevy::prelude::*;

use crate::hud::status_panel::job_abbrev;
use crate::hud::style::{self, theme};
use crate::scene::Target;
use crate::snapshot::SceneState;

// ---- geometry constants (spec sections 4/5) -----------------------------

// L1 — Party A "compact vertical". Widths are template * BASE_MULT.
const BASE_MULT: f32 = 0.8;
const L1_HP_BASE_W: f32 = 150.0;
const L1_MP_BASE_W: f32 = 100.0;
const L1_HP_W_MULT: f32 = 0.82;
const L1_MP_EXTRA_W_MULT: f32 = 0.9;
const L1_BAR_H: f32 = 20.0;
#[allow(dead_code)]
const L1_ICON_SIZE: f32 = 28.0;
#[allow(dead_code)]
const L1_BAR_INSET: f32 = 4.0;

fn l1_hp_w() -> f32 {
    L1_HP_BASE_W * BASE_MULT * L1_HP_W_MULT
} // ~98
fn l1_mp_w() -> f32 {
    L1_MP_BASE_W * BASE_MULT * L1_HP_W_MULT * L1_MP_EXTRA_W_MULT
} // ~74

// L2 — Alliance B/C "super compact".
const L2_HP_BASE_W: f32 = 135.0;
const L2_MP_BASE_W: f32 = 80.0;
const L2_BAR_H: f32 = 12.0;

fn l2_hp_w() -> f32 {
    L2_HP_BASE_W * BASE_MULT
} // ~108
fn l2_mp_w() -> f32 {
    L2_MP_BASE_W * BASE_MULT
} // ~64

// Text sizes
const NAME_PX: f32 = 12.0;
const JOB_PX: f32 = 11.0;
const TITLE_PX: f32 = 14.0;

// Bar colors
const MP_COLOR: Color = Color::srgb(0.30, 0.50, 0.90);
const TP_FULL: Color = Color::srgb(1.00, 0.80, 0.20);
const TP_DIM: Color = Color::srgb(0.55, 0.55, 0.55);
const BAR_TRACK: Color = theme::CELL_BG;
const OUT_OF_ZONE_BLOCK: Color = Color::srgb(0.02, 0.02, 0.02);
const LEADER_DOT: Color = Color::srgb(1.00, 0.82, 0.25);

// ---- components ----------------------------------------------------------

/// Root of one party window (A/B/C).
#[derive(Component)]
pub struct PartyFrameRoot {
    pub party_no: u8,
}

/// Title text ("Solo"/"Party"/"Party B"/"Party C").
#[derive(Component)]
pub struct PartyTitle {
    pub party_no: u8,
}

/// Container that holds the member rows for one window.
#[derive(Component)]
pub struct PartyRowsHost {
    pub party_no: u8,
}

// ---- HP color ramp (spec 6.1) -------------------------------------------

pub fn hp_ramp(pct: u8) -> Color {
    let p = pct as f32;
    if p >= 70.0 {
        Color::srgb(0.25, 0.80, 0.30)
    } else if p >= 40.0 {
        // yellow-green -> yellow lerp across 40..70
        let t = (p - 40.0) / 30.0;
        lerp_rgb((0.85, 0.80, 0.20), (0.35, 0.75, 0.25), t)
    } else if p >= 20.0 {
        // orange band 20..40
        let t = (p - 20.0) / 20.0;
        lerp_rgb((0.90, 0.45, 0.15), (0.85, 0.75, 0.20), t)
    } else {
        Color::srgb(0.85, 0.20, 0.20)
    }
}

fn lerp_rgb(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::srgb(
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

// ---- spawn ---------------------------------------------------------------

pub fn spawn_party_frames(mut commands: Commands) {
    // Party A always exists (self is row 0). B/C spawn hidden; the update
    // system toggles Display based on whether that window has members.
    for party_no in 0u8..3 {
        commands
            .spawn((
                crate::components::InGameEntity,
                PartyFrameRoot { party_no },
                crate::hud::panel_column::ColumnPanel::ROSTER,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(style::PANEL_COLUMN_BOTTOM_PX),
                    right: Val::Px(style::PANEL_COLUMN_RIGHT_PX),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect {
                        left: Val::Px(10.0),
                        right: Val::Px(10.0),
                        top: Val::Px(TITLE_PX * 0.75 + 3.0),
                        bottom: Val::Px(6.0),
                    },
                    border: UiRect::all(Val::Px(1.0)),
                    row_gap: Val::Px(2.0),
                    overflow: Overflow::visible(),
                    // A visible by default; B/C hidden until populated.
                    display: if party_no == 0 {
                        Display::Flex
                    } else {
                        Display::None
                    },
                    ..default()
                },
                BackgroundColor(theme::FRAME_BG),
                BorderColor::all(theme::FRAME_EDGE),
            ))
            .with_children(|root| {
                // Title straddling the top border.
                root.spawn((
                    PartyTitle { party_no },
                    Text::new(if party_no == 0 { "Solo" } else { "" }),
                    style::text_font(TITLE_PX),
                    TextColor(theme::TEXT),
                    TextLayout::justify(Justify::Center),
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(-TITLE_PX * 0.75),
                        left: Val::Px(0.0),
                        right: Val::Px(0.0),
                        ..default()
                    },
                ));
                // Rows host (member entries are (de)spawned here each frame).
                root.spawn((
                    PartyRowsHost { party_no },
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(3.0),
                        ..default()
                    },
                ));
            });
    }
}

// ---- per-frame update ----------------------------------------------------

/// Rebuilds the member rows from the snapshot each dirty frame. Simple
/// clear-and-respawn (v1): despawn all row children, respawn from grouped
/// members. Row counts are tiny (<=6 per window) so the churn is negligible;
/// the spec's diff-and-reuse is a later optimization.
pub fn update_party_frame_system(
    mut commands: Commands,
    state: Res<SceneState>,
    target: Res<Target>,
    mut root_q: Query<(&PartyFrameRoot, &mut Node), Without<PartyRowsHost>>,
    mut title_q: Query<(&PartyTitle, &mut Text)>,
    host_q: Query<(Entity, &PartyRowsHost, Option<&Children>)>,
) {
    if !state.dirty {
        return;
    }
    let snap = &state.snapshot;

    // Self zone for out-of-zone comparison.
    let self_zone = crate::snapshot::resolve_self(&snap.party, snap.self_char_id).map(|m| m.zone_no);

    // Group members by party_no (0/1/2); sort each by act_index.
    let self_id = crate::snapshot::resolve_self(&snap.party, snap.self_char_id).map(|m| m.id);

    let mut windows: [Vec<&kuluu_snapshot::PartyMember>; 3] = [vec![], vec![], vec![]];
    for m in &snap.party {
        // Self is ALWAYS row 0 of window A — even when solo, where the server
        // reports party_no == NO_PARTY (3). Everyone else groups by party_no
        // (0/1/2); anything else is skipped.
        if Some(m.id) == self_id {
            windows[0].push(m);
        } else if (m.party_no as usize) < 3 {
            windows[m.party_no as usize].push(m);
        }
    }
    // Sort each window by act_index, but keep self pinned to row 0 of A.
    for (i, w) in windows.iter_mut().enumerate() {
        if i == 0 {
            w.sort_by_key(|m| (Some(m.id) != self_id, m.act_index));
        } else {
            w.sort_by_key(|m| m.act_index);
        }
    }

    // Any member in party 1/2 => we're in an alliance.
    let in_alliance = !windows[1].is_empty() || !windows[2].is_empty();
    // "In a party" for the A title = self has a real party_no (not NO_PARTY).
    let self_in_party = crate::snapshot::resolve_self(&snap.party, snap.self_char_id)
        .map(|m| m.party_no != ffxi_proto::decode::NO_PARTY)
        .unwrap_or(false);

    // Show/hide window roots.
    for (root, mut node) in root_q.iter_mut() {
        let has_members = !windows[root.party_no as usize].is_empty();
        let show = root.party_no == 0 || has_members;
        node.display = if show { Display::Flex } else { Display::None };
    }

    // Titles.
    for (title, mut text) in title_q.iter_mut() {
        let want = match title.party_no {
            0 => {
                if self_in_party {
                    "Party"
                } else {
                    "Solo"
                }
            }
            1 => "Party B",
            2 => "Party C",
            _ => "",
        };
        if **text != want {
            **text = want.to_string();
        }
    }
    let _ = in_alliance; // reserved for future alliance-title tile work

    // Rebuild rows per window.
    for (host_entity, host, children) in host_q.iter() {
        // Despawn existing rows.
        if let Some(children) = children {
            for c in children.iter() {
                commands.entity(c).despawn();
            }
        }
        let members = &windows[host.party_no as usize];
        let is_l1 = host.party_no == 0;

        commands.entity(host_entity).with_children(|host_cb| {
            for m in members {
                let out_of_zone = matches!((self_zone, Some(m.zone_no)), (Some(sz), Some(mz)) if sz != mz);
                let is_target = target.id == Some(m.id);
                // Self's PartyMember.name is often None (the party packet
                // doesn't carry own name); fall back to the snapshot char_name.
                let name_override = if Some(m.id) == self_id {
                    snap.char_name.clone()
                } else {
                    None
                };
                spawn_member_row(host_cb, m, is_l1, out_of_zone, is_target, name_override);
            }
        });
    }
}

fn spawn_member_row(
    parent: &mut ChildSpawnerCommands,
    m: &kuluu_snapshot::PartyMember,
    is_l1: bool,
    out_of_zone: bool,
    is_target: bool,
    name_override: Option<String>,
) {
    let hp_w = if is_l1 { l1_hp_w() } else { l2_hp_w() };
    let mp_w = if is_l1 { l1_mp_w() } else { l2_mp_w() };
    let bar_h = if is_l1 { L1_BAR_H } else { L2_BAR_H };
    let mp_h = bar_h * 0.75;

    let name = name_override
        .or_else(|| m.name.clone())
        .unwrap_or_else(|| "???".to_string());
    let name_label = if out_of_zone {
        format!("{} ({})", name, short_zone(m.zone_no))
    } else {
        name
    };
    let job = job_abbrev(m.main_job);

    // Left column width: enough for name + "JOB 00 9999" line.
    let left_w = 92.0;

    // Row container (optional target highlight behind everything).
    let mut row = parent.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(6.0),
        padding: UiRect {
            left: Val::Px(2.0),
            right: Val::Px(2.0),
            top: Val::Px(NAME_PX * 0.5),
            bottom: Val::Px(2.0),
        },
        ..default()
    });
    if is_target {
        row.insert(BackgroundColor(Color::srgba(0.35, 0.55, 0.95, 0.35)));
    }

    row.with_children(|row| {
        // ---- LEFT column: name (row1), "JOB lv  TP" (row2) ----
        row.spawn(Node {
            width: Val::Px(left_w),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|left| {
            // Leader dot + name on one line.
            left.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|nameline| {
                if m.is_party_leader || m.is_alliance_leader {
                    nameline.spawn((
                        Node {
                            width: Val::Px(6.0),
                            height: Val::Px(6.0),
                            border_radius: BorderRadius::MAX,
                            flex_shrink: 0.0,
                            ..default()
                        },
                        BackgroundColor(LEADER_DOT),
                    ));
                }
                nameline.spawn((
                    Text::new(name_label),
                    style::text_font(NAME_PX),
                    TextColor(Color::WHITE),
                ));
            });

            // "JOB lv   TP"  (job + level, then TP in gold when >=1000).
            left.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|jobline| {
                jobline.spawn((
                    Text::new(job),
                    style::text_font(JOB_PX),
                    TextColor(theme::MUTED),
                ));
                if is_l1 {
                    jobline.spawn((
                        Text::new(format!("{}", m.tp)),
                        style::text_font(JOB_PX),
                        TextColor(if m.tp >= 1000 { TP_FULL } else { TP_DIM }),
                    ));
                }
            });
        });

        // ---- RIGHT column: HP bar (row1), MP bar (row2) ----
        row.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|bars| {
            // HP bar.
            bars.spawn((
                Node {
                    width: Val::Px(hp_w),
                    height: Val::Px(bar_h),
                    flex_shrink: 0.0,
                    position_type: PositionType::Relative,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(BAR_TRACK),
            ))
            .with_children(|hp| {
                if out_of_zone {
                    hp.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(OUT_OF_ZONE_BLOCK),
                    ));
                } else {
                    hp.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            width: Val::Percent(m.hp_pct as f32),
                            ..default()
                        },
                        BackgroundColor(hp_ramp(m.hp_pct)),
                    ));
                    hp.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(4.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            justify_content: JustifyContent::FlexEnd,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|v| {
                        v.spawn((
                            Text::new(format!("{}", m.hp)),
                            style::text_font(NAME_PX),
                            TextColor(Color::WHITE),
                        ));
                    });
                }
            });

            // MP bar (narrower + shorter), value right-aligned inside.
            bars.spawn((
                Node {
                    width: Val::Px(mp_w),
                    height: Val::Px(mp_h),
                    flex_shrink: 0.0,
                    position_type: PositionType::Relative,
                    overflow: Overflow::clip(),
                    // right-align the MP bar under the HP bar's right edge
                    margin: UiRect {
                        left: Val::Px(hp_w - mp_w),
                        ..default()
                    },
                    ..default()
                },
                BackgroundColor(BAR_TRACK),
            ))
            .with_children(|mp| {
                if !out_of_zone {
                    mp.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            width: Val::Percent(m.mp_pct as f32),
                            ..default()
                        },
                        BackgroundColor(MP_COLOR),
                    ));
                    mp.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(4.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            justify_content: JustifyContent::FlexEnd,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|v| {
                        v.spawn((
                            Text::new(format!("{}", m.mp)),
                            style::text_font(JOB_PX),
                            TextColor(Color::WHITE),
                        ));
                    });
                }
            });
        });
    });
}

/// XIUI shortenZoneName: strip apostrophes; "X of Y" -> Y; 2 words -> first
/// 2 chars + second word; 3+ -> initials of all but last + last word. Zone id
/// is passed through; without a name lookup we show the raw id as a fallback.
fn short_zone(zone_no: u16) -> String {
    // TODO: wire to the minimap/zone-flash zone-name lookup. Until then the id
    // is a stable placeholder so the out-of-zone marker still reads.
    format!("Z{zone_no}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hp_ramp_bands() {
        // High = green-ish (G dominant), low = red-ish (R dominant).
        let hi = hp_ramp(100);
        let lo = hp_ramp(5);
        let hi_srgba = hi.to_srgba();
        let lo_srgba = lo.to_srgba();
        assert!(hi_srgba.green > hi_srgba.red, "high hp should be green-dominant");
        assert!(lo_srgba.red > lo_srgba.green, "low hp should be red-dominant");
    }

    #[test]
    fn l1_bar_widths_reasonable() {
        assert!((l1_hp_w() - 98.4).abs() < 1.0);
        assert!((l1_mp_w() - 59.0).abs() < 2.0);
    }
}
