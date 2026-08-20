use bevy::prelude::*;

use crate::camera::ChaseCamera;
#[cfg(not(target_arch = "wasm32"))]
use crate::camera::OperatorCamera;
#[cfg(not(target_arch = "wasm32"))]
use crate::components::{IsSelf, WorldEntity};
use crate::hud::style::{self, theme};
#[cfg(not(target_arch = "wasm32"))]
use crate::snapshot::SceneState;

#[derive(Component)]
pub struct CompassPanel;

#[derive(Component)]
pub struct CompassLabel;

/// Camera-relative pointer at the wide-scan tracked (0x0F5) target — the
/// adaptation of retail's compass tracking arrow to the compass chip
/// (kuluu-rodm). Colored like the map's tracked marker so they read as one.
#[derive(Component)]
pub struct CompassTrackPointer;

const PANEL_SIZE_PX: f32 = 32.0;

const OVERLAY_BG: Color = Color::srgba(0.04, 0.04, 0.04, 0.66);

/// Track pointer pin size and its gap from the compass chip's right edge.
const TRACK_POINTER_PX: f32 = 11.0;
const TRACK_POINTER_GAP_PX: f32 = 3.0;

#[cfg(not(target_arch = "wasm32"))]
fn spawn_track_pointer_as_child(p: &mut ChildSpawnerCommands) {
    p.spawn((
        CompassTrackPointer,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(100.0),
            top: Val::Percent(50.0),
            width: Val::Px(TRACK_POINTER_PX),
            height: Val::Px(TRACK_POINTER_PX),
            margin: UiRect {
                left: Val::Px(TRACK_POINTER_GAP_PX),
                top: Val::Px(-TRACK_POINTER_PX * 0.5),
                ..default()
            },
            display: Display::None,
            border_radius: crate::minimap::overlay::pin_border_radius(TRACK_POINTER_PX),
            ..default()
        },
        BackgroundColor(crate::hud::map_screen::TRACKED_MARKER_COLOR),
        UiTransform::default(),
    ));
}

