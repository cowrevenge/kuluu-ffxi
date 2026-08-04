use ffxi_dat::{chunk::walk, generator::Generator, kind::ChunkKind, mzb, DatRoot};

fn main() {
    let root = DatRoot::from_env_or_default().expect("root");
    let id: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .expect("file id");
    let loc = root.resolve(id).expect("resolve");
    let bytes = std::fs::read(loc.path_under(&root)).expect("read");

    // point lights (DAT model space: x,y,z; vertical = y)
    let mut lights: Vec<[f32; 3]> = Vec::new();
    for c in walk(&bytes).flatten() {
        if ChunkKind::from_u8(c.kind) != Some(ChunkKind::Generator) {
            continue;
        }
        if let Ok(Some(pl)) = Generator::parse_point_light(c.data) {
            if pl.range > 0.0 {
                lights.push(pl.base_position);
            }
        }
    }
    // mmb placements
    let mzb_chunk = walk(&bytes)
        .flatten()
        .find(|c| c.kind == ChunkKind::Mzb as u8)
        .expect("mzb chunk");
    let plain = mzb::decrypt(mzb_chunk.data).expect("decrypt");
    let header = mzb::MzbHeader::parse(&plain).expect("header");
    let placements = mzb::parse_mmb_placements(&plain, &header).expect("placements");
    println!(
        "{} lights, {} mmb placements",
        lights.len(),
        placements.len()
    );

    for (li, l) in lights.iter().enumerate().take(8) {
        // nearest placements by XZ distance
        let mut near: Vec<(f32, &mzb::MmbPlacement)> = placements
            .iter()
            .map(|p| {
                let dx = p.trans[0] - l[0];
                let dz = p.trans[2] - l[2];
                ((dx * dx + dz * dz).sqrt(), p)
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        println!("light[{li}] pos=({:.1},{:.1},{:.1}):", l[0], l[1], l[2]);
        for (d, p) in near.iter().take(4) {
            println!(
                "   {:>8} xzdist={:.2} yDelta={:+.2} at=({:.1},{:.1},{:.1})",
                p.id_str(),
                d,
                p.trans[1] - l[1],
                p.trans[0],
                p.trans[1],
                p.trans[2]
            );
        }
    }
}
