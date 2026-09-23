//! Probe: decode event 8700's home-point favorites branch (block offsets
//! ~960..1010) — the 0x24/0x25 pair at 966/973 and the 0x02 IF at 974 — to
//! pin the loop condition that jumps back onto the 0x25.

use ffxi_dat::event_dat::EventDat;

const BASTOK_MARKETS: u16 = 235;
const HOME_POINT_EVENT: u16 = 8700;

fn main() {
    let root = ffxi_dat::archive::open_test_install().expect("test install");
    let loc = root
        .resolve(ffxi_dat::event_locate::event_dat_file_id(BASTOK_MARKETS))
        .expect("event dat");
    let dat = EventDat::parse(&std::fs::read(loc.path_under(&root)).expect("read")).expect("parse");
    for (i, b) in dat.blocks.iter().enumerate() {
        if let Some(entry) = b.event_entry_exact(HOME_POINT_EVENT) {
            println!(
                "block {i}: actor={} data_len={} exact_entry={} wildcard_entry={:?}",
                b.actor,
                b.event_data.len(),
                entry,
                b.event_entry_exact(0xFFFF)
            );
        }
    }
    let block = dat
        .blocks
        .iter()
        .find(|b| b.event_entry_exact(HOME_POINT_EVENT).is_some())
        .expect("a block owns the home point event");
    let data = &block.event_data;
    println!(
        "found block: actor={} data_len={} ref0={:?} refs={:?}",
        block.actor,
        data.len(),
        block.references.first(),
        &block.references[..block.references.len().min(12)]
    );
    // Every 0x2C SCHEDULOR in the block with its raw actor operands.
    for i in 0..data.len().saturating_sub(11) {
        if data[i] == 0x2C {
            let a1 = u32::from_le_bytes([data[i + 1], data[i + 2], data[i + 3], data[i + 4]]);
            let key = &data[i + 5..i + 9];
            let a2 = u32::from_le_bytes([data[i + 9], data[i + 10], data[i + 11], data[i + 12]]);
            println!("0x2C at {i} (0x{i:04X}): actor1=0x{a1:08X} key={:?} actor2=0x{a2:08X}", std::str::from_utf8(key).unwrap_or("?"));
        }
    }
    // Every 0x24 in the block, with its message/default operands.
    for i in 0..data.len() {
        if data[i] == 0x24 {
            let msg = u16::from_le_bytes([data[i + 1], data[i + 2]]);
            let default = u16::from_le_bytes([data[i + 3], data[i + 4]]);
            println!("0x24 at {i} (0x{i:04X}): msg=0x{msg:04X} default=0x{default:04X}");
        }
    }
    // Every word in the block equal to 0x03CD (973), and the opcode that owns it.
    for i in (0..data.len().saturating_sub(1)).step_by(1) {
        let w = u16::from_le_bytes([data[i], data[i + 1]]);
        if w == 0x03CD {
            let owner = data.get(i.saturating_sub(1)).copied();
            let owner6 = data.get(i.saturating_sub(6)).copied();
            println!(
                "word 0x03CD at {i}: prev_byte={owner:02X?} (GOTO/JMP target?) prev6={owner6:02X?} (IF target?)"
            );
        }
    }

    let word = |ofs: usize| {
        u16::from_le_bytes([data.get(ofs).copied().unwrap_or(0), data.get(ofs + 1).copied().unwrap_or(0)])
    };
    // Follow control flow from the favorites 0x25 at 1197.
    let jump_target = |j: usize, op: u8| -> Option<usize> {
        let t = u16::from_le_bytes([data.get(j + 1).copied().unwrap_or(0), data.get(j + 2).copied().unwrap_or(0)]);
        match op {
            0x01 | 0x1A => Some(t as usize),
            0x02 => Some(u16::from_le_bytes([data.get(j + 6).copied().unwrap_or(0), data.get(j + 7).copied().unwrap_or(0)]) as usize),
            _ => None,
        }
    };
    let mut j = 1197usize;
    for _ in 0..40 {
        let Some(&op) = data.get(j) else { break };
        let Some(meta) = ffxi_event::opcode_meta::OPCODE_META.get(op as usize).copied() else {
            println!("flow {j:04X}: 0x{op:02X} not in meta; stop");
            break;
        };
        let extra = match op {
            0x02 => {
                let v1 = u16::from_le_bytes([data[j + 1], data[j + 2]]);
                let v2 = u16::from_le_bytes([data[j + 3], data[j + 4]]);
                let kind = data[j + 5] & 0x0F;
                format!(" IF v1=0x{v1:04X} v2=0x{v2:04X} kind={kind} target={}", jump_target(j, op).unwrap_or(0))
            }
            0x01 | 0x1A => format!(" target={}", jump_target(j, op).unwrap_or(0)),
            0x24 => {
                let msg = u16::from_le_bytes([data[j + 1], data[j + 2]]);
                format!(" QUERY msg=0x{msg:04X}")
            }
            _ => String::new(),
        };
        println!("flow {j:04X}: 0x{op:02X} size={}{}", meta.size, extra);
        if op == 0x00 {
            break;
        }
        j += meta.size as usize;
    }
    let mut j = 966usize;
    while j < 1010 {
        let Some(&op) = data.get(j) else { println!("walk stopped: data len {}", data.len()); break };
        let Some(meta) = ffxi_event::opcode_meta::OPCODE_META.get(op as usize).copied() else {
            println!("{j:04X} (??): 0x{op:02X} not in OPCODE_META");
            break;
        };
        let extra = match op {
            0x02 => {
                let v1 = word(j + 1);
                let v2 = word(j + 3);
                let kind = data[j + 5] & 0x0F;
                let target = word(j + 6);
                format!(
                    " IF v1=0x{v1:04X} v2=0x{v2:04X} kind={kind} target={target}"
                )
            }
            0x01 => format!(" GOTO target={}", word(j + 1)),
            0x1A => format!(" JMP target={}", word(j + 1)),
            0x24 => format!(
                " QUERY msg=0x{:04X} default=0x{:04X}",
                word(j + 1),
                word(j + 3)
            ),
            _ => String::new(),
        };
        println!("{j:04X} ({}): 0x{op:02X} size={} jumps={} valid={}{}", j, meta.size, meta.jumps, meta.valid, extra);
        if !meta.valid || meta.size == 0 {
            break;
        }
        j += meta.size as usize;
    }
}
