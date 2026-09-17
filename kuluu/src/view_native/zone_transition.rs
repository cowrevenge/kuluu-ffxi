use bevy::picking::Pickable;
use bevy::prelude::*;

use kuluu_render::dat_mzb::{LastAutoLoadedZone, LoadMzbInFlight, ZONE_SLOT_MAIN};
use kuluu_render::SceneState;
use kuluu_session::state::AgentCommand;
use kuluu_snapshot::{Stage, Vec3 as WireVec3};

use super::input::CommandTx;
use super::AppPhase;

const FADE_OUT_SECS: f32 = 0.2;

const FADE_IN_SECS: f32 = 0.4;

const FADE_HOLD_MIN_SECS: f32 = 0.35;

const MAX_HOLD_SECS: f32 = 15.0;

/// A new character's first CHAR_PC lands at the origin (the pre-cutscene
/// "unplaced" position) and the cutscene quest corrects it a few seconds in.
/// Hold the loading overlay until a real position arrives; if none does within
/// this window, disconnect and return to the login screen (retail behavior).
const POSITION_WAIT_TIMEOUT_SECS: f32 = 30.0;

const LOADING_TEXT: &str = "Downloading data";

/// The origin is the "unplaced" sentinel a first-login CHAR_PC carries before
/// the cutscene quest sets the real spawn; any real zone position is far from
/// it, so a per-axis epsilon cleanly separates the two.
fn position_is_real(pos: &WireVec3) -> bool {
    const EPS: f32 = 1e-2;
    pos.x.abs() > EPS || pos.y.abs() > EPS || pos.z.abs() > EPS
}

const DOT_FRAMES: [&str; 4] = ["   ", ".  ", ".. ", "..."];

const DOT_PERIOD_SECS: f32 = 0.4;

#[derive(Resource, Default, Clone, Copy, PartialEq, Debug)]
enum ZoneOverlayFade {
    #[default]
    Idle,
    FadingOut {
        elapsed: f32,
    },
    Holding {
        elapsed: f32,
    },
    FadingIn {
        elapsed: f32,
    },
}

impl ZoneOverlayFade {
    fn alpha(&self) -> f32 {
        match *self {
            ZoneOverlayFade::Idle => 0.0,
            ZoneOverlayFade::FadingOut { elapsed } => (elapsed / FADE_OUT_SECS).clamp(0.0, 1.0),
            ZoneOverlayFade::Holding { .. } => 1.0,
            ZoneOverlayFade::FadingIn { elapsed } => 1.0 - (elapsed / FADE_IN_SECS).clamp(0.0, 1.0),
        }
    }
}

fn tick(state: ZoneOverlayFade, dt: f32, ready: bool) -> ZoneOverlayFade {
    match state {
        ZoneOverlayFade::Idle => ZoneOverlayFade::Idle,
        ZoneOverlayFade::FadingOut { elapsed } => {
            let next = elapsed + dt;
            if next >= FADE_OUT_SECS {
                ZoneOverlayFade::Holding { elapsed: 0.0 }
            } else {
                ZoneOverlayFade::FadingOut { elapsed: next }
            }
        }
        ZoneOverlayFade::Holding { elapsed } => {
            let next = elapsed + dt;
            if (next >= FADE_HOLD_MIN_SECS && ready) || next >= MAX_HOLD_SECS {
                ZoneOverlayFade::FadingIn { elapsed: 0.0 }
            } else {
                ZoneOverlayFade::Holding { elapsed: next }
            }
        }
        ZoneOverlayFade::FadingIn { elapsed } => {
            let next = elapsed + dt;
            if next >= FADE_IN_SECS {
                ZoneOverlayFade::Idle
            } else {
                ZoneOverlayFade::FadingIn { elapsed: next }
            }
        }
    }
}

#[derive(Resource, Default)]
struct HudVisibilityStash(std::collections::HashMap<Entity, Visibility>);

#[derive(Resource, Default)]
struct LoadingDots {
    elapsed: f32,
    last_frame: usize,
}

/// One-shot guard so the position-wait timeout disconnects only once per zone
/// transition, not every frame while the overlay holds.
#[derive(Resource, Default, Clone, Copy, PartialEq, Debug)]
struct PositionTimeoutFired(bool);

#[derive(Component)]
struct ZoneOverlayRoot;

#[derive(Component)]
struct ZoneOverlayLabel;

type HudRootFilter = (With<Node>, Without<ChildOf>, Without<ZoneOverlayRoot>);

pub struct ZoneTransitionOverlayPlugin;

