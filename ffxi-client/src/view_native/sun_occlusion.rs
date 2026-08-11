use bevy::prelude::*;
use ffxi_viewer_core::camera::OperatorCamera;
use ffxi_viewer_core::lens_flare::SunOcclusion;
use ffxi_viewer_core::sun_moon::{sun_angular_radius, sun_direction, VanaSky, SKY_RADIUS};

use super::collision_bvh::{CollisionBvh, ZoneCollisionBvh};

// research/xim src/jsMain/kotlin/xim/poc/ParticleDrawer.kt:243-245 - retail's flare occlusion
// draws a screen-space quad the size of the sun particle with colour writes off and takes the
// PERCENTAGE of pixels that pass, so the sampled set is the sun's own disc, not a jitter around
// its centre. Equal-area rings make the unoccluded tap count an estimate of that area fraction;
// a point sample is a 0/1 result that flips frame to frame when a grazing ray crosses a ridge.
const SUN_OCCLUSION_RINGS: usize = 3;
const SUN_OCCLUSION_RING_TAPS: usize = 8;
const SUN_OCCLUSION_TAP_COUNT: usize = 1 + SUN_OCCLUSION_RINGS * SUN_OCCLUSION_RING_TAPS;

// research/xim ParticleDrawer.kt:239-240 - retail consumes the occlusion query one frame after
// issuing it, so a short lag is retail-shaped; ours additionally smooths the 1/TAP_COUNT
// quantization of the disc sample (~150ms time constant).
const SUN_VISIBILITY_FADE_PER_SEC: f32 = 6.0;

// Exponential smoothing never reaches its target; snapping within this
// sub-perceptual band lets visibility hit exactly 0.0 (the Hidden draw skip in
// lens_flare_system) and 1.0.
const SUN_VISIBILITY_SNAP_EPS: f32 = 1e-3;

fn occlusion_ray_dirs(sun_dir: Vec3) -> [Vec3; SUN_OCCLUSION_TAP_COUNT] {
    let right = sun_dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(sun_dir);
    let disc = sun_angular_radius();
    let mut dirs = [sun_dir; SUN_OCCLUSION_TAP_COUNT];
    let mut n = 1;
    for ring in 0..SUN_OCCLUSION_RINGS {
        // sqrt spacing puts equal disc area between successive rings, so every tap stands for
        // the same share of the sun's screen coverage.
        let r = disc * ((ring + 1) as f32 / SUN_OCCLUSION_RINGS as f32).sqrt();
        // Half-step stagger per ring so taps do not line up radially and read one thin occluder
        // as a whole spoke.
        let phase = ring as f32 * std::f32::consts::TAU / (2.0 * SUN_OCCLUSION_RING_TAPS as f32);
        for tap in 0..SUN_OCCLUSION_RING_TAPS {
            let a = phase + tap as f32 * std::f32::consts::TAU / SUN_OCCLUSION_RING_TAPS as f32;
            dirs[n] = (sun_dir + right * (r * a.cos()) + up * (r * a.sin())).normalize();
            n += 1;
        }
    }
    dirs
}

