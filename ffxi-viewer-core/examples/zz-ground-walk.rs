//! Walks a straight line between two ffxi (x, y) points, applying the client's
//! per-frame grounding (`ground_step` seeded with the previous frame's height)
//! and reporting every height change, so a walk that misbehaves in game can be
//! replayed without a live session.
//!
//! Usage: zz-ground-walk <zone_id> <x0> <y0> <z0> <x1> <y1> [step]

use bevy::math::Vec2;
use bevy::tasks::AsyncComputeTaskPool;
use ffxi_viewer_core::dat_mzb::{build_collision_geometry, load_mzb_placed, MAX_GROUND_STEP_UP};

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

    let geom = build_collision_geometry(&submeshes, &instances, Some(file_id));

    // KULUU_RISE_HIST=cx,cy,radius,step — histogram the per-frame upward snap
    // over a lattice of walks across an area, to separate the stair/ramp regime
    // from pathological jumps between surfaces. Deliberately unbounded
    // (`ground_nearest`, not `ground_step`): this is what sizes MAX_GROUND_STEP_UP,
    // so it has to see the snaps that bound would reject.
    if let Ok(spec) = std::env::var("KULUU_RISE_HIST") {
        let p: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let (cx, cy, radius, gs) = (p[0], p[1], p[2], p[3]);
        let lanes = (radius * 2.0 / gs) as i32;
        let mut hist = [0usize; 12];
        let mut rises: Vec<f32> = Vec::new();
        for lane in 0..=lanes {
            for axis in 0..2 {
                let off = -radius + lane as f32 * gs;
                let mut y_prev: Option<f32> = None;
                let n = (radius * 2.0 / step) as i32;
                for i in 0..=n {
                    let t = -radius + i as f32 * step;
                    let (px, py) = if axis == 0 {
                        (cx + off, cy + t)
                    } else {
                        (cx + t, cy + off)
                    };
                    let seed = y_prev.unwrap_or(1.0);
                    let Some(hit) = geom.ground_nearest(Vec2::new(px, -py), seed) else {
                        continue;
                    };
                    if let Some(prev) = y_prev {
                        let rise = hit - prev;
                        if rise > 0.001 {
                            rises.push(rise);
                            let b = (rise / 0.25).floor().min(11.0) as usize;
                            hist[b] += 1;
                        }
                    }
                    y_prev = Some(hit);
                }
            }
        }
        rises.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "upward snaps over {} lattice walks: {}",
            lanes * 2,
            rises.len()
        );
        for (b, c) in hist.iter().enumerate() {
            if *c > 0 {
                println!(
                    "  [{:.2}, {:.2}): {c}",
                    b as f32 * 0.25,
                    (b + 1) as f32 * 0.25
                );
            }
        }
        for q in [0.5, 0.9, 0.99, 0.999] {
            let i = ((rises.len() as f64 - 1.0) * q) as usize;
            println!("  p{:<5} = {:.3}", q * 100.0, rises[i]);
        }
        println!("  max     = {:.3}", rises.last().unwrap());
        return;
    }

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
        let Some(hit) = geom.ground_step(Vec2::new(x, -y), bevy_y, MAX_GROUND_STEP_UP) else {
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