impl Plugin for ZoneTransitionOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZoneOverlayFade>()
            .init_resource::<HudVisibilityStash>()
            .init_resource::<LoadingDots>()
            .init_resource::<PositionTimeoutFired>()
            .add_systems(OnEnter(AppPhase::InGame), spawn_zone_overlay)
            .add_systems(
                Update,
                (drive_zone_overlay_fade, apply_zone_overlay_alpha)
                    .chain()
                    .run_if(in_state(AppPhase::InGame)),
            );
    }
}

fn spawn_zone_overlay(
    mut commands: Commands,
    mut fade: ResMut<ZoneOverlayFade>,
    mut stash: ResMut<HudVisibilityStash>,
    mut fired: ResMut<PositionTimeoutFired>,
) {
    *fade = ZoneOverlayFade::Holding { elapsed: 0.0 };
    *fired = PositionTimeoutFired(false);
    stash.0.clear();

    commands
        .spawn((
            super::InGameEntity,
            ZoneOverlayRoot,
            kuluu_render::hud_hide::HudHideExempt,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),

                align_items: AlignItems::FlexEnd,
                justify_content: JustifyContent::FlexEnd,
                padding: UiRect {
                    right: Val::Px(40.0),
                    bottom: Val::Px(28.0),
                    ..default()
                },
                ..default()
            },
            BackgroundColor(Color::BLACK.with_alpha(1.0)),
            GlobalZIndex(i32::MAX),
            Pickable::IGNORE,
        ))
        .with_children(|p| {
            p.spawn((
                ZoneOverlayLabel,
                Text::new(format!("{LOADING_TEXT}{}", DOT_FRAMES[0])),
                TextFont {
                    font_size: 20.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE.with_alpha(1.0)),
                Pickable::IGNORE,
            ));
        });
}

fn drive_zone_overlay_fade(
    time: Res<Time>,
    scene: Res<SceneState>,
    mzb_in_flight: Res<LoadMzbInFlight>,
    last_auto: Res<LastAutoLoadedZone>,
    mut fade: ResMut<ZoneOverlayFade>,
    mut stash: ResMut<HudVisibilityStash>,
    mut fired: ResMut<PositionTimeoutFired>,
    cmd_tx: Res<CommandTx>,
    mut hud_roots: Query<(Entity, &mut Visibility), HudRootFilter>,
) {
    let stage = scene.snapshot.stage;

    if stage == Stage::Zoning
        && matches!(
            *fade,
            ZoneOverlayFade::Idle | ZoneOverlayFade::FadingIn { .. }
        )
    {
        *fade = ZoneOverlayFade::FadingOut { elapsed: 0.0 };
        *fired = PositionTimeoutFired(false);

        if stash.0.is_empty() {
            for (e, mut vis) in hud_roots.iter_mut() {
                stash.0.insert(e, *vis);
                *vis = Visibility::Hidden;
            }
        }
    }

    let want_file_id = kuluu_render::snapshot::effective_zone_file_id(&scene.snapshot);
    let pos_real = position_is_real(&scene.snapshot.self_pos.pos);
    let ready = stage == Stage::InZone
        && last_auto.file_id.is_some()
        && last_auto.file_id == want_file_id
        // Slot 0 only: a sub-area interior streaming in behind a doorway is not
        // a zone transition, and gating on it re-raises the loading overlay
        // every time the player walks into a shop.
        && !mzb_in_flight.pending_in_slot(ZONE_SLOT_MAIN)
        // A first-login character sits at the origin (the pre-cutscene
        // "unplaced" position) until the cutscene quest sets the real spawn;
        // hold the overlay for that so the world is ready when the position
        // lands and the player never drops through the ground.
        && pos_real;

    let dt = time.delta_secs();
    // While the self position is still the unplaced origin, hold the overlay
    // past the MZB safety cap. If no real position arrives within the window,
    // disconnect and return to the login screen (retail behavior).
    if let ZoneOverlayFade::Holding { elapsed } = *fade {
        if !pos_real {
            let next = elapsed + dt;
            if next >= POSITION_WAIT_TIMEOUT_SECS && !fired.0 {
                fired.0 = true;
                tracing::warn!(
                    "zone-in: no real self position after {POSITION_WAIT_TIMEOUT_SECS:.0}s — disconnecting to login"
                );
                let _ = cmd_tx.0.try_send(AgentCommand::Disconnect);
            }
            *fade = ZoneOverlayFade::Holding { elapsed: next };
            return;
        }
    }

    let prev = *fade;
    *fade = tick(*fade, dt, ready);

    if !matches!(prev, ZoneOverlayFade::Idle)
        && *fade == ZoneOverlayFade::Idle
        && !stash.0.is_empty()
    {
        for (e, mut vis) in hud_roots.iter_mut() {
            if let Some(prev_vis) = stash.0.get(&e) {
                *vis = *prev_vis;
            }
        }
        stash.0.clear();
    }
}

