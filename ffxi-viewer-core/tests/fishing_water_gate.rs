//! The terrain half of the fishing gate, against retail DATs.
//!
//! `facing_water` decides locally whether retail would offer "Fish", and it
//! rests on reading the MZB collision triangle nibble as a terrain type — a
//! claim that comes from research/xim (tier 6), so it is checked here against an
//! independent authority: LSB's `fishing_area` table, which was dumped from
//! retail server data and never from these DATs.

use bevy::math::{Vec2, Vec3};
use ffxi_dat::mzb::TerrainType;
use ffxi_viewer_core::dat_mzb::{
    build_collision_geometry, facing_water, load_mzb_placed, MzbCollisionGeometry,
};
use ffxi_viewer_core::scene::mzb_to_bevy;

/// Carpenters' Landing. Its whole shoreline is fishable, and LSB gives it six
/// radial areas to check against.
const ZONE_CARPENTERS_LANDING: u16 = 2;

/// `fishing_area` rows for zone 2 (vendor/server/sql/fishing_area.sql:49-54):
/// centre x/z and radius, in the same coordinate space as the raw MZB.
const CARPENTERS_AREAS: &[(&str, f32, f32, f32)] = &[
    ("South Landing", 172.250, -475.286, 150.0),
    ("Other Waterside South", -101.576, -484.401, 60.0),
    ("Central Landing", -164.099, 59.123, 80.0),
    ("North Landing", -332.920, 564.747, 150.0),
];

fn zone() -> Option<MzbCollisionGeometry> {
    if std::env::var("FFXI_DAT_PATH").is_err() && ffxi_dat::DatRoot::from_env_or_default().is_err()
    {
        eprintln!("no FFXI install; skipping");
        return None;
    }
    bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let file_id =
        ffxi_dat::zone_dat::effective_zone_dat_file_id(Some(ZONE_CARPENTERS_LANDING), None)?;
    let (submeshes, instances) = load_mzb_placed(file_id, None).ok()?;
    Some(build_collision_geometry(
        &submeshes,
        &instances,
        Some(file_id),
    ))
}

/// `load_mzb_placed` returns Bevy-space geometry; LSB's coordinates are raw MZB
/// space, so they go through the same transform the loader applies.
fn to_bevy_xz(x: f32, z: f32) -> Vec2 {
    let p = mzb_to_bevy(ffxi_viewer_wire::Vec3 { x, y: 0.0, z });
    Vec2::new(p.x, p.z)
}

#[test]
fn terrain_is_carried_through_to_the_collision_geometry() {
    let Some(geom) = zone() else { return };
    assert_eq!(
        geom.tri_terrain.len(),
        geom.tri_count(),
        "terrain must be per placed triangle, like camera_skip"
    );
    assert!(
        geom.tri_terrain
            .iter()
            .any(|&t| TerrainType::from_nibble(t).is_some_and(TerrainType::is_water)),
        "Carpenters' Landing has no water triangles — the nibble is not the terrain type"
    );
}

/// Every LSB fishing area must contain water the client can see. If the nibble
/// meant something else, these cylinders would look like any other patch of
/// ground and `/fish` would refuse everywhere.
#[test]
fn every_lsb_fishing_area_contains_water_the_client_can_find() {
    let Some(geom) = zone() else { return };

    for (name, cx, cz, radius) in CARPENTERS_AREAS {
        let centre = to_bevy_xz(*cx, *cz);
        // Sample a coarse lattice across the cylinder rather than the exact
        // centre: LSB's centre is a spawn-area midpoint, not guaranteed to be
        // over water itself.
        const LATTICE: i32 = 12;
        let step = 2.0 * radius / LATTICE as f32;
        let mut water = 0usize;
        let mut floors = 0usize;
        for ix in 0..=LATTICE {
            for iz in 0..=LATTICE {
                let p = centre + Vec2::new(-radius + step * ix as f32, -radius + step * iz as f32);
                if p.distance(centre) > *radius {
                    continue;
                }
                if let Some(t) = geom.terrain_nearest(p, 0.0) {
                    floors += 1;
                    water += usize::from(t.is_water());
                }
            }
        }
        assert!(floors > 0, "{name}: no floor found anywhere in the area");
        assert!(
            water > 0,
            "{name}: {floors} floors sampled, none of them water"
        );
    }
}

/// The gate must actually flip on facing: standing on water-adjacent ground and
/// looking at the water passes, looking away does not. Without this the entry
/// would either never appear or appear everywhere.
#[test]
fn facing_water_depends_on_which_way_the_player_looks() {
    let Some(geom) = zone() else { return };

    // Find a spot with water ahead in exactly one of the four cardinal
    // directions, so "faces water" and "faces away" are both exercised.
    let (name, cx, cz, radius) = CARPENTERS_AREAS[0];
    let centre = to_bevy_xz(cx, cz);
    const CARDINALS: [Vec3; 4] = [
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(-1.0, 0.0, 0.0),
    ];

    const LATTICE: i32 = 40;
    let step = 2.0 * radius / LATTICE as f32;
    let mut found = None;
    'search: for ix in 0..=LATTICE {
        for iz in 0..=LATTICE {
            let xz = centre + Vec2::new(-radius + step * ix as f32, -radius + step * iz as f32);
            let Some(terrain) = geom.terrain_nearest(xz, 0.0) else {
                continue;
            };
            // Stand on dry land, not in the water.
            if terrain.is_water() {
                continue;
            }
            let Some(y) = geom.ground_nearest(xz, 0.0) else {
                continue;
            };
            let pos = Vec3::new(xz.x, y, xz.y);
            let hits = CARDINALS
                .iter()
                .filter(|d| facing_water(&geom, pos, **d))
                .count();
            if hits > 0 && hits < CARDINALS.len() {
                found = Some((pos, hits));
                break 'search;
            }
        }
    }

    let (pos, hits) = found
        .unwrap_or_else(|| panic!("{name}: no shoreline spot where facing decides the outcome"));
    assert!(
        CARDINALS.iter().any(|d| facing_water(&geom, pos, *d)),
        "facing water must pass at {pos:?} ({hits} of 4 headings do)"
    );
    assert!(
        CARDINALS.iter().any(|d| !facing_water(&geom, pos, *d)),
        "facing away from water must fail at {pos:?}"
    );
}
