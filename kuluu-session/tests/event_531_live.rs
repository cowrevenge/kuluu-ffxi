// Live end-to-end check of event 531, the Windurst Waters new-character
// opening cutscene, against a local LSB stack with retail DATs mounted.
//
// Self-skips when the auth port or xidb is unreachable, or when no FFXI
// install can be opened. 531 is a multi-entity event: ROM/21/47.DAT holds
// five exact owners (the 163-step main program, a [HIDE_SELF, END] block, and
// three more NPC blocks). Kuluu's single event VM resolves the zone-in
// trigger through EventVm::driving_block, whose sole-participant scan
// fall-throughs to the master block's trivial [END] program whenever two or
// more blocks hold a non-END exact entry — so kuluu auto-releases 531 with an
// immediate c2s 0x05B instead of playing the main program (ffxi-event/src/
// vm.rs driving_block). The server's onEventFinish[531] then runs:
// messageText, item 536, and setPos to the CS exit (vendor/server/scripts/
// quests/hiddenQuests/New_Character_Cutscenes.lua WINDURST_WATERS).
//
// This test asserts the observed behavior: the event releases promptly on
// zone-in and the onEventFinish rewards land. A fresh fixture char spawns at
// the schema default (0, 0, 0), so the exit position arrives via the
// post-event WPOS (ForcedMove), not the zone-in stream.
//
// The 0x47/0x05C onEventUpdate round trip itself is live-verified by
// event_568_live.rs; 531's onEventUpdate/0x47 position-tag programs live only
// in master-block 0xFFFF REQSET children spawned by the 906/907 family, so a
// retail new-Hume 531 never fires that round trip either.

mod common;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use kuluu_session::{
    session::{self, CharSelection, Config},
    state::{AgentCommand, AgentEvent, InventoryUpdate, Position, Stage, Vec3},
};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
    time::timeout,
};

use common::EphemeralChar;

// Windurst Waters zone id, the onZoneIn key of the 531 new-character
// cutscene trigger (vendor/server/scripts/quests/hiddenQuests/
// New_Character_Cutscenes.lua WINDURST_WATERS onZoneIn).
const WINDURST_WATERS_ZONE: u32 = 238;
// xi.item.ADVENTURER_COUPON, granted by onEventFinish[531].
const COUPON_ITEM_NO: u16 = 536;
// The char_vars row that arms the trigger (quest var 'notSeen' of hidden quest
// newCharacterCS).
const NOT_SEEN_VAR: &str = "HQuest[newCharacterCS]notSeen";

// onEventFinish[531] ends with setPos(-40.611, -5, 102.5, 57): native
// (x=-40.611, y=-5, z=102.5) arrives as wire Position pos = (-40.611, 102.5,
// -5); the wire .y is horizontal Z and .z is vertical. The earlier
// setPos(-40, -5, 100) of the same handler precedes it on the wire, so the
// assertion takes the last position.
const EXIT_WIRE_X: f32 = -40.611;
const EXIT_WIRE_Y: f32 = 102.5;
const EXIT_WIRE_Z: f32 = -5.0;
const EXIT_HEADING: u8 = 57;
const EXIT_TOLERANCE: f32 = 0.5;

const LOGIN_DEADLINE: Duration = Duration::from_secs(90);
// The auto-release 0x05B goes out on the same keepalive tick as the zone-in
// event consumption; the onEventFinish rewards follow within a second. A
// minute from InZone is generous.
const REWARD_GRACE: Duration = Duration::from_secs(60);

// FFXI_DAT_PATH wins when set; otherwise fall back to the retail install on
// this machine so a plain `cargo test` still mounts real DATs.
const DEFAULT_RETAIL_INSTALL: &str = r"C:\PhoenixXI\SquareEnix\FINAL FANTASY XI";

