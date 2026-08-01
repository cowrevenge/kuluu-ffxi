use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use ffxi_dat::mzb::{self, MmbRenderType, UNDERSCORE_AT_GROUP_MAX_SUBCHUNKS};
use ffxi_dat::{walk, ChunkKind, DatRoot};

const ASCII_PRINTABLE: std::ops::Range<u8> = 0x20..0x7F;

fn fourcc(v: u32) -> String {
    let printable: String = v
        .to_le_bytes()
        .iter()
        .map(|&c| {
            if ASCII_PRINTABLE.contains(&c) {
                c as char
            } else {
                '.'
            }
        })
        .collect();
    format!("0x{v:08X} '{printable}'")
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: FFXI_DAT_PATH=... [MZB_RT_VERBOSE=1] {} <file_id> [file_id...]",
            args.first()
                .map(String::as_str)
                .unwrap_or("dat-mzb-rendertype")
        );
        return ExitCode::from(2);
    }
    let verbose = env::var_os("MZB_RT_VERBOSE").is_some();
    let root = match DatRoot::from_env() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("DatRoot::from_env failed: {e}");
            return ExitCode::from(2);
        }
    };

    for arg in &args[1..] {
        let Ok(file_id) = arg.parse::<u32>() else {
            eprintln!("bad file_id: {arg}");
            return ExitCode::from(2);
        };
        let Ok(location) = root.resolve(file_id) else {
            eprintln!("{file_id}: unresolved");
            continue;
        };
        let path = location.path_under(root.root());
        let Ok(bytes) = fs::read(&path) else {
            eprintln!("{file_id}: read {} failed", path.display());
            continue;
        };
        let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();
        let Some(chunk) = chunks.iter().find(|c| c.kind == ChunkKind::Mzb as u8) else {
            eprintln!("{file_id}: no MZB chunk");
            continue;
        };
        let Ok(plain) = mzb::decrypt(chunk.data) else {
            eprintln!("{file_id}: decrypt failed");
            continue;
        };
        let Ok(header) = mzb::MzbHeader::parse(&plain) else {
            eprintln!("{file_id}: header parse failed");
            continue;
        };
        let placements = match mzb::parse_mmb_placements(&plain, &header) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{file_id}: parse_mmb_placements failed: {e}");
                continue;
            }
        };
        let drawn = mzb::drawn_placements(&placements, None);

        let mut by_type: BTreeMap<u8, usize> = BTreeMap::new();
        let mut groups: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        let mut sub_links: BTreeMap<u32, usize> = BTreeMap::new();
        for p in &placements {
            *by_type
                .entry(MmbRenderType::classify(p, None) as u8)
                .or_default() += 1;
            if p.block_id != 0 {
                groups
                    .entry(p.block_id)
                    .or_default()
                    .push(p.id_str().trim_end().to_string());
            }
            if p.sub_area_link != 0 {
                *sub_links.entry(p.sub_area_link).or_default() += 1;
            }
        }

        println!(
            "file_id {file_id}  ({})  version={} placements={}",
            path.display(),
            header.version,
            placements.len()
        );
        for (rt, n) in &by_type {
            println!("  RenderType {rt}: {n}");
        }
        println!(
            "  hidden-by-gate: {}",
            drawn.iter().filter(|d| !**d).count()
        );
        for (id, names) in &groups {
            let over = names.len() > UNDERSCORE_AT_GROUP_MAX_SUBCHUNKS;
            if verbose || over {
                let tag = if over { "OVER-CAP " } else { "" };
                println!(
                    "  {tag}block_id {}  x{}  {names:?}",
                    fourcc(*id),
                    names.len()
                );
            }
        }
        for (id, n) in &sub_links {
            println!("  sub_area_link 0x{id:X}  x{n}");
        }
        if verbose {
            for ((i, p), d) in placements.iter().enumerate().zip(&drawn) {
                println!(
                    "  [{i:3}] rt={:?} drawn={d} {:<16} t=({:>9.2},{:>9.2},{:>9.2}) block={} area={} sub=0x{:X}",
                    MmbRenderType::classify(p, None),
                    p.id_str().trim_end(),
                    p.trans[0],
                    p.trans[1],
                    p.trans[2],
                    fourcc(p.block_id),
                    fourcc(p.area_resource_id),
                    p.sub_area_link,
                );
            }
        }
        println!();
    }

    ExitCode::SUCCESS
}
