use bevy::prelude::*;

use kuluu_render::components::IsSelf;
use kuluu_render::dat_mzb::{CameraCollisionSource, DrawDistance, ZoneGeomMode};
use kuluu_render::scene::BakedActor;
use kuluu_render::snapshot::SceneState;
use kuluu_render::{
    third_person_anchor_y, yaw_for_heading, CameraMode, ChaseCamera, OperatorCamera,
};

use super::collision_bvh::{CollisionBvh, ZoneCollisionBvh};

/// The focus dead zone, yalms: the camera's focus holds still while the pivot
/// moves inside it, and is dragged to exactly this distance once the pivot
/// leaves it. Small enough that the player never reads off-centre, large
/// enough to swallow the per-frame wobble of the player transform (a server
/// correction, an interpolation seam, a stair step). The idea is the orbit
/// camera's focus radius (catlikecoding.com, Orbit Camera, "focus radius").
const FOCUS_DEADZONE: f32 = 0.25;

/// The eye's slack band: it holds still while its horizontal distance from the
/// focus is between `max * LEASH_SLACK_MIN_RATIO` and `max` (the zoom), and is
/// dragged or pushed to the band's edge outside it. The reference client pulls
/// its eye in past 6 and pushes it out under 3
/// (research/XIClient/src/XIClient/source/World/Camera/CameraManager.cpp
/// CameraManager::UpdatePlayerFollowingCamera), hence one half.
const LEASH_SLACK_MIN_RATIO: f32 = 0.5;

/// Drag `point` toward `anchor` until it is no farther than `max` and no nearer
/// than `min`; inside the band it does not move. `fallback` is the direction
/// used when the two coincide. Instant: a clamp to a distance, never a rate.
pub fn leash(point: Vec2, anchor: Vec2, min: f32, max: f32, fallback: Vec2) -> Vec2 {
    let v = point - anchor;
    let d = v.length();
    if d < 1e-5 {
        return anchor + fallback * min;
    }
    let clamped = d.clamp(min, max);
    if clamped == d {
        point
    } else {
        anchor + v / d * clamped
    }
}

