//! Survey every mapped zone's MZB zone-block size and placement count, to find
//! zones whose placement grid does not use the common 10-sub-blocks stride.

use ffxi_dat::{mzb, walk, zone_dat, ChunkKind, DatRoot};
use std::fs;

fn main() {
    let root = DatRoot::from_env_or_default().expect("DatRoot");
    let mut by_block: std::collections::BTreeMap<(u8, u8), Vec<u16>> = Default::default();

    for zone_id in 0u16..=299 {
        let Some(file_id) = zone_dat::effective_zone_dat_file_id(Some(zone_id), None) else {
            continue;
        };
        let Ok(loc) = root.resolve(file_id) else {
            continue;
        };
        let Ok(bytes) = fs::read(loc.path_under(&root)) else {
            continue;
        };
        let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();
        let Some(chunk) = chunks.iter().find(|c| c.kind == ChunkKind::Mzb as u8) else {
            continue;
        };
        let Ok(plain) = mzb::decrypt(chunk.data) else {
            continue;
        };
        let Ok(header) = mzb::MzbHeader::parse(&plain) else {
            continue;
        };
        let n = mzb::parse_placements(&plain, &header)
            .map(|p| p.len())
            .unwrap_or(0);
        // What the old hardcoded "10 sub-blocks per block" stride would have found.
        let mut legacy_header = header;
        legacy_header.block_width = 40;
        legacy_header.block_length = 40;
        let legacy = mzb::parse_placements(&plain, &legacy_header)
            .map(|p| p.len())
            .unwrap_or(0);
        by_block
            .entry((header.block_width, header.block_length))
            .or_default()
            .push(zone_id);
        if header.block_width != 40 || header.block_length != 40 || n == 0 {
            println!(
                "zone {zone_id:3} file {file_id:4}: blocks={}x{} block_size={}x{} cells={}x{} placements={n}",
                header.zone_blocks_x,
                header.zone_blocks_z,
                header.block_width,
                header.block_length,
                header.grid_cells_x(),
                header.grid_cells_z(),
            );
            if legacy != n {
                println!("      legacy(x10) stride would have found {legacy}");
            }
        }
    }

    println!("\nblock-size histogram:");
    for ((w, l), zones) in &by_block {
        println!("  {w}x{l}: {} zones {:?}", zones.len(), zones);
    }
}
