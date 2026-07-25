// Reports, per MMB model in a zone DAT, the brightest per-vertex colour channel.
// Zone vertex colours are baked lighting stored /128, so a peak above 1.0 marks
// geometry the retail artists lit as an emitter (lantern glass, flames). This is
// the channel zone_lights.rs::cluster_overbright mines to place glow emitters, so
// the peaks here bound what any overbright-driven lamp glow can detect.
use std::env;
use std::fs;
use std::process::ExitCode;

use ffxi_dat::mmb::{self, MmbHeader};
use ffxi_dat::{walk, DatRoot};

// dat_mmb.rs decodes vertex colour bytes as raw/128.
const VERTEX_COLOR_SCALE: f32 = 128.0;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: FFXI_DAT_PATH=... {} <file_id> [name_filter]",
            args[0]
        );
        return ExitCode::from(2);
    }
    let file_id: u32 = args[1].parse().unwrap();
    let filter = args.get(2).map(|s| s.to_ascii_lowercase());

    let root = DatRoot::from_env_or_default().unwrap();
    let location = root.resolve(file_id).unwrap();
    let bytes = fs::read(location.path_under(root.root())).unwrap();

    let mut overbright_models = 0usize;
    let mut scanned = 0usize;

    for (idx, c) in walk(&bytes).filter_map(Result::ok).enumerate() {
        if c.kind != 0x2E {
            continue;
        }
        let name = String::from_utf8_lossy(&c.name)
            .trim_end_matches(['\0', ' '])
            .to_ascii_lowercase();
        if let Some(f) = &filter {
            if !name.contains(f.as_str()) {
                continue;
            }
        }
        let Ok(decrypted) = mmb::decrypt(c.data) else {
            continue;
        };
        if MmbHeader::parse(&decrypted).is_err() {
            continue;
        }
        let models = mmb::parse_models(&decrypted);
        scanned += 1;

        let mut peak = 0.0f32;
        let mut over_verts = 0usize;
        let mut total_verts = 0usize;
        for m in &models {
            for v in &m.vertices {
                let raw = v.rgba[0].max(v.rgba[1]).max(v.rgba[2]);
                let p = raw as f32 / VERTEX_COLOR_SCALE;
                peak = peak.max(p);
                total_verts += 1;
                if p > 1.0 {
                    over_verts += 1;
                }
            }
        }
        if total_verts == 0 {
            continue;
        }
        if over_verts > 0 {
            overbright_models += 1;
        }
        println!(
            "[{idx:>4}] {name:<12} peak={peak:.3} (raw {:.0}) over1.0={over_verts}/{total_verts} subs={}",
            peak * VERTEX_COLOR_SCALE,
            models.len(),
        );
    }

    println!("scanned {scanned} model(s), {overbright_models} with any vertex over 1.0");
    ExitCode::SUCCESS
}
