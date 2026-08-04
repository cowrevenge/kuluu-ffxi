use std::env;
use std::fs;
use std::process::ExitCode;

use ffxi_dat::{mzb, walk, ChunkKind, DatRoot};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: FFXI_DAT_PATH=... {} <file_id>", args[0]);
        return ExitCode::from(2);
    }
    let file_id: u32 = args[1].parse().unwrap();
    let root = DatRoot::from_env().unwrap();
    let location = root.resolve(file_id).unwrap();
    let bytes = fs::read(location.path_under(&root)).unwrap();
    let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();

    let (chunk_idx, mzb_chunk) = chunks
        .iter()
        .enumerate()
        .find(|(_, c)| c.kind == ChunkKind::Mzb as u8)
        .expect("no MZB chunk");
    let plain = mzb::decrypt(mzb_chunk.data).unwrap();
    let header = mzb::MzbHeader::parse(&plain).unwrap();

    println!(
        "DAT {file_id} MZB at chunk[{chunk_idx}] body_len={}",
        plain.len()
    );
    println!("  decode_length      {}", header.decode_length);
    println!("  node_count         {}", header.node_count);
    println!("  version            {}", header.version);
    println!("  key_index          {}", header.key_index);
    println!("  zone_blocks_x      {}", header.zone_blocks_x);
    println!("  zone_blocks_z      {}", header.zone_blocks_z);
    println!("  block_width        {}", header.block_width);
    println!("  block_length       {}", header.block_length);
    println!(
        "  grid cells         {}x{}",
        header.grid_cells_x(),
        header.grid_cells_z()
    );
    println!(
        "  collision_data_off 0x{:08x}",
        header.collision_data_offset
    );
    println!("  quadtree_offset    {:?}", header.quadtree_offset());
    println!("  group_list_count   {:?}", header.group_list_count());
    println!("  group_list_offset  0x{:08x}", header.group_list_offset);
    println!("  lighting_offset    0x{:08x}", header.lighting_offset);
    println!("  substructure_type  {}", header.substructure_type);
    println!("  collision_flags    0x{:02x}", header.collision_flags);
    println!();
    let placements_size = (header.node_count as usize) * mzb::PLACEMENT_RECORD_LEN;
    let placements_end = mzb::MZB_HEADER_LEN + placements_size;
    println!(
        "placement table: 0x{:x}..0x{:x}  ({} bytes, stride {})",
        mzb::MZB_HEADER_LEN,
        placements_end,
        placements_size,
        mzb::PLACEMENT_RECORD_LEN
    );
    println!(
        "byte gap to collision data: {} bytes",
        (header.collision_data_offset as i64) - (placements_end as i64)
    );
    if let Some(quadtree) = header.quadtree_offset() {
        println!(
            "byte gap to quadtree:   {} bytes",
            (quadtree as i64) - (placements_end as i64)
        );
    }
    println!(
        "byte gap to group list: {} bytes",
        (header.group_list_offset as i64) - (placements_end as i64)
    );
    println!();

    if placements_end < plain.len() {
        let peek_len = 256.min(plain.len() - placements_end);
        println!(
            "first {peek_len} bytes after placement table (offset 0x{:x}):",
            placements_end
        );
        for chunk in plain[placements_end..placements_end + peek_len].chunks(16) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let ascii: String = chunk
                .iter()
                .map(|&b| {
                    if b.is_ascii_graphic() || b == b' ' {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            println!("  {}  |{}|", hex.join(" "), ascii);
        }
    }

    ExitCode::SUCCESS
}
