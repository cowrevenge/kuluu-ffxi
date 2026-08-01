use ffxi_dat::{chunk::walk, generator::Generator, kind::ChunkKind, mzb, DatRoot};
use std::collections::{HashMap, HashSet};

fn light_name(id: mzb::LightId) -> String {
    String::from_utf8_lossy(&id.to_le_bytes())
        .trim_end_matches('\0')
        .to_string()
}

fn main() {
    let root = DatRoot::from_env_or_default().expect("root");
    let ids: Vec<u32> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();

    for id in ids {
        let Ok(loc) = root.resolve(id) else { continue };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            continue;
        };
        let mut generators: HashMap<String, f32> = HashMap::new();
        for c in walk(&bytes).flatten() {
            if ChunkKind::from_u8(c.kind) != Some(ChunkKind::Generator) {
                continue;
            }
            if let Ok(Some(pl)) = Generator::parse_point_light(c.data) {
                generators.insert(
                    String::from_utf8_lossy(&c.name)
                        .trim_end_matches('\0')
                        .into(),
                    pl.range,
                );
            }
        }
        let Some(mzb_chunk) = walk(&bytes)
            .flatten()
            .find(|c| c.kind == ChunkKind::Mzb as u8)
        else {
            continue;
        };
        let plain = mzb::decrypt(mzb_chunk.data).expect("decrypt");
        let header = mzb::MzbHeader::parse(&plain).expect("header");
        let placements = mzb::parse_mmb_placements(&plain, &header).unwrap_or_default();
        let bindings = mzb::parse_light_bindings(&plain, &header);

        let mut bound_chunks = 0usize;
        let mut dark_refs = 0usize;
        let mut bound: HashSet<String> = HashSet::new();
        for p in &placements {
            let slots = mzb::resolve_chunk_lights(&p.light_references, &bindings);
            if slots.iter().any(Option::is_some) {
                bound_chunks += 1;
            }
            for (slot, light) in slots.iter().enumerate() {
                match light {
                    Some(lid) => {
                        bound.insert(light_name(*lid));
                    }
                    None if p.light_references[slot] != 0 => dark_refs += 1,
                    None => {}
                }
            }
        }
        let authored: HashSet<String> = bindings.iter().copied().map(light_name).collect();
        let no_generator: Vec<&String> = authored
            .iter()
            .filter(|n| !generators.contains_key(*n))
            .collect();
        let unbound: Vec<&String> = generators
            .keys()
            .filter(|n| !authored.contains(*n))
            .collect();
        println!(
            "dat {id} v{} table={} placements={} bound={bound_chunks} darkRefs={dark_refs} \
             generators={} boundLights={}",
            header.version,
            bindings.len(),
            placements.len(),
            generators.len(),
            bound.len(),
        );
        println!("   authored with no point-light generator: {no_generator:?}");
        println!("   point-light generators no chunk binds:  {unbound:?}");
    }
}
