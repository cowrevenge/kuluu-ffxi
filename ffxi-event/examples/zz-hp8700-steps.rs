//! Probe: step-level trace of event 8700's favorites selection. Drives the
//! public EventVm to the favorites menu, selects slot 1, then prints every
//! StepResult until the stream stops changing, to find the cycling state.

use std::io::Write;

use ffxi_dat::dmsg::StringDat;
use ffxi_dat::event_dat::EventDat;
use ffxi_event::vm::EventVm;
use ffxi_event::StepResult;

const BASTOK_MARKETS: u16 = 235;
const HOME_POINT_EVENT: u16 = 8700;

fn main() {
    let log = std::fs::File::create(r"C:\tmp\hp8700_step.txt").expect("log");
    let mut log = std::io::BufWriter::with_capacity(1024, log);
    let mut note = |log: &mut std::io::BufWriter<std::fs::File>, line: &str| {
        let _ = writeln!(log, "{line}");
        let _ = log.flush();
    };

    let root = ffxi_dat::archive::open_test_install().expect("test install");
    let loc = root
        .resolve(ffxi_dat::event_locate::event_dat_file_id(BASTOK_MARKETS))
        .expect("event dat");
    let dat = EventDat::parse(&std::fs::read(loc.path_under(&root)).expect("read")).expect("parse");
    let file_id = ffxi_dat::zone_dat::string_dat_file_id(BASTOK_MARKETS);
    let sloc = root.resolve(file_id).expect("string dat");
    let strings =
        StringDat::parse(&std::fs::read(sloc.path_under(&root)).expect("read")).expect("parse");

    let block = dat
        .blocks
        .iter()
        .find(|b| b.event_entry_exact(HOME_POINT_EVENT).is_some())
        .expect("a block owns the home point event");
    let mut vm = EventVm::start(block, HOME_POINT_EVENT, 0, vec![0; 8]).expect("start");

    // Walk to the favorites menu: info -> main(0) -> travel info -> region(1)
    // -> favorites.
    let mut choices_seen = 0usize;
    for _ in 0..20 {
        let step = vm.step();
        match step {
            StepResult::AwaitMessage(m) => {
                note(&mut log, &format!("AwaitMessage({})", m.message_id));
                vm.dismiss_message();
            }
            StepResult::AwaitMessageAck => {
                note(&mut log, "AwaitMessageAck");
                vm.dismiss_message();
            }
            StepResult::AwaitChoice(c) => {
                choices_seen += 1;
                let raw = strings.text(c.message_id as usize).unwrap_or_default();
                note(&mut log, &format!(
                    "AwaitChoice #{} msg={} default={}: {}",
                    choices_seen,
                    c.message_id,
                    c.default_index,
                    raw.split('\n').take(2).collect::<Vec<_>>().join(" / ")
                ));
                // main menu: pick 0 (travel); region menu: pick 1 (favorites).
                let pick = if choices_seen == 1 { 0u32 } else { 1 };
                vm.select_choice(Some(pick));
            }
            StepResult::AwaitServerAck(t) => {
                note(&mut log, &format!("AwaitServerAck({t:?}); acking"));
                vm.ack_server();
            }
            other => {
                note(&mut log, &format!("reached {other:?} before favorites"));
                return;
            }
        }
        if choices_seen >= 3 {
            break;
        }
    }
    note(
        &mut log,
        &format!("at favorites menu after {choices_seen} choices; selecting slot 1"),
    );
    let data = &block.event_data;
    let entry = block.event_entry_exact(HOME_POINT_EVENT).expect("entry");
    note(&mut log, &format!("event entry at block offset {entry}"));
    // Walk the branch from 974 with the meta widths, flagging jump opcodes.
    let mut j = 974usize;
    for _ in 0..40 {
        let Some(&op) = data.get(j) else { break };
        let meta = ffxi_event::opcode_meta::OPCODE_META[op as usize];
        note(
            &mut log,
            &format!(
                "  op {:04X}: 0x{op:02X} size={} jumps={} valid={}",
                j, meta.size, meta.jumps, meta.valid
            ),
        );
        if !meta.valid || meta.size == 0 {
            break;
        }
        j += meta.size as usize;
    }

    // Now select the first favorite and trace the step stream.
    vm.select_choice(Some(1));
    let mut last: Option<String> = None;
    let mut repeats = 0usize;
    for i in 0..200_000usize {
        let step = vm.step();
        let label = match &step {
            StepResult::AwaitMessage(m) => format!("AwaitMessage({})", m.message_id),
            StepResult::AwaitMessageAck => "AwaitMessageAck".to_string(),
            StepResult::AwaitChoice(c) => format!("AwaitChoice({})", c.message_id),
            StepResult::Done => "Done".to_string(),
            StepResult::Cancelled => "Cancelled".to_string(),
            StepResult::Unimplemented(op) => format!("Unimplemented(0x{op:02X})"),
            StepResult::Spun(op) => format!("Spun(0x{op:02X})"),
            StepResult::Waiting => "Waiting".to_string(),
            StepResult::AwaitServerAck(t) => format!("AwaitServerAck({t:?})"),
        };
        let ep = vm.exec_pointer();
        if Some(label.clone()) == last && i > 60 {
            repeats += 1;
            if repeats == 1 || repeats == 100 || repeats % 10000 == 0 {
                note(
                    &mut log,
                    &format!("step {i}: {label} ep={ep} (repeated {}x)", repeats + 1),
                );
            }
        } else {
            repeats = 0;
            note(&mut log, &format!("step {i}: {label} ep={ep}"));
            last = Some(label);
        }
        match step {
            StepResult::AwaitMessage(_) => vm.dismiss_message(),
            StepResult::AwaitMessageAck => vm.dismiss_message(),
            StepResult::AwaitChoice(_) => {
                note(&mut log, "  (trace stops at the next choice)");
                break;
            }
            StepResult::Waiting => {
                // Answer a timed wait the host would.
                vm.tick(3600.0);
            }
            StepResult::AwaitServerAck(t) => {
                note(&mut log, &format!("  acking {t:?}"));
                vm.ack_server();
            }
            _ => break,
        }
    }
    note(&mut log, "trace done");
}
