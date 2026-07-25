use std::process::ExitCode;

use ffxi_dat::{chunk::walk, kind::ChunkKind, DatRoot};

// Trace the StaticMesh particle chain: generator mesh_id -> D3M chunk -> its 16-byte qualified
// texture name -> the Img chunk that should back it, reporting how each key lines up.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: FFXI_DAT_PATH=<path> dat-d3m-texture <file_id>...");
        return ExitCode::from(2);
    }
    let root = match DatRoot::from_env_or_default() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("could not open DAT root: {e}");
            return ExitCode::from(1);
        }
    };
    for arg in &args {
        let Ok(file_id) = arg.parse::<u32>() else {
            continue;
        };
        let Ok(loc) = root.resolve(file_id) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            continue;
        };
        println!("== file {file_id}");
        let mut img_dat_ids: Vec<[u8; 4]> = Vec::new();
        let mut img_names: Vec<String> = Vec::new();
        for c in walk(&bytes).flatten() {
            if ChunkKind::from_u8(c.kind) == Some(ChunkKind::Img) {
                img_dat_ids.push(c.name);
                if let Some(n) = ffxi_dat::texture::extract_texture_name(c.data) {
                    img_names.push(n);
                }
            }
        }
        println!(
            "  img DatIds : {:?}",
            img_dat_ids
                .iter()
                .map(|n| String::from_utf8_lossy(n).trim_end().to_string())
                .collect::<Vec<_>>()
        );
        println!("  img names  : {img_names:?}");
        for c in walk(&bytes).flatten() {
            if ChunkKind::from_u8(c.kind) != Some(ChunkKind::D3m) {
                continue;
            }
            let Ok(d3m) = ffxi_dat::d3m::D3m::parse(c.name, c.data) else {
                println!(
                    "  d3m {:<6} PARSE FAILED",
                    String::from_utf8_lossy(&c.name).trim_end()
                );
                continue;
            };
            let full = String::from_utf8_lossy(&d3m.texture_name).to_string();
            let local: [u8; 4] = d3m.texture_name[8..12].try_into().unwrap_or([0; 4]);
            let local_s = String::from_utf8_lossy(&local).trim_end().to_string();
            println!(
                "  d3m {:<6} verts={:<4} texture_name={full:?} local[8..12]={local_s:?} \
                 hits_img_datid={} hits_img_name={}",
                String::from_utf8_lossy(&c.name).trim_end(),
                d3m.vertices.len(),
                img_dat_ids.contains(&local),
                img_names.iter().any(|n| n.as_bytes() == local_s.as_bytes()),
            );
        }
    }
    ExitCode::SUCCESS
}
