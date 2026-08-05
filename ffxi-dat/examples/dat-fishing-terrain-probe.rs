//! Cross-checks the MZB collision terrain nibble against LSB's `fishing_area`
//! table: for every radial fishing area, histogram the terrain type of the
//! placed collision triangles inside the cylinder, and compare against the
//! whole-zone histogram.
//!
//! If the nibble really is the terrain type (research/xim ZoneDefParser.kt
//! TerrainType), water indices must be strongly over-represented inside a
//! fishing area relative to the zone at large.
//!
//! usage: cargo run -p ffxi-dat --example dat-fishing-terrain-probe [zone_id ...]

use std::collections::BTreeMap;
use std::process::ExitCode;

use ffxi_dat::mzb;
use ffxi_dat::zone_dat::zone_id_to_mzb_file_id;
use ffxi_dat::DatRoot;

const FISHING_AREA_SQL: &str = "vendor/server/sql/fishing_area.sql";

const BOUND_TYPE_RADIAL: u8 = 1;

const TERRAIN_NAMES: [&str; 16] = [
    "Object",
    "Path",
    "Grass",
    "Sand",
    "Snow",
    "Stone",
    "Metal",
    "Wood",
    "ShallowWater",
    "DeepWater",
    "Unk0xA",
    "Unk0xB",
    "Unk0xC",
    "Unk0xD",
    "Unk0xE",
    "Unk0xF",
];

struct RadialArea {
    zone: u16,
    name: String,
    center: [f32; 3],
    radius: f32,
}

fn parse_areas(sql: &str) -> Vec<RadialArea> {
    let mut out = Vec::new();
    for line in sql.lines() {
        let Some(rest) = line.strip_prefix("INSERT INTO `fishing_area` VALUES (") else {
            continue;
        };
        let Some(rest) = rest.strip_suffix(");") else {
            continue;
        };
        // zoneid, areaid, 'name', bound_type, bound_height, bound_radius,
        // bounds, center_x, center_y, center_z
        let mut fields: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut in_quote = false;
        for ch in rest.chars() {
            match ch {
                '\'' => in_quote = !in_quote,
                ',' if !in_quote => {
                    fields.push(std::mem::take(&mut cur));
                }
                _ => cur.push(ch),
            }
        }
        fields.push(cur);
        if fields.len() < 10 {
            continue;
        }
        let num = |i: usize| fields[i].trim().parse::<f32>().ok();
        let (Some(zone), Some(bt), Some(radius), Some(cx), Some(cy), Some(cz)) =
            (num(0), num(3), num(5), num(7), num(8), num(9))
        else {
            continue;
        };
        if bt as u8 != BOUND_TYPE_RADIAL || radius <= 0.0 {
            continue;
        }
        out.push(RadialArea {
            zone: zone as u16,
            name: fields[2].trim().to_string(),
            center: [cx, cy, cz],
            radius,
        });
    }
    out
}

/// Placed collision triangle centroids paired with their terrain nibble.
fn zone_triangles(root: &DatRoot, zone: u16) -> Option<Vec<([f32; 3], u8)>> {
    let file_id = zone_id_to_mzb_file_id(zone)?;
    let loc = root.resolve(file_id).ok()?;
    let bytes = std::fs::read(loc.path_under(root)).ok()?;
    let chunk = ffxi_dat::chunk::walk(&bytes)
        .flatten()
        .find(|c| c.kind == ffxi_dat::kind::ChunkKind::Mzb as u8)?;
    let body = mzb::decrypt(chunk.data).ok()?;
    let header = mzb::MzbHeader::parse(&body).ok()?;
    let placements = mzb::parse_placements(&body, &header).ok()?;

    let mut cache: BTreeMap<u32, mzb::MzbMesh> = BTreeMap::new();
    let mut out = Vec::new();
    for p in &placements {
        let mesh = match cache.entry(p.geometry_offset) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                match mzb::parse_mesh_at(&body, p.geometry_offset as usize) {
                    Ok(m) => e.insert(m),
                    Err(_) => continue,
                }
            }
        };
        for (tri, info) in mesh.triangles.iter().zip(mesh.tri_info.iter()) {
            let mut c = [0.0f32; 3];
            let mut ok = true;
            for &vi in tri {
                let Some(v) = mesh.vertices.get(vi as usize) else {
                    ok = false;
                    break;
                };
                let w = mzb::apply_placement(&p.transform, v.pos);
                c[0] += w[0] / 3.0;
                c[1] += w[1] / 3.0;
                c[2] += w[2] / 3.0;
            }
            if ok {
                out.push((c, info.terrain));
            }
        }
    }
    Some(out)
}

fn histogram(tris: impl Iterator<Item = u8>) -> BTreeMap<u8, usize> {
    let mut h = BTreeMap::new();
    for m in tris {
        *h.entry(m).or_insert(0) += 1;
    }
    h
}

fn water_pct(h: &BTreeMap<u8, usize>) -> f32 {
    let total: usize = h.values().sum();
    if total == 0 {
        return 0.0;
    }
    let water: usize = h.get(&8).copied().unwrap_or(0) + h.get(&9).copied().unwrap_or(0);
    100.0 * water as f32 / total as f32
}

fn fmt_hist(h: &BTreeMap<u8, usize>) -> String {
    let total: usize = h.values().sum();
    h.iter()
        .map(|(m, n)| {
            format!(
                "{}={:.0}%",
                TERRAIN_NAMES[(*m & 0xF) as usize],
                100.0 * *n as f32 / total.max(1) as f32
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> ExitCode {
    let sql = match std::fs::read_to_string(FISHING_AREA_SQL) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {FISHING_AREA_SQL}: {e}");
            return ExitCode::from(1);
        }
    };
    let areas = parse_areas(&sql);
    let want: Vec<u16> = std::env::args()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();

    let root = match DatRoot::from_env_or_default() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("no FFXI install: {e}");
            return ExitCode::from(1);
        }
    };

    let mut zones: Vec<u16> = areas.iter().map(|a| a.zone).collect();
    zones.sort_unstable();
    zones.dedup();
    if !want.is_empty() {
        zones.retain(|z| want.contains(z));
    }

    for zone in zones {
        let Some(tris) = zone_triangles(&root, zone) else {
            eprintln!("zone {zone}: no collision geometry");
            continue;
        };
        let zone_hist = histogram(tris.iter().map(|(_, m)| *m));
        println!(
            "\nzone {zone}: {} tris, water {:.1}%  [{}]",
            tris.len(),
            water_pct(&zone_hist),
            fmt_hist(&zone_hist)
        );

        for area in areas.iter().filter(|a| a.zone == zone) {
            let r2 = area.radius * area.radius;
            let inside = histogram(tris.iter().filter_map(|(c, m)| {
                let dx = c[0] - area.center[0];
                let dz = c[2] - area.center[2];
                (dx * dx + dz * dz <= r2).then_some(*m)
            }));
            let n: usize = inside.values().sum();
            println!(
                "  r={:<5.0} {:<28} n={:<6} water {:>5.1}%  [{}]",
                area.radius,
                area.name,
                n,
                water_pct(&inside),
                fmt_hist(&inside)
            );
        }
    }
    ExitCode::SUCCESS
}
