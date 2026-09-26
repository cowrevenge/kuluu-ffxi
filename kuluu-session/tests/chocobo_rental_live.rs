// Live reproduction of the chocobo-rental cutscene (zone 230 event 601)
// against a local LSB stack with retail DATs mounted.
//
// Self-skips when the auth port or xidb is unreachable, or when no FFXI
// install can be opened. This is the item-1 reproduction harness: it drives
// the rental CS with the `!cs` GM command and records the full protocol
// timeline — every CutsceneCue, every mount-state write (SelfServerStatus
// from the server's 0x037 and CsMountArmed from the CS 0x7E cue), and every
// dialog frame — with t0-relative timestamps, so the summary and the JSONL
// show exactly which entity's visibility / mount state flaps and when. It
// asserts only that the CS actually ran (started and ended); the flap
// analysis lives in the captured evidence. With the cs_mount_armed latch,
// the CS cue's mount write (CsMountArmed status=5) survives the server's
// stale on-foot 0x037 at CS end, so the mount no longer flaps invisible.
//
// The rental CS program (ffxi-event example zz-cs-trace 230 600, block
// 0x010E6030): frames 0-1 are the chat-only "You can rent a chocobo…" lines
// (the user's "chocobo lines"); then CameraLock + the NPC chocobo (0x010E6033)
// sits; then Scheduler chc0/fdo0 on the sentinel 0x7FFFFF08, ActorHide on the
// NPC chocobo, Mount status 5 on the sentinel 0x7FFFFF00, and fdi0 on
// 0x7FFFFF08.

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
    state::{AgentCommand, AgentEvent, CutsceneCue, DialogState, Stage},
};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
    time::timeout,
};

use common::EphemeralChar;

const SANDORIA: u32 = 230;
/// The success rental event (vendor/server/scripts/zones/Southern_San_dOria/
/// npcs/Meuneille.lua eventSucceed); 604 is the fail event. Blocks 0x010E6031
/// (renter) + 0x010E6034 (chocobo NPC).
const RENTAL_CS: u16 = 601;
/// `!cs 601 <price> <currency> <soundParam>` — the operands the retail
/// renterOnTrigger passes (vendor/server/scripts/globals/chocobo.lua). The
/// event program gates the mount path on currency (op2) >= price (op1), so
/// op2 must be the char's real gil.
const CS_TRIGGER: &str = "!cs 601 50 100 0";
/// The fixture's starting gil; must be >= the CS price (50).
const FIXTURE_GIL: u32 = 100;
/// CHOCOBO_LICENSE (vendor/server/scripts/enum/key_item.lua).
const CHOCOBO_LICENSE: u16 = 138;

const LOGIN_DEADLINE: Duration = Duration::from_secs(90);
/// Observe this long after the CS starts: the CS itself is a few seconds, but
/// the mount is set when the event finishes and any flap happens around/after
/// that, so the window has to outlive the CS plus the post-CS settle.
const OBSERVE_SECS: u32 = 90;

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
    cs_started_at: Option<Instant>,
    cs_ended_at: Option<Instant>,
    /// (t secs, cue description) for every CutsceneCue.
    cues: Vec<(f32, String)>,
    /// (t secs, status, mount_id) for every SelfServerStatus.
    mount_states: Vec<(f32, u8, u8)>,
    /// (t secs, text snippet) for every dialog frame.
    frames: Vec<(f32, String)>,
    disconnected_reason: Option<String>,
    /// The dialog frame currently up, for driving the manual frames/choices.
    last_dialog: Option<DialogState>,
    /// When the current frame appeared, so the drive can hold it briefly before
    /// acting (a frame must be up before it can be advanced or answered).
    last_dialog_at: Option<Instant>,
    /// How many frames the drive has already advanced/answered.
    frames_acted: usize,
}

/// How long a frame must be up before the drive acts on it. Long enough that
/// the frame is fully rendered and its prompt stable, short enough to keep the
/// CS moving.
const FRAME_ACT_DELAY: Duration = Duration::from_secs(3);

fn snippet(s: &str) -> String {
    s.chars().take(60).collect()
}