fn open_dat_root() -> Option<ffxi_dat::DatRoot> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("FFXI_DAT_PATH")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let default_install = PathBuf::from(DEFAULT_RETAIL_INSTALL);
    if !candidates.contains(&default_install) {
        candidates.push(default_install);
    }
    for candidate in candidates {
        if !candidate.join("VTABLE.DAT").exists() {
            eprintln!("dat root candidate {candidate:?} has no VTABLE.DAT; skipping it");
            continue;
        }
        match ffxi_dat::DatRoot::open(&candidate) {
            Ok(root) => return Some(root),
            Err(e) => eprintln!("opening dat root at {candidate:?}: {e:#}; trying next"),
        }
    }
    None
}

fn artifact_dir() -> PathBuf {
    std::env::var_os("VERIFY_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace root above kuluu-session");
            workspace_root.join("artifacts").join("verify")
        })
}

#[derive(Default)]
struct Tally {
    stages_seen: Vec<Stage>,
    inzone_at: Option<Instant>,
    item_536_slot: Option<u8>,
    // The last position the session reported, from either ForcedMove (the
    // post-event WPOS) or PositionChanged (the zone-in seed). The exit
    // setPos is the last of the two, so the last reported position is the
    // assertion target.
    last_position: Option<Position>,
    forced_moves: Vec<Position>,
    auto_skipped_line: Option<String>,
    disconnected_reason: Option<String>,
}

fn handle_event(tally: &mut Tally, ev: &AgentEvent, now: Instant) {
    match ev {
        AgentEvent::StageChanged { stage } => {
            if !tally.stages_seen.contains(stage) {
                tally.stages_seen.push(*stage);
            }
            if *stage == Stage::InZone && tally.inzone_at.is_none() {
                tally.inzone_at = Some(now);
                eprintln!("[live] InZone at t+{:.1}s", now.elapsed().as_secs_f32());
            }
        }
        AgentEvent::ChatLine { line, .. } => {
            if line.text.contains("auto-skipped") && tally.auto_skipped_line.is_none() {
                tally.auto_skipped_line = Some(line.text.clone());
                eprintln!("[live] AUTO-SKIP CHAT LINE: {}", line.text);
            }
        }
        AgentEvent::InventoryUpdated { update, .. } => {
            if let InventoryUpdate::SlotChanged { slot } = update {
                if slot.item_no == COUPON_ITEM_NO && tally.item_536_slot.is_none() {
                    tally.item_536_slot = Some(slot.index);
                    eprintln!(
                        "[live] item {COUPON_ITEM_NO} granted to inventory slot {} at t+{:.1}s",
                        slot.index,
                        now.elapsed().as_secs_f32()
                    );
                }
            }
        }
        AgentEvent::ForcedMove { target, .. } => {
            tally.forced_moves.push(*target);
            tally.last_position = Some(*target);
            eprintln!(
                "[live] ForcedMove to ({:.1}, {:.1}, {:.1}) heading {} at t+{:.1}s",
                target.pos.x,
                target.pos.y,
                target.pos.z,
                target.heading,
                now.elapsed().as_secs_f32()
            );
        }
        AgentEvent::PositionChanged { pos } => {
            tally.last_position = Some(*pos);
        }
        AgentEvent::Disconnected { reason } => {
            tally.disconnected_reason = Some(reason.clone());
        }
        _ => {}
    }
}

fn is_exit(pos: &Vec3) -> bool {
    (pos.x - EXIT_WIRE_X).abs() <= EXIT_TOLERANCE
        && (pos.y - EXIT_WIRE_Y).abs() <= EXIT_TOLERANCE
        && (pos.z - EXIT_WIRE_Z).abs() <= EXIT_TOLERANCE
}

