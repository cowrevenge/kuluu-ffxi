//! Single-authority player walker: retail's mover machine over kuluu's zone
//! data.
//!
//! One module owns horizontal slide, vertical position, falling, and dynamic
//! obstacles. Geometry queries stay on `MzbCollisionGeometry` (kuluu-render):
//! grid `cell_index`, column queries, and the triangle contact helpers. No ECS
//! inside [`step`] — pure over its inputs so the test matrices can drive it
//! headless.
//!
//! Layout: `consts` (every walker constant in one place), `sweep` (horizontal
//! slide against walls), `obstacles` (doors + mobs, rebuilt every fixed tick),
//! `step` (the tick itself).

pub mod consts;
pub mod obstacles;
pub mod step;
pub mod sweep;

pub use step::step;

use bevy::prelude::*;

/// Retail's actor-contact state: the contact actor, the tick budget and
/// whether that budget is still live, from research/XIClient/src/XIClient/source/World/Actor/ControllableActor.cpp
/// ControllableActor::CheckContactActor.
///
/// Contact blocks rather than pushes -- its caller
/// (ControllableActor::HandleThirdPersonControl) simply skips the frame's
/// `newPosition += movementDirection` and never depenetrates, so actors may
/// overlap and a standing player is never shoved. The expiring budget is what
/// turns sustained input into a walk-through instead of a jam.
#[derive(Default)]
pub struct ActorContact {
    target: Option<u32>,
    ticks_left: f32,
    live: bool,
}

impl ActorContact {
    /// The post-nearest-actor half of the rule: `mob` is the single nearest
    /// candidate and is confirmed overlapping. True when this tick's movement
    /// is dropped.
    pub fn contact(&mut self, mob: u32, dt: f32) -> bool {
        if self.target != Some(mob) {
            self.target = Some(mob);
            self.ticks_left = consts::CONTACT_BLOCK_TICKS;
            self.live = true;
            return true;
        }
        if self.live {
            self.ticks_left -= dt * consts::CONTACT_TICKS_PER_SEC;
            if self.ticks_left >= 0.0 {
                return true;
            }
            self.live = false;
        }
        false
    }

    /// No candidate overlapping this tick: retail clears all three fields, so
    /// the next approach gets a fresh budget.
    pub fn clear(&mut self) {
        self.target = None;
        self.ticks_left = 0.0;
        self.live = false;
    }

    /// The actor the budget is running against (None when cleared).
    pub fn target(&self) -> Option<u32> {
        self.target
    }
}

/// The walker's vertical mode. Driven by input: `want_len == 0` is
/// Stopped this tick; Airborne persists until a landing, which picks the next
/// mode from that tick's input.
#[derive(Clone, Copy, Debug, Default)]
pub enum WalkMode {
    #[default]
    Stopped,
    Walking,
    /// Falling. There is no velocity to carry: retail's fall is a fixed
    /// per-frame step (`consts::FALL_STEP_FRAME_PROBE`, alternate
    /// `FALL_STEP_FRAME_ALT`) scaled by one lean float, applied fresh every
    /// frame from the first moment of the drop. `lean` is that float; neutral
    /// posture is `consts::LEAN_DEFAULT_NEUTRAL`.
    Airborne {
        lean: f32,
    },
}

/// Cross-tick walker state, held as a Local in dispatch (`DispatchLocals`).
#[derive(Default)]
pub struct Walker {
    pub mode: WalkMode,
    /// Retail's contact block against the actor currently being walked into.
    pub contact: ActorContact,
    /// The frame's published contact-probe vertical (yalms, up-positive like
    /// `step`'s interior): retail keeps one scene-wide measurement word that its
    /// integration units add straight into position vertical, so kuluu publishes
    /// the same thing here - exactly what `step` added to Y this tick.
    pub probe_measurement_y: f32,
}

/// One tick's outcome. `dx`/`dy` are the allowed horizontal displacement in
/// wire units; `feet_z` is where the walker puts its feet this tick (wire z,
/// grows down).
#[derive(Clone, Copy, Debug)]
pub struct StepResult {
    pub dx: f32,
    pub dy: f32,
    pub feet_z: f32,
    /// The mode after this tick's vertical pass.
    pub mode: WalkMode,
    /// What the vertical pass did.
    pub decision: VerticalDecision,
    /// What the horizontal pass did (the panel's "why am I stopped").
    pub outcome: HorizontalOutcome,
}

/// What this tick's horizontal pass did. One per tick; the panel shows the
/// last two beside the vertical decisions, and the gizmo marks the blocking
/// contact when the move was withheld.
#[derive(Clone, Copy, Debug)]
pub enum HorizontalOutcome {
    /// No input this tick: the sweep and contact check ran with a zero move.
    NoInput,
    /// Full input length honored: the sweep was clear (or the body ran exactly
    /// into a face) and no actor withheld the move.
    Moved,
    /// A wall clipped the move: `ratio` is allowed length over input length.
    Slid { ratio: f32 },
    /// The sweep found no usable slide direction and held this tick's
    /// position; `contact` is the feet xz at the stop (bevy xz).
    WallHold { contact: Option<Vec2> },
    /// The sweep stopped on its boxed-in rule; `contact` as in WallHold.
    WallReversal { contact: Option<Vec2> },
    /// Retail's all-or-nothing actor contact withheld the move; `mob` is the
    /// blocking actor's wire id and `contact` the projected feet xz.
    ActorContact { mob: u32, contact: Option<Vec2> },
    /// Noclip: the sweep and contact check were bypassed.
    Noclip,
}

