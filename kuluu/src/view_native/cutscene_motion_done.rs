//! The renderer's finish report for cutscene motion routines: the event VM
//! parks its WAIT* on a pending hold until this report releases it (the
//! session's deadline sweep is the last-resort release for stopped routines
//! and sessions that run no renderer at all).

use bevy::prelude::*;
use kuluu_render::scheduler_runtime::CutsceneMotionDone;

use super::input::CommandTx;
use kuluu_session::state::{AgentCommand, CutsceneActor};

/// The report carries kuluu-snapshot's actor copy; the command wants
/// kuluu-session's. Same shape, both resolved against the running event's
/// entity before the cue crossed the boundary.
fn session_actor(actor: kuluu_snapshot::CutsceneActor) -> CutsceneActor {
    match actor {
        kuluu_snapshot::CutsceneActor::LocalPlayer => CutsceneActor::LocalPlayer,
        kuluu_snapshot::CutsceneActor::Entity { server_id } => CutsceneActor::Entity {
            server_id,
        },
    }
}

pub fn report_cutscene_motion_done_system(
    cmd_tx: Res<CommandTx>,
    mut done: MessageReader<CutsceneMotionDone>,
) {
    for CutsceneMotionDone { actor, key } in done.read() {
        let _ = cmd_tx.0.try_send(AgentCommand::CutsceneMotionDone {
            actor: session_actor(*actor),
            key: *key,
        });
    }
}