fn sun_visibility_target(bvh: &CollisionBvh, origin: Vec3, sun_dir: Vec3, reach: f32) -> f32 {
    let dirs = occlusion_ray_dirs(sun_dir);
    let unoccluded = dirs
        .iter()
        .filter(|dir| bvh.ray_cast(origin, **dir, reach).is_none())
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
    settings: Res<ffxi_viewer_core::graphics_settings::GraphicsSettings>,
    cam_q: Query<&GlobalTransform, With<OperatorCamera>>,
    time: Res<Time>,
    mut occlusion: ResMut<SunOcclusion>,
) {
    // research/xim ParticleDrawer.kt:239-248 queryLensFlare: retail's visibility is a
    // depth-buffer occlusion query, so only geometry rendered this frame occludes. Zone
    // geometry is spawned only inside view_distance (dat_mmb.rs:436, dat_mzb.rs:2366) while the
    // collision BVH holds the whole zone block, so a SKY_RADIUS ray at sunrise is answered
    // almost entirely by terrain that was never drawn and sits past the fog.
    let reach = settings.view_distance.min(SKY_RADIUS);
    let sun_up = sky.sun_altitude > 0.0;
    let target = match (sun_up, zone_bvh.0.as_ref(), cam_q.single()) {
        (true, Some(bvh), Ok(cam)) => {
            sun_visibility_target(bvh, cam.translation(), sun_direction(sky.hour), reach)
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

    use std::f32::consts::TAU;

    const NOON_HOUR: f32 = 12.0;
    const TEST_DT_SECS: f32 = 1.0 / 60.0;

    // The default graphics preset's view_distance, i.e. what update_sun_occlusion_system passes.
    const TEST_REACH: f32 = 500.0;

    // The point-sample cone this fix replaced (~6px at a 1080p-tall 60-degree FoV viewport).
    const LEGACY_TAP_ANGLE_RAD: f32 = 0.006;

    // Cut-line orientation for the half-coverage case, chosen so no ring tap angle lands on the
    // half-plane boundary (ring taps sit on multiples of TAU/16).
    const HALF_PLANE_CUT_ANGLE_RAD: f32 = TAU / 32.0;

    fn plane_half_cover(
        origin: Vec3,
        sun_dir: Vec3,
        dist: f32,
        cut_angle: f32,
        cut_offset: f32,
        size: f32,
    ) -> Vec<[Vec3; 3]> {
        let right = sun_dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(sun_dir);
        let n = up * cut_angle.cos() + right * cut_angle.sin();
        let t = right * cut_angle.cos() - up * cut_angle.sin();
        let centre = origin + sun_dir * dist - n * (size + cut_offset);
        let a = centre + (-n - t) * size;
        let b = centre + (n - t) * size;
        let c = centre + (n + t) * size;
        let d = centre + (-n + t) * size;
        vec![[a, b, c], [a, c, d]]
    }

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
        assert_eq!(
            sun_visibility_target(&bvh, origin, sun_dir, TEST_REACH),
            0.0
        );
    }

    #[test]
    fn wall_away_from_the_sun_path_is_fully_visible() {
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let sun_dir = sun_direction(NOON_HOUR);
        let bvh =
            CollisionBvh::from_world_triangles(wall_quad(origin - sun_dir * 10.0, sun_dir, 50.0));
        assert_eq!(
            sun_visibility_target(&bvh, origin, sun_dir, TEST_REACH),
            1.0
        );
    }

    // research/xim ParticleDrawer.kt:239-248: retail's flare visibility is a depth-buffer query,
    // so only what was drawn this frame occludes. Collision past the spawn radius is invisible
    // to the player and must not answer the query.
    #[test]
    fn geometry_beyond_the_rendered_draw_distance_does_not_occlude() {
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let sun_dir = sun_direction(NOON_HOUR);
        let far = CollisionBvh::from_world_triangles(wall_quad(
            origin + sun_dir * (TEST_REACH * 2.0),
            sun_dir,
            200.0,
        ));
        assert_eq!(
            sun_visibility_target(&far, origin, sun_dir, TEST_REACH),
            1.0
        );

        let near = CollisionBvh::from_world_triangles(wall_quad(
            origin + sun_dir * (TEST_REACH * 0.5),
            sun_dir,
            200.0,
        ));
        assert_eq!(
            sun_visibility_target(&near, origin, sun_dir, TEST_REACH),
            0.0
        );
    }

    #[test]
    fn the_sample_cone_is_the_sun_disc_angular_radius() {
        let sun_dir = sun_direction(NOON_HOUR);
        let dirs = occlusion_ray_dirs(sun_dir);
        assert_eq!(dirs.len(), SUN_OCCLUSION_TAP_COUNT);
        assert!(dirs[0].abs_diff_eq(sun_dir, 1e-6), "centre tap is the axis");
        let mut widest = 0.0f32;
        for dir in dirs {
            assert!((dir.length() - 1.0).abs() < 1e-5, "tap is not unit length");
            widest = widest.max(dir.angle_between(sun_dir));
        }
        assert!(
            (widest - sun_angular_radius()).abs() < 1e-5,
            "outer ring should sit on the sun's own rim, got {widest}"
        );
    }

    // research/xim ParticleDrawer.kt:243-245: the flare's opacity is the percentage of the sun
    // quad's pixels that pass the depth test, so half the disc covered is half visibility.
    #[test]
    fn a_wall_covering_half_the_disc_gives_a_half_area_fraction() {
        let origin = Vec3::ZERO;
        let sun_dir = sun_direction(NOON_HOUR);
        let wall_dist = 100.0;
        let disc_extent = wall_dist * sun_angular_radius();
        let bvh = CollisionBvh::from_world_triangles(plane_half_cover(
            origin,
            sun_dir,
            wall_dist,
            HALF_PLANE_CUT_ANGLE_RAD,
            disc_extent * 0.01,
            disc_extent * 10.0,
        ));
        let vis = sun_visibility_target(&bvh, origin, sun_dir, TEST_REACH);
        let tolerance = 1.5 / SUN_OCCLUSION_TAP_COUNT as f32;
        assert!(
            (vis - 0.5).abs() <= tolerance,
            "half-covered disc should read ~0.5, got {vis}"
        );
    }

    // The direct anti-strobe guard: an occluder narrower than the sun disc dims the flare
    // instead of hard-hiding it, so a grazing ray crossing a ridge slides rather than flips.
    #[test]
    fn a_wall_smaller_than_the_sun_disc_only_dims_the_flare() {
        let origin = Vec3::ZERO;
        let sun_dir = sun_direction(NOON_HOUR);
        let wall_dist = 100.0;
        let bvh = CollisionBvh::from_world_triangles(wall_quad(
            origin + sun_dir * wall_dist,
            sun_dir,
            wall_dist * LEGACY_TAP_ANGLE_RAD,
        ));
        let vis = sun_visibility_target(&bvh, origin, sun_dir, TEST_REACH);
        assert!(
            vis > 0.5 && vis < 1.0,
            "sub-disc occluder should dim, not hide, got {vis}"
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
