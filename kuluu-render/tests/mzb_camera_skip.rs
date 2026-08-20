use bevy::math::{Vec2, Vec3};
use kuluu_render::dat_mzb::{build_collision_geometry, load_mzb_placed, MAX_GROUND_STEP_UP};

/// Lower Jeuno (DAT 345). Skips without a retail DAT install.
fn jeuno() -> Option<kuluu_render::dat_mzb::MzbCollisionBlock> {
    if std::env::var("FFXI_DAT_PATH").is_err() {
        eprintln!("FFXI_DAT_PATH unset; skipping");
        return None;
    }
    bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let (submeshes, instances) = load_mzb_placed(345, None).expect("load DAT 345");
    Some(build_collision_geometry(&submeshes, &instances, Some(345)))
}

/// Retail's chase camera passes through triangles its movement query does not
/// (CollisionQuery.hpp: DoubleSidedSkipPolicy vs BacksideCullingPolicy). Pins
/// the skip set against the measurement that sized it.
#[test]
fn camera_skip_set_matches_the_measured_shape() {
    let Some(geom) = jeuno() else { return };

    assert_eq!(geom.camera_skip.len(), geom.tri_count());
    let skipped = geom.camera_skip.iter().filter(|s| **s).count();

    // Measured 2026-07-30 over DAT 345 with `zz-mzb-tri-probe` KULUU_MESHFLAGS:
    // 1092 of 48305 placed world triangles (2.26%), of which 984 walls and 108
    // up-facing floors, no ceilings. Bracketed rather than pinned exactly so a
    // DAT from a different client patch doesn't fail the suite spuriously.
    assert!(
        skipped > 0,
        "no triangles skipped — the camera predicate is not reaching the geometry"
    );
    let pct = 100.0 * skipped as f32 / geom.tri_count() as f32;
    assert!(
        (0.5..10.0).contains(&pct),
        "skip set is {pct:.2}% of {} triangles; measured 2.26% — a jump this large \
         means the predicate changed meaning",
        geom.tri_count()
    );
    assert_eq!(geom.camera_triangles().len(), geom.tri_count() - skipped);
}

/// The camera must actually stop seeing a skipped triangle — a filter that
/// computes the right set but never gets applied would pass every other check.
#[test]
fn camera_triangles_really_drops_a_skipped_surface() {
    let Some(geom) = jeuno() else { return };

    let idx = geom
        .camera_skip
        .iter()
        .position(|s| *s)
        .expect("a skipped triangle");
    let tri = &geom.indices[idx * 3..idx * 3 + 3];
    let v = [
        geom.positions[tri[0] as usize],
        geom.positions[tri[1] as usize],
        geom.positions[tri[2] as usize],
    ];
    let centroid = (v[0] + v[1] + v[2]) / 3.0;
    let normal = (v[1] - v[0]).cross(v[2] - v[0]).normalize();

    // Fire straight at the face from just off it, short enough that nothing
    // behind it can be mistaken for the same hit.
    const STANDOFF: f32 = 2.0;
    let origin = centroid + normal * STANDOFF;
    let dir = -normal;

    // City geometry is dense enough that something else is usually nearer than
    // the standoff, so "nearest hit" says nothing. Count hits at the target's own
    // distance instead: removing it must remove exactly one of them, which is
    // also correct when coincident triangles share that depth.
    let hits_at_target = |tris: &[[Vec3; 3]]| -> usize {
        tris.iter()
            .filter_map(|t| ray_tri(origin, dir, t[0], t[1], t[2]))
            .filter(|t| (t - STANDOFF).abs() < 1e-3)
            .count()
    };

    let all: Vec<[Vec3; 3]> = geom
        .indices
        .chunks_exact(3)
        .map(|t| {
            [
                geom.positions[t[0] as usize],
                geom.positions[t[1] as usize],
                geom.positions[t[2] as usize],
            ]
        })
        .collect();

    let before = hits_at_target(&all);
    assert!(
        before > 0,
        "the unfiltered set must hit the chosen triangle"
    );
    assert_eq!(
        hits_at_target(&geom.camera_triangles()),
        before - 1,
        "camera_triangles must drop exactly the skipped triangle at this depth"
    );
}

/// The acceptance criterion, stated directly: movement uses
/// `BacksideCullingPolicy`, which skips nothing, so grounding must be bit-identical
/// whether or not the camera skip set is populated.
#[test]
fn grounding_is_unaffected_by_the_camera_skip() {
    let Some(geom) = jeuno() else { return };
    assert!(
        geom.camera_skip.iter().any(|s| *s),
        "nothing to be affected by"
    );

    let mut without = build_collision_geometry(
        &load_mzb_placed(345, None).unwrap().0,
        &load_mzb_placed(345, None).unwrap().1,
        Some(345),
    );
    without.camera_skip.clear();

    // The Lower Jeuno anchor the BVH test uses, which is also where the
    // fly-up-onto-the-roof bug (kuluu-0nnl) was reported.
    let (cx, cz) = (16.84_f32, -41.35_f32);
    let geom = kuluu_render::dat_mzb::MzbCollisionGeometry::from_block(geom);
    let without = kuluu_render::dat_mzb::MzbCollisionGeometry::from_block(without);
    for iz in -20..=20 {
        for ix in -20..=20 {
            let xz = Vec2::new(cx + ix as f32, cz + iz as f32);
            for ref_y in [-5.0, 1.0, 7.0] {
                assert_eq!(
                    geom.ground_nearest(xz, ref_y),
                    without.ground_nearest(xz, ref_y),
                    "ground_nearest diverged at {xz:?} ref_y={ref_y}"
                );
                assert_eq!(
                    geom.ground_step(xz, ref_y, MAX_GROUND_STEP_UP),
                    without.ground_step(xz, ref_y, MAX_GROUND_STEP_UP),
                    "ground_step diverged at {xz:?} ref_y={ref_y}"
                );
            }
        }
    }
}

fn ray_tri(orig: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    const EPS: f32 = 1e-7;
    let (e1, e2) = (v1 - v0, v2 - v0);
    let h = dir.cross(e2);
    let a = e1.dot(h);
    if a.abs() < EPS {
        return None;
    }
    let f = 1.0 / a;
    let s = orig - v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * e2.dot(q);
    (t > EPS).then_some(t)
}
