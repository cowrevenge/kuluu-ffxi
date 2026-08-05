//! Drives one zone event through the VM offline and reports where it stops, so a
//! cutscene that auto-releases in game can be diagnosed without a live session.
//!
//! `cargo run -p ffxi-event --example zz-event-drive -- <zone> <event id> [params...]`

use ffxi_dat::dmsg::StringDat;
use ffxi_dat::event_dat::EventDat;
use ffxi_dat::DatRoot;
use ffxi_event::{DialogRunner, DialogStep};

fn main() {
    let mut args = std::env::args().skip(1);
    let zone: u16 = args.next().and_then(|s| s.parse().ok()).expect("zone id");
    let event_id: u16 = args.next().and_then(|s| s.parse().ok()).expect("event id");
    let params: Vec<i32> = args.filter_map(|s| s.parse().ok()).collect();

    let root = DatRoot::from_env_or_default().expect("DatRoot");

    let loc = ffxi_dat::event_locate::zone_id_to_event_location(zone).expect("event DAT mapping");
    let bytes = std::fs::read(loc.path_under(&root)).expect("read event DAT");
    let dat = EventDat::parse(&bytes).expect("parse event DAT");

    let file_id = ffxi_dat::zone_dat::zone_id_to_string_file_id(zone).expect("string DAT mapping");
    let sloc = root.resolve(file_id).expect("resolve string DAT");
    let sbytes = std::fs::read(sloc.path_under(&root)).expect("read string DAT");
    let strings = StringDat::parse(&sbytes).expect("parse string DAT");

    let owners: Vec<&ffxi_dat::event_dat::EventBlock> = dat
        .blocks
        .iter()
        .filter(|b| b.event_entry_exact(event_id).is_some())
        .collect();
    println!(
        "zone {zone} event {event_id}: {} of {} blocks own it {:?}",
        owners.len(),
        dat.blocks.len(),
        owners
            .iter()
            .map(|b| format!("0x{:08X}", b.actor))
            .collect::<Vec<_>>()
    );

    for block in owners {
        println!("--- block 0x{:08X} ---", block.actor);
        let Some(mut runner) = DialogRunner::start(block, event_id, 0, params.clone()) else {
            println!("  no entry");
            continue;
        };
        let mut response = None;
        for step in 0..64 {
            match runner.advance(response.take(), &strings) {
                DialogStep::Frame(f) => {
                    println!(
                        "  {step:2}. frame speaker={:?} choices={} text={:?}",
                        f.speaker_index,
                        f.choices.len(),
                        f.text
                    );
                    if !f.choices.is_empty() {
                        response = Some(0);
                    }
                }
                DialogStep::Ended { end_para } => {
                    println!("  {step:2}. ended end_para={end_para}");
                    break;
                }
                DialogStep::Stopped(op) => {
                    println!("  {step:2}. STOPPED on opcode 0x{op:02X}");
                    break;
                }
            }
        }
    }
}
