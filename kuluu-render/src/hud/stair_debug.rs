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
                top: Val::Px(60.0),
                right: Val::Px(10.0),
                width: Val::Px(360.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
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
                style::text_font(12.0),
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

    // Build the status text. Header, then per-orb lines. Orbs grouped by
    // color so a glance tells you the classification breakdown; totals at
    // the top show how many samples classify each way.
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "STAIR STATUS  drawing={}",
        if snap.drawing_enabled { "on" } else { "off" }
    ));
    lines.push(format!(
        "player  xz=({:+.2},{:+.2})  y={:+.2}",
        snap.player_xz.x, snap.player_xz.y, snap.player_y,
    ));
    lines.push(format!(
        "slope   up={}  down={}",
        fmt_opt(snap.slope_up),
        fmt_opt(snap.slope_down),
    ));
    lines.push(format!(
        "counts  green={}  up-band={}  down-band={}  gray={}  red={}",
        snap.count_green, snap.count_up, snap.count_down, snap.count_gray, snap.count_red,
    ));
    // Per-orb dump. Compact.
    for (i, o) in snap.orbs.iter().take(snap.orb_count).enumerate() {
        lines.push(format!(
            "  #{:02}  {}  xz=({:+.2},{:+.2})  y={:+.2}  dy={:+.2}",
            i, o.tag, o.xz.x, o.xz.y, o.y, o.y - snap.player_y,
        ));
    }
    let want = lines.join("\n");
    if **text != want { **text = want; }
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
            OrbTag::Green => write!(f, "green   "),
            OrbTag::UpBand(n) => write!(f, "up+{}   ", n),
            OrbTag::DownBand(n) => write!(f, "down-{} ", n),
            OrbTag::Gray => write!(f, "gray    "),
            OrbTag::Red => write!(f, "red     "),
            OrbTag::Empty => write!(f, "empty   "),
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
