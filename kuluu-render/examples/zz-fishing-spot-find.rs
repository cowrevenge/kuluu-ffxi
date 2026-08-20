//! Find a stand-here-and-face-this spot where the client-side fishing gate
//! passes, so a verification drive can teleport straight to one instead of
//! wandering the shoreline.
//!
//! Prints LSB/server coordinates (what `!pos` takes) and the heading byte.
//!
//! usage: cargo run -p kuluu-render --example zz-fishing-spot-find <zone_id> [count]

use bevy::math::{Vec2, Vec3};
use kuluu_render::combat_stance::heading_forward;
use kuluu_render::dat_mzb::{
    build_collision_geometry, facing_water, load_mzb_placed, MzbCollisionGeometry,
};

/// Search half-extent and step in yalms around the zone origin.
const SEARCH_RADIUS: f32 = 400.0;
const STEP: f32 = 2.0;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let zone: u16 = args.first().and_then(|s| s.parse().ok()).unwrap_or(232);
    let want: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);

    bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let file_id = ffxi_dat::zone_dat::effective_zone_dat_file_id(Some(zone), None)
        .expect("zone -> dat file id");
    let (submeshes, instances) = load_mzb_placed(file_id, None).expect("load mzb");
    let geom = MzbCollisionGeometry::from_block(build_collision_geometry(
        &submeshes,
        &instances,
        Some(file_id),
    ));
    println!(
        "zone {zone} (DAT {file_id}): {} triangles",
        geom.tri_count()
    );

    // Probe mode: check the exact spots the user reported, in client-display
    // coordinates (x, ground, vertical) -> Bevy (x, -vertical, -ground).
    for (label, dx, dground, dvert) in [
        ("reported FAIL (quay)", -36.45f32, 62.31f32, -4.00f32),
        ("reported OK (in water)", -11.59, 46.84, 6.00),
    ] {
        let pos = Vec3::new(dx, -dvert, -dground);
        let terrain = geom.terrain_nearest(Vec2::new(pos.x, pos.z), pos.y);
        let pass: Vec<u16> = (0u16..256)
            .step_by(8)
            .filter(|h| facing_water(&geom, pos, heading_forward(*h as u8)))
            .collect();
        println!("  {label}: standing on {terrain:?}, headings that pass: {pass:?}");
    }

    let mut found = 0usize;
    let mut x = -SEARCH_RADIUS;
    while x <= SEARCH_RADIUS && found < want {
        let mut z = -SEARCH_RADIUS;
        while z <= SEARCH_RADIUS && found < want {
            let xz = Vec2::new(x, z);
            let terrain = geom.terrain_nearest(xz, 0.0);
            // Stand on dry land next to the water, the way a player would.
            if terrain.is_some_and(|t| !t.is_water()) {
                if let Some(y) = geom.ground_nearest(xz, 0.0) {
                    let pos = Vec3::new(xz.x, y, xz.y);
                    for heading in (0u16..256).step_by(8) {
                        let h = heading as u8;
                        if facing_water(&geom, pos, heading_forward(h)) {
                            // Bevy -> LSB/server space is the same y/z negation
                            // the loader applied on the way in.
                            println!(
                                "  !pos {:.3} {:.3} {:.3}   heading {h}  (standing on {:?})",
                                pos.x, -pos.y, -pos.z, terrain
                            );
                            found += 1;
                            break;
                        }
                    }
                }
            }
            z += STEP;
        }
        x += STEP;
    }
    if found == 0 {
        println!("  no water-facing spot found");
    }
}