fn describe_cue(cue: &CutsceneCue) -> String {
    match cue {
        CutsceneCue::ActorHide { target, hide } => {
            format!("ActorHide target={target:?} hide={hide}")
        }
        CutsceneCue::Mount {
            target,
            status_event,
            mount_id,
        } => format!("Mount target={target:?} status={status_event} mount_id={mount_id:?}"),
        CutsceneCue::Scheduler {
            dat_id, actor, tag, ..
        } => {
            let fourcc: String = tag.iter().map(|b| *b as char).collect();
            format!("Scheduler dat={dat_id} actor={actor:?} tag={fourcc}")
        }
        CutsceneCue::ActorMotion { actor, key, .. } => {
            let fourcc: String = key.iter().map(|b| *b as char).collect();
            format!("ActorMotion actor={actor:?} key={fourcc}")
        }
        CutsceneCue::CameraLock { lock } => format!("CameraLock {lock}"),
        CutsceneCue::LocalMode { mode } => format!("LocalMode {mode}"),
        other => format!("{other:?}"),
    }
}

/// Drive the running CS forward: once the current frame has been up for
/// FRAME_ACT_DELAY, advance it (a no-choice line) or answer it with choice 0
/// (the "Yes, rent" row of the rental menu). One action per frame, so a slow
/// round-trip cannot double-advance. Stops once the CS has ended.
async fn maybe_drive_frame(tally: &mut Tally, cmd_tx: &mpsc::Sender<AgentCommand>) {
    if tally.cs_ended_at.is_some() {
        return;
    }
    let Some(dialog) = tally.last_dialog.clone() else {
        return;
    };
    let Some(appeared) = tally.last_dialog_at else {
        return;
    };
    if Instant::now().duration_since(appeared) < FRAME_ACT_DELAY {
        return;
    }
    // Only act on a frame we have not already acted on: the frame count is the
    // number of EventDialogs seen, and frames_acted is how many we answered.
    if tally.frames_acted >= tally.frames.len() {
        return;
    }
    let label = dialog
        .choices
        .first()
        .cloned()
        .unwrap_or_else(|| "<advance>".into());
    eprintln!(
        "[live] driving frame {} ({:?})",
        tally.frames_acted + 1,
        snippet(&label)
    );
    tally.frames_acted += 1;
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
            if raw == u32::from(RENTAL_CS) && tally.cs_started_at.is_none() {
                tally.cs_started_at = Some(now);
                eprintln!("[live] rental CS started at t+{t:.1}s");
            }
        }
        AgentEvent::CutsceneCue { cue } => {
            let desc = describe_cue(cue);
            eprintln!("[live] t+{t:.2}s cue {desc}");
            tally.cues.push((t, desc));
        }
        AgentEvent::SelfServerStatus { status, mount_id } => {
            eprintln!("[live] t+{t:.2}s SelfServerStatus status={status} mount_id={mount_id}");
            tally.mount_states.push((t, *status, *mount_id));
        }
        AgentEvent::CsMountArmed { status, mount_id } => {
            eprintln!("[live] t+{t:.2}s CsMountArmed status={status} mount_id={mount_id}");
            tally.mount_states.push((t, *status, *mount_id));
        }
        AgentEvent::EventDialog { dialog } => {
            let prompt = dialog.prompt.clone().unwrap_or_default();
            eprintln!(
                "[live] t+{t:.2}s frame ({} choices): {:?}",
                dialog.choices.len(),
                snippet(&prompt)
            );
            tally.frames.push((t, snippet(&prompt)));
            tally.last_dialog = Some(dialog.clone());
            tally.last_dialog_at = Some(now);
        }
        AgentEvent::EventEnded => {
            if tally.cs_ended_at.is_none() && tally.cs_started_at.is_some() {
                tally.cs_ended_at = Some(now);
                eprintln!("[live] rental CS ended at t+{t:.1}s");
            }
        }
        AgentEvent::Disconnected { reason } => {
            tally.disconnected_reason = Some(reason.clone());
        }
        _ => {}
    }
}

