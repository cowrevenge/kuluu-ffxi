// Live end-to-end playback of the Bastok new-character opening — event 0
// (the timed narration) and event 7 (the NPC cutscene), zone 235 — against a
// local LSB stack with retail DATs mounted.
//
// Self-skips when the auth port or xidb is unreachable, or when no FFXI
// install can be opened. Proves the retail continue-prompt mechanism
// (research/cexi-docs/dialog/format.md, "the continue-prompt codes"):
// event 0's narration lines 7416-7424 end in `7F 34 NN` auto-prompt codes and
// dismiss themselves on the session's clock after NN seconds — with no player
// input at all — while event 7's lines 7425-7439 end in `7F 31` manual codes
// and hold until Enter.
//
// The trigger is the hidden quest newCharacterCS
// (vendor/server/scripts/quests/hiddenQuests/New_Character_Cutscenes.lua
// BASTOK_MARKETS): onZoneIn starts event 0, onEventFinish[0] chains event 7.

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
    state::{AgentCommand, AgentEvent, DialogState, Stage},
};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
    time::timeout,
};

use common::EphemeralChar;

const BASTOK_MARKETS: u32 = 235;
const EVENT_NARRATION: u16 = 0;
const EVENT_NPC_CS: u16 = 7;
/// The char_vars row that arms the trigger (quest var 'notSeen' of hidden
/// quest newCharacterCS,
/// vendor/server/scripts/quests/hiddenQuests/New_Character_Cutscenes.lua).
const NOT_SEEN_VAR: &str = "HQuest[newCharacterCS]notSeen";
/// The retail auto-advance seconds of the nine narration lines 7416-7424
/// (ROM/25/44.DAT, the `7F 34 NN` continue-prompt codes), in display order.
const NARRATION_SECONDS: [u8; 9] = [5, 7, 7, 5, 7, 7, 6, 5, 5];
/// Event 0's narration alone is 54 s of auto-advance plus the authored waits
/// between segments; an event ending faster than this means the
/// auto-advance clock did not run (the skip-to-end failure mode).
const MIN_EVENT0_SECS: f32 = 45.0;
/// A manual frame must outlive this with no input: the longest auto-advance
/// in the zone's DAT is 7 s, so 20 s is unambiguous.
const MANUAL_HOLD_SECS: f32 = 20.0;
/// How many event-7 frames the test advances by hand before stopping: enough
/// to prove the Enter path still works, short of the map beat that follows
/// the 15 lines.
const MANUAL_ADVANCES: u32 = 3;

const LOGIN_DEADLINE: Duration = Duration::from_secs(90);
const PLAYBACK_DEADLINE: Duration = Duration::from_secs(6 * 60);
/// After event 0 ends, onEventFinish[0] chains event 7; no start within a
/// minute of the end means the chain did not fire.
const CHAIN_GRACE: Duration = Duration::from_secs(60);
/// After InZone, no event 0 start within a minute means the trigger did not
/// fire and waiting longer changes nothing.
const NO_EVENT_GRACE: Duration = Duration::from_secs(60);

fn open_dat_root() -> Option<ffxi_dat::DatRoot> {
    ffxi_dat::archive::open_test_install()
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
    event0_started_at: Option<Instant>,
    event0_ended_at: Option<Instant>,
    event7_started_at: Option<Instant>,
    event7_ended_at: Option<Instant>,
    event7_first_frame_at: Option<Instant>,
    /// The first event-7 frame outlived MANUAL_HOLD_SECS with no input.
    event7_held_manual: bool,
    /// (text snippet, auto_advance seconds, appeared) for every event-0
    /// frame — the test sends no input for these.
    intro_frames: Vec<(String, Option<u8>, Instant)>,
    /// (index of the frame being dismissed, observed gap secs, that frame's
    /// auto-advance secs) between consecutive event-0 frames: the gap up to
    /// the next frame's appearance is governed by the dismissed frame's prompt.
    intro_gaps: Vec<(usize, f32, u8)>,
    /// (text snippet, auto_advance seconds) for every event-7 frame.
    event7_frames: Vec<(String, Option<u8>)>,
    /// The event-7 frame currently up, for the manual EndEventChoice.
    last_event7_dialog: Option<DialogState>,
    manual_advances_sent: u32,
    auto_skipped_line: Option<String>,
    disconnected_reason: Option<String>,
}

