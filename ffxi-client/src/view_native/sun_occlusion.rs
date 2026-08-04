use bevy::prelude::*;
use ffxi_viewer_core::camera::OperatorCamera;
use ffxi_viewer_core::lens_flare::SunOcclusion;
use ffxi_viewer_core::sun_moon::{sun_direction, VanaSky, SKY_RADIUS};

use super::collision_bvh::{CollisionBvh, ZoneCollisionBvh};

const SUN_OCCLUSION_TAP_COUNT: usize = 5;

// Parity with the replaced shader path's ±6px depth-prepass taps: ~6px at a
// 1080p-tall, 60° FoV viewport ≈ 0.006 rad off the sun's center.
const SUN_OCCLUSION_TAP_ANGLE_RAD: f32 = 0.006;

// Eased fade masks the 5-ray 0.2-step quantization and matches retail's soft
// flare fade across a roofline (hand-tuned, ~150ms time constant).
const SUN_VISIBILITY_FADE_PER_SEC: f32 = 6.0;

// Exponential smoothing never reaches its target; snapping within this
// sub-perceptual band lets visibility hit exactly 0.0 (the Hidden draw skip in
// lens_flare_system) and 1.0.
const SUN_VISIBILITY_SNAP_EPS: f32 = 1e-3;

fn occlusion_ray_dirs(sun_dir: Vec3) -> [Vec3; SUN_OCCLUSION_TAP_COUNT] {
    let right = sun_dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(sun_dir);
    let jitter = SUN_OCCLUSION_TAP_ANGLE_RAD;
    [
        sun_dir,
        (sun_dir + right * jitter).normalize(),
        (sun_dir - right * jitter).normalize(),
        (sun_dir + up * jitter).normalize(),
        (sun_dir - up * jitter).normalize(),
    ]
}

fn sun_visibility_target(bvh: &CollisionBvh, origin: Vec3, sun_dir: Vec3) -> f32 {
    let dirs = occlusion_ray_dirs(sun_dir);
    let unoccluded = dirs
        .iter()
        .filter(|dir| bvh.ray_cast(origin, **dir, SKY_RADIUS).is_none())
        .count();
    unoccluded as f32 / dirs.len() as f32
}

fn smoothed_visibility(current: f32, target: f32, dt_secs: f32) -> f32 {
    let blend = 1.0 - (-SUN_VISIBILITY_FADE_PER_SEC * dt_secs).exp();
    let next = current + (target - current) * blend;
    if (next - target).abs() < SUN_VISIBILITY_SNAP_EPS {
        target
    } else {
        next
    }
}

pub fn update_sun_occlusion_system(
    sky: Res<VanaSky>,
    zone_bvh: Res<ZoneCollisionBvh>,
    cam_q: Query<&GlobalTransform, With<OperatorCamera>>,
    time: Res<Time>,
    mut occlusion: ResMut<SunOcclusion>,
) {
    let sun_up = sky.sun_altitude > 0.0;
    let target = match (sun_up, zone_bvh.0.as_ref(), cam_q.single()) {
        (true, Some(bvh), Ok(cam)) => {
            sun_visibility_target(bvh, cam.translation(), sun_direction(sky.hour))
        }
        _ => 1.0,
    };
    let next = smoothed_visibility(occlusion.visibility, target, time.delta_secs());
    if next != occlusion.visibility {
        occlusion.visibility = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON_HOUR: f32 = 12.0;
    const TEST_DT_SECS: f32 = 1.0 / 60.0;

    fn wall_quad(centre: Vec3, facing: Vec3, half_extent: f32) -> Vec<[Vec3; 3]> {
        let right = facing.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(facing);
        // Asymmetric nudge so no ray lands exactly on the shared diagonal edge
        // of the two triangles.
        let centre = centre + (right * 0.05 + up * 0.03) * half_extent;
        let a = centre + (-right - up) * half_extent;
        let b = centre + (right - up) * half_extent;
        let c = centre + (right + up) * half_extent;
        let d = centre + (-right + up) * half_extent;
        vec![[a, b, c], [a, c, d]]
    }

    #[test]
    fn wall_between_camera_and_sun_fully_occludes() {
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let sun_dir = sun_direction(NOON_HOUR);
        let bvh =
            CollisionBvh::from_world_triangles(wall_quad(origin + sun_dir * 10.0, sun_dir, 50.0));
        assert_eq!(sun_visibility_target(&bvh, origin, sun_dir), 0.0);
    }

    #[test]
    fn wall_away_from_the_sun_path_is_fully_visible() {
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let sun_dir = sun_direction(NOON_HOUR);
        let bvh =
            CollisionBvh::from_world_triangles(wall_quad(origin - sun_dir * 10.0, sun_dir, 50.0));
        assert_eq!(sun_visibility_target(&bvh, origin, sun_dir), 1.0);
    }

    #[test]
    fn wall_covering_only_the_center_ray_is_partial() {
        let origin = Vec3::ZERO;
        let sun_dir = sun_direction(NOON_HOUR);
        let wall_dist = 100.0;
        let tap_spread = wall_dist * SUN_OCCLUSION_TAP_ANGLE_RAD;
        let bvh = CollisionBvh::from_world_triangles(wall_quad(
            origin + sun_dir * wall_dist,
            sun_dir,
            tap_spread * 0.5,
        ));
        let vis = sun_visibility_target(&bvh, origin, sun_dir);
        let expected = (SUN_OCCLUSION_TAP_COUNT - 1) as f32 / SUN_OCCLUSION_TAP_COUNT as f32;
        assert!(
            (vis - expected).abs() < 1e-6,
            "center-only wall should occlude exactly one tap, got {vis}"
        );
    }

    #[test]
    fn smoothing_is_monotone_and_snaps_to_target() {
        for (start, target) in [(1.0_f32, 0.0_f32), (0.0, 1.0)] {
            let mut v = start;
            let mut prev = v;
            for _ in 0..600 {
                v = smoothed_visibility(v, target, TEST_DT_SECS);
                let toward = (v - prev) * (target - start) >= 0.0;
                assert!(toward, "stepped away from target: {prev} -> {v}");
                let overshot = (v - target) * (start - target) < 0.0;
                assert!(!overshot, "overshot target: {v}");
                prev = v;
            }
            assert_eq!(v, target, "did not snap to target");
        }
    }
}
