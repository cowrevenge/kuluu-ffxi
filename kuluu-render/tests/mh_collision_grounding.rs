use bevy::math::{Vec2, Vec3};
use kuluu_render::dat_mzb::{
    build_collision_geometry, load_mzb_placed, MzbCollisionGeometry, FLOOR_NORMAL_MIN,
    MAX_GROUND_STEP_UP,
};

/// Pins the Mog House grounding geometry against the real Windurst MH DAT
/// (391): at the server spawn column (0,0) the interior floor sits near y=0 and
/// the model also carries a roof plane at y=5 — grounding with the wire spawn Y
/// must pick the floor, never the roof. (The roof was exactly where entities
/// ended up when a stale previous-zone collision set inflated their reference Y
/// mid-transition.) Skips without a retail DAT install.
#[test]
fn mh_391_spawn_column_grounds_to_interior_floor_not_roof() {
    if std::env::var("FFXI_DAT_PATH").is_err() {
        eprintln!("FFXI_DAT_PATH unset; skipping");
        return;
    }
    bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let (submeshes, instances) = load_mzb_placed(391, None).expect("load DAT 391");
    let geom = MzbCollisionGeometry::from_block(build_collision_geometry(
        &submeshes,
        &instances,
        Some(391),
    ));
    assert!(geom.tri_count() > 1000, "MH collision unexpectedly small");

    let spawn = Vec2::new(0.0, 0.0);
    let hits = geom.ground_raycast_all(spawn);
    let top = hits.first().expect("no surfaces at spawn column").0;
    assert!(
        top > 4.0,
        "expected a roof plane above the interior (got top {top})"
    );

    let grounded = geom
        .ground_nearest(spawn, 0.0)
        .expect("no floor at spawn column");
    assert!(
        grounded.abs() < 0.5,
        "wire spawn Y=0 must ground to the interior floor, got {grounded}"
    );

    let stale_ref = geom.ground_nearest(spawn, 40.0).unwrap_or(f32::NAN);
    assert!(
        stale_ref > 3.0,
        "a stale high reference Y sticks to the roof band ({stale_ref}) — \
         which is why the collision set must be cleared on zone transition"
    );

    // Walking, as opposed to placing an entity, must never reach that roof band
    // from the floor however the column is shaped (kuluu-0nnl).
    let stepped = geom.ground_step(spawn, grounded, MAX_GROUND_STEP_UP);
    assert!(
        stepped.is_some_and(|y| (y - grounded).abs() <= MAX_GROUND_STEP_UP),
        "a walker on the interior floor stays within one step of it, got {stepped:?}"
    );

    // Authored normals, not winding, are what separate those two bands: every
    // surface here is either a floor or a ceiling, never ambiguous.
    let (floors, ceilings) =
        geom.ground_raycast_all(spawn)
            .iter()
            .fold((0, 0), |(f, c), (_, n)| match n.y {
                y if y >= FLOOR_NORMAL_MIN => (f + 1, c),
                y if y <= -FLOOR_NORMAL_MIN => (f, c + 1),
                _ => (f, c),
            });
    assert!(
        floors > 0 && ceilings > 0,
        "expected both up- and down-facing surfaces in the MH spawn column \
         (floors={floors} ceilings={ceilings}); if ceilings is 0 the authored \
         normals are not reaching MzbCollisionGeometry"
    );
}

/// The Mog House exit door is not MZB geometry — it is an MMB static placement.
/// That is why `camera_collides_with_mmb` forces MMB collision on inside a Mog
/// House regardless of the collision-source setting (kuluu-da4e): with MZB alone
/// the chase camera escapes through the doorway gap.
///
/// kuluu-19oc supposed the door had since been covered by kuluu-0nnl, which put
/// every MZB submesh into the collision set. It has not: `build_collision_geometry`
/// consumes only `MzbSubMesh`/`MzbInstance`, and MMB placements are a separate
/// set. This pins the gap so the special case isn't deleted as redundant.
///
/// Skips without a retail DAT install.
#[test]
fn mh_391_doorway_is_a_gap_in_mzb_collision() {
    if std::env::var("FFXI_DAT_PATH").is_err() {
        eprintln!("FFXI_DAT_PATH unset; skipping");
        return;
    }
    bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let (submeshes, instances) = load_mzb_placed(391, None).expect("load DAT 391");
    let geom = MzbCollisionGeometry::from_block(build_collision_geometry(
        &submeshes,
        &instances,
        Some(391),
    ));
    let tris = geom.camera_triangles();

    // Head height at the server spawn column, the anchor the chase camera orbits.
    let anchor = Vec3::new(0.0, 1.265, 0.0);
    // Past the far wall (the room measures 6.5-11.1 across from here) but well
    // short of anything outside it.
    const REACH: f32 = 20.0;

    let mut walls = 0;
    let mut gaps = Vec::new();
    for step in 0..24 {
        let yaw = step as f32 * std::f32::consts::TAU / 24.0;
        let dir = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        let hit = tris
            .iter()
            .filter_map(|t| ray_tri(anchor, dir, t[0], t[1], t[2]))
            .filter(|t| *t < REACH)
            .min_by(|a, b| a.partial_cmp(b).unwrap());
        match hit {
            Some(_) => walls += 1,
            None => gaps.push(yaw.to_degrees().round() as i32),
        }
    }

    assert!(
        walls >= 20,
        "expected the room to be walled on nearly every heading, got {walls}/24"
    );
    assert!(
        !gaps.is_empty(),
        "no doorway gap in MZB — if the door is now MZB geometry the Mog House \
         MMB camera special case may genuinely be redundant (kuluu-19oc)"
    );
    eprintln!("MH 391 MZB doorway gap at yaw {gaps:?} deg; walls on {walls}/24 headings");
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
