use std::path::PathBuf;
use std::process::ExitCode;

use ffxi_dat::{chunk::walk, kind::ChunkKind, particle_gen::ParticleGeneratorDef};

fn collect_dats(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_dats(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dat")) {
            out.push(p);
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: dat-scan-generators <file_or_dir>");
        return ExitCode::from(1);
    }
    let target = PathBuf::from(&args[0]);
    let mut files = Vec::new();
    if target.is_file() {
        files.push(target);
    } else {
        collect_dats(&target, &mut files);
    }

    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut auto = 0usize;
        let mut total = 0usize;
        let mut samples: Vec<String> = Vec::new();
        for c in walk(&bytes).flatten() {
            if ChunkKind::from_u8(c.kind) != Some(ChunkKind::Generator) {
                continue;
            }
            if let Ok(Some(def)) = ParticleGeneratorDef::parse(c.data) {
                total += 1;
                if def.auto_run {
                    auto += 1;
                    if samples.len() < 6 {
                        samples.push(format!(
                            "{}[mesh={} life={} blend={:?}]",
                            String::from_utf8_lossy(&c.name).trim_end(),
                            String::from_utf8_lossy(&def.mesh_id).trim_end(),
                            def.max_life_frames,
                            def.blend,
                        ));
                    }
                }
            }
        }
        if auto > 0 {
            println!(
                "{}: {auto}/{total} auto-run gen  {}",
                path.display(),
                samples.join(" ")
            );
        }
    }
    ExitCode::SUCCESS
}
