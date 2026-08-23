use bevy::prelude::*;

use crate::hud::style::{self, theme};

#[derive(Component)]
pub struct StairDebugHud;

#[derive(Component)]
pub struct StairDebugHudText;

pub fn spawn_stair_debug_hud(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            StairDebugHud,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(10.0),
                left: Val::Px(10.0),
                width: Val::Px(520.0),
                max_height: Val::Percent(90.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(theme::FRAME_BG),
            BorderColor::all(theme::CURSOR),
            Visibility::Hidden,
        ))
        .with_children(|p| {
            p.spawn((
                StairDebugHudText,
                Text::new(""),
                style::text_font(11.0),
                TextColor(theme::TEXT),
            ));
        });
}

/// Reads FootprintDebug (owned by the input crate — kuluu-render can't
/// depend on kuluu, so we pull it via a shared resource type re-exported
/// by kuluu_render::stair_debug_view). We keep the render side ignorant of
/// FootprintDebug's field layout by pulling only the summary snapshot.
///
/// A separate system in the kuluu crate populates StairDebugSnapshot from
/// its FootprintDebug each frame.
pub fn update_stair_debug_hud(
    snap: Res<StairDebugSnapshot>,
    panels: Res<crate::hud::HudPanels>,
    mut hud_q: Query<&mut Visibility, With<StairDebugHud>>,
    mut text_q: Query<&mut Text, With<StairDebugHudText>>,
) {
    let Ok(mut vis) = hud_q.single_mut() else { return; };
    let Ok(mut text) = text_q.single_mut() else { return; };

    if !panels.stair_debug {
        if *vis != Visibility::Hidden { *vis = Visibility::Hidden; }
        return;
    }
    if *vis != Visibility::Inherited { *vis = Visibility::Inherited; }

    let want = build_status_text(&snap);
    if **text != want { **text = want; }
}

/// Build the multi-line status string. Layout:
///
///   === STAIR DEBUG ===
///   drawing : on
///   player  : xz=(+0.00,+0.00)  y=+0.00
///   slope   : up=+0.123  down=-
///
///   counts  green=12  up=4  down=2  gray=1  red=0
///
///   -- orbs (grouped) --
///   green:
///     #01 xz=(+0.10,+0.20) y=+0.05 dy=+0.05
///     ...
///   up-band:
///     ...
fn build_status_text(snap: &StairDebugSnapshot) -> String {
    let mut out = String::with_capacity(2048);

    // header block
    out.push_str("=== STAIR DEBUG ===\n");
    out.push_str(&format!(
        "drawing : {}\n",
        if snap.drawing_enabled { "on" } else { "off" }
    ));
    out.push_str(&format!(
        "player  : xz=({:+.2},{:+.2})  y={:+.2}\n",
        snap.player_xz.x, snap.player_xz.y, snap.player_y,
    ));
    out.push_str(&format!(
        "slope   : up={}  down={}\n",
        fmt_opt(snap.slope_up),
        fmt_opt(snap.slope_down),
    ));

    out.push('\n');
    out.push_str(&format!(
        "counts  : green={}  up={}  down={}  gray={}  red={}\n",
        snap.count_green,
        snap.count_up,
        snap.count_down,
        snap.count_gray,
        snap.count_red,
    ));

    // per-orb dump, grouped by tag so you can scan a category at a glance
    out.push('\n');
    out.push_str("-- orbs (grouped) --\n");

    let live = &snap.orbs[..snap.orb_count.min(snap.orbs.len())];

    push_group(&mut out, "green",   live, snap.player_y, |t| matches!(t, OrbTag::Green));
    push_group(&mut out, "up-band", live, snap.player_y, |t| matches!(t, OrbTag::UpBand(_)));
    push_group(&mut out, "down-band", live, snap.player_y, |t| matches!(t, OrbTag::DownBand(_)));
    push_group(&mut out, "gray",    live, snap.player_y, |t| matches!(t, OrbTag::Gray));
    push_group(&mut out, "red",     live, snap.player_y, |t| matches!(t, OrbTag::Red));

    out
}

fn push_group(
    out: &mut String,
    label: &str,
    orbs: &[OrbInfo],
    player_y: f32,
    pred: impl Fn(&OrbTag) -> bool,
) {
    let matching: Vec<(usize, &OrbInfo)> = orbs
        .iter()
        .enumerate()
        .filter(|(_, o)| pred(&o.tag))
        .collect();
    if matching.is_empty() {
        return;
    }
    out.push_str(&format!("{}: ({})\n", label, matching.len()));
    for (i, o) in matching {
        out.push_str(&format!(
            "  #{:02} {} xz=({:+.2},{:+.2}) y={:+.2} dy={:+.2}\n",
            i,
            o.tag,
            o.xz.x,
            o.xz.y,
            o.y,
            o.y - player_y,
        ));
    }
}

fn fmt_opt(v: Option<f32>) -> String {
    match v {
        Some(x) => format!("{:+.3}", x),
        None => "-".to_string(),
    }
}

/// Snapshot of stair-debug state, populated by the input crate each frame
/// and consumed by the render crate's status panel. Kept as a plain data
/// resource so the render crate has no dependency on the input crate.
#[derive(Resource, Debug, Clone)]
pub struct StairDebugSnapshot {
    pub drawing_enabled: bool,
    pub player_xz: Vec2,
    pub player_y: f32,
    pub slope_up: Option<f32>,
    pub slope_down: Option<f32>,
    pub count_green: usize,
    pub count_up: usize,
    pub count_down: usize,
    pub count_gray: usize,
    pub count_red: usize,
    pub orb_count: usize,
    pub orbs: [OrbInfo; 60],
}

#[derive(Debug, Clone, Copy)]
pub struct OrbInfo {
    pub xz: Vec2,
    pub y: f32,
    pub tag: OrbTag,
}

#[derive(Debug, Clone, Copy)]
pub enum OrbTag {
    Green,
    UpBand(i8),
    DownBand(i8),
    Gray,
    Red,
    Empty,
}

impl std::fmt::Display for OrbTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrbTag::Green => write!(f, "green  "),
            OrbTag::UpBand(n) => write!(f, "up+{:<3}", n),
            OrbTag::DownBand(n) => write!(f, "down-{:<2}", n),
            OrbTag::Gray => write!(f, "gray   "),
            OrbTag::Red => write!(f, "red    "),
            OrbTag::Empty => write!(f, "empty  "),
        }
    }
}

impl Default for StairDebugSnapshot {
    fn default() -> Self {
        Self {
            drawing_enabled: true,
            player_xz: Vec2::ZERO,
            player_y: 0.0,
            slope_up: None,
            slope_down: None,
            count_green: 0,
            count_up: 0,
            count_down: 0,
            count_gray: 0,
            count_red: 0,
            orb_count: 0,
            orbs: [OrbInfo { xz: Vec2::ZERO, y: 0.0, tag: OrbTag::Empty }; 60],
        }
    }
}
