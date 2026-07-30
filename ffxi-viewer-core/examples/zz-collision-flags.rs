//! Raycasts a column against the collision set (`flags & 1 == 0`) and the
//! non-collision set separately, to tell "the floor is genuinely absent" apart
//! from "the floor is present but excluded/misclassified".
//!
//! Usage: zz-collision-flags <zone_id> <x> <y> [<x2> <y2> ...]

use bevy::math::{Vec2, Vec3};
use bevy::tasks::AsyncComputeTaskPool;
use ffxi_viewer_core::dat_mzb::{load_mzb_placed, MzbCollisionGeometry, FLOOR_NORMAL_MIN};

fn build(
    submeshes: &[ffxi_viewer_core::dat_mzb::MzbSubMesh],
    instances: &[ffxi_viewer_core::dat_mzb::MzbInstance],
    want_collision: bool,
) -> MzbCollisionGeometry {
    let mut positions: Vec<Vec3> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for inst in instances {
        let sub = &submeshes[inst.submesh_idx];
        if (sub.flags & 1 == 0) != want_collision {
            continue;
        }
        let base = positions.len() as u32;
        for v in &sub.positions {
            positions.push(inst.bevy_transform.transform_point(Vec3::from_array(*v)));
        }
        indices.extend(sub.indices.iter().map(|i| i + base));
    }
    MzbCollisionGeometry {
        cell_index: Default::default(),
        positions,
        indices,
        source_file_id: None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let zone_id: u16 = args[0].parse().expect("zone id");
    AsyncComputeTaskPool::get_or_init(Default::default);

    let file_id = ffxi_dat::zone_dat::effective_zone_dat_file_id(Some(zone_id), None)
        .expect("zone -> mzb file id");
    let (submeshes, instances) = load_mzb_placed(file_id, None).expect("load_mzb_placed");

    let coll = build(&submeshes, &instances, true);
    let noncoll = build(&submeshes, &instances, false);
    println!(
        "zone {zone_id} (DAT {file_id}): collision tris={} non-collision tris={}",
        coll.tri_count(),
        noncoll.tri_count()
    );

    if let Ok(spec) = std::env::var("KULUU_SWEEP") {
        let p: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let (cx, cy, radius, step) = (p[0], p[1], p[2], p[3]);
        let n = (radius / step) as i32;
        let (mut both, mut coll_only, mut noncoll_only, mut neither) = (0, 0, 0, 0);
        for iy in -n..=n {
            for ix in -n..=n {
                let xz = Vec2::new(cx + ix as f32 * step, -(cy + iy as f32 * step));
                let f = |g: &MzbCollisionGeometry| {
                    g.ground_raycast_all(xz)
                        .iter()
                        .any(|(_, nrm)| nrm.y.abs() >= FLOOR_NORMAL_MIN)
                };
                match (f(&coll), f(&noncoll)) {
                    (true, true) => both += 1,
                    (true, false) => coll_only += 1,
                    (false, true) => noncoll_only += 1,
                    (false, false) => neither += 1,
                }
            }
        }
        let total = both + coll_only + noncoll_only + neither;
        println!(
            "\nsweep {radius} radius @ {step} around ({cx},{cy}): {total} columns\n  \
             floor in collision set only : {coll_only}\n  \
             floor in BOTH sets          : {both}\n  \
             floor ONLY in non-collision : {noncoll_only}  <-- dropped from grounding\n  \
             no floor in either          : {neither}"
        );
    }

    for pair in args[1..].chunks_exact(2) {
        let x: f32 = pair[0].parse().unwrap();
        let y: f32 = pair[1].parse().unwrap();
        let xz = Vec2::new(x, -y);
        println!("\nffxi=({x:.3}, {y:.3})");
        for (label, geom) in [("collision", &coll), ("non-collision", &noncoll)] {
            let hits = geom.ground_raycast_all(xz);
            if hits.is_empty() {
                println!("  {label:>13}: (no hits)");
            }
            for (hy, n) in hits {
                let kind = if n.y.abs() >= FLOOR_NORMAL_MIN {
                    "FLOOR"
                } else {
                    "wall "
                };
                println!("  {label:>13}: bevy_y={hy:+8.3} n.y={:+.3} [{kind}]", n.y);
            }
        }
    }
}
