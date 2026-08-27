use bevy::prelude::*;

use avian3d::prelude::*;

use kuluu_render::components::IsSelf;
use kuluu_render::dat_mzb::{CameraCollisionSource, DrawDistance, ZoneGeomMode};
use kuluu_render::scene::BakedActor;
use kuluu_render::snapshot::SceneState;
use kuluu_render::{third_person_anchor_y, yaw_for_heading, CameraMode, ChaseCamera, OperatorCamera};

use super::avian_bridge::camera_mask;
use super::collision_bvh::{CollisionBvh, ZoneCollisionBvh};

/// Gap-proportional pull rate (1/sec) for the position spring, HORIZONTAL only.
/// At run speed ~6 yalms/sec the settled horizontal gap is speed / PULL_RATE
/// = ~3 yalms: the character leads, the camera trails. Rotation is never
/// lagged (this engine has no lean); only position.
const CAM_PULL_RATE: f32 = 2.0;

/// Cap on the spring's horizontal per-second travel toward the player. Must
/// exceed sprint speed or the gap grows without bound; warps use snap_to_anchor.
const CAM_MAX_SPEED: f32 = 12.0;

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

// research/xim/src/jsMain/kotlin/xim/poc/camera/PolarCamera.kt:209 —
// `(distance - 0.25f).coerceAtLeast(0.5f)`: pad off the wall, but never pull the
// camera closer than 0.5 to the anchor (tiny interiors like the Mog House would
// otherwise collapse it inside the character model).
const WALL_PAD: f32 = 0.25;

const CAMERA_MIN_DISTANCE: f32 = 0.5;

const OUTWARD_LERP: f32 = 0.18;

const INWARD_LERP: f32 = 0.45;

pub fn resolve_camera(
    mode: Res<CameraMode>,
    settings: Res<kuluu_render::GraphicsSettings>,
    mut chase: ResMut<ChaseCamera>,
    step: Res<kuluu_render::camera::CameraStepSmoothing>,
    mut follow: ResMut<kuluu_render::camera::AnchorFollow>,
    time: Res<Time>,
    scene_state: Res<SceneState>,
    sq: SpatialQuery,
    self_q: Query<(&Transform, Option<&BakedActor>), (With<IsSelf>, Without<OperatorCamera>)>,
    mut cam_q: Query<&mut Transform, (With<OperatorCamera>, Without<IsSelf>)>,
    mut smoothed_effective: Local<Option<f32>>,
) {
    if !matches!(*mode, CameraMode::Chase) {
        *smoothed_effective = None;
        return;
    }
    let Ok((self_t, baked)) = self_q.single() else {
        return;
    };
    let Ok(mut cam_t) = cam_q.single_mut() else {
        return;
    };

    // Init sync: align yaw behind the player on the first frame (moved here
    // from the retired chase_camera_system).
    if !chase.synced_initial {
        chase.yaw = yaw_for_heading(scene_state.snapshot.self_pos.heading);
        chase.synced_initial = true;
    }

    // --- Pass 1: position spring (HORIZONTAL only) ---
    // Rate-limited follow: move the anchor toward the player at
    // min(gap*rate, max_speed), travel capped at the gap so it can't overshoot,
    // no easing so it can't wobble. Y is taken direct from the (already
    // render-smoothed) player Transform, so the camera never floats above the
    // player on stairs. snap_to_anchor (zone/warp) resets to exact position.
    let player_pos = self_t.translation;
    let follow_pos = match follow.pos {
        Some(prev) if settings.camera_spring && !chase.snap_to_anchor => {
            let gap_xz = Vec2::new(player_pos.x - prev.x, player_pos.z - prev.z);
            let dist = gap_xz.length();
            let new_xz = if dist < 1e-5 {
                Vec2::new(prev.x, prev.z)
            } else {
                let dt = time.delta_secs().max(1e-4);
                let speed = (dist * CAM_PULL_RATE).min(CAM_MAX_SPEED);
                let travel = (speed * dt).min(dist);
                Vec2::new(prev.x, prev.z) + gap_xz / dist * travel
            };
            Vec3::new(new_xz.x, player_pos.y, new_xz.y)
        }
        _ => player_pos,
    };
    follow.pos = Some(follow_pos);

    // --- Pass 2: orbit (instant rotation, never lagged) ---
    let anchor = follow_pos + Vec3::Y * (third_person_anchor_y(baked) - step.offset);
    let cos_p = chase.pitch.cos();
    let sin_p = chase.pitch.sin();
    let dir = Vec3::new(chase.yaw.sin() * cos_p, sin_p, chase.yaw.cos() * cos_p);
    let wanted = chase.orbit_radius();

    // --- Pass 3: collision pull-in against the AVIAN world ---
    // Ray from the anchor along the boom; walls+doors block, mobs never do.
    // Same solid world the walker sweeps. Nearest hit shortens the boom so the
    // camera never clips through geometry.
    let mut hit_t = wanted;
    if let Ok(ray_dir) = Dir3::new(dir) {
        if let Some(hit) = sq.cast_ray(
            anchor,
            ray_dir,
            wanted,
            true,
            &SpatialQueryFilter::from_mask(camera_mask()),
        ) {
            if hit.distance < hit_t {
                hit_t = hit.distance;
            }
        }
    }

    let target = clamped_camera_distance(hit_t, wanted);

    // Boom-LENGTH easing (not position): snap in fast when a wall appears, ease
    // out slow when it clears, so the camera doesn't jitter at wall edges. The
    // position spring is pass 1; this only smooths the pull-in distance.
    let effective = if !settings.camera_spring {
        target
    } else {
        match *smoothed_effective {
            Some(prev) if target < prev => target * INWARD_LERP + prev * (1.0 - INWARD_LERP),
            Some(prev) => prev + (target - prev) * OUTWARD_LERP,
            None => target,
        }
    };
    *smoothed_effective = Some(effective);

    // --- Single write: this system is the sole camera authority ---
    cam_t.translation = anchor + dir * effective;
    cam_t.look_at(anchor, Vec3::Y);
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
        // XIM PolarCamera.kt:209: (distance - 0.25).coerceAtLeast(0.5) — a wall
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
}
