use std::process::ExitCode;

use ffxi_dat::chunk::walk;
use ffxi_dat::generator::Generator;
use ffxi_dat::kind::ChunkKind;
use ffxi_dat::mmb::{self, MmbHeader};
use ffxi_dat::mzb;
use ffxi_dat::particle_gen::ParticleGeneratorDef;
use ffxi_dat::DatRoot;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file_id) = args.first().and_then(|s| s.parse::<u32>().ok()) else {
        eprintln!("usage: dat-water-probe <file_id>");
        return ExitCode::from(1);
    };
    let root = DatRoot::from_env_or_default().expect("dat root");
    let loc = root.resolve(file_id).expect("resolve");
    let path = loc.path_under(root.root());
    let bytes = std::fs::read(&path).expect("read");
    let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();
    println!(
        "file {file_id} = {} ({} chunks)",
        path.display(),
        chunks.len()
    );

    let mut mmb_names: Vec<String> = Vec::new();
    let mut datids: Vec<String> = Vec::new();
    for c in &chunks {
        if c.kind != ChunkKind::Mmb as u8 {
            continue;
        }
        let id = String::from_utf8_lossy(&c.name)
            .trim_end_matches('\0')
            .trim_end()
            .to_string();
        let name = mmb::decrypt(c.data)
            .ok()
            .and_then(|d| MmbHeader::parse(&d).ok().map(|h| h.zone_mesh_name()))
            .unwrap_or_default();
        datids.push(id);
        mmb_names.push(name);
    }
    println!("\n== MMB chunks: {} ==", mmb_names.len());
    for (id, name) in datids.iter().zip(&mmb_names) {
        let l = name.to_lowercase();
        if l.contains("sea") || l.contains("umi") || l.contains("wat") || l.contains("riv") {
            println!("  watery-name MMB: datid={id:?} zone_mesh_name={name:?}");
        }
    }

    let zone_prefix = mzb::infer_zone_prefix(&mmb_names);
    println!("zone_prefix = {zone_prefix:?}");

    println!("\n== Generator chunks with model-spawn ==");
    for c in &chunks {
        if c.kind != ChunkKind::Generator as u8 {
            continue;
        }
        let gname = String::from_utf8_lossy(&c.name).trim_end().to_string();
        let follows = Generator::parse_cloud_generator(c.name, c.data)
            .ok()
            .flatten()
            .is_some_and(|d| d.follow_camera);
        let life = ParticleGeneratorDef::parse(c.data)
            .ok()
            .flatten()
            .map(|d| d.max_life_frames);
        match Generator::parse_model_spawn(c.data) {
            Ok(Some(ms)) => {
                let name = ms.model_name_str().trim_end().to_string();
                let by_datid = datids.iter().any(|d| d == &name);
                let by_meshname = mzb::resolve_mmb_index(&name, &zone_prefix, &mmb_names).is_some();
                println!(
                    "  gen {gname:?} -> model {name:?} scroll={:?} tint={:?} pos={:?} follow_cam={follows} life={life:?} resolves: datid={by_datid} meshname={by_meshname}",
                    ms.uv_scroll, ms.tint, ms.base_position
                );
            }
            Ok(None) => {}
            Err(e) => println!("  gen {gname:?}: model_spawn parse error: {e}"),
        }
    }

    println!("\n== MZB object placements referencing watery MMB names ==");
    {
        let mzb_chunk = chunks
            .iter()
            .find(|c| c.kind == ChunkKind::Mzb as u8)
            .expect("mzb chunk");
        let plain = mzb::decrypt(mzb_chunk.data).expect("mzb decrypt");
        let header = mzb::MzbHeader::parse(&plain).expect("mzb header");
        let obj = mzb::parse_mmb_placements(&plain, &header).expect("mmb placements");
        println!("  total object placements: {}", obj.len());
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for p in &obj {
            let id = p.id_str().trim_end_matches('\0').trim_end().to_string();
            *counts.entry(id).or_default() += 1;
        }
        for (id, n) in &counts {
            let l = id.to_lowercase();
            if l.contains("sea") || l.contains("umi") || l.contains("wat") || l.contains("riv") {
                println!("  placement id {id:?} x{n}");
            }
        }
        for p in &obj {
            let id = p.id_str().trim_end_matches('\0').trim_end().to_lowercase();
            if id.contains("wat") || id.contains("funsui") || id.contains("sea") {
                println!(
                    "  obj {id:?} trans={:?} rot={:?} scale={:?}",
                    p.trans, p.rot, p.scale
                );
            }
        }
        for name in ["lowsea", "2lowsea", "water", "lows", "2low", "wate"] {
            let hit = counts.contains_key(name);
            println!("  placed({name:?}) = {hit}");
        }
    }

    println!("\n== MZB water_height placements ==");
    let mzb_chunk = chunks
        .iter()
        .find(|c| c.kind == ChunkKind::Mzb as u8)
        .expect("mzb chunk");
    let plain = mzb::decrypt(mzb_chunk.data).expect("mzb decrypt");
    let header = mzb::MzbHeader::parse(&plain).expect("mzb header");
    println!(
        "  has_collision_data={} substructure_type={}",
        header.has_collision_data(),
        header.substructure_type
    );
    if header.has_collision_data() {
        let placements = mzb::parse_placements(&plain, &header).expect("placements");
        let watered: Vec<_> = placements
            .iter()
            .filter(|p| p.water_height.is_some())
            .collect();
        println!(
            "  placements={} with water_height={}",
            placements.len(),
            watered.len()
        );
        let mut heights: Vec<f32> = watered.iter().filter_map(|p| p.water_height).collect();
        heights.sort_by(f32::total_cmp);
        heights.dedup();
        println!("  distinct water heights (ffxi-space): {heights:?}");
    }
    ExitCode::SUCCESS
}