/// What this tick's vertical pass did. One per tick; the test matrices assert
/// on it.
#[derive(Clone, Copy, Debug)]
pub enum VerticalDecision {
    /// Idle tick: settled toward h0 at speed (or held when already there).
    Stopped { settling: bool },
    /// Walking a staircase window: merged toward the envelope at speed.
    Ramp { g: bevy::math::Vec2, target: f32 },
    /// One riser in the window: climbed/descended it at speed as h0 moved.
    SingleStep { up: bool },
    /// Continuous surface (a slope): followed h0 at speed.
    Slope,
    /// Flat ground: no vertical move to make.
    Flat,
    /// Dead band: no ramp in the window, snapped to h0 instantly.
    Poof { delta: f32 },
    /// No contact this frame: the fall step was applied (`delta_y` is this
    /// tick's change of feet height, yalms up-positive so negative while
    /// falling; zero on the entry tick) and no floor entered the swept band.
    Airborne { delta_y: f32, lean: f32 },
    /// Was airborne; a floor entered the swept band and we landed on it.
    Landed,
    /// A rise was rejected for the tick (body top would push into geometry).
    CeilingHold,
    /// The field saw a wall face ahead capping the chain: held h0, no rise —
    /// the sweep slides on it.
    WallAhead,
    /// No floor source for this zone yet (main MZB block not landed): gravity
    /// off, height held. Distinct from Airborne — there is nothing to fall
    /// through until the geometry can answer column queries.
    NoGeometry,
}

impl HorizontalOutcome {
    /// Compact ASCII label for the panel (the render crate can't name our
    /// types). Kept short: the panel prints two of these on one line.
    pub fn label(&self) -> String {
        match self {
            Self::NoInput => "NoInput".into(),
            Self::Moved => "Moved".into(),
            Self::Slid { ratio } => format!("Slid {ratio:.0}%"),
            Self::WallHold { .. } => "WallHold".into(),
            Self::WallReversal { .. } => "WallRev".into(),
            Self::ActorContact { mob, .. } => format!("Actor #{mob}"),
            Self::Noclip => "Noclip".into(),
        }
    }

    /// The blocking contact point in bevy xz, when the move was withheld by a
    /// wall or an actor (None: nothing to mark, or no contact info).
    pub fn contact_point(&self) -> Option<Vec2> {
        match self {
            Self::WallHold { contact }
            | Self::WallReversal { contact }
            | Self::ActorContact { contact, .. } => *contact,
            _ => None,
        }
    }
}

impl VerticalDecision {
    /// Compact ASCII label for the panel (the render crate can't name our types).
    pub fn label(&self) -> String {
        match self {
            Self::Stopped { settling } => {
                if *settling {
                    "Stopped(settle)".into()
                } else {
                    "Stopped(hold)".into()
                }
            }
            Self::Ramp { g, target } => format!("Ramp g=({:+.2},{:+.2}) t={:.2}", g.x, g.y, target),
            Self::SingleStep { up } => {
                if *up {
                    "StepUp".into()
                } else {
                    "StepDown".into()
                }
            }
            Self::Slope => "Slope".into(),
            Self::Flat => "Flat".into(),
            Self::Poof { delta } => format!("Poof d={:+.2}", delta),
            Self::Airborne { delta_y, lean } => {
                format!("Air dy={:+.2} lean={:+.1}", delta_y, lean)
            }
            Self::Landed => "Landed".into(),
            Self::CeilingHold => "CeilHold".into(),
            Self::WallAhead => "WallAhead".into(),
            Self::NoGeometry => "Hold(noGeom)".into(),
        }
    }
}

/// Registers the walker's resources and systems: the obstacle rebuild runs in
/// FixedUpdate before dispatch, so the walker reads fresh obstacles (the slot
/// the avian collider syncs used).
pub struct WalkerPlugin;

impl Plugin for WalkerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<obstacles::ObstacleSet>();
        app.add_systems(
            FixedUpdate,
            (
                obstacles::snapshot_mob_block_radius.before(obstacles::rebuild_obstacles_system),
                obstacles::rebuild_obstacles_system.before(super::input::dispatch_movement_system),
            )
                .run_if(in_state(super::AppPhase::InGame)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::consts::{CONTACT_BLOCK_TICKS, CONTACT_TICKS_PER_SEC};
    use super::ActorContact;

    /// field_5AC = 30 decremented by CheckTick() (60 / EffectiveFramerate)
    /// is 30 sixtieths of a second regardless of our tick rate: the same
    /// wall-clock window at 60 Hz and at 30 Hz.
    #[test]
    fn contact_budget_expires_after_retail_window() {
        for hz in [60.0f32, 30.0] {
            let dt = 1.0 / hz;
            let mut c = ActorContact::default();
            let mut blocked = 0u32;
            while c.contact(1, dt) {
                blocked += 1;
            }
            let secs = blocked as f32 * dt;
            let want = CONTACT_BLOCK_TICKS / CONTACT_TICKS_PER_SEC;
            assert!(
                (secs - want).abs() <= dt + 1e-6,
                "at {hz} Hz blocked {blocked} ticks ({secs} s), want ~{want} s"
            );
        }
    }

    /// Exhaust the budget against actor 1; a different nearest actor re-arms
    /// it (what makes a crowd stutter rather than open up); separating
    /// clears all three fields, so the next approach blocks again.
    #[test]
    fn contact_budget_rearms_on_target_change_and_clear() {
        let dt = 1.0 / 60.0;
        let mut c = ActorContact::default();
        while c.contact(1, dt) {}
        assert!(
            !c.contact(1, dt),
            "expired budget must keep passing through"
        );
        assert!(c.contact(2, dt), "target switch must re-arm");
        while c.contact(2, dt) {}
        c.clear();
        assert_eq!(c.target(), None);
        assert!(c.contact(2, dt), "clear must re-arm the same actor");
    }
}
