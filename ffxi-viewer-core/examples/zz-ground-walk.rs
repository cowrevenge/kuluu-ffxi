//! Walks a straight line between two ffxi (x, y) points, applying the client's
//! per-frame grounding (`ground_nearest` seeded with the previous frame's
//! height) and reporting every height change. Reproduces offline what the
//! player sees when a short walk snaps them onto a roof.
//!
//! Usage: zz-ground-walk <zone_id> <x0> <y0> <z0> <x1> <y1> [step]

use bevy::math::{Vec2, Vec3};
use bevy::tasks::AsyncComputeTaskPool;
use ffxi_viewer_core::dat_mzb::{load_mzb_placed, MzbCollisionGeometry};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut next = |d: f32| args.next().and_then(|s| s.parse().ok()).unwrap_or(d);
    let zone_id = next(245.0) as u16;
    let x0 = next(0.0);
    let y0 = next(0.0);
    let z0 = next(0.0);
    let x1 = next(0.0);
    let y1 = next(0.0);
    let step = next(0.05);

    AsyncComputeTaskPool::get_or_init(Default::default);

    let file_id = ffxi_dat::zone_dat::effective_zone_dat_file_id(Some(zone_id), None)
        .expect("zone -> mzb file id");
    let (submeshes, instances) = load_mzb_placed(file_id, None).expect("load_mzb_placed");

    let mut positions: Vec<Vec3> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for inst in &instances {
        let sub = &submeshes[inst.submesh_idx];
        if sub.flags & 1 != 0 {
            continue;
        }
        let base = positions.len() as u32;
        for v in &sub.positions {
            positions.push(inst.bevy_transform.transform_point(Vec3::from_array(*v)));
        }
        indices.extend(sub.indices.iter().map(|i| i + base));
    }
    let geom = MzbCollisionGeometry {
        cell_index: Default::default(),
        positions,
        indices,
        source_file_id: Some(file_id),
    };

    let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    let n = (dist / step).ceil().max(1.0) as usize;
    println!("zone {zone_id} (DAT {file_id}): ({x0:.2},{y0:.2}) -> ({x1:.2},{y1:.2}), {dist:.2} units in {n} steps of {step}");
    println!("start ffxi_z={z0:.3} (bevy_y={:.3})\n", -z0);

    // bevy.x = ffxi.x, bevy.z = -ffxi.y, bevy.y = -ffxi.z
    let mut bevy_y = -z0;
    let mut biggest: f32 = 0.0;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = x0 + (x1 - x0) * t;
        let y = y0 + (y1 - y0) * t;
        let Some(hit) = geom.ground_nearest(Vec2::new(x, -y), bevy_y) else {
            println!("  step {i:4}: ffxi=({x:7.3},{y:7.3})  NO FLOOR (kept bevy_y={bevy_y:.3})");
            continue;
        };
        let delta = hit - bevy_y;
        if delta.abs() > 0.05 {
            println!(
                "  step {i:4}: ffxi=({x:7.3},{y:7.3})  bevy_y {bevy_y:+7.3} -> {hit:+7.3}  ({delta:+.3})  [ffxi_z {:+.3}]",
                -hit
            );
            biggest = biggest.max(delta.abs());
        }
        bevy_y = hit;
    }
    println!(
        "\nend bevy_y={bevy_y:.3} (ffxi_z={:.3}); largest single-step snap {biggest:.3}",
        -bevy_y
    );
}
