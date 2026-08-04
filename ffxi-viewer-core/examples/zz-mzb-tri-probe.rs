//! Reports every MZB triangle in a vertical column with all per-triangle
//! attributes ffxi-dat parses, so a collision predicate can be designed from
//! data rather than guessed: authored normal (vs the winding-derived one),
//! is_invalid, camera-transparent bit, material, raw mesh flags, placement
//! determinant.
//!
//! Usage: zz-mzb-tri-probe <zone_id> <x> <y> [<x2> <y2> ...]
//! Set KULUU_DAT_FILE_ID to probe a DAT the zone table doesn't reach (Mog House
//! interiors, whose file id depends on the myroom model rather than the zone).

use bevy::math::{Mat3, Mat4, Vec3};
use ffxi_dat::chunk::walk;
use ffxi_dat::mzb;
use ffxi_dat::{ChunkKind, DatRoot};
use ffxi_viewer_core::camera::ChaseCamera;

struct Tri {
    v: [Vec3; 3],
    authored_n: Vec3,
    geom_n: Vec3,
    invalid: bool,
    camera_transparent: bool,
    material: u8,
    mesh_flags: u16,
    det_neg: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let zone_id: u16 = args[0].parse().expect("zone id");

    let file_id = match std::env::var("KULUU_DAT_FILE_ID") {
        Ok(s) => s.parse().expect("KULUU_DAT_FILE_ID"),
        Err(_) => ffxi_dat::zone_dat::effective_zone_dat_file_id(Some(zone_id), None)
            .expect("zone -> mzb file id"),
    };
    let root = DatRoot::from_env_or_default().expect("DatRoot");
    let path = root.resolve(file_id).expect("resolve").path_under(&root);
    let bytes = std::fs::read(&path).expect("read dat");
    let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();
    let chunk = chunks
        .iter()
        .find(|c| c.kind == ChunkKind::Mzb as u8)
        .expect("mzb chunk");
    let plain = mzb::decrypt(chunk.data).expect("decrypt");
    let header = mzb::MzbHeader::parse(&plain).expect("header");
    let placements = mzb::parse_placements(&plain, &header).expect("placements");

    let to_bevy = Mat4::from_cols(
        [1.0, 0.0, 0.0, 0.0].into(),
        [0.0, -1.0, 0.0, 0.0].into(),
        [0.0, 0.0, -1.0, 0.0].into(),
        [0.0, 0.0, 0.0, 1.0].into(),
    );