fn snippet(s: &str) -> String {
    s.chars().take(60).collect()
}

/// Event 7 only: once the first frame has outlived the manual hold, advance
/// the frame that is up by hand (Enter), one advance per frame. Event 0 must
/// never see input — that is the point under test. Self-pacing: an advance
/// goes out only for a frame that has arrived since the last one, so a slow
/// round-trip cannot double-advance.
async fn maybe_manual_advance(tally: &mut Tally, cmd_tx: &mpsc::Sender<AgentCommand>) {
    if !tally.event7_held_manual || tally.manual_advances_sent >= MANUAL_ADVANCES {
        return;
    }
    if (tally.event7_frames.len() as u32) <= tally.manual_advances_sent {
        return;
    }
    let Some(dialog) = tally.last_event7_dialog.clone() else {
        return;
    };
    let label = dialog
        .choices
        .first()
        .cloned()
        .unwrap_or_else(|| "<dismiss>".into());
    eprintln!(
        "[live] manual advance {} ({}): {:?}",
        tally.manual_advances_sent + 1,
        label,
        snippet(&dialog.prompt.clone().unwrap_or_default())
    );
    tally.manual_advances_sent += 1;
    let _ = cmd_tx
        .send(AgentCommand::EndEventChoice {
            event_id: dialog.event_id,
            act_index: dialog.act_index,
            event_num: dialog.event_num,
            choice: 0,
        })
        .await;
}

fn handle_event(tally: &mut Tally, ev: &AgentEvent, now: Instant, t0: Instant) {
    let t = now.duration_since(t0).as_secs_f32();
    match ev {
        AgentEvent::StageChanged { stage } => {
            if !tally.stages_seen.contains(stage) {
                tally.stages_seen.push(*stage);
            }
            if *stage == Stage::InZone && tally.inzone_at.is_none() {
                tally.inzone_at = Some(now);
                eprintln!("[live] InZone at t+{t:.1}s");
            }
        }
        AgentEvent::CutsceneStarted { event_id } => {
            let raw = *event_id & 0xFFFF;
            if raw == u32::from(EVENT_NARRATION) && tally.event0_started_at.is_none() {
                tally.event0_started_at = Some(now);
                eprintln!("[live] event 0 started (agent id 0x{event_id:08X}) at t+{t:.1}s");
            }
            if raw == u32::from(EVENT_NPC_CS) && tally.event7_started_at.is_none() {
                tally.event7_started_at = Some(now);
                eprintln!("[live] event 7 started (agent id 0x{event_id:08X}) at t+{t:.1}s");
            }
        }
        AgentEvent::EventEnded => {
            if tally.event0_ended_at.is_none()
                && tally.event0_started_at.is_some()
                && tally.event7_started_at.is_none()
            {
                tally.event0_ended_at = Some(now);
                eprintln!("[live] event 0 ended at t+{t:.1}s");
            } else if tally.event7_ended_at.is_none() && tally.event7_started_at.is_some() {
                tally.event7_ended_at = Some(now);
                eprintln!("[live] event 7 ended at t+{t:.1}s");
            }
        }
        AgentEvent::ChatLine { line, .. } => {
            if line
                .text
                .contains(kuluu_session::session::EVENT_AUTO_SKIPPED_MARKER)
                && tally.auto_skipped_line.is_none()
            {
                tally.auto_skipped_line = Some(line.text.clone());
                eprintln!("[live] AUTO-SKIP CHAT LINE: {}", line.text);
            }
        }
        AgentEvent::EventDialog { dialog } => {
            let prompt = dialog.prompt.clone().unwrap_or_default();
            match dialog.event_para {
                EVENT_NARRATION => {
                    // The gap up to this frame's appearance is governed by the
                    // prompt of the frame being dismissed — the last one recorded.
                    if let Some((_, _, prev)) = tally.intro_frames.last() {
                        let dismissed = tally.intro_frames.len() - 1;
                        let gap = now.duration_since(*prev).as_secs_f32();
                        tally
                            .intro_gaps
                            .push((dismissed, gap, NARRATION_SECONDS[dismissed]));
                    }
                    eprintln!(
                        "[live] intro frame {}: auto_advance={:?} ({:?})",
                        tally.intro_frames.len() + 1,
                        dialog.auto_advance,
                        snippet(&prompt)
                    );
                    tally
                        .intro_frames
                        .push((snippet(&prompt), dialog.auto_advance, now));
                }
                EVENT_NPC_CS => {
                    if tally.event7_first_frame_at.is_none() {
                        tally.event7_first_frame_at = Some(now);
                    }
                    eprintln!(
                        "[live] event 7 frame {}: auto_advance={:?} ({:?})",
                        tally.event7_frames.len() + 1,
                        dialog.auto_advance,
                        snippet(&prompt)
                    );
                    tally
                        .event7_frames
                        .push((snippet(&prompt), dialog.auto_advance));
                    tally.last_event7_dialog = Some(dialog.clone());
                }
                _ => {}
            }
        }
        AgentEvent::Disconnected { reason } => {
            tally.disconnected_reason = Some(reason.clone());
        }
        _ => {}
    }
}