pub fn spawn_compass_overlay_as_child(p: &mut ChildSpawnerCommands) {
    p.spawn((
        CompassPanel,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(3.0),
            left: Val::Px(3.0),
            min_width: Val::Px(20.0),
            padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
            border: UiRect::all(Val::Px(1.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        ZIndex(15),
        BackgroundColor(OVERLAY_BG),
        BorderColor::all(theme::FRAME_EDGE),
    ))
    .with_children(|p| {
        p.spawn((
            CompassLabel,
            Text::new("—"),
            style::text_font(13.0),
            TextColor(theme::TITLE),
        ));
        #[cfg(not(target_arch = "wasm32"))]
        spawn_track_pointer_as_child(p);
    });
}

pub fn spawn_compass_as_child(p: &mut ChildSpawnerCommands) {
    p.spawn((
        CompassPanel,
        Node {
            flex_shrink: 0.0,
            width: Val::Px(PANEL_SIZE_PX),
            height: Val::Px(PANEL_SIZE_PX),
            padding: UiRect::all(Val::Px(2.0)),
            border: UiRect::all(Val::Px(1.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(theme::FRAME_BG),
        BorderColor::all(theme::FRAME_EDGE),
    ))
    .with_children(|p| {
        p.spawn((
            CompassLabel,
            Text::new("—"),
            style::text_font(14.0),
            TextColor(theme::TITLE),
        ));
        #[cfg(not(target_arch = "wasm32"))]
        spawn_track_pointer_as_child(p);
    });
}

pub fn update_compass(chase: Res<ChaseCamera>, mut label_q: Query<&mut Text, With<CompassLabel>>) {
    let Ok(mut text) = label_q.single_mut() else {
        return;
    };
    let want = direction_label(chase.yaw);
    if **text != want {
        **text = want.into();
    }
}

/// Clockwise screen angle from "ahead" to the target: the pointer reads like
/// retail's compass arrow, up = the camera's current facing. Inputs are
/// map-plane vectors in the minimap screen basis (world +X right, +Z down).
pub fn track_pointer_theta(facing_xz: Vec2, to_target_xz: Vec2) -> f32 {
    let cross = facing_xz.x * to_target_xz.y - facing_xz.y * to_target_xz.x;
    let dot = facing_xz.dot(to_target_xz);
    cross.atan2(dot)
}

/// Rotate the compass track pointer at the wide-scan tracked target, hidden
/// while nothing is tracked or the geometry degenerates (kuluu-rodm).
#[cfg(not(target_arch = "wasm32"))]
pub fn update_compass_track_pointer(
    scene_state: Res<SceneState>,
    cam_q: Query<&GlobalTransform, With<OperatorCamera>>,
    q_self: Query<&Transform, With<IsSelf>>,
    q_entities: Query<(&Transform, &WorldEntity), Without<IsSelf>>,
    mut ptr_q: Query<(&mut Node, &mut UiTransform), With<CompassTrackPointer>>,
) {
    let theta =
        crate::hud::map_screen::tracked_world(scene_state.snapshot.widescan.tracked, |act_index| {
            q_entities
                .iter()
                .find(|(_, we)| we.act_index == act_index)
                .map(|(t, _)| t.translation)
        })
        .zip(q_self.single().ok())
        .zip(cam_q.single().ok())
        .and_then(|((target, self_t), cam)| {
            let to_target = target - self_t.translation;
            let d = Vec2::new(to_target.x, to_target.z);
            let f3 = cam.forward();
            let f = Vec2::new(f3.x, f3.z);
            (d.length_squared() > f32::EPSILON && f.length_squared() > f32::EPSILON)
                .then(|| track_pointer_theta(f, d))
        });

    for (mut node, mut transform) in ptr_q.iter_mut() {
        match theta {
            Some(theta) => {
                let rot = Rot2::radians(theta - crate::minimap::overlay::PIN_TIP_BEARING);
                if transform.rotation != rot {
                    transform.rotation = rot;
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

pub fn direction_label(yaw: f32) -> &'static str {
    const LABELS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let tau = std::f32::consts::TAU;

    let normalized = yaw.rem_euclid(tau);

    let octant = ((normalized + tau / 16.0) / (tau / 8.0)) as usize;
    LABELS[octant % LABELS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_zero_is_north() {
        assert_eq!(direction_label(0.0), "N");
    }

    #[test]
    fn quarter_turns_are_cardinals() {
        let q = std::f32::consts::FRAC_PI_2;
        assert_eq!(direction_label(q), "E");
        assert_eq!(direction_label(2.0 * q), "S");
        assert_eq!(direction_label(3.0 * q), "W");
    }

    #[test]
    fn eighths_are_diagonals() {
        let e = std::f32::consts::FRAC_PI_4;
        assert_eq!(direction_label(e), "NE");
        assert_eq!(direction_label(3.0 * e), "SE");
        assert_eq!(direction_label(5.0 * e), "SW");
        assert_eq!(direction_label(7.0 * e), "NW");
    }

    #[test]
    fn negative_yaw_normalizes() {
        assert_eq!(direction_label(-std::f32::consts::FRAC_PI_2), "W");
    }

    #[test]
    fn boundary_just_under_half_octant_stays_north() {
        let almost_ne = std::f32::consts::FRAC_PI_4 - 0.001;

        assert_eq!(direction_label(almost_ne), "NE");
    }

    #[test]
    fn track_pointer_ahead_is_zero() {
        let ahead = Vec2::new(0.0, -1.0);
        assert!(track_pointer_theta(ahead, ahead * 5.0).abs() < 1e-6);
    }

    #[test]
    fn track_pointer_right_of_facing_is_clockwise_quarter_turn() {
        // Screen basis has +y down, so a target to the camera's right rotates
        // the pointer +PI/2 (clockwise).
        let facing = Vec2::new(0.0, -1.0);
        let right = Vec2::new(3.0, 0.0);
        assert!((track_pointer_theta(facing, right) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn track_pointer_behind_is_half_turn() {
        let facing = Vec2::new(0.0, -1.0);
        let behind = Vec2::new(0.0, 2.0);
        assert!(
            (track_pointer_theta(facing, behind).abs() - std::f32::consts::PI).abs() < 1e-6,
            "directly behind resolves to +/- PI"
        );
    }

    #[test]
    fn track_pointer_is_camera_relative() {
        // Same world target, camera turned to face it: the pointer recenters.
        let target = Vec2::new(1.0, -1.0);
        let theta_offset = track_pointer_theta(Vec2::new(0.0, -1.0), target);
        let theta_facing = track_pointer_theta(target, target);
        assert!(theta_offset.abs() > 0.1 && theta_facing.abs() < 1e-6);
    }
}