    let mut tris: Vec<Tri> = Vec::new();
    for p in &placements {
        let Ok(m) = mzb::parse_mesh_at(&plain, p.geometry_offset as usize) else {
            continue;
        };
        let m_bevy = to_bevy * Mat4::from_cols_array(&p.transform);
        let n_mat = Mat3::from_mat4(m_bevy).inverse().transpose();
        for (i, t) in m.triangles.iter().enumerate() {
            let v: Vec<Vec3> = t
                .iter()
                .filter_map(|&idx| m.vertices.get(idx as usize))
                .map(|vt| m_bevy.transform_point3(Vec3::from_array(vt.pos)))
                .collect();
            if v.len() != 3 {
                continue;
            }
            let authored = m
                .triangle_normals
                .get(i)
                .and_then(|&ni| m.normals.get(ni as usize))
                .map(|n| (n_mat * Vec3::from_array(n.n)).normalize_or_zero())
                .unwrap_or(Vec3::ZERO);
            let info = &m.tri_info[i];
            tris.push(Tri {
                v: [v[0], v[1], v[2]],
                authored_n: authored,
                geom_n: (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero(),
                invalid: info.is_invalid,
                camera_transparent: info.camera_transparent,
                material: info.material,
                mesh_flags: m.flags,
                det_neg: p.flip_winding,
            });
        }
    }
    println!("zone {zone_id} (DAT {file_id}): {} triangles", tris.len());

    let (mut anti, mut agree, mut other, mut degen) = (0, 0, 0, 0);
    let (mut n_inval, mut n_cam_transparent, mut n_flag0) = (0, 0, 0);
    for t in &tris {
        let d = t.authored_n.dot(t.geom_n);
        if t.geom_n == Vec3::ZERO || t.authored_n == Vec3::ZERO {
            degen += 1;
        } else if d < -0.99 {
            anti += 1;
        } else if d > 0.99 {
            agree += 1;
        } else {
            other += 1;
        }
        n_inval += t.invalid as usize;
        n_cam_transparent += t.camera_transparent as usize;
        n_flag0 += (t.mesh_flags & 1 != 0) as usize;
    }
    println!(
        "  authored vs winding-derived normal: antiparallel={anti} parallel={agree} \
         neither={other} degenerate={degen}"
    );
    println!(
        "  is_invalid={n_inval}  camera_transparent_bit={n_cam_transparent}  mesh_flag0={n_flag0}"
    );

    // KULUU_MESHFLAGS — sizes the retail camera-skip predicate before we ship it
    // (kuluu-eg5g). M1: is `flags != 0` (what CollisionQuery.hpp tests) actually
    // distinguishable from `flags & 1` (what doesnt_block_los uses) on real data?
    // M2: what does the skip set contain — the camera has no ground clamp, so
    // up-facing floors in it mean the camera can sink under terrain. M3: is the
    // skipped count per unique mesh or per placed world triangle?
    if std::env::var("KULUU_MESHFLAGS").is_ok() {
        let mut per_mesh: std::collections::BTreeMap<u16, usize> = Default::default();
        let mut seen_offsets = std::collections::HashSet::new();
        for p in &placements {
            if !seen_offsets.insert(p.geometry_offset) {
                continue;
            }
            if let Ok(m) = mzb::parse_mesh_at(&plain, p.geometry_offset as usize) {
                *per_mesh.entry(m.flags).or_default() += 1;
            }
        }
        println!("\nM1 unique-mesh flags histogram (raw u16):");
        for (flags, count) in &per_mesh {
            println!("  flags=0x{flags:04X}  meshes={count}");
        }
        let m_nonzero: usize = per_mesh
            .iter()
            .filter(|(f, _)| **f != 0)
            .map(|(_, c)| c)
            .sum();
        let m_bit0: usize = per_mesh
            .iter()
            .filter(|(f, _)| **f & 1 != 0)
            .map(|(_, c)| c)
            .sum();
        println!(
            "  meshes flags!=0: {m_nonzero}   meshes flags&1: {m_bit0}   {}",
            if m_nonzero == m_bit0 {
                "IDENTICAL on this zone"
            } else {
                "*** DIFFER — `!= 0` and `& 1` are different predicates ***"
            }
        );

        let (mut skip_floor, mut skip_wall, mut skip_ceil) = (0usize, 0usize, 0usize);
        let mut skipped = 0usize;
        for t in &tris {
            if !(t.mesh_flags != 0 && t.camera_transparent) {
                continue;
            }
            skipped += 1;
            if t.authored_n.y >= 0.5 {
                skip_floor += 1;
            } else if t.authored_n.y <= -0.5 {
                skip_ceil += 1;
            } else {
                skip_wall += 1;
            }
        }
        println!("\nM2 camera skip set (flags != 0 && camera_transparent), world triangles:");
        println!(
            "  skipped={skipped} of {} ({:.2}%)  retained={}",
            tris.len(),
            100.0 * skipped as f32 / tris.len().max(1) as f32,
            tris.len() - skipped
        );
        println!(
            "  up-facing FLOOR={skip_floor}  wall={skip_wall}  down-facing ceiling={skip_ceil}"
        );
        if skip_floor > 0 {
            println!("  ^ floors in the skip set: camera can sink under terrain there");
        }

        let mesh_level_transparent: usize = {
            let mut n = 0;
            let mut seen = std::collections::HashSet::new();
            for p in &placements {
                if !seen.insert(p.geometry_offset) {
                    continue;
                }
                if let Ok(m) = mzb::parse_mesh_at(&plain, p.geometry_offset as usize) {
                    n += m.tri_info.iter().filter(|t| t.camera_transparent).count();
                }
            }
            n
        };
        println!(
            "\nM3 camera_transparent bit: {mesh_level_transparent} per unique mesh, \
             {n_cam_transparent} per placed world triangle"
        );
        return;
    }

    // KULUU_COVERAGE=cx,cy,radius,step — per-column comparison of the retired
    // grounding predicate (|n.y| over flag0-clear submeshes) against the current
    // one (authored n.y, all submeshes). "lost" is the number that regressed to
    // no floor at all, which would drop a walker through the world.
    if let Ok(spec) = std::env::var("KULUU_COVERAGE") {
        let p: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let (cx, cy, radius, step) = (p[0], p[1], p[2], p[3]);
        let n = (radius / step) as i32;
        let (mut both, mut gained, mut lost, mut neither) = (0, 0, 0, 0);
        for iy in -n..=n {
            for ix in -n..=n {
                let orig = Vec3::new(cx + ix as f32 * step, 1000.0, -(cy + iy as f32 * step));
                let (mut old_has, mut new_has) = (false, false);
                for t in &tris {
                    if ray_tri(orig, Vec3::NEG_Y, t.v[0], t.v[1], t.v[2]).is_none() {
                        continue;
                    }
                    old_has |= (t.mesh_flags & 1 == 0) && t.geom_n.y.abs() >= 0.5;
                    new_has |= t.authored_n.y >= 0.5;
                }
                match (old_has, new_has) {
                    (true, true) => both += 1,
                    (false, true) => gained += 1,
                    (true, false) => {
                        if lost < 6 {
                            println!("  lost column ffxi=({:.3}, {:.3})", orig.x, -orig.z);
                        }
                        lost += 1;
                    }
                    (false, false) => neither += 1,
                }
            }
        }
        println!(
            "\ncoverage over {} columns @ {step} around ({cx},{cy}):\n  \
             floor under both predicates : {both}\n  \
             gained (was a hole)         : {gained}\n  \
             LOST (would fall through)   : {lost}\n  \
             no floor either way         : {neither}",
            both + gained + lost + neither
        );
        return;
    }

    // KULUU_CAMSWEEP=x,y,z,distance — sweep the chase-camera orbit at an ffxi
    // position and report, per direction, what the anchor->camera ray hits. This
    // is the query clamp_chase_camera_to_collision makes every frame.
    if let Ok(spec) = std::env::var("KULUU_CAMSWEEP") {
        let p: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        const THIRD_PERSON_ANCHOR: f32 = 2.3 * 0.55;
        let anchor = Vec3::new(p[0], -p[2] + THIRD_PERSON_ANCHOR, -p[1]);
        // The zoom level the player has dialled in; the radius the camera
        // actually reaches is ChaseCamera's, not this — pitching grows it.
        let zoom = p[3];
        println!("\nanchor bevy={anchor:?}  zoom={zoom}");
        println!(
            "  {:>5} {:>6} {:>7}  {:>8} {:>9} {:>8}",
            "yaw", "pitch", "radius", "hit_t", "eye_y", "n.y"
        );
        const PITCH_STEPS: usize = 4;
        for yaw_i in 0..24 {
            let yaw = yaw_i as f32 * std::f32::consts::TAU / 24.0;
            for pitch_i in 0..=PITCH_STEPS {
                let chase = ChaseCamera {
                    yaw,
                    pitch: ChaseCamera::PITCH_MAX * pitch_i as f32 / PITCH_STEPS as f32,
                    distance: zoom,
                    ..Default::default()
                };
                let (pitch, wanted) = (chase.pitch, chase.orbit_radius());
                let (cos_p, sin_p) = (pitch.cos(), pitch.sin());
                let dir = Vec3::new(yaw.sin() * cos_p, sin_p, yaw.cos() * cos_p);
                let mut best: Option<(f32, &Tri)> = None;
                for t in &tris {
                    if let Some(hit) = ray_tri(anchor, dir, t.v[0], t.v[1], t.v[2]) {
                        if hit < wanted && best.is_none_or(|(b, _)| hit < b) {
                            best = Some((hit, t));
                        }
                    }
                }
                match best {
                    Some((hit_t, t)) => println!(
                        "  {:>5.0} {:>6.2} {wanted:>7.3}  {hit_t:>8.3} {:>9.3} {:>8.3}",
                        yaw.to_degrees(),
                        pitch,
                        (anchor + dir * hit_t).y,
                        t.authored_n.y
                    ),
                    None => println!(
                        "  {:>5.0} {:>6.2} {wanted:>7.3}  {:>8} {:>9.3}  free flight",
                        yaw.to_degrees(),
                        pitch,
                        "MISS",
                        (anchor + dir * wanted).y
                    ),
                }
            }
        }
        return;
    }

    for pair in args[1..].chunks_exact(2) {
        let (x, y): (f32, f32) = (pair[0].parse().unwrap(), pair[1].parse().unwrap());
        let orig = Vec3::new(x, 1000.0, -y);
        let dir = Vec3::NEG_Y;
        println!("\nffxi=({x:.3}, {y:.3})   [bevy xz = ({x:.3}, {:.3})]", -y);
        println!(
            "  {:>8}  {:>7} {:>7}  {:>5} {:>7} {:>3}  {:>5} {:>4}",
            "bevy_y", "auth.y", "geom.y", "inval", "barrier", "mat", "flag0", "det-"
        );
        let mut hits: Vec<&Tri> = Vec::new();
        let mut ys: Vec<f32> = Vec::new();
        for t in &tris {
            if let Some(hit) = ray_tri(orig, dir, t.v[0], t.v[1], t.v[2]) {
                hits.push(t);
                ys.push(1000.0 - hit);
            }
        }
        let mut order: Vec<usize> = (0..hits.len()).collect();
        order.sort_by(|a, b| ys[*b].partial_cmp(&ys[*a]).unwrap());
        for i in order {
            let t = hits[i];
            println!(
                "  {:>+8.3}  {:>+7.3} {:>+7.3}  {:>5} {:>7} {:>3}  {:>5} {:>4}",
                ys[i],
                t.authored_n.y,
                t.geom_n.y,
                t.invalid,
                t.camera_transparent,
                t.material,
                t.mesh_flags & 1 != 0,
                t.det_neg
            );
        }
        if hits.is_empty() {
            println!("  (no hits)");
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
