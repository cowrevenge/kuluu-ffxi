//! Single-authority player walker (rebuild of the avian bridge + legacy sweep).
//!
//! One module owns horizontal slide, vertical position, falling, and dynamic
//! obstacles. Geometry queries stay on `MzbCollisionGeometry` (kuluu-render):
//! grid `cell_index`, column queries (`ground_raycast` / `ground_step`), and
//! the triangle contact helpers. No ECS inside [`step`] — pure over its
//! inputs so the test matrices can drive it headless.
//!
//! Layout: `consts` (one place, plan §2.7), `field` (the ramp field + support
//! probe), `sweep` (horizontal slide against walls), `obstacles` (doors +
//! mobs, rebuilt every fixed tick), `step` (the tick itself), `debug`
//! (FieldDebug resource + panel/gizmo plumbing).

pub mod consts;
pub mod debug;
pub mod field;
pub mod obstacles;
pub mod step;
pub mod sweep;

#[cfg(test)]
mod live_tests;

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
    /// must be dropped.
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

/// The walker's vertical mode (plan §2.3). Driven by input: `want_len == 0` is
/// Stopped this tick; Airborne persists until a landing, which picks the next
/// mode from that tick's input.
#[derive(Clone, Copy, Debug, Default)]
pub enum WalkMode {
    #[default]
    Stopped,
    Walking,
    /// Falling: `vy` in yalms/s (negative = down), integrated by FallModel.
    Airborne {
        vy: f32,
    },
}

/// Cross-tick walker state, held as a Local in dispatch (`DispatchLocals`).
#[derive(Default)]
pub struct Walker {
    pub mode: WalkMode,
    /// Retail's contact block against the actor currently being walked into.
    pub contact: ActorContact,
    /// Fall feel (plan §0 Q3): fast and smooth, tuned by walking off ledges —
    /// swap for the real constant if the XiClient source ever turns up one.
    pub fall: consts::FallModel,
    /// Slew-limited envelope gradient carried across ticks (plan §2.2): a
    /// wobbly estimate or a fast 180 can't spike g.
    pub grad: bevy::math::Vec2,
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
    /// What the vertical pass did (plan §3).
    pub decision: VerticalDecision,
}

/// What this tick's vertical pass did (plan §3). One per tick; the panel shows
/// the last two and step 4's live tests assert on it.
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
    /// Gravity integrated; no landing this tick.
    Airborne { vy: f32 },
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
            Self::Airborne { vy } => format!("Air vy={:+.2}", vy),
            Self::Landed => "Landed".into(),
            Self::CeilingHold => "CeilHold".into(),
            Self::WallAhead => "WallAhead".into(),
            Self::NoGeometry => "Hold(noGeom)".into(),
        }
    }
}

/// One fixed tick of the walker (wire coordinates at the boundary; see
/// `step.rs` for the pass order).
pub use step::step;

/// Registers the walker's resources and systems: the obstacle rebuild runs in
/// FixedUpdate before dispatch (the slot the avian collider syncs used); the
/// debug gizmo + snapshot systems run every frame.
pub struct WalkerPlugin;

impl Plugin for WalkerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<obstacles::ObstacleSet>();
        app.init_resource::<debug::FieldDebug>();
        app.init_resource::<debug::StairDebugZoneCache>();
        // Dynamic obstacles before the walker reads them (plan §2.5).
        app.add_systems(
            FixedUpdate,
            (
                obstacles::snapshot_mob_block_radius.before(obstacles::rebuild_obstacles_system),
                obstacles::rebuild_obstacles_system.before(super::input::dispatch_movement_system),
            )
                .run_if(in_state(super::AppPhase::InGame)),
        );
        // In-world ramp-field gizmos behind the `stair_draw` toggle.
        app.add_systems(
            Update,
            debug::draw_walker_field_gizmos.run_if(in_state(super::AppPhase::InGame)),
        );
        // Panel snapshot: FieldDebug -> StairDebugSnapshot every frame (the
        // render crate's stair_debug panel reads it).
        app.add_systems(
            Update,
            debug::update_stair_debug_snapshot_system.run_if(in_state(super::AppPhase::InGame)),
        );
    }
}