#[tokio::test]
async fn bastok_intro_auto_advances_and_event7_stays_manual() {
    let server_host = std::env::var("SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let auth_port = std::env::var("AUTH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ffxi_proto::login::LOGIN_AUTH_PORT);
    let data_port = std::env::var("DATA_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ffxi_proto::login::LOGIN_DATA_PORT);
    let view_port = std::env::var("VIEW_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ffxi_proto::login::LOGIN_VIEW_PORT);

    if !is_reachable(&server_host, auth_port).await {
        eprintln!(
            "skipping: no LSB stack reachable at {server_host}:{auth_port}. \
             To run this test, start the dev stack and re-run with SERVER_HOST/AUTH_PORT set."
        );
        return;
    }

    let Some(dat_root) = open_dat_root() else {
        eprintln!(
            "skipping: no FFXI install found (register one with \
             `kuluu install`, or set FFXI_DAT_PATH); without DATs the holds cannot arm"
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

    let Some(fixture) = EphemeralChar::create_in_zone(&server_host, auth_port, BASTOK_MARKETS)
        .await
        .expect("provisioning ephemeral LSB account+char in Bastok Markets")
    else {
        eprintln!("skipping: xidb not reachable; treating the LSB stack as absent");
        return;
    };
    fixture
        .set_char_var(NOT_SEEN_VAR, 1)
        .await
        .expect("arming the new-character CS trigger in char_vars");
    eprintln!(
        "fixture: user={} accid={} charid={} charname={}",
        fixture.username, fixture.accid, fixture.charid, fixture.charname,
    );

    let cfg = Config {
        server: server_host.clone(),
        map_host_override: None,
        auth_port,
        data_port,
        view_port,
        user: fixture.username.clone(),
        password: fixture.password.clone(),
        char_selection: CharSelection::Name(fixture.charname.clone()),
        initial_state: None,
        playonline_session: None,
        dat_root: Some(Arc::new(dat_root)),
        user_driven_events: false,
    };

    let (cmd_tx, cmd_rx) = mpsc::channel::<AgentCommand>(32);
    let (event_tx, mut event_rx) = broadcast::channel::<AgentEvent>(512);

    let session_task = tokio::spawn(session::run(cfg, cmd_rx, event_tx));

    let out_dir = artifact_dir();
    fs::create_dir_all(&out_dir).expect("creating artifacts/verify");
    let events_path = out_dir.join("bastok_intro_events.jsonl");
    let mut events_log = fs::File::create(&events_path).expect("opening bastok_intro_events.jsonl");

    let t0 = Instant::now();
    let hard_deadline = t0 + LOGIN_DEADLINE + PLAYBACK_DEADLINE;
    let mut tally = Tally::default();

    let stop_reason: Option<String> = loop {
        if Instant::now() >= hard_deadline {
            break Some("hard deadline reached".into());
        }
        match timeout(Duration::from_millis(250), event_rx.recv()).await {
            Ok(Ok(ev)) => {
                let now = Instant::now();
                handle_event(&mut tally, &ev, now, t0);

                let event_value = serde_json::to_value(&ev).expect("serializing AgentEvent");
                let line = serde_json::json!({
                    "t_ms": now.duration_since(t0).as_millis(),
                    "event": event_value,
                });
                writeln!(events_log, "{line}").expect("writing events.jsonl");

                if tally.disconnected_reason.is_some() {
                    break Some("session disconnected".into());
                }

                maybe_manual_advance(&mut tally, &cmd_tx).await;

                // The manual phase is done once the hand advances have been
                // sent and the frames they produced are tallied — or event 7
                // ended on its own after the advances.
                if tally.event7_held_manual
                    && tally.manual_advances_sent >= MANUAL_ADVANCES
                    && (tally.event7_frames.len() > MANUAL_ADVANCES as usize
                        || tally.event7_ended_at.is_some())
                {
                    break Some("manual phase complete".into());
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                eprintln!("[live] event stream lagged, dropped {n} events");
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                break Some("event stream closed".into());
            }
            Err(_) => {
                // No event this tick: age the manual hold and the chain grace.
                let now = Instant::now();
                if let (Some(first), false) =
                    (tally.event7_first_frame_at, tally.event7_held_manual)
                {
                    // The first frame still has to be the only one up: a second
                    // frame this early is an auto-advance the manual gate must
                    // not allow.
                    if now.duration_since(first).as_secs_f32() >= MANUAL_HOLD_SECS
                        && tally.event7_frames.len() == 1
                    {
                        tally.event7_held_manual = true;
                        eprintln!(
                            "[live] event 7 frame held {}s with no input (manual)",
                            MANUAL_HOLD_SECS
                        );
                    }
                }

                maybe_manual_advance(&mut tally, &cmd_tx).await;
                if let Some(ended) = tally.event0_ended_at {
                    if tally.event7_started_at.is_none() && now.duration_since(ended) > CHAIN_GRACE
                    {
                        break Some("event 7 never chained after event 0".into());
                    }
                } else if let Some(started) = tally.event0_started_at {
                    if now.duration_since(started) > PLAYBACK_DEADLINE {
                        break Some("event 0 playback deadline exceeded".into());
                    }
                } else if let Some(inzone) = tally.inzone_at {
                    if now.duration_since(inzone) > NO_EVENT_GRACE {
                        break Some("event 0 never started after zone-in".into());
                    }
                }
            }
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

    write_summary(&out_dir, &tally, &stop_reason, t0);

    assert!(
        tally.stages_seen.contains(&Stage::InZone),
        "session never reached InZone (stages: {:?}, stop: {stop_reason})",
        tally.stages_seen,
    );
    assert!(
        tally.auto_skipped_line.is_none(),
        "the intro auto-skipped instead of playing: {:?}",
        tally.auto_skipped_line
    );
    let started_at = tally.event0_started_at.unwrap_or_else(|| {
        panic!(
            "event 0 never started (stop: {stop_reason}, frames so far: {:?})",
            &tally.intro_frames[..tally.intro_frames.len().min(8)]
        )
    });
    assert!(
        tally.event0_ended_at.is_some(),
        "event 0 never ended on its own (stop: {stop_reason}, frames: {})",
        tally.intro_frames.len()
    );
    let ended_at = tally.event0_ended_at.unwrap();
    let playback_secs = ended_at.duration_since(started_at).as_secs_f32();
    assert!(
        playback_secs >= MIN_EVENT0_SECS,
        "event 0 took {playback_secs:.1}s < {MIN_EVENT0_SECS}s: the auto-advance \
         clock did not run (skip-to-end failure mode)"
    );
    assert_eq!(
        tally.intro_frames.len(),
        NARRATION_SECONDS.len(),
        "expected all {} narration frames, saw {}; frames: {:?}",
        NARRATION_SECONDS.len(),
        tally.intro_frames.len(),
        tally
            .intro_frames
            .iter()
            .map(|(t, a, _)| (t, a))
            .collect::<Vec<_>>()
    );
    for (i, (text, auto, _)) in tally.intro_frames.iter().enumerate() {
        assert_eq!(
            *auto,
            Some(NARRATION_SECONDS[i]),
            "intro frame {i} ({text:?}) auto_advance={auto:?}, expected {}s from the DAT",
            NARRATION_SECONDS[i]
        );
    }
    for (i, gap, expected) in &tally.intro_gaps {
        let expected = *expected as f32;
        assert!(
            *gap >= expected - 0.5,
            "intro frame {i} advanced after {gap:.1}s < its {expected}s auto-prompt: \
             the clock fired early"
        );
        assert!(
            *gap <= expected + 10.0,
            "intro frame {i} took {gap:.1}s to advance, far past its {expected}s \
             auto-prompt plus the authored waits: the clock did not run"
        );
    }
    assert!(
        tally.event7_started_at.is_some(),
        "event 7 never started after event 0 (stop: {stop_reason})"
    );
    assert!(
        !tally.event7_frames.is_empty(),
        "no event 7 frames observed (stop: {stop_reason})"
    );
    for (i, (text, auto)) in tally.event7_frames.iter().enumerate() {
        assert!(
            auto.is_none(),
            "event 7 frame {i} ({text:?}) auto-advances ({auto:?}) — its retail \
             prompt is manual (7F 31)"
        );
    }
    assert!(
        tally.event7_held_manual,
        "no event 7 frame outlived {MANUAL_HOLD_SECS}s without input: the manual \
         gate is not holding (stop: {stop_reason})"
    );
    assert!(
        tally.event7_frames.len() > 1,
        "the manual EndEventChoice produced no next frame: the Enter path is \
         broken (stop: {stop_reason})"
    );

    eprintln!(
        "[live] PASS: event 0 auto-advanced all {} narration frames in {playback_secs:.1}s \
         with zero input; event 7 held manual for {MANUAL_HOLD_SECS}s then advanced by hand \
         ({} of {} frames)",
        tally.intro_frames.len(),
        tally.manual_advances_sent,
        tally.event7_frames.len()
    );
}

fn write_summary(out_dir: &Path, tally: &Tally, stop_reason: &str, t0: Instant) {
    let summary_path = out_dir.join("bastok_intro_summary.txt");
    let mut s = String::new();
    let secs = |i: Option<Instant>| match i {
        Some(t) => format!("t+{:.1}s", t.duration_since(t0).as_secs_f32()),
        None => "n/a".into(),
    };
    push_line(&mut s, &format!("stop reason: {stop_reason}"));
    push_line(&mut s, &format!("stages: {:?}", tally.stages_seen));
    push_line(
        &mut s,
        &format!(
            "InZone at {}, event 0 started at {}, ended at {}, event 7 started at {}",
            secs(tally.inzone_at),
            secs(tally.event0_started_at),
            secs(tally.event0_ended_at),
            secs(tally.event7_started_at)
        ),
    );
    if let Some(a) = &tally.auto_skipped_line {
        push_line(&mut s, &format!("AUTO-SKIP LINE: {a}"));
    }
    for (i, (text, auto, at)) in tally.intro_frames.iter().enumerate() {
        push_line(
            &mut s,
            &format!(
                "intro frame {i}: auto={auto:?} at t+{:.1}s | {text}",
                at.duration_since(t0).as_secs_f32()
            ),
        );
    }
    for (i, gap, expected) in &tally.intro_gaps {
        push_line(
            &mut s,
            &format!("intro gap {i}: {gap:.1}s (auto-prompt {expected}s)"),
        );
    }
    for (i, (text, auto)) in tally.event7_frames.iter().enumerate() {
        push_line(
            &mut s,
            &format!("event 7 frame {i}: auto={auto:?} | {text}"),
        );
    }
    push_line(
        &mut s,
        &format!(
            "event 7 held manual: {}; manual advances sent: {}",
            tally.event7_held_manual, tally.manual_advances_sent
        ),
    );
    if let Some(r) = &tally.disconnected_reason {
        push_line(&mut s, &format!("disconnected: {r}"));
    }
    fs::write(&summary_path, s).expect("writing bastok_intro_summary.txt");
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
