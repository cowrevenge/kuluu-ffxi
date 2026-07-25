//! Dump the MZB mesh-table header words and the placement grid pointer table,
//! to diagnose zones whose `parse_placements` yields nothing.

use ffxi_dat::{mzb, walk, ChunkKind, DatRoot};
use std::fs;

fn main() {
    let file_id: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("usage: dat-mzb-gridprobe <file_id>");

    let root = DatRoot::from_env_or_default().expect("DatRoot");
    let loc = root.resolve(file_id).expect("resolve");
    let bytes = fs::read(loc.path_under(root.root())).expect("read");
    let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();
    let chunk = chunks
        .iter()
        .find(|c| c.kind == ChunkKind::Mzb as u8)
        .expect("mzb chunk");
    let plain = mzb::decrypt(chunk.data).expect("decrypt");
    let header = mzb::MzbHeader::parse(&plain).expect("header");

    let mt = header.mesh_table_offset as usize;
    println!(
        "file {file_id}: body={} mesh_table=0x{mt:X} grid={}x{}",
        plain.len(),
        header.grid_cells_x(),
        header.grid_cells_z()
    );
    println!(
        "header[0x0C..0x10] = zoneBlocksX={} zoneBlocksZ={} blockWidth={} blockLength={}",
        plain[0x0C], plain[0x0D], plain[0x0E], plain[0x0F]
    );
    print!("mesh_table words:");
    for k in 0..8 {
        let o = mt + k * 4;
        let v = u32::from_le_bytes([plain[o], plain[o + 1], plain[o + 2], plain[o + 3]]);
        print!(" [0x{:02X}]=0x{v:X}", k * 4);
    }
    println!();

    let grid_offset = u32::from_le_bytes([
        plain[mt + 0x10],
        plain[mt + 0x11],
        plain[mt + 0x12],
        plain[mt + 0x13],
    ]) as usize;
    println!(
        "grid_offset=0x{grid_offset:X} (body len 0x{:X})",
        plain.len()
    );
    if grid_offset == 0 || grid_offset >= plain.len() {
        println!("  -> OUT OF RANGE / zero: parse_placements returns empty");
        return;
    }

    let gw = header.grid_cells_x();
    let gh = header.grid_cells_z();
    let mut nonzero = 0usize;
    let mut oob = 0usize;
    let mut first = Vec::new();
    for y in 0..gh {
        for x in 0..gw {
            let off = grid_offset + (y * gw + x) * 4;
            if off + 4 > plain.len() {
                continue;
            }
            let v = u32::from_le_bytes([plain[off], plain[off + 1], plain[off + 2], plain[off + 3]])
                as usize;
            if v == 0 {
                continue;
            }
            nonzero += 1;
            if v >= plain.len() {
                oob += 1;
            }
            if first.len() < 8 {
                first.push((x, y, v));
            }
        }
    }
    println!(
        "grid {gw}x{gh}: nonzero cells={nonzero} (of {}) out-of-range={oob}",
        gw * gh
    );
    for (x, y, v) in first {
        println!("  cell({x},{y}) -> 0x{v:X}");
    }

    // Where in the body do pointers into the cell-entry region actually live?
    let entry_lo = u32::from_le_bytes([
        plain[mt + 0x0C],
        plain[mt + 0x0D],
        plain[mt + 0x0E],
        plain[mt + 0x0F],
    ]) as usize;
    let entry_hi = grid_offset;
    println!("scanning body for u32 in [0x{entry_lo:X}, 0x{entry_hi:X}) ...");
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut cur: Option<(usize, usize)> = None;
    let mut o = 0usize;
    while o + 4 <= plain.len() {
        let v = u32::from_le_bytes([plain[o], plain[o + 1], plain[o + 2], plain[o + 3]]) as usize;
        let hit = v >= entry_lo && v < entry_hi;
        match (&mut cur, hit) {
            (None, true) => cur = Some((o, 1)),
            (Some(c), true) => c.1 += 1,
            (Some(c), false) => {
                if c.1 >= 8 {
                    runs.push(*c);
                }
                cur = None;
            }
            (None, false) => {}
        }
        o += 4;
    }
    if let Some(c) = cur {
        if c.1 >= 8 {
            runs.push(c);
        }
    }
    println!("pointer runs (>=8 consecutive): {}", runs.len());
    for (start, count) in runs.iter().take(10) {
        println!("  run @0x{start:X} count={count}");
    }
}