#[tokio::test]
async fn chocobo_rental_cs_protocol_timeline() {
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

    let Some(fixture) = EphemeralChar::create_in_zone(&server_host, auth_port, SANDORIA)
        .await
        .expect("provisioning ephemeral LSB account+char in Southern San d'Oria")
    else {
        eprintln!("skipping: xidb not reachable; treating the LSB stack as absent");
        return;
    };
    eprintln!(
        "fixture: user={} accid={} charid={} charname={}",
        fixture.username, fixture.accid, fixture.charid, fixture.charname,
    );
    fixture
        .add_gil(FIXTURE_GIL)
        .await
        .expect("granting fixture gil");
    fixture
        .add_key_item(CHOCOBO_LICENSE)
        .await
        .expect("granting fixture the Chocobo License");

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
    let events_path = out_dir.join("chocobo_rental_events.jsonl");
    let mut events_log =
        fs::File::create(&events_path).expect("opening chocobo_rental_events.jsonl");

    let t0 = Instant::now();
    let mut tally = Tally::default();
    let mut cs_sent = false;

    let stop_reason: Option<String> = loop {
        if let Some(inzone) = tally.inzone_at {
            let elapsed = t0.elapsed();
            let since_inzone = inzone.elapsed();
            // Once in-zone, fire the CS trigger once (after a short settle so
            // the zone's entities are present), then observe for OBSERVE_SECS.
            if !cs_sent && since_inzone > Duration::from_secs(6) {
                eprintln!("[live] sending {CS_TRIGGER:?}");
                let _ = cmd_tx
                    .send(AgentCommand::Chat {
                        kind: 0,
                        text: CS_TRIGGER.to_string(),
                    })
                    .await;
                cs_sent = true;
            }
            if since_inzone > Duration::from_secs(OBSERVE_SECS as u64) {
                break Some("observation window complete".into());
            }
            let _ = elapsed;
        }
        if t0.elapsed()
            > LOGIN_DEADLINE + Duration::from_secs(OBSERVE_SECS as u64) + Duration::from_secs(60)
        {
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

                maybe_drive_frame(&mut tally, &cmd_tx).await;
            }
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                eprintln!("[live] event stream lagged, dropped {n} events");
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                break Some("event stream closed".into());
            }
            Err(_) => {
                // No event this tick: age the frame drive so a quiet frame
                // (no further protocol traffic) still gets advanced/answered.
                maybe_drive_frame(&mut tally, &cmd_tx).await;
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
        tally.cs_started_at.is_some(),
        "the rental CS never started after {CS_TRIGGER:?} (stop: {stop_reason})"
    );
    eprintln!(
        "[live] captured {} cues and {} mount-state events; summary in artifacts/verify",
        tally.cues.len(),
        tally.mount_states.len()
    );
}

fn write_summary(out_dir: &Path, tally: &Tally, stop_reason: &str, t0: Instant) {
    let summary_path = out_dir.join("chocobo_rental_summary.txt");
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
            "InZone at {}, CS started at {}, CS ended at {}",
            secs(tally.inzone_at),
            secs(tally.cs_started_at),
            secs(tally.cs_ended_at)
        ),
    );
    push_line(&mut s, &format!("--- {} cues ---", tally.cues.len()));
    for (t, desc) in &tally.cues {
        push_line(&mut s, &format!("t+{t:.2}s {desc}"));
    }
    push_line(
        &mut s,
        &format!("--- {} mount-state events ---", tally.mount_states.len()),
    );
    for (t, status, mount_id) in &tally.mount_states {
        push_line(
            &mut s,
            &format!("t+{t:.2}s status={status} mount_id={mount_id}"),
        );
    }
    push_line(
        &mut s,
        &format!("--- {} dialog frames ---", tally.frames.len()),
    );
    for (t, text) in &tally.frames {
        push_line(&mut s, &format!("t+{t:.2}s {text}"));
    }
    if let Some(r) = &tally.disconnected_reason {
        push_line(&mut s, &format!("disconnected: {r}"));
    }
    fs::write(&summary_path, s).expect("writing chocobo_rental_summary.txt");
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
