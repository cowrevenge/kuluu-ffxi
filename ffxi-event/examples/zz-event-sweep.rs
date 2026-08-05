//! Runs every event in every zone through the VM and reports what stops it, so a
//! change to opcode coverage can be measured against the whole retail corpus
//! rather than the one cutscene that prompted it.
//!
//! `cargo run -p ffxi-event --example zz-event-sweep [-- <zone>...]`

use std::collections::BTreeMap;

use ffxi_dat::event_dat::EventDat;
use ffxi_dat::DatRoot;
use ffxi_event::{EventVm, StepResult};

/// Enough to walk any authored event to its end; a bytecode loop would run
/// forever otherwise.
const STEP_LIMIT: usize = 4096;

fn main() {
    let root = DatRoot::from_env_or_default().expect("DatRoot");
    let zones: Vec<u16> = {
        let named: Vec<u16> = std::env::args()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if named.is_empty() {
            (0..=299).collect()
        } else {
            named
        }
    };

    let mut ended = 0usize;
    let mut cancelled = 0usize;
    let mut ran_out = 0usize;
    let mut past_end = 0usize;
    let mut stops: BTreeMap<u8, usize> = BTreeMap::new();

    for zone in zones {
        let Some(loc) = ffxi_dat::event_locate::zone_id_to_event_location(zone) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
            continue;
        };
        let Ok(dat) = EventDat::parse(&bytes) else {
            continue;
        };

        for block in &dat.blocks {
            for &event_id in &block.event_ids {
                let Some(mut vm) = EventVm::start(block, event_id, 0, vec![0; 8]) else {
                    continue;
                };
                let mut steps = 0;
                loop {
                    steps += 1;
                    if steps > STEP_LIMIT {
                        ran_out += 1;
                        break;
                    }
                    match vm.step() {
                        StepResult::AwaitMessage(_) | StepResult::AwaitMessageAck => {
                            vm.dismiss_message()
                        }
                        StepResult::AwaitChoice(_) => vm.select_choice(Some(0)),
                        StepResult::Done => {
                            ended += 1;
                            if vm.ran_past_end() {
                                past_end += 1;
                            }
                            break;
                        }
                        StepResult::Cancelled => {
                            cancelled += 1;
                            break;
                        }
                        StepResult::Unimplemented(op) => {
                            *stops.entry(op).or_default() += 1;
                            break;
                        }
                    }
                }
            }
        }
    }

    let stopped: usize = stops.values().sum();
    println!(
        "ended {ended} (ran past end {past_end})  cancelled {cancelled}  \
         stopped {stopped}  step-limited {ran_out}"
    );
    let mut by_count: Vec<(u8, usize)> = stops.into_iter().collect();
    by_count.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    for (op, n) in by_count.iter().take(20) {
        println!("  0x{op:02X}  {n}");
    }
}
