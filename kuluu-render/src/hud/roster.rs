//! Party HUD — XiUI "compact vertical" bars (see docs/cowxiui_port_layout.md).
//!
//! One panel, one row per party member (self included), always visible — solo or
//! grouped. Each row: nameplate header (name + job/level line), then the compact
//! bar stack from XiUI's `layoutCompact` spec: HP full-width 150px, MP narrow
//! 90px under it, and a thin TP strip for self only. Bar geometry/colors come
//! straight from the CowXIUI config dump (`modules/presets/*.lua`) plus the
//! reference screenshot palette (#1e2a47 panel, #08152e bar tracks).

use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use kuluu_snapshot::PartyMember;

use crate::hud::style::{self, theme};
use crate::snapshot::SceneState;

#[derive(Component)]
pub struct RosterPanel;

#[derive(Component)]
pub struct RosterRow {
    pub member_id: u32,
}

#[derive(Component)]
pub struct RosterNameText;

#[derive(Component)]
pub struct RosterJobsText;

#[derive(Component)]
pub struct RosterBarFill {
    pub stat: BarStat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarStat {
    Hp,
    Mp,
    Tp,
}

// --- XiUI layoutCompact spec (modules/presets/*.lua) -------------------------
const BAR_HEIGHT_PX: f32 = 15.0; // barHeight = 15
const BAR_SPACING_PX: f32 = 8.0; // barSpacing = 8
const HP_BAR_WIDTH_PX: f32 = 150.0; // hpBarWidth = 150 (full width)
const MP_BAR_WIDTH_PX: f32 = 90.0; // mpBarWidth = 90 (narrow, under HP)
// tpBarWidth = 0 in the preset dump; the live reference shows a thin strip for
// self instead — 4px so it reads as an indicator, not a bar.
const TP_STRIP_HEIGHT_PX: f32 = 4.0;

fn bar_width(stat: BarStat) -> f32 {
    match stat {
        BarStat::Hp => HP_BAR_WIDTH_PX,
        BarStat::Mp => MP_BAR_WIDTH_PX,
        BarStat::Tp => HP_BAR_WIDTH_PX, // thin strip spans the HP width
    }
}

// --- Reference palette --------------------------------------------------------
/// XiUI Global "Background Color" #1e2a47 (panel backing), lightly translucent.
const PANEL_BG: Color = Color::srgba(0.118, 0.165, 0.278, 0.92);
/// Bar track behind the fill — dark navy from the reference (#08152e).
const TRACK_BG: Color = Color::srgb(0.012, 0.082, 0.180);
/// Thin light edge around each track (reference bar border, #3f4270-ish).
const TRACK_EDGE: Color = Color::srgb(0.25, 0.26, 0.42);
/// Entity name color — near-white per XiUI Global "Entity Name Color".
const NAME_COLOR: Color = Color::srgb(1.0, 1.0, 0.96);

// Bar fills are vertical gradients (lighter top -> darker bottom), generated
// once as tiny textures and stretched by the UI node.
fn grad_hp() -> ([f32; 3], [f32; 3]) {
    ([0.78, 0.92, 0.58], [0.46, 0.74, 0.32])
}
fn grad_mp() -> ([f32; 3], [f32; 3]) {
    ([0.42, 0.65, 1.00], [0.20, 0.42, 0.88])
}
fn grad_tp() -> ([f32; 3], [f32; 3]) {
    ([1.00, 0.74, 0.30], [0.92, 0.52, 0.16])
}

/// Vertical gradient texture (light top -> dark bottom) for bar fills.
fn bar_gradient(images: &mut Assets<Image>, top: [f32; 3], bottom: [f32; 3]) -> Handle<Image> {
    const W: u32 = 4;
    const H: u32 = 32;
    let mut bytes = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        let t = y as f32 / (H as f32 - 1.0);
        let r = (top[0] + (bottom[0] - top[0]) * t) * 255.0;
        let g = (top[1] + (bottom[1] - top[1]) * t) * 255.0;
        let b = (top[2] + (bottom[2] - top[2]) * t) * 255.0;
        for _x in 0..W {
            bytes.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    images.add(image)
}

/// Generated bar-fill textures. Component on the panel so row builders (which
/// run from the update system with only Commands) reuse them instead of
/// re-generating per party change.
#[derive(Component, Clone)]
pub struct BarGradients {
    pub hp: Handle<Image>,
    pub mp: Handle<Image>,
    pub tp: Handle<Image>,
}

impl BarGradients {
    fn new(images: &mut Assets<Image>) -> Self {
        let (hp_top, hp_bot) = grad_hp();
        let (mp_top, mp_bot) = grad_mp();
        let (tp_top, tp_bot) = grad_tp();
        Self {
            hp: bar_gradient(images, hp_top, hp_bot),
            mp: bar_gradient(images, mp_top, mp_bot),
            tp: bar_gradient(images, tp_top, tp_bot),
        }
    }

    fn handle(&self, stat: BarStat) -> Handle<Image> {
        match stat {
            BarStat::Hp => self.hp.clone(),
            BarStat::Mp => self.mp.clone(),
            BarStat::Tp => self.tp.clone(),
        }
    }
}

pub fn spawn_roster_panel(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let grads = BarGradients::new(&mut images);

    commands.spawn((
        crate::components::InGameEntity,
        RosterPanel,
        grads,
        crate::hud::panel_column::ColumnPanel::ROSTER,
        Node {
            position_type: PositionType::Absolute,

            // Hidden until the first update: `panel_column` has not measured it
            // yet, so showing it at spawn would flash an unpositioned frame.
            display: Display::None,
            right: Val::Px(style::PANEL_COLUMN_RIGHT_PX),
            width: Val::Px(style::PANEL_WIDTH_PX),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(BAR_SPACING_PX),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(theme::FRAME_EDGE),
    ));
}

pub fn update_roster_panel_system(
    state: Res<SceneState>,
    panel_q: Query<(Entity, &BarGradients), With<RosterPanel>>,
    mut panel_node_q: Query<&mut Node, (With<RosterPanel>, Without<RosterBarFill>)>,
    rows_q: Query<(Entity, &RosterRow, &Children)>,
    children_q: Query<&Children>,
    mut name_q: Query<&mut Text, (With<RosterNameText>, Without<RosterJobsText>)>,
    mut jobs_q: Query<&mut Text, (With<RosterJobsText>, Without<RosterNameText>)>,
    mut bar_q: Query<(&RosterBarFill, &mut Node), Without<RosterPanel>>,
    mut commands: Commands,
) {
    let Ok((panel, grads)) = panel_q.single() else {
        return;
    };

    // Always visible — the party HUD doubles as the solo vitals display.
    if let Ok(mut node) = panel_node_q.single_mut() {
        if node.display != Display::Flex {
            node.display = Display::Flex;
        }
    }

    let party = &state.snapshot.party;
    let self_id = state.snapshot.self_char_id;

    // Rebuild rows when the membership shape changes (compare id sets, not
    // order — the server may reorder).
    let existing: Vec<u32> = rows_q.iter().map(|(_, r, _)| r.member_id).collect();
    let want_ids: Vec<u32> = party.iter().map(|m| m.id).collect();
    let shape_changed = {
        let mut a = existing;
        let mut b = want_ids;
        a.sort_unstable();
        b.sort_unstable();
        a != b
    };

    if shape_changed {
        for (e, _, _) in rows_q.iter() {
            commands.entity(e).despawn();
        }
        commands.entity(panel).with_children(|p| {
            for member in party {
                spawn_member_row(p, grads, member, Some(member.id) == self_id);
            }
        });
    }

    // Per-member content: header text + fill widths.
    for (_, row, row_children) in rows_q.iter() {
        let Some(member) = party.iter().find(|m| m.id == row.member_id) else {
            continue;
        };

        for child in row_children.iter() {
            // Header texts (direct children of the row).
            if let Ok(mut text) = name_q.get_mut(child) {
                let want = member.name.clone().unwrap_or_else(|| "?".into());
                if **text != want {
                    **text = want;
                }
                continue;
            }
            if let Ok(mut text) = jobs_q.get_mut(child) {
                let want = jobs_label(member);
                if **text != want {
                    **text = want;
                }
                continue;
            }

            // TP strip: a bare fill that is a direct child of the row.
            if let Ok((fill, mut node)) = bar_q.get_mut(child) {
                node.width = Val::Px(bar_width(fill.stat) * stat_pct(member, fill.stat));
                continue;
            }

            // Track nodes: their children are the fills.
            if let Ok(track_children) = children_q.get(child) {
                for fill_e in track_children.iter() {
                    if let Ok((fill, mut node)) = bar_q.get_mut(fill_e) {
                        node.width = Val::Px(bar_width(fill.stat) * stat_pct(member, fill.stat));
                    }
                }
            }
        }
    }
}

fn spawn_member_row(
    parent: &mut ChildSpawnerCommands,
    grads: &BarGradients,
    member: &PartyMember,
    is_self: bool,
) {
    parent
        .spawn((
            RosterRow {
                member_id: member.id,
            },
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(BAR_SPACING_PX),
                ..default()
            },
        ))
        .with_children(|row| {
            // Nameplate header: name left, job/level right (XiUI style).
            row.spawn(Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Baseline,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|hdr| {
                hdr.spawn((
                    RosterNameText,
                    Text::new(member.name.clone().unwrap_or_else(|| "?".into())),
                    style::text_font(13.0),
                    TextColor(NAME_COLOR),
                ));
                hdr.spawn((
                    RosterJobsText,
                    Text::new(jobs_label(member)),
                    style::text_font(12.0),
                    TextColor(theme::MUTED),
                ));
            });

            spawn_bar(row, grads, BarStat::Hp);
            spawn_bar(row, grads, BarStat::Mp);
            if is_self {
                row.spawn((
                    RosterBarFill { stat: BarStat::Tp },
                    Node {
                        width: Val::Px(0.0), // filled by the update system (tp/3000)
                        height: Val::Px(TP_STRIP_HEIGHT_PX),
                        ..default()
                    },
                    ImageNode::new(grads.handle(BarStat::Tp)),
                ));
            }
        });
}

fn spawn_bar(row: &mut ChildSpawnerCommands, grads: &BarGradients, stat: BarStat) {
    let height = match stat {
        BarStat::Hp | BarStat::Mp => BAR_HEIGHT_PX,
        BarStat::Tp => TP_STRIP_HEIGHT_PX,
    };
    row.spawn((
        Node {
            width: Val::Px(bar_width(stat)),
            height: Val::Px(height),
            border: UiRect::all(Val::Px(1.0)),
            overflow: Overflow::hidden(),
            ..default()
        },
        BackgroundColor(TRACK_BG),
        BorderColor::all(TRACK_EDGE),
    ))
    .with_children(|track| {
        track.spawn((
            RosterBarFill { stat },
            Node {
                width: Val::Px(bar_width(stat)), // updated to pct * width each refresh
                height: Val::Px(height),
                ..default()
            },
            ImageNode::new(grads.handle(stat)),
        ));
    });
}

fn jobs_label(member: &PartyMember) -> String {
    let main = crate::hud::status_panel::job_abbrev(member.main_job);
    if member.sub_job == 0 {
        format!("{main}{lv}", lv = member.main_job_lv)
    } else {
        let sub = crate::hud::status_panel::job_abbrev(member.sub_job);
        format!("{main}{ml}/{sub}{sl}", ml = member.main_job_lv, sl = member.sub_job_lv)
    }
}

fn stat_pct(member: &PartyMember, stat: BarStat) -> f32 {
    let pct = match stat {
        BarStat::Hp => member.hp_pct as f32 / 100.0,
        BarStat::Mp => member.mp_pct as f32 / 100.0,
        BarStat::Tp => (member.tp as f32 / 3000.0).min(1.0),
    };
    pct.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pm(id: u32, hp_pct: u8, mp_pct: u8, tp: u32) -> PartyMember {
        PartyMember {
            id,
            act_index: 0,
            name: Some("Test".into()),
            hp: 1000,
            mp: 500,
            tp,
            hp_pct,
            mp_pct,
            zone_no: 0,
            main_job: 2,
            main_job_lv: 75,
            sub_job: 12,
            sub_job_lv: 75,
            is_party_leader: false,
            is_alliance_leader: false,
            party_no: 0,
            in_mog_house: false,
        }
    }

    #[test]
    fn stat_pct_clamps() {
        assert_eq!(stat_pct(&pm(1, 100, 50, 3000), BarStat::Hp), 1.0);
        assert!((stat_pct(&pm(1, 42, 50, 1500), BarStat::Tp) - 0.5).abs() < f32::EPSILON);
        assert_eq!(stat_pct(&pm(1, 0, 0, 0), BarStat::Mp), 0.0);
    }

    #[test]
    fn jobs_label_main_and_sub() {
        let m = pm(1, 100, 100, 0);
        assert_eq!(jobs_label(&m), "MNK75/SAM75");
        let mut solo = m;
        solo.sub_job = 0;
        assert_eq!(jobs_label(&solo), "MNK75");
    }

    #[test]
    fn bar_widths_match_compact_spec() {
        assert_eq!(bar_width(BarStat::Hp), 150.0);
        assert_eq!(bar_width(BarStat::Mp), 90.0);
        assert_eq!(bar_width(BarStat::Tp), 150.0);
    }
}
