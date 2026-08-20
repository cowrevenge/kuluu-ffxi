use bevy::math::{Vec2, Vec3};
use kuluu_render::dat_mzb::{
    build_collision_geometry, load_mzb_placed, MzbCollisionGeometry, FLOOR_NORMAL_MIN,
    MAX_GROUND_STEP_UP,
};

/// West Ronfaure (zone 100) at the riverbed column of the kuluu-mo4q live
/// repro: ffxi x=-390.20 y=-437.21, whose only floor is ffxi z=-9.279. A self
/// seed of wire z=0 leaves the player 9.28 under the world, where `ground_step`
/// refuses forever and the wedged height goes out in c2s 0x015. Skips without a
/// retail DAT install.
const REPRO_ZONE: u16 = 100;
const REPRO_FFXI_X: f32 = -390.20;
const REPRO_FFXI_Y: f32 = -437.21;
const REPRO_FLOOR_BEVY_Y: f32 = 9.2785;
const WEDGED_SEED_WIRE_Z: f32 = 0.0;

fn ronfaure_collision() -> Option<MzbCollisionGeometry> {
    if std::env::var("FFXI_DAT_PATH").is_err() {
        eprintln!("FFXI_DAT_PATH unset; skipping");
        return None;
    }
    bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let file_id = ffxi_dat::zone_dat::effective_zone_dat_file_id(Some(REPRO_ZONE), None)
        .expect("zone 100 -> mzb file id");
    let (submeshes, instances) = load_mzb_placed(file_id, None).expect("load zone 100 MZB");
    Some(MzbCollisionGeometry::from_block(build_collision_geometry(
        &submeshes,
        &instances,
        Some(file_id),
    )))
}

#[test]
fn ronfaure_wedged_seed_recovers_to_the_riverbed_floor() {
    let Some(geom) = ronfaure_collision() else {
        return;
    };
    let xz = Vec2::new(REPRO_FFXI_X, -REPRO_FFXI_Y);
    let feet_y = -WEDGED_SEED_WIRE_Z;

    let hits = geom.ground_raycast_all(xz);
    let floors: Vec<(f32, Vec3)> = hits
        .into_iter()
        .filter(|(_, n)| n.y >= FLOOR_NORMAL_MIN)
        .collect();
    assert_eq!(
        floors.len(),
        1,
        "the repro column holds exactly one floor, got {floors:?}"
    );
    assert!(
        (floors[0].0 - REPRO_FLOOR_BEVY_Y).abs() < 1e-2,
        "riverbed floor moved: {}",
        floors[0].0
    );

    assert_eq!(
        geom.ground_step(xz, feet_y, MAX_GROUND_STEP_UP),
        None,
        "the wedge itself: the riverbed is past the step-up bound from z=0"
    );

    let recovered = geom
        .ground_or_recover(xz, feet_y, MAX_GROUND_STEP_UP)
        .expect("recovery found no floor");
    assert!(
        (recovered - REPRO_FLOOR_BEVY_Y).abs() < 1e-2,
        "recovery must land on the riverbed, got {recovered}"
    );

    let wire_z = geom
        .ground_or_recover_wire_z(REPRO_FFXI_X, REPRO_FFXI_Y, WEDGED_SEED_WIRE_Z)
        .expect("wire recovery found no floor");
    assert!(
        (wire_z + REPRO_FLOOR_BEVY_Y).abs() < 1e-2,
        "wire z must be the negated floor height, got {wire_z}"
    );
}

/// Standing on the riverbed is a fixed point: recovery is inert once the player
/// is correctly grounded, so it can never ratchet upward.
#[test]
fn ronfaure_grounded_player_is_a_fixed_point() {
    let Some(geom) = ronfaure_collision() else {
        return;
    };
    let recovered = geom
        .ground_or_recover_wire_z(REPRO_FFXI_X, REPRO_FFXI_Y, -REPRO_FLOOR_BEVY_Y)
        .expect("recovery found no floor");
    assert!(
        (recovered + REPRO_FLOOR_BEVY_Y).abs() < 1e-2,
        "already grounded, recovery must not move us, got {recovered}"
    );
}
