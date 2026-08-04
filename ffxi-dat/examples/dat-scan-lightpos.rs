use ffxi_dat::{chunk::walk, generator::Generator, kind::ChunkKind, DatRoot};
fn main() {
    let root = DatRoot::from_env_or_default().expect("root");
    for id in std::env::args()
        .skip(1)
        .filter_map(|a| a.parse::<u32>().ok())
    {
        let Ok(loc) = root.resolve(id) else { continue };
        let path = loc.path_under(&root);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut ys = Vec::new();
        for c in walk(&bytes).flatten() {
            if ChunkKind::from_u8(c.kind) != Some(ChunkKind::Generator) {
                continue;
            }
            if let Ok(Some(pl)) = Generator::parse_point_light(c.data) {
                if pl.range <= 0.0 {
                    continue;
                }
                let bp = pl.base_position;
                // mzb_to_bevy: (x,y,z)->(x,-y,-z); bevy Y = -bp[1]
                ys.push((
                    String::from_utf8_lossy(&c.name).trim_end().to_string(),
                    bp,
                    -bp[1],
                    pl.range,
                ));
            }
        }
        println!("{id} ({path:?}): {} point lights", ys.len());
        for (n, bp, bevy_y, r) in ys.iter().take(20) {
            println!(
                "  {n}: dat=({:.1},{:.1},{:.1}) bevyY={:.1} range={:.1}",
                bp[0], bp[1], bp[2], bevy_y, r
            );
        }
    }
}