fn apply_zone_overlay_alpha(
    fade: Res<ZoneOverlayFade>,
    time: Res<Time>,
    mut dots: ResMut<LoadingDots>,
    mut root_q: Query<(&mut BackgroundColor, &mut Node), With<ZoneOverlayRoot>>,
    mut label_q: Query<(&mut Text, &mut TextColor), With<ZoneOverlayLabel>>,
) {
    let alpha = fade.alpha();
    let idle = matches!(*fade, ZoneOverlayFade::Idle);
    let want_display = if idle { Display::None } else { Display::Flex };
    if let Ok((mut bg, mut node)) = root_q.single_mut() {
        if node.display != want_display {
            node.display = want_display;
        }
        if (bg.0.alpha() - alpha).abs() > 0.001 {
            bg.0 = Color::BLACK.with_alpha(alpha);
        }
    }

    if idle {
        dots.elapsed = 0.0;
    } else {
        dots.elapsed += time.delta_secs();
    }
    let frame = ((dots.elapsed / DOT_PERIOD_SECS) as usize) % DOT_FRAMES.len();
    if let Ok((mut text, mut tc)) = label_q.single_mut() {
        if frame != dots.last_frame {
            dots.last_frame = frame;
            **text = format!("{LOADING_TEXT}{}", DOT_FRAMES[frame]);
        }
        if (tc.0.alpha() - alpha).abs() > 0.001 {
            tc.0 = Color::WHITE.with_alpha(alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_is_real_separates_origin_from_spawn() {
        // The pre-cutscene "unplaced" sentinel is the origin.
        assert!(!position_is_real(&WireVec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0
        }));
        // A real spawn (Bastok's first-login correction) is far from it.
        assert!(position_is_real(&WireVec3 {
            x: -280.0,
            y: -12.0,
            z: -90.0
        }));
        // A single non-zero axis counts as real (a ground-plane spawn).
        assert!(position_is_real(&WireVec3 {
            x: 0.0,
            y: 0.0,
            z: 5.0
        }));
        // Sub-centimeter jitter at the origin stays "unplaced".
        assert!(!position_is_real(&WireVec3 {
            x: 0.001,
            y: 0.0,
            z: -0.001
        }));
    }

    #[test]
    fn alpha_edges() {
        assert_eq!(ZoneOverlayFade::Idle.alpha(), 0.0);
        assert_eq!((ZoneOverlayFade::Holding { elapsed: 0.0 }).alpha(), 1.0);

        let half_out = ZoneOverlayFade::FadingOut {
            elapsed: FADE_OUT_SECS / 2.0,
        }
        .alpha();
        assert!((half_out - 0.5).abs() < 0.01, "fade-out mid: {half_out}");
        let half_in = ZoneOverlayFade::FadingIn {
            elapsed: FADE_IN_SECS / 2.0,
        }
        .alpha();
        assert!((half_in - 0.5).abs() < 0.01, "fade-in mid: {half_in}");
    }

    #[test]
    fn fade_out_advances_to_hold() {
        let s = tick(
            ZoneOverlayFade::FadingOut { elapsed: 0.0 },
            FADE_OUT_SECS,
            false,
        );
        assert_eq!(s, ZoneOverlayFade::Holding { elapsed: 0.0 });
    }

    #[test]
    fn hold_waits_for_ready() {
        let s = tick(
            ZoneOverlayFade::Holding {
                elapsed: FADE_HOLD_MIN_SECS,
            },
            0.016,
            false,
        );
        assert!(matches!(s, ZoneOverlayFade::Holding { .. }));

        let s = tick(
            ZoneOverlayFade::Holding {
                elapsed: FADE_HOLD_MIN_SECS,
            },
            0.016,
            true,
        );
        assert_eq!(s, ZoneOverlayFade::FadingIn { elapsed: 0.0 });
    }

    #[test]
    fn hold_does_not_fade_in_before_minimum_even_if_ready() {
        let s = tick(ZoneOverlayFade::Holding { elapsed: 0.0 }, 0.016, true);
        assert!(matches!(s, ZoneOverlayFade::Holding { .. }));
    }

    #[test]
    fn hold_times_out_without_ready() {
        let s = tick(
            ZoneOverlayFade::Holding {
                elapsed: MAX_HOLD_SECS,
            },
            0.016,
            false,
        );
        assert_eq!(s, ZoneOverlayFade::FadingIn { elapsed: 0.0 });
    }

    #[test]
    fn fade_in_completes_to_idle() {
        let s = tick(
            ZoneOverlayFade::FadingIn { elapsed: 0.0 },
            FADE_IN_SECS,
            false,
        );
        assert_eq!(s, ZoneOverlayFade::Idle);
    }
}
