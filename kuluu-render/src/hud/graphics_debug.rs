//! Graphics-debug panel: window/image/panel metrics split out of the stair
//! HUD, plus the rolling panel-position capture (panelpositions.txt) behind
//! its own Debug-menu toggle so the game never spams a log unasked.

use bevy::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// Actual swapchain/surface size as the RENDER world sees it, bridged to the
/// main world through atomics (debug-only plumbing). If the user's stale
/// initial-value theory is right, `win=` will move on resize while this
/// stays frozen at the boot size until a fullscreen toggle forces the full
/// reconfigure path.
pub static SURFACE_W: AtomicU32 = AtomicU32::new(0);
pub static SURFACE_H: AtomicU32 = AtomicU32::new(0);

/// RENDER-WORLD system: records the primary window's extracted surface size.
pub fn record_surface_size(windows: Res<bevy::render::view::ExtractedWindows>) {
    if let Some(w) = windows.primary.and_then(|e| windows.windows.get(&e)) {
        SURFACE_W.store(w.physical_width, Ordering::Relaxed);
        SURFACE_H.store(w.physical_height, Ordering::Relaxed);
    }
}

#[derive(Resource, Default)]
pub struct GraphicsDebugState {
    /// Window physical size + scale factor.
    pub win: (u32, u32, f32),
    /// Render-scale off-screen image size (0x0 when the path is inactive).
    pub img: (u32, u32),
    /// The measured panel's laid-out rect: center (physical px) + size.
    pub panel: (f32, f32, f32, f32),
}

#[derive(Component)]
pub struct GraphicsDebugHud;

#[derive(Component)]
pub struct GraphicsDebugText;

pub fn spawn_graphics_debug_hud(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            GraphicsDebugHud,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(10.0),
                left: Val::Px(540.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(crate::hud::style::theme::FRAME_BG),
            BorderColor::all(crate::hud::style::theme::CURSOR),
            Visibility::Hidden,
        ))
        .with_children(|p| {
            p.spawn((GraphicsDebugText, Text::new("")));
        });
}

pub fn update_graphics_debug_hud(
    panels: Res<crate::hud::HudPanels>,
    state: Res<GraphicsDebugState>,
    mut q_root: Query<&mut Visibility, With<GraphicsDebugHud>>,
    mut q_text: Query<&mut Text, With<GraphicsDebugText>>,
) {
    let Ok(mut vis) = q_root.single_mut() else {
        return;
    };
    if !panels.graphics_debug {
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
        return;
    }
    if *vis != Visibility::Inherited {
        *vis = Visibility::Inherited;
    }
    let Ok(mut text) = q_text.single_mut() else {
        return;
    };
    // img: "off" when render-scale isn't producing an off-screen image
    // (Render Scale = 100%), otherwise the target size.
    let img = if state.img.0 == 0 && state.img.1 == 0 {
        "off (Render Scale = 100%)".to_string()
    } else {
        format!("{}x{}", state.img.0, state.img.1)
    };
    let (sw, sh) = (SURFACE_W.load(Ordering::Relaxed), SURFACE_H.load(Ordering::Relaxed));
    let agree = if sw == state.win.0 && sh == state.win.1 {
        "MATCH"
    } else {
        "MISMATCH"
    };
    let s = format!(
        "=== GRAPHICS DEBUG ===\nwin   : {}x{}  sf={:.3}\nsurf  : {}x{}  [{}]\nimg   : {}\npanel : Party   ({:.2},{:.2}) {:.2}x{:.2}\nposlog: {}",
        state.win.0,
        state.win.1,
        state.win.2,
        sw,
        sh,
        agree,
        img,
        state.panel.0,
        state.panel.1,
        state.panel.2,
        state.panel.3,
        if panels.position_log { "on" } else { "off" },
    );
    if text.0 != s {
        text.0 = s;
    }
}

/// Metrics + optional position log. Measures the STAIR panel's rect (the
/// jitter proxy) via UiGlobalTransform -- the component bevy 0.19 layout
/// actually writes (plain GlobalTransform on UI stays identity). Duplicate
/// tolerant: takes the largest laid-out match. The file capture only runs
/// while the Debug-menu "Position Log" toggle is on; turning it off clears
/// the buffer so a later session starts fresh.
pub fn graphics_debug_metrics_system(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    // Party/self frame (HP/MP/TP/job/Solo). self_hud::SelfHud marks the
    // Absolute root node; StatusPanel is the character-profile screen and
    // is display: None until opened -- wrong target.
    panel: Query<
        (&bevy::ui::ComputedNode, &bevy::ui::UiGlobalTransform),
        With<crate::hud::self_hud::SelfHudPanel>,
    >,
    panels: Res<crate::hud::HudPanels>,
    mut state: ResMut<GraphicsDebugState>,
    mut history: Local<std::collections::VecDeque<(u64, f32, f32, f32, f32)>>,
    mut frame: Local<u64>,
) {
    if let Ok(w) = windows.single() {
        let p = w.physical_size();
        state.win = (p.x, p.y, w.scale_factor());
    }
    let best = panel
        .iter()
        .map(|(c, t)| (c.size(), t.translation))
        .max_by(|a, b| (a.0.x * a.0.y).total_cmp(&(b.0.x * b.0.y)));
    if let Some((sz, ctr)) = best {
        state.panel = (ctr.x, ctr.y, sz.x, sz.y);
        if panels.position_log {
            *frame += 1;
            history.push_back((*frame, ctr.x, ctr.y, sz.x, sz.y));
            while history.len() > 500 {
                history.pop_front();
            }
            if *frame % 30 == 0 {
                let mut out = String::with_capacity(history.len() * 48 + 40);
                out.push_str("frame\tcenter_x\tcenter_y\tw\th\n");
                for (f, x, y, w, h) in history.iter() {
                    out.push_str(&format!("{f}\t{x:.3}\t{y:.3}\t{w:.3}\t{h:.3}\n"));
                }
                let _ = std::fs::write("panelpositions.txt", out);
            }
        } else if !history.is_empty() {
            history.clear();
            *frame = 0;
        }
    }
}