#[tokio::test]
async fn event_531_auto_release_and_on_event_finish_against_live_lsb() {
    let server_host = std::env::var("SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let auth_port = std::env::var("AUTH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(54231);

    if !is_reachable(&server_host, auth_port).await {
        eprintln!(
            "skipping: no LSB stack reachable at {server_host}:{auth_port}. \
             To run this test, start the dev stack and re-run with SERVER_HOST set."
        );
        return;
    }

    let Some(dat_root) = open_dat_root() else {
        eprintln!(
            "skipping: no FFXI install found (set FFXI_DAT_PATH or install at \
             {DEFAULT_RETAIL_INSTALL}); without DATs the VM cannot resolve the \
             531 block and this test cannot observe the auto-release"
        );
        return;
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,kuluu_session=debug")),
        )
        .with_test_writer()
        .try_init();

    common::pin_unique_local_port();

    let Some(fixture) =
        EphemeralChar::create_in_zone(&server_host, auth_port, WINDURST_WATERS_ZONE)
            .await
            .expect("provisioning ephemeral LSB account+char")
    else {
        eprintln!("skipping: xidb not reachable; treating the LSB stack as absent");
        return;
    };
    fixture
        .set_char_var(NOT_SEEN_VAR, 1)
        .await
        .expect("arming the new-character CS trigger in char_vars");
    eprintln!(
        "fixture: user={} accid={} charid={} charname={} zone={WINDURST_WATERS_ZONE}",
        fixture.username, fixture.accid, fixture.charid, fixture.charname,
    );

    let cfg = Config {
        server: server_host.clone(),
        map_host_override: None,
        auth_port,
        data_port: 54230,
        view_port: 54001,
        user: fixture.username.clone(),
        password: fixture.password.clone(),
        char_selection: CharSelection::Name(fixture.charname.clone()),
        initial_state: None,
        dat_root: Some(Arc::new(dat_root)),
        user_driven_events: false,
    };

    let (cmd_tx, cmd_rx) = mpsc::channel::<AgentCommand>(32);
    let (event_tx, mut event_rx) = broadcast::channel::<AgentEvent>(512);

    let session_task = tokio::spawn(session::run(cfg, cmd_rx, event_tx));

    let out_dir = artifact_dir();
    fs::create_dir_all(&out_dir).expect("creating artifacts/verify");
    let events_path = out_dir.join("event_531_events.jsonl");
    let mut events_log = fs::File::create(&events_path).expect("opening event_531_events.jsonl");

    let t0 = Instant::now();
    let hard_deadline = t0 + LOGIN_DEADLINE + REWARD_GRACE;
    let mut tally = Tally::default();

    let stop_reason: Option<String> = loop {
        if Instant::now() >= hard_deadline {
            break Some("hard deadline reached".into());
        }
        match timeout(Duration::from_millis(250), event_rx.recv()).await {
            Ok(Ok(ev)) => {
                let now = Instant::now();
                handle_event(&mut tally, &ev, now);

                let event_value = serde_json::to_value(&ev).expect("serializing AgentEvent");
                let line = serde_json::json!({
                    "t_ms": now.duration_since(t0).as_millis(),
                    "event": event_value,
                });
                writeln!(events_log, "{line}").expect("writing event_531_events.jsonl");

                if tally.disconnected_reason.is_some() {
                    break Some("session disconnected".into());
                }

                // Success: the onEventFinish rewards have both landed — the item
                // and the exit setPos (the last position the session reported).
                if tally.item_536_slot.is_some()
                    && tally
                        .last_position
                        .is_some_and(|p| is_exit(&p.pos) && p.heading == EXIT_HEADING)
                {
                    break Some("onEventFinish rewards observed (item 536 + exit setPos)".into());
                }

                if let Some(inzone_at) = tally.inzone_at {
                    if now - inzone_at > REWARD_GRACE {
                        break Some("onEventFinish rewards never landed after zone-in".into());
                    }
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                eprintln!("[live] event stream lagged, dropped {n} events");
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                break Some("event stream closed".into());
            }
            Err(_) => continue,
        }
    };

    if tally.disconnected_reason.is_none() {
        let _ = cmd_tx.send(AgentCommand::Disconnect).await;
    }
    drop(cmd_tx);

    let stop_reason = stop_reason.unwrap_or_else(|| "unknown (loop fell through)".into());

    match timeout(Duration::from_secs(10), session_task).await {
        Ok(Ok(Ok(()))) => eprintln!("[live] session task ended cleanly"),
        Ok(Ok(Err(e))) => eprintln!("[live] session task returned Err: {e:#}"),
        Ok(Err(join_err)) => eprintln!("[live] session task panicked: {join_err}"),
        Err(_) => eprintln!(
            "[live] session task did not finish within 10s after disconnect \
             (last stages: {:?})",
            tally.stages_seen
        ),
    }

    if let Err(e) = fixture.cleanup().await {
        eprintln!("fixture cleanup failed (non-fatal for this test): {e:#}");
    }

    write_summary(&out_dir, &tally, &stop_reason);

    assert!(
        tally.stages_seen.contains(&Stage::InZone),
        "session never reached InZone (stages: {:?}, stop: {stop_reason})",
        tally.stages_seen,
    );
    assert!(
        tally.auto_skipped_line.is_none(),
        "event 531 auto-skipped with an unimplemented-opcode line instead of \
         auto-releasing cleanly: {:?}",
        tally.auto_skipped_line
    );
    assert!(
        tally.item_536_slot.is_some(),
        "onEventFinish never granted item {COUPON_ITEM_NO} (stop: {stop_reason})"
    );
    let last = tally.last_position.expect(&format!(
        "no position observed after event end (stop: {stop_reason})"
    ));
    assert!(
        is_exit(&last.pos),
        "last position ({:.3}, {:.3}, {:.3}) is not the CS-exit setPos \
         (-40.611, 102.5, -5 wire); all forced moves: {:?}",
        last.pos.x,
        last.pos.y,
        last.pos.z,
        tally.forced_moves
    );
    assert_eq!(
        last.heading, EXIT_HEADING,
        "last position heading {} != 57 from setPos(-40.611, -5, 102.5, 57)",
        last.heading
    );
    assert!(
        tally.disconnected_reason.is_none(),
        "session disconnected: {:?}",
        tally.disconnected_reason
    );

    eprintln!(
        "[live] PASS: event 531 auto-released on zone-in and onEventFinish rewards \
         landed — item 536 -> slot {:?}, exit position ({:.3}, {:.3}, {:.3}) heading {}",
        tally.item_536_slot,
        last.pos.x,
        last.pos.y,
        last.pos.z,
        last.heading
    );
}

fn write_summary(out_dir: &Path, tally: &Tally, stop_reason: &str) {
    let summary_path = out_dir.join("event_531_summary.txt");
    let mut s = String::new();
    let secs = |i: Option<Instant>| match i {
        Some(t) => format!("{:.1}s", t.elapsed().as_secs_f32()),
        None => "n/a".into(),
    };
    push_line(&mut s, &format!("stop reason: {stop_reason}"));
    push_line(&mut s, &format!("stages: {:?}", tally.stages_seen));
    push_line(
        &mut s,
        &format!("InZone at {}", secs(tally.inzone_at)),
    );
    if let Some(a) = &tally.auto_skipped_line {
        push_line(&mut s, &format!("AUTO-SKIP LINE: {a}"));
    }
    push_line(
        &mut s,
        &format!(
            "item 536 slot: {:?}; last position: {:?}",
            tally.item_536_slot, tally.last_position
        ),
    );
    for (i, p) in tally.forced_moves.iter().enumerate() {
        push_line(
            &mut s,
            &format!(
                "forced move {i}: ({:.3}, {:.3}, {:.3}) heading {}",
                p.pos.x, p.pos.y, p.pos.z, p.heading
            ),
        );
    }
    if let Some(r) = &tally.disconnected_reason {
        push_line(&mut s, &format!("disconnected: {r}"));
    }
    fs::write(&summary_path, s).expect("writing event_531_summary.txt");
    eprintln!("[live] summary written to {}", summary_path.display());
}

fn push_line(s: &mut String, line: &str) {
    s.push_str(line);
    s.push('\n');
}

async fn is_reachable(host: &str, port: u16) -> bool {
    timeout(Duration::from_millis(750), TcpStream::connect((host, port)))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}
