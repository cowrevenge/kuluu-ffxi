use bevy::math::Vec2;
use ffxi_viewer_core::dat_mzb::{
    build_collision_geometry, load_mzb_placed, FLOOR_NORMAL_MIN, MAX_GROUND_STEP_UP,
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
    let geom = build_collision_geometry(&submeshes, &instances, Some(391));
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