/// `next` as a yaw continuous with `yaw`: step by the wrapped difference, so
/// the accumulated chase yaw never jumps a full turn.
fn continuous_yaw(yaw: f32, next: f32) -> f32 {
    yaw + ((next - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI)
}

/// Where the camera's focus and eye sit in the world, bevy xz, carried frame
/// to frame; `yaw` is the chase yaw this system last wrote, so a different
/// value next frame means the player turned the camera.
#[derive(Default)]
pub struct LeashState {
    focus: Option<Vec2>,
    eye: Option<Vec2>,
    yaw: Option<f32>,
}

/// Whether the chase camera should collide with zone MMB static placements (Mog
/// House furniture and the exit-door model). Inside a Mog House this is always
/// on — retail's "furniture camera collision" — because the interior is sealed
/// only by ~two dozen MMB placements and the closed door is one of them; without
/// this the camera slips through the doorway gap in the MZB wall and escapes the
/// room. Enabling it zone-wide would raycast thousands of city placements every
/// frame, so outside a Mog House it stays gated on the explicit source setting.
/// The BVH-build gate ([`super::collision_bvh::build_collision_bvh_system`]) and
/// the camera raycast MUST use this same predicate or they disagree on coverage.
///
/// The gap is real and still open: `mh_391_doorway_is_a_gap_in_mzb_collision`
/// finds MZB walls on 23 of 24 headings from the spawn anchor and nothing at all
/// on the 24th. So this is not made redundant by kuluu-0nnl putting every MZB
/// submesh into the collision set — MMB placements are a separate set entirely.
///
/// Note the two camera sources now differ in *policy*, not just coverage: MZB
/// triangles are filtered by retail's `DoubleSidedSkipPolicy`
/// ([`ffxi_dat::mzb::double_sided_skip`]) while MMB models carry no
/// `CollisionMeshHeader.Flags` and are raycast whole.
pub fn camera_collides_with_mmb(source: CameraCollisionSource, in_mog_house: bool) -> bool {
    source.uses_mmb() || in_mog_house
}

// research/xim/src/jsMain/kotlin/xim/poc/camera/PolarCamera.kt getAdjustedRadiusFromCollision collisionDistance —
// `(distance - 0.25f).coerceAtLeast(0.5f)`: pad off the wall, but never pull the
// camera closer than 0.5 to the anchor (tiny interiors like the Mog House would
// otherwise collapse it inside the character model).
const WALL_PAD: f32 = 0.25;

const CAMERA_MIN_DISTANCE: f32 = 0.5;

const OUTWARD_LERP: f32 = 0.18;

const INWARD_LERP: f32 = 0.45;

/// The single chase-camera authority: a leash with slack at both ends, and
/// the wall pull-in against the zone MZB BVH, with one transform write at
/// the end.
///
/// The camera is a world point (the eye) on a leash from a focus point. The
/// focus is what the camera looks at: the player's anchor, or, locked on,
/// the midpoint of the player and the target (the height stays the
/// player's anchor). It holds still while the pivot moves inside
/// FOCUS_DEADZONE and is dragged to exactly that distance once the pivot
/// leaves it, so the per-frame wobble of the player transform (a server
/// correction, an interpolation seam, a stair step) never reaches the
/// camera. The eye holds still while its horizontal distance from the focus
/// is between half and all of the zoom (LEASH_SLACK_MIN_RATIO); past either
/// edge it is dragged or pushed to the band's edge in one step. The yaw is
/// the direction from the focus to the eye; the one exception is a frame
/// where something else wrote chase.yaw since last frame (the mouse, the yaw
/// keys, a stair warp) — then the eye swings around the focus to that yaw
/// and keeps its distance. No rates anywhere: every drag is a clamp to a
/// distance, never a speed.
///
/// Init sync aligns yaw behind the player on the first frame.
/// snap_to_anchor (zone/warp) resets the leash to the exact position.
///
/// Pass 3 is the collision pull-in: a ray from the pivot along the boom,
/// where walls block and mobs do not — the same solid world the walker
/// sweeps (the door triangles join this ray when the obstacle set lands).
/// The nearest hit shortens the boom so the camera does not clip through
/// geometry; cast from the pivot (the player), not the glide origin, so wall
/// pull-in is measured from where the camera is actually looking. The BVH
/// rebuilds ~1 s after zone geometry goes quiet; until then there is no ray
/// and the boom runs unclipped.
///
/// Boom-length easing (not position) snaps in fast when a wall appears and
/// eases out slow when it clears, so the camera does not jitter at wall
/// edges; the position spring is pass 1, this only smooths the pull-in
/// distance. The single write places the eye along the boom from the glide
/// origin while the camera looks at the player pivot, so however the rig
/// glides the player stays centered and rotation orbits the player; with the
/// spring off, boom_origin equals the pivot and this reduces to the plain
/// centered behavior.
pub fn resolve_camera(
    mode: Res<CameraMode>,
    settings: Res<kuluu_render::GraphicsSettings>,
    mut chase: ResMut<ChaseCamera>,
    step: Res<kuluu_render::camera::CameraStepSmoothing>,
    scene_state: Res<SceneState>,
    zone_bvh: Res<ZoneCollisionBvh>,
    self_q: Query<(&Transform, Option<&BakedActor>), (With<IsSelf>, Without<OperatorCamera>)>,
    mut cam_q: Query<&mut Transform, (With<OperatorCamera>, Without<IsSelf>)>,
    lock_on: Res<kuluu_render::lock_on::LockOn>,
    target_q: Query<
        (&kuluu_render::components::WorldEntity, &Transform),
        (Without<IsSelf>, Without<OperatorCamera>),
    >,
    mut smoothed_effective: Local<Option<f32>>,
    mut leash_state: Local<LeashState>,
) {
    if !matches!(*mode, CameraMode::Chase) {
        *smoothed_effective = None;
        *leash_state = LeashState::default();
        return;
    }
    let Ok((self_t, baked)) = self_q.single() else {
        *smoothed_effective = None;
        *leash_state = LeashState::default();
        return;
    };
    let Ok(mut cam_t) = cam_q.single_mut() else {
        return;
    };

    if !chase.synced_initial {
        chase.yaw = yaw_for_heading(scene_state.snapshot.self_pos.heading);
        chase.synced_initial = true;
    }

    let player_pos = self_t.translation;
    let anchor_y = Vec3::Y * (third_person_anchor_y(baked) - step.offset);
    // Locked on, the camera looks at the midpoint of the player and the target
    // (research/XIClient CameraManager::UpdatePlayerFollowingCamera, the
    // non-free-run branch's Vector3::Lerp(..., 0.5f)); height stays the
    // player's anchor.
    let lock_offset = lock_on
        .target_id
        .and_then(|id| target_q.iter().find(|(we, _)| we.id == id))
        .map(|(_, t)| {
            let half = (t.translation - player_pos) * 0.5;
            Vec2::new(half.x, half.z)
        })
        .unwrap_or(Vec2::ZERO);
    let pivot_y = player_pos.y + anchor_y.y;
    let pivot_xz = Vec2::new(player_pos.x, player_pos.z) + lock_offset;

    let cos_p = chase.pitch.cos().max(1e-3);
    let sin_p = chase.pitch.sin();
    let max_h = chase.orbit_radius() * cos_p;
    let (deadzone, min_h) = if settings.camera_spring {
        (FOCUS_DEADZONE, max_h * LEASH_SLACK_MIN_RATIO)
    } else {
        (0.0, max_h)
    };
    let yaw_dir = |yaw: f32| Vec2::new(yaw.sin(), yaw.cos());

    // Focus: held inside the dead zone, dragged to its edge outside it.
    let focus = match leash_state.focus {
        Some(f) if !chase.snap_to_anchor => leash(f, pivot_xz, 0.0, deadzone, Vec2::ZERO),
        _ => pivot_xz,
    };
    // Eye: where it was, swung around the focus if the player turned the
    // camera since last frame, then held inside the slack band.
    let eye = match (leash_state.eye, leash_state.yaw) {
        (Some(e), Some(last_yaw)) if !chase.snap_to_anchor => {
            if last_yaw == chase.yaw {
                e
            } else {
                let h = (e - focus).length().clamp(min_h, max_h);
                focus + yaw_dir(chase.yaw) * h
            }
        }
        _ => focus + yaw_dir(chase.yaw) * max_h,
    };
    let eye = leash(eye, focus, min_h, max_h, yaw_dir(chase.yaw));
    let to_eye = eye - focus;
    chase.yaw = continuous_yaw(chase.yaw, to_eye.x.atan2(to_eye.y));
    *leash_state = LeashState {
        focus: Some(focus),
        eye: Some(eye),
        yaw: Some(chase.yaw),
    };

    let pivot = Vec3::new(focus.x, pivot_y, focus.y);
    let dir = Vec3::new(chase.yaw.sin() * cos_p, sin_p, chase.yaw.cos() * cos_p);
    let wanted = to_eye.length() / cos_p;

    let mut hit_t = wanted;
    if let Some(bvh) = zone_bvh.0.as_ref() {
        if let Some(t) = bvh.ray_cast(pivot, dir, wanted) {
            hit_t = t.min(hit_t);
        }
    }

    let target = clamped_camera_distance(hit_t, wanted);

    let effective = if !settings.camera_spring || chase.snap_to_anchor {
        target
    } else {
        match *smoothed_effective {
            Some(prev) if target < prev => target * INWARD_LERP + prev * (1.0 - INWARD_LERP),
            Some(prev) => prev + (target - prev) * OUTWARD_LERP,
            None => target,
        }
    };
    *smoothed_effective = Some(effective);

    cam_t.translation = pivot + dir * effective;
    cam_t.look_at(pivot, Vec3::Y);
    chase.snap_to_anchor = false;
}

fn clamped_camera_distance(hit_t: f32, wanted: f32) -> f32 {
    (hit_t - WALL_PAD).min(wanted).max(CAMERA_MIN_DISTANCE)
}

pub fn draw_camera_collision_debug(
    draw: Res<DrawDistance>,
    mode: Res<CameraMode>,
    chase: Res<ChaseCamera>,
    self_q: Query<(&Transform, Option<&BakedActor>), (With<IsSelf>, Without<OperatorCamera>)>,
    cam_q: Query<&Transform, (With<OperatorCamera>, Without<IsSelf>)>,
    bvh_q: Query<&CollisionBvh>,
    zone_bvh: Res<ZoneCollisionBvh>,
    mut gizmos: Gizmos,
) {
    if draw.zone_geom_mode != ZoneGeomMode::Camera {
        return;
    }

    let source = draw.camera_collision_source;

    let mut draw_aabb = |mn: Vec3, mx: Vec3, color: Color| {
        gizmos.primitive_3d(
            &Cuboid::from_size(mx - mn),
            Isometry3d::from_translation((mn + mx) * 0.5),
            color,
        );
    };

    if source.uses_mzb() {
        if let Some((mn, mx)) = zone_bvh.0.as_ref().and_then(|b| b.root_aabb()) {
            draw_aabb(mn, mx, Color::srgba(0.20, 0.80, 1.0, 0.55));
        }
    }

    if source.uses_mmb() {
        for bvh in bvh_q.iter() {
            if let Some((mn, mx)) = bvh.root_aabb() {
                draw_aabb(mn, mx, Color::srgba(1.0, 0.55, 0.10, 0.55));
            }
        }
    }

    let Ok((self_t, baked)) = self_q.single() else {
        return;
    };
    let anchor = self_t.translation + Vec3::Y * third_person_anchor_y(baked);

    let cross = 0.3;
    let cross_color = Color::srgba(1.0, 1.0, 1.0, 0.90);
    gizmos.line(
        anchor - Vec3::X * cross,
        anchor + Vec3::X * cross,
        cross_color,
    );
    gizmos.line(
        anchor - Vec3::Y * cross,
        anchor + Vec3::Y * cross,
        cross_color,
    );
    gizmos.line(
        anchor - Vec3::Z * cross,
        anchor + Vec3::Z * cross,
        cross_color,
    );

    if !matches!(*mode, CameraMode::Chase) {
        return;
    }

    let cos_p = chase.pitch.cos();
    let sin_p = chase.pitch.sin();
    let dir = Vec3::new(chase.yaw.sin() * cos_p, sin_p, chase.yaw.cos() * cos_p);
    let wanted_end = anchor + dir * chase.orbit_radius();

    let effective_end = cam_q.single().map(|t| t.translation).unwrap_or(wanted_end);

    gizmos.line(anchor, effective_end, Color::srgba(1.0, 0.85, 0.15, 0.85));

    let clip_amount = (wanted_end - effective_end).length();
    if clip_amount > 0.05 {
        gizmos.line(
            effective_end,
            wanted_end,
            Color::srgba(1.0, 0.25, 0.55, 0.85),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_distance_never_collapses_into_the_anchor() {
        // XIM PolarCamera.kt getAdjustedRadiusFromCollision collisionDistance: (distance - 0.25).coerceAtLeast(0.5) — a wall
        // right at the anchor (tiny Mog House rooms) must not pull the camera
        // inside the character model.
        assert_eq!(clamped_camera_distance(0.0, 6.0), CAMERA_MIN_DISTANCE);
        assert_eq!(clamped_camera_distance(0.3, 6.0), CAMERA_MIN_DISTANCE);
    }

    #[test]
    fn collision_sweeps_the_distance_the_camera_actually_travels() {
        let chase = ChaseCamera {
            distance: ChaseCamera::DIST_MIN,
            pitch: ChaseCamera::PITCH_MAX,
            ..Default::default()
        };
        assert!(
            chase.orbit_radius() > chase.distance + 0.5,
            "a pitched close camera swings out well past chase.distance \
             ({} vs {}) — sweeping chase.distance here would leave the far end \
             of the eye's travel unswept, which is how it left the building",
            chase.orbit_radius(),
            chase.distance
        );
    }

    #[test]
    fn camera_distance_pads_off_walls_and_caps_at_wanted() {
        assert_eq!(clamped_camera_distance(3.0, 6.0), 3.0 - WALL_PAD);
        assert_eq!(clamped_camera_distance(100.0, 6.0), 6.0);
    }

    #[test]
    fn mog_house_camera_always_collides_with_mmb_furniture() {
        // The MH exit door is a zone MMB static placement, not MZB wall geometry;
        // with the default Mzb source the camera would slip through the doorway
        // gap. Inside a Mog House, MMB collision must be on regardless of source.
        assert!(camera_collides_with_mmb(CameraCollisionSource::Mzb, true));
        assert!(!camera_collides_with_mmb(CameraCollisionSource::Mzb, false));
        assert!(camera_collides_with_mmb(CameraCollisionSource::Mmb, false));
        assert!(camera_collides_with_mmb(CameraCollisionSource::Both, false));
    }

    /// Zone-in snap must land on frame one — no lerp from wherever the last
    /// zone left the eye. The empty zone BVH is resolve_camera's hard
    /// requirement: it raycasts the boom against zone MZB.
    #[test]
    fn snap_to_anchor_places_eye_behind_player_without_smoothing() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(CameraMode::Chase)
            .insert_resource(kuluu_render::GraphicsSettings {
                camera_spring: false,
                ..Default::default()
            })
            .insert_resource(SceneState::default())
            .init_resource::<ZoneCollisionBvh>()
            .insert_resource(kuluu_render::camera::CameraStepSmoothing::default())
            .init_resource::<kuluu_render::lock_on::LockOn>()
            .init_resource::<ZoneCollisionBvh>()
            .insert_resource(ChaseCamera {
                snap_to_anchor: true,
                ..Default::default()
            })
            .add_systems(Update, resolve_camera);

        let player_pos = Vec3::new(10.0, 1.0, -4.0);
        app.world_mut()
            .spawn((IsSelf, Transform::from_translation(player_pos)));
        let cam = app
            .world_mut()
            .spawn((OperatorCamera, Transform::from_xyz(999.0, 500.0, -999.0)))
            .id();

        app.update();

        let chase = app.world().resource::<ChaseCamera>();
        assert!(!chase.snap_to_anchor, "snap flag consumed by the update");
        let expected_yaw = yaw_for_heading(
            app.world()
                .resource::<SceneState>()
                .snapshot
                .self_pos
                .heading,
        );
        assert_eq!(
            chase.yaw, expected_yaw,
            "zone-in yaw follows player heading"
        );

        let anchor = player_pos + Vec3::Y * third_person_anchor_y(None);
        let expected_dist = clamped_camera_distance(chase.orbit_radius(), chase.orbit_radius());
        let cos_p = chase.pitch.cos();
        let sin_p = chase.pitch.sin();
        let dir = Vec3::new(
            expected_yaw.sin() * cos_p,
            sin_p,
            expected_yaw.cos() * cos_p,
        );
        let expected_eye = anchor + dir * expected_dist;
        let cam_t = *app.world().get::<Transform>(cam).unwrap();
        assert!(
            (cam_t.translation - expected_eye).length() < 1e-4,
            "eye {:?} snapped to {expected_eye:?} behind the player, no lerp from the old zone",
            cam_t.translation
        );
        let look = *cam_t.forward();
        let want = (anchor - expected_eye).normalize();
        assert!(
            (look - want).length() < 1e-4,
            "camera faces along the player's heading: {look:?} != {want:?}"
        );
    }

    /// Inside the dead zone the focus does not move: a pivot wobbling 5 cm
    /// every frame never reaches the camera. This is the jitter the old
    /// follow passed straight through.
    #[test]
    fn focus_deadzone_swallows_pivot_wobble() {
        let mut focus = Vec2::new(10.0, -4.0);
        let start = focus;
        for i in 0..600 {
            let wobble = Vec2::new(
                if i % 2 == 0 { 0.05 } else { -0.05 },
                if i % 3 == 0 { 0.04 } else { -0.03 },
            );
            focus = leash(focus, start + wobble, 0.0, FOCUS_DEADZONE, Vec2::ZERO);
            assert_eq!(focus, start, "frame {i}: the focus moved");
        }
    }

    /// Inside the slack band the eye does not move, and so the yaw does not
    /// either; outside it the eye is dragged to the band's edge in one step.
    #[test]
    fn eye_holds_inside_the_band_and_clamps_outside_it() {
        let focus = Vec2::ZERO;
        let fb = Vec2::Y;
        let e = Vec2::new(0.0, 4.5);
        assert_eq!(leash(e, focus, 3.0, 6.0, fb), e);
        let far = leash(Vec2::new(0.0, 9.0), focus, 3.0, 6.0, fb);
        assert!((far.length() - 6.0).abs() < 1e-5);
        let near = leash(Vec2::new(0.0, 1.0), focus, 3.0, 6.0, fb);
        assert!((near.length() - 3.0).abs() < 1e-5);
    }

    /// Held D against this camera: the player runs camera-right every step.
    /// The yaw turns one way only, by at most the step over the band's inner
    /// edge per step. It never spins, and it never reverses.
    #[test]
    fn held_d_turns_the_leash_one_way_without_spinning() {
        const MAX: f32 = 6.0;
        const MIN: f32 = 3.0;
        const STEP: f32 = 0.1;
        let fb = Vec2::Y;
        let mut yaw = 0.0_f32;
        let mut focus = Vec2::ZERO;
        let mut eye = focus + Vec2::new(yaw.sin(), yaw.cos()) * MAX;
        let mut pivot = focus;
        let mut first_sign = 0.0_f32;
        for i in 0..600 {
            // Camera forward is -(sin yaw, cos yaw) in bevy xz; right of a
            // forward (fx, fz) with Y up is (-fz, fx), here (cos yaw, -sin yaw).
            pivot += Vec2::new(yaw.cos(), -yaw.sin()) * STEP;
            focus = leash(focus, pivot, 0.0, FOCUS_DEADZONE, Vec2::ZERO);
            eye = leash(eye, focus, MIN, MAX, fb);
            let v = eye - focus;
            let next = continuous_yaw(yaw, v.x.atan2(v.y));
            let d = next - yaw;
            assert!(
                d.abs() <= (STEP / MIN).atan() * 1.05,
                "step {i}: yaw jumped {d}"
            );
            if d != 0.0 {
                if first_sign == 0.0 {
                    first_sign = d.signum();
                } else {
                    assert_eq!(d.signum(), first_sign, "step {i}: the turn reversed");
                }
            }
            yaw = next;
        }
        assert!(first_sign != 0.0, "a held D must turn the camera");
    }

    /// Running straight away drags the eye straight behind: no turn.
    #[test]
    fn leash_run_away_keeps_the_yaw() {
        let yaw = 0.7_f32;
        let away = -Vec2::new(yaw.sin(), yaw.cos());
        let fb = Vec2::new(yaw.sin(), yaw.cos());
        let mut focus = Vec2::ZERO;
        let mut eye = focus + fb * 6.0;
        let mut pivot = focus;
        for _ in 0..300 {
            pivot += away * 0.1;
            focus = leash(focus, pivot, 0.0, FOCUS_DEADZONE, Vec2::ZERO);
            eye = leash(eye, focus, 3.0, 6.0, fb);
        }
        let v = eye - focus;
        assert!((v.x.atan2(v.y) - yaw).abs() < 1e-4);
    }
}
