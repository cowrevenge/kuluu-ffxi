//! Offline probe of the MZB collision column the player grounds on.
//!
//! Usage: zz-collision-column <zone_id> <ffxi_x> <ffxi_y> <ffxi_z>
//! (ffxi_y is horizontal, ffxi_z is height — matching the client's Vec3.)

use bevy::math::{Vec2, Vec3};
use bevy::tasks::AsyncComputeTaskPool;
use ffxi_viewer_core::dat_mzb::{build_collision_geometry, load_mzb_placed, FLOOR_NORMAL_MIN};

fn main() {
    let mut args = std::env::args().skip(1);
    let zone_id: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(246);
    let px: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let py: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let pz: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);

    AsyncComputeTaskPool::get_or_init(Default::default);

    let file_id = ffxi_dat::zone_dat::effective_zone_dat_file_id(Some(zone_id), None)
        .expect("zone -> mzb file id");
    println!("zone {zone_id} -> DAT file {file_id}");

    let (submeshes, instances) = load_mzb_placed(file_id, None).expect("load_mzb_placed");
    println!(
        "submeshes={} instances={}",
        submeshes.len(),
        instances.len()
    );

    let mut positions: Vec<Vec3> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut n_collision_inst = 0usize;
    for inst in &instances {
        let sub = &submeshes[inst.submesh_idx];
        if sub.flags & 1 != 0 {
            continue;
        }
        n_collision_inst += 1;
        let base = positions.len() as u32;
        for v in &sub.positions {
            positions.push(inst.bevy_transform.transform_point(Vec3::from_array(*v)));
        }
        indices.extend(sub.indices.iter().map(|i| i + base));
    }
    println!(
        "collision placements={n_collision_inst} verts={} tris={}",
        positions.len(),
        indices.len() / 3
    );

    {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for p in &positions {
            min = min.min(*p);
            max = max.max(*p);
        }
        println!("collision AABB (bevy): min={min:?} max={max:?}");
    }

    let geom = build_collision_geometry(&submeshes, &instances, Some(file_id));

    // bevy.x = ffxi.x, bevy.z = -ffxi.y, bevy.y = -ffxi.z
    let bevy_xz = Vec2::new(px, -py);
    let bevy_y = -pz;
    println!(
        "\nplayer ffxi=({px:.2}, {py:.2}, {pz:.2})  bevy=({:.2}, {bevy_y:.2}, {:.2})",
        bevy_xz.x, bevy_xz.y
    );

    let hits = geom.ground_raycast_all(bevy_xz);
    println!("column hits at player xz: {}", hits.len());
    for (i, (y, n)) in hits.iter().enumerate() {
        let kind = if n.y.abs() >= FLOOR_NORMAL_MIN {
            "FLOOR"
        } else {
            "wall"
        };
        println!(
            "  #{}: bevy_y={y:+8.3} (ffxi_z={:+8.3}) n.y={:+.3} [{kind}]",
            i + 1,
            -y,
            n.y
        );
    }
    println!(
        "ground_nearest(ref={bevy_y:.2}) = {:?}",
        geom.ground_nearest(bevy_xz, bevy_y)
    );

    // Grid sweep: where does the floor vanish around this spot?
    println!("\nfloor map (rows = bevy z, cols = bevy x, step 2y, '.' = no floor):");
    let step = 2.0;
    let half = 12;
    print!("        ");
    for cx in -half..=half {
        print!("{:>7.0}", bevy_xz.x + cx as f32 * step);
    }
    println!();
    for cz in -half..=half {
        let z = bevy_xz.y + cz as f32 * step;
        print!("{z:>7.0} ");
        for cx in -half..=half {
            let x = bevy_xz.x + cx as f32 * step;
            match geom.ground_nearest(Vec2::new(x, z), bevy_y) {
                Some(y) => print!("{y:>7.1}"),
                None => print!("      ."),
            }
        }
        println!();
    }
}
