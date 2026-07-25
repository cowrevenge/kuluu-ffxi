use std::collections::HashMap;
use std::process::ExitCode;

use ffxi_dat::{chunk::walk, kind::ChunkKind, texture, DatRoot};

// Trace the StaticMesh particle chain for one DAT file: generator mesh_id -> D3M chunk -> its
// 16-byte qualified texture name -> the Img chunk that backs it, printing the tiers in the order
// particle_sim::resolve_mesh tries them so the probe and the client cannot disagree.
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

        let mut by_dat_id: HashMap<[u8; 4], String> = HashMap::new();
        let mut by_qualified: HashMap<(String, String), String> = HashMap::new();
        let mut by_local: HashMap<String, String> = HashMap::new();
        let mut d3ms = Vec::new();
        // ChunkWalker does not advance its cursor when a header is truncated, so `.flatten()`
        // spins forever on a short tail (ROM3/0/66.DAT is 4 zero bytes). Stop at the first error.
        for c in walk(&bytes).map_while(Result::ok) {
            match ChunkKind::from_u8(c.kind) {
                Some(ChunkKind::Img) => {
                    if texture::decode_texture(c.data).is_err() {
                        continue;
                    }
                    let dat_id = String::from_utf8_lossy(&c.name).trim_end().to_string();
                    let label = match texture::extract_texture_tokens(c.data) {
                        Some((ns, local)) => {
                            let label = format!("{dat_id}[{ns}/{local}]");
                            by_qualified.insert((ns, local.clone()), label.clone());
                            by_local.insert(local, label.clone());
                            label
                        }
                        None => format!("{dat_id}[unnamed {:#04x}]", c.data[0]),
                    };
                    by_dat_id.insert(c.name, label);
                }
                Some(ChunkKind::D3m) => match ffxi_dat::d3m::D3m::parse(c.name, c.data) {
                    Ok(d) => d3ms.push(d),
                    Err(_) => println!(
                        "  d3m {:<6} PARSE FAILED",
                        String::from_utf8_lossy(&c.name).trim_end()
                    ),
                },
                _ => {}
            }
        }

        let mut labels: Vec<&String> = by_dat_id.values().collect();
        labels.sort_unstable();
        println!("  img chunks : {labels:?}");
        for d in &d3ms {
            let (ns, local) = d.texture_name_tokens();
            let qualified = by_qualified.get(&(ns.clone(), local.clone()));
            let local_hit = by_local.get(&local);
            let dat_id = by_dat_id.get(&d.texture_dat_id());
            let resolved = qualified.or(local_hit).or(dat_id);
            println!(
                "  d3m {:<6} verts={:<4} texture_name={:?} -> {}",
                String::from_utf8_lossy(&d.name).trim_end(),
                d.vertices.len(),
                String::from_utf8_lossy(&d.texture_name),
                resolved.map_or("NO TEXTURE", String::as_str),
            );
            println!(
                "      by qualified {:?} = {:?} | by local {local:?} = {:?} | by DatId {:?} = {:?}",
                (&ns, &local),
                qualified,
                local_hit,
                String::from_utf8_lossy(&d.texture_dat_id()),
                dat_id,
            );
        }
    }
    ExitCode::SUCCESS
}
