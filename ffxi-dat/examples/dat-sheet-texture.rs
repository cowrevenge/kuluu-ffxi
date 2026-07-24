use std::process::ExitCode;

use ffxi_dat::{chunk::walk, kind::ChunkKind, sprite_sheet::ParticleSpriteSheet, DatRoot};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: FFXI_DAT_PATH=<path> dat-sheet-texture <file_id>...");
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
        for c in walk(&bytes).flatten() {
            let name = String::from_utf8_lossy(&c.name).trim_end().to_string();
            match ChunkKind::from_u8(c.kind) {
                Some(ChunkKind::SpriteSheet) => {
                    if let Some(ss) = ParticleSpriteSheet::parse(c.data) {
                        println!(
                            "  sheet {name:<6} frames={:<3} category={:?} id={:?}",
                            ss.frames.len(),
                            ss.category,
                            ss.id
                        );
                    }
                }
                Some(ChunkKind::Img) => {
                    println!(
                        "  img   {name:<6} internal_name={:?}",
                        ffxi_dat::texture::extract_texture_name(c.data)
                    );
                }
                _ => {}
            }
        }
    }
    ExitCode::SUCCESS
}
