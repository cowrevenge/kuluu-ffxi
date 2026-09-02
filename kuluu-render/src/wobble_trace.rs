//! Wobble-hunt instrumentation. Compiled always, ACTIVE only when the
//! `FFXI_WOBBLE_TRACE` env var is set at launch — otherwise every system here
//! early-outs on a cached bool, so shipping this costs nothing.
//!
//! Purpose: the two-week "texture behind wobbles the image on top" ghost.
//! Screenshots are clean, so the doubling is TEMPORAL: consecutive frames put
//! the camera (or the player the camera tracks) at positions whose deltas are
//! not smooth, and the eye integrates the alternation into a double image.
//! This module writes one JSON line per render frame with everything needed to
//! see that numerically:
//!
//!   t_ms        milliseconds since trace start (Time<Real>, wall clock)
//!   dt_ms       this frame's delta (Time<Real>)
//!   fixed_ran   how many FixedUpdate ticks ran since the previous line
//!   overstep    Time<Fixed>::overstep_fraction() at Update time
//!   px,py,pz    player (IsSelf) Transform.translation AFTER interpolation
//!   cx,cy,cz    OperatorCamera Transform.translation as resolve_camera left it
//!   crx..crw    camera rotation quaternion (wobble can be rotational)
//!
//! Reading it: walk in a straight line at run speed for ~5 s. In a smooth
//! world, per-frame |Δp| and |Δc| are near-constant and Δc tracks Δp. The
//! wobble shows as alternating large/small (or sign-flipping) deltas — and
//! whichever column alternates first names the culprit stage.
//!
//! Output: `wobble_trace.jsonl` in the working directory, truncated at launch.

use bevy::prelude::*;
use std::io::Write;
use std::sync::Mutex;

use crate::camera::OperatorCamera;
use crate::components::IsSelf;

static TRACE: Mutex<Option<std::fs::File>> = Mutex::new(None);

fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FFXI_WOBBLE_TRACE").is_some())
}

/// Counts FixedUpdate runs so each frame line can report how many sim ticks
/// happened since the previous render frame (the 0/1 alternation at 120 Hz
/// display vs 60 Hz sim is expected; the interpolation exists to hide it).
#[derive(Resource, Default)]
pub struct WobbleFixedTicks(pub u32);

pub fn count_fixed_tick(mut n: ResMut<WobbleFixedTicks>) {
    if trace_enabled() {
        n.0 += 1;
    }
}

/// Runs LAST in Update (after resolve_camera has written the camera transform
/// and after interpolate_self_transform_system has written the player's) and
/// appends one line. Everything it reads is the final state the renderer will
/// draw this frame.
pub fn wobble_trace_system(
    real: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    mut ticks: ResMut<WobbleFixedTicks>,
    q_self: Query<&Transform, (With<IsSelf>, Without<OperatorCamera>)>,
    q_cam: Query<&Transform, (With<OperatorCamera>, Without<IsSelf>)>,
) {
    if !trace_enabled() {
        return;
    }
    let (Ok(p), Ok(c)) = (q_self.single(), q_cam.single()) else {
        return;
    };

    let mut guard = TRACE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = std::fs::File::create("wobble_trace.jsonl").ok();
    }
    let Some(f) = guard.as_mut() else { return };

    let fr = ticks.0;
    ticks.0 = 0;
    let r = c.rotation;
    // One compact line; f32 display precision is plenty for delta analysis.
    let _ = writeln!(
        f,
        "{{\"t_ms\":{:.3},\"dt_ms\":{:.3},\"fixed_ran\":{},\"overstep\":{:.4},\
         \"px\":{:.4},\"py\":{:.4},\"pz\":{:.4},\
         \"cx\":{:.4},\"cy\":{:.4},\"cz\":{:.4},\
         \"crx\":{:.5},\"cry\":{:.5},\"crz\":{:.5},\"crw\":{:.5}}}",
        real.elapsed_secs_f64() * 1000.0,
        real.delta_secs_f64() * 1000.0,
        fr,
        fixed.overstep_fraction(),
        p.translation.x,
        p.translation.y,
        p.translation.z,
        c.translation.x,
        c.translation.y,
        c.translation.z,
        r.x,
        r.y,
        r.z,
        r.w,
    );
}
