// Alpha histogram of the weat/<type>/ sky textures, per the dithered-transparency
// diagnosis: {0, 119, 136} is DXT3 dithered-opaque (0x80 is not representable in a
// 4-bit nibble, so encoders alternate nibble 7/8 to average out at 128), {0, 255} is
// DXT1 1-bit punch-through, and a smooth spread means the dither is not the problem.
//
//   cargo run -p ffxi-dat --example dat-sky-alpha-histogram -- <zone_file_id>...

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use ffxi_dat::texture::{decode_texture, extract_texture_name, ffxi_alpha_remap, TexFormat};
use ffxi_dat::{walk_tree, ChunkKind, ChunkNode, DatRoot};

fn name4(n: &[u8; 4]) -> String {
    n.iter()
        .map(|&c| {
            if (0x20..0x7f).contains(&c) {
                c as char
            } else {
                '.'
            }
        })
        .collect()
}

fn visit(node: &ChunkNode, in_weat: Option<String>) {
    for child in &node.children {
        let tag = name4(&child.chunk.name);
        if child.chunk.kind == ChunkKind::Img as u8 {
            if let Some(weat) = &in_weat {
                report(weat, &tag, child.chunk.data);
            }
            continue;
        }
        let next = if child.chunk.name == *b"weat" {
            Some(String::new())
        } else if in_weat.as_deref() == Some("") {
            Some(tag.clone())
        } else {
            in_weat.clone()
        };
        visit(child, next);
    }
}

fn report(weat_type: &str, chunk_tag: &str, body: &[u8]) {
    let Ok(tex) = decode_texture(body) else {
        return;
    };
    let mut raw: BTreeMap<u8, usize> = BTreeMap::new();
    for px in tex.rgba.chunks_exact(4) {
        *raw.entry(px[3]).or_default() += 1;
    }
    let total = (tex.width as usize) * (tex.height as usize);
    let name = extract_texture_name(body).unwrap_or_default();
    println!(
        "weat/{weat_type:<4} chunk={chunk_tag:<4} name={name:<16} {}x{} {:?} distinct_alpha={}",
        tex.width,
        tex.height,
        tex.format_tag,
        raw.len()
    );
    for (value, count) in &raw {
        let pct = 100.0 * *count as f32 / total.max(1) as f32;
        println!(
            "    raw a={value:>3} (0x{value:02x})  {count:>8}  {pct:>5.1}%   -> remapped {}",
            ffxi_alpha_remap(*value)
        );
    }
    if tex.format_tag == TexFormat::Dxt3 {
        let dithered = raw
            .keys()
            .filter(|a| matches!(**a, 119 | 136))
            .map(|a| raw[a])
            .sum::<usize>();
        if dithered > 0 {
            println!(
                "    DITHERED-OPAQUE BAND: {:.1}% of texels are nibble 7/8 (119/136)",
                100.0 * dithered as f32 / total.max(1) as f32
            );
        }
    }
    if tex.format_tag == TexFormat::Dxt3 {
        let mut sums = [[0u64; 3]; 2];
        let mut counts = [0u64; 2];
        for px in tex.rgba.chunks_exact(4) {
            let bucket = match px[3] {
                119 => 0,
                136 => 1,
                _ => continue,
            };
            counts[bucket] += 1;
            for c in 0..3 {
                sums[bucket][c] += px[c] as u64;
            }
        }
        if counts[0] > 0 && counts[1] > 0 {
            let mean = |b: usize| [0, 1, 2].map(|c| sums[b][c] as f32 / counts[b] as f32);
            let (lo, hi) = (mean(0), mean(1));
            println!(
                "    mean RGB at a=119 {lo:?} vs a=136 {hi:?} (equal => only alpha is dithered)"
            );
            // A global mean split can just mean the two nibbles cluster in different
            // regions of the texture. Compare each nibble-8 texel against its own
            // 4-neighbourhood instead: a nonzero delta here is a true per-texel stipple
            // in the colour channel, which no alpha-side fix can remove.
            let (w, h) = (tex.width as usize, tex.height as usize);
            let luma = |x: usize, y: usize| {
                let p = (y * w + x) * 4;
                (tex.rgba[p] as f32 + tex.rgba[p + 1] as f32 + tex.rgba[p + 2] as f32) / 3.0
            };
            let mut delta_sum = 0.0f64;
            let mut delta_n = 0u64;
            for y in 1..h.saturating_sub(1) {
                for x in 1..w.saturating_sub(1) {
                    if tex.rgba[(y * w + x) * 4 + 3] != 136 {
                        continue;
                    }
                    let neighbours = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)];
                    let lows: Vec<f32> = neighbours
                        .iter()
                        .filter(|(nx, ny)| tex.rgba[(ny * w + nx) * 4 + 3] == 119)
                        .map(|(nx, ny)| luma(*nx, *ny))
                        .collect();
                    if lows.is_empty() {
                        continue;
                    }
                    delta_sum += (luma(x, y) - lows.iter().sum::<f32>() / lows.len() as f32) as f64;
                    delta_n += 1;
                }
            }
            if delta_n > 0 {
                println!(
                    "    per-texel luma delta (a=136 minus its a=119 neighbours): {:+.2} over {delta_n} texels",
                    delta_sum / delta_n as f64
                );
            }
        }
    }
    if env::var_os("SKY_ALPHA_MAP").is_some() {
        let side = 32.min(tex.width).min(tex.height) as usize;
        println!("    alpha map, top-left {side}x{side} ('.'=119 '#'=136 else hex nibble):");
        for y in 0..side {
            let row: String = (0..side)
                .map(|x| {
                    let a = tex.rgba[((y * tex.width as usize) + x) * 4 + 3];
                    match a {
                        119 => '.',
                        136 => '#',
                        _ => char::from_digit((a >> 4) as u32, 16).unwrap_or('?'),
                    }
                })
                .collect();
            println!("      {row}");
        }
    }
}

fn main() -> ExitCode {
    let root = match DatRoot::from_env_or_default() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("dat root: {e}");
            return ExitCode::FAILURE;
        }
    };
    let ids: Vec<u32> = env::args()
        .skip(1)
        .filter_map(|a| a.parse::<u32>().ok())
        .collect();
    if ids.is_empty() {
        eprintln!("usage: dat-sky-alpha-histogram <zone_file_id>...");
        return ExitCode::FAILURE;
    }
    for id in ids {
        let Ok(loc) = root.resolve(id) else {
            eprintln!("file {id}: unresolved");
            continue;
        };
        let Ok(bytes) = fs::read(loc.path_under(&root)) else {
            eprintln!("file {id}: read error");
            continue;
        };
        println!("== zone DAT file {id} ==");
        visit(&walk_tree(&bytes), None);
    }
    ExitCode::SUCCESS
}
