//! One fixed tick of the player walker: retail's mover machine over kuluu's
//! zone data.
//!
//! Wire coordinates at the boundary (x/y horizontal, z grows DOWN; bevy space
//! inside). The law implemented here is the mover probe machine as recorded in
//! `.agents/skills/retail-observe/references/2026-10-25-collision-query.md`:
//! ground follows measured height at a rate bound with no maximum-rise cap
//! ("Step up", "Fall and landing"), terrain never blocks horizontally (walls
//! come from the separately authored blocker class - "Face classification" and
//! "Blocking belongs to other candidates"), falling is one fixed step per
//! movement frame times lean with no ramp and no terminal speed, landing is a
//! contact gate flip ("Landing is a gate flip, not a snap"), and the probe
//! answer's horizontal pair shortens the next step at `PROBE_MARGIN_SCALE`
//! while its vertical word feeds aim height ("Result vector (consumer pass)").

use bevy::math::{Vec2, Vec3};
use kuluu_render::dat_mzb::MzbCollisionGeometry;

use super::consts::{
    BODY_HEIGHT, BODY_RADIUS, CONTACT_SEARCH_RADIUS, FALL_STEP_FRAME_ALT, FALL_STEP_FRAME_PROBE,
    FLOOR_COS, LEAN_DEFAULT_NEUTRAL, PROBE_MARGIN_SCALE, STEP_MAX,
};
use super::obstacles::{DoorObstacle, MobObstacle, ObstacleSet};
use super::sweep::{self, WallSource};
use super::{ActorContact, HorizontalOutcome, StepResult, VerticalDecision, WalkMode, Walker};

/// One follow word: the magnitude bounding any vertical change per movement
/// frame, up or down - retail carries one symbol for both directions (the
/// collision record's step-acceptance delta row). Single definition lives in
/// `dat_mzb::FOLLOW_WORD_YAMLS_PER_FRAME` because the wall sweep's riser band
/// uses the same number: a face that query suppresses is exactly one this
/// machine can climb; never re-typed.
pub const FOLLOW_WORD_YAMLS_PER_FRAME: f32 = kuluu_render::dat_mzb::FOLLOW_WORD_YAMLS_PER_FRAME;

/// Alternate-gate penalty, carried as sourced data: kuluu models one gate pair
/// so nothing selects it (same scope note as `walker/consts.rs`).
pub const FALL_PENALTY_ALT_YAMLS_PER_FRAME: f32 = FALL_STEP_FRAME_ALT;

/// The only vertical extent authored on retail's shape block at construction
/// ("Step reach and actor shape", collision record). Narrow-phase use is
/// unestablished there, so this constant documents the seed and seeds nothing.
pub const SHAPE_EXTENT_SEED_YAMLS: f32 = 2.0;

/// The mover probe's answer block, named per "Result vector (consumer pass)"
/// (collision record): words one and three are the horizontal pair consumed
/// only as a margin length; word two is the vertical accumulation that moves
/// both player and camera; word four has no established consumer.
#[derive(Clone, Copy, Debug, Default)]
pub struct ResultVector {
    pub lateral_x: f32,
    pub vertical: f32,
    pub lateral_z: f32,
    /// No consumer found at retail's anchors; carried for layout completeness.
    pub word4_unconsumed: f32,
}

impl ResultVector {
    /// The step shortening the movement integration applies: root of squares
    /// over the horizontal pair times the recorded scale; the vertical word is
    /// skipped by construction ("the margin wrapper skips the accumulated
    /// vertical word ...", collision record delta).
    pub fn margin(&self) -> f32 {
        (self.lateral_x * self.lateral_x + self.lateral_z * self.lateral_z).sqrt()
            * PROBE_MARGIN_SCALE
    }

    /// The camera unit's law for aim height: add this answer's vertical word
    /// straight into the accumulator - no intermediate global, no scaling.
    pub fn apply_to_aim_height(&self, accumulator_y: f32) -> f32 {
        accumulator_y + self.vertical
    }
}

/// Walls are the authored blocker class (kuluu's body queries over MZB wall
/// faces plus closed doors): terrain answers standability and never blocks;
/// this source is the counterpart of retail's separately registered shape
/// candidates ("Blocking belongs to other candidates", collision record).
struct AuthoredWalls<'a> {
    geom: &'a MzbCollisionGeometry,
    doors: &'a [DoorObstacle],
}

impl WallSource for AuthoredWalls<'_> {
    fn nearest_wall(&self, center: Vec3, r: f32, feet_y: f32) -> Option<(f32, Vec3)> {
        let mut best = self.geom.nearest_wall_contact(center, r, feet_y);
        let r2 = r * r;
        for d in self.doors {
            if center.x < d.min.x - r
                || center.x > d.max.x + r
                || center.z < d.min.z - r
                || center.z > d.max.z + r
            {
                continue;
            }
            for (v, n) in &d.tris {
                if n.y >= FLOOR_COS {
                    continue;
                }
                if v[0].y.max(v[1].y).max(v[2].y) <= feet_y + STEP_MAX {
                    continue;
                }
                let d2 = kuluu_render::dat_mzb::point_tri_dist_sq(center, v[0], v[1], v[2]);
                if d2 < r2 && best.is_none_or(|(bd, _)| d2 < bd) {
                    best = Some((d2, *n));
                }
            }
        }
        best
    }
}

/// Downward vertical ray at `xz` against triangle `v`: the hit height, or None
/// when the face is parallel to the column. Ported from the pre-retail tick's
/// door floor source: a closed door is a wall for the sweep AND a floor for
/// the column probe (a closed drawbridge; an elevator platform is the floor
/// under a rider at whatever height it reached - `walker/obstacles.rs`).
fn tri_hits_column(v: [Vec3; 3], xz: Vec2) -> Option<f32> {
    let (a, b, c) = (v[0], v[1], v[2]);
    let e1 = b - a;
    let e2 = c - a;
    let det = e1.x * e2.z - e1.z * e2.x;
    if det.abs() < 1e-9 {
        return None;
    }
    let qx = xz.x - a.x;
    let qz = xz.y - a.z;
    let u = (qx * e2.z - qz * e2.x) / det;
    if u < 0.0 || u > 1.0 {
        return None;
    }
    let v_ = (e1.x * qz - e1.z * qx) / det;
    if v_ < 0.0 || u + v_ > 1.0 {
        return None;
    }
    Some(a.y + u * e1.y + v_ * e2.y)
}

/// Highest up-facing closed-door face in the column at or below `ceiling_y`.
fn door_floor_at(d: &DoorObstacle, xz: Vec2, ceiling_y: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for (v, n) in &d.tris {
        if n.y < FLOOR_COS {
            continue;
        }
        if let Some(h) = tri_hits_column(*v, xz) {
            if h <= ceiling_y && best.is_none_or(|prev| prev < h) {
                best = Some(h);
            }
        }
    }
    best
}

/// Any closed-door triangle crossing the column slab `(lo_y, hi_y]` - the door
/// half of the ceiling hold (ported with the same record-silent authored
/// status as `sweep::ceiling_holds`).
fn doors_in_column_slab(doors: &[DoorObstacle], xz: Vec2, lo_y: f32, hi_y: f32) -> bool {
    for d in doors {
        if xz.x < d.min.x || xz.x > d.max.x || xz.y < d.min.z || xz.y > d.max.z {
            continue;
        }
        if d.max.y <= lo_y || d.min.y > hi_y {
            continue;
        }
        for (v, _) in &d.tris {
            if let Some(h) = tri_hits_column(*v, xz) {
                if h > lo_y && h <= hi_y {
                    return true;
                }
            }
        }
    }
    false
}

/// Standable answer for this pose: the measured floor (MZB plus closed-door
/// up-faces) when it sits within one follow word of the feet (containment
/// window; beyond that the probe answers "no contact", which is what makes a
/// height pop hover then fall - see "Landing is a gate flip, not a snap" and
/// its hover note in the collision record). Kuluu's authored floor filter
/// inside `ground_nearest` stays (record silent on it; kept per the
/// floor-discrimination delta row).
fn contact_floor(
    geom: &MzbCollisionGeometry,
    doors: &[DoorObstacle],
    xz: Vec2,
    feet_y: f32,
) -> Option<f32> {
    let window = FOLLOW_WORD_YAMLS_PER_FRAME + 1e-6;
    let mut best: Option<f32> = None;
    let mut consider = |h: f32| {
        if (h - feet_y).abs() <= window
            && best.is_none_or(|prev| (prev - feet_y).abs() <= (h - feet_y).abs())
        {
            best = Some(h);
        }
    };
    if let Some(h) = geom.ground_nearest(xz, feet_y) {
        consider(h);
    }
    for d in doors {
        if xz.x < d.min.x || xz.x > d.max.x || xz.y < d.min.z || xz.y > d.max.z {
            continue;
        }
        if let Some(h) = door_floor_at(d, xz, f32::MAX) {
            consider(h);
        }
    }
    best
}

/// Retail's contact test over the tick's mob circles
/// (research/XIClient/src/XIClient/source/World/Actor/ControllableActor.cpp
/// ControllableActor::CheckContactActor): true when this tick's movement must
/// be dropped. `projected` is the position the move would reach, not the
/// current one; only the nearest candidate is considered.
fn contact_blocks(
    obstacles: &ObstacleSet,
    projected: Vec2,
    dt: f32,
    state: &mut ActorContact,
) -> bool {
    let mut closest_d2 = CONTACT_SEARCH_RADIUS * CONTACT_SEARCH_RADIUS;
    let mut closest: Option<&MobObstacle> = None;
    for mob in &obstacles.mobs {
        let d2 = (mob.center - projected).length_squared();
        if d2 < closest_d2 {
            closest_d2 = d2;
            closest = Some(mob);
        }
    }
    let Some(mob) = closest else { return false };
    // Retail's own radius is 0.8x the XZ magnitude of model locator 44
    // (research/XIClient/src/XIClient/source/World/Actor/ControllableActor.cpp ControllableActor::GetCollisionSize);
    // we parse no locators yet, so the sweep body radius stands in (kuluu-53tn).
    if closest_d2.sqrt() - BODY_RADIUS - mob.radius >= 0.0 {
        state.clear();
        return false;
    }
    state.contact(mob.id, dt)
}

/// One tick of retail's machine. Same inputs and outputs as `walker::step`, so
/// dispatch picks a walker with one branch; state carries in the same `Walker`
/// (mode + published measurement).
pub fn step(
    geom: &MzbCollisionGeometry,
    obstacles: &ObstacleSet,
    state: &mut Walker,
    x: f32,
    y: f32,
    z_wire: f32,
    dx: f32,
    dy: f32,
    speed_yps: f32,
    dt: f32,
    noclip: bool,
    geometry_ready: bool,
) -> StepResult {
    let _ = (speed_yps, dt); // the speed law is settled upstream in input; this vertical law is per frame.
    let feet_xz = Vec2::new(x, -y);
    let mut up_y = -z_wire;
    let d_in = Vec2::new(dx, -dy);
    let want_len = d_in.length();

    // 1. Probe the pose against the authored wall class (answer's horizontal
    //    pair = measured intrusion) before moving: walking into geometry cuts
    //    this step short; it never gates it (the sweep alone hard-blocks).
    let walls = AuthoredWalls {
        geom,
        doors: &obstacles.doors,
    };
    let center = Vec3::new(feet_xz.x, feet_y_of(up_y), feet_xz.y);
    let answer = probe_answer(&walls, center, up_y, BODY_RADIUS);

    // 2. Horizontal through the authored sweep, step shortened by the margin.
    let mut outcome;
    let mut d;
    if want_len <= 1e-6 {
        outcome = HorizontalOutcome::NoInput;
        d = d_in;
    } else if noclip {
        outcome = HorizontalOutcome::Noclip;
        d = d_in;
    } else {
        let margin = answer.margin();
        let shortened = if margin > 0.0 && want_len > 1e-6 {
            d_in * ((want_len - margin).max(0.0) / want_len)
        } else {
            d_in
        };
        let (swept, exit) = sweep::sweep(&walls, feet_xz, up_y, shortened);
        d = swept;
        // A slide that clears its wall exits the loop as Clean with a clipped
        // length, so - like the pre-retail tick - Moved vs Slid comes from
        // allowed-over-input, not from the exit label.
        let ratio = (d.length() / want_len).min(1.0);
        outcome = match exit {
            sweep::SweepExit::Hold => HorizontalOutcome::WallHold {
                contact: Some(feet_xz + swept),
            },
            sweep::SweepExit::Reversal => HorizontalOutcome::WallReversal {
                contact: Some(feet_xz + swept),
            },
            _ if ratio >= 0.999 => HorizontalOutcome::Moved,
            _ => HorizontalOutcome::Slid { ratio },
        };
    }

    // Retail's all-or-nothing actor contact, ported from the pre-retail tick
    // unchanged (ControllableActor::CheckContactActor - see `contact_blocks`):
    // it drops the move rather than pushing out; the expiring budget lives on
    // Walker::contact across ticks.
    if !noclip && contact_blocks(obstacles, feet_xz + d, dt, &mut state.contact) {
        d = Vec2::ZERO;
        outcome = HorizontalOutcome::ActorContact {
            mob: state.contact.target().unwrap_or(0),
            contact: Some(feet_xz + d_in),
        };
    }

    // 3. Vertical by the recorded law ("Fall and landing (mover-probe
    //    re-read)"): measured ground within one follow word of the feet holds
    //    or drags at <= the rate bound both ways; no contact applies exactly
    //    FALL_STEP_FRAME_PROBE * lean this frame, fresh every frame with no
    //    ramp and no terminal speed; landing is the gate flipping yes. The
    //    tick's height change publishes to `Walker::probe_measurement_y`
    //    exactly as moved (measurement-feed parity with the old walker).
    let decision = if !geometry_ready {
        VerticalDecision::NoGeometry
    } else {
        step_vertical(geom, &obstacles.doors, state, feet_xz + d, &mut up_y)
    };

    StepResult {
        dx: d.x,
        dy: -d.y,
        feet_z: -up_y,
        mode: state.mode,
        decision,
        outcome,
    }
}

/// Body center height used for the wall probe at a given feet height (the same
/// mid-body anchor kuluu's authored chain queries; record silent on retail's
/// exact anchor, so this stays kuluu-authored).
fn feet_y_of(feet_y: f32) -> f32 {
    feet_y + BODY_RADIUS
}

/// Vertical pass. Applies the tick's height change to `up_y` and returns the
/// decision; also publishes `Walker::probe_measurement_y`.
fn step_vertical(
    geom: &MzbCollisionGeometry,
    doors: &[DoorObstacle],
    state: &mut Walker,
    xz: Vec2,
    up_y: &mut f32,
) -> VerticalDecision {
    let lean = match state.mode {
        WalkMode::Airborne { lean } => lean,
        _ => LEAN_DEFAULT_NEUTRAL,
    };
    if matches!(state.mode, WalkMode::Airborne { .. }) {
        // Fall first, then ask: one fixed step this frame regardless of how
        // long the drop has run. Landing is contact answering at or above the
        // stepped feet (crossing within one follow word, never a snap from
        // farther away).
        let prev_up = *up_y;
        let stepped = prev_up + FALL_STEP_FRAME_PROBE * lean;
        match contact_floor(geom, doors, xz, stepped) {
            Some(h)
                if (h - stepped).abs() <= FOLLOW_WORD_YAMLS_PER_FRAME + 1e-6 && h >= stepped =>
            {
                *up_y = h;
                state.mode = WalkMode::Walking;
                state.probe_measurement_y = h - prev_up;
                VerticalDecision::Landed
            }
            _ => {
                *up_y = stepped;
                state.probe_measurement_y = FALL_STEP_FRAME_PROBE * lean;
                VerticalDecision::Airborne {
                    delta_y: FALL_STEP_FRAME_PROBE * lean,
                    lean,
                }
            }
        }
    } else {
        match contact_floor(geom, doors, xz, *up_y) {
            Some(h) => {
                // Ground follow at the rate bound: this is the no-cap
                // acceptance - bounded by FOLLOW_WORD only, never a rise gate.
                let dy =
                    (h - *up_y).clamp(-FOLLOW_WORD_YAMLS_PER_FRAME, FOLLOW_WORD_YAMLS_PER_FRAME);
                if dy > 0.0 && ceiling_blocks(geom, doors, xz, *up_y, *up_y + dy) {
                    // Authored kuluu gate kept against record silence (same
                    // status as the pre-retail tick's ceiling hold): a rise
                    // that would push the body top through any face holds.
                    state.probe_measurement_y = 0.0;
                    return VerticalDecision::CeilingHold;
                }
                *up_y += dy;
                state.probe_measurement_y = dy;
                if dy.abs() <= 1e-6 {
                    VerticalDecision::Flat
                } else if dy > 0.0 {
                    VerticalDecision::SingleStep { up: true }
                } else {
                    VerticalDecision::SingleStep { up: false }
                }
            }
            // Support missed: retail's single probe pass steps the frame the
            // contact is lost - fall immediately, no held tick.
            None => {
                state.mode = WalkMode::Airborne { lean };
                *up_y += FALL_STEP_FRAME_PROBE * lean;
                state.probe_measurement_y = FALL_STEP_FRAME_PROBE * lean;
                VerticalDecision::Airborne {
                    delta_y: FALL_STEP_FRAME_PROBE * lean,
                    lean,
                }
            }
        }
    }
}

/// The ceiling hold (authored kuluu law kept at record silence): any face
/// crossing the body-top slab `(feet_old + BODY_HEIGHT, feet_new + BODY_HEIGHT]`
/// in the column rejects a rise. MZB and closed-door faces both count; a
/// descent never trips it.
fn ceiling_blocks(
    geom: &MzbCollisionGeometry,
    doors: &[DoorObstacle],
    xz: Vec2,
    feet_old: f32,
    feet_new: f32,
) -> bool {
    sweep::ceiling_holds(geom, xz, feet_old, feet_new)
        || doors_in_column_slab(
            doors,
            xz,
            feet_old + BODY_HEIGHT - 1.0e-6,
            feet_new + BODY_HEIGHT,
        )
}

/// The movement axes as retail carries them into its displacement blend: `forward` along the aim
/// (negative is the backward component, which blends against heading minus a half turn) and `lateral`
/// across it, positive to the actor's right. Callers hand these in already normalized and gait-scaled by
/// [`consts::scaled_move_axes`].
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveAxes {
    pub forward: f32,
    pub lateral: f32,
}

/// The displacement half of retail's per-frame step - `.agents/skills/retail-observe/references/
/// 2026-10-25-walker-speed-and-step.md`, end of the "Speed law" section: forward and lateral components
/// are blended against heading with sine/cosine, the backward component read from heading minus a half
/// turn (the reversed aim) rather than as a negated forward vector, each term scaled by this tick's step.
/// Kuluu keeps facing as the wire's 256-step byte, so `aim` and `right` are that byte's unit vectors; the
/// blend lives here so nothing upstream pre-scales terms itself.
pub fn heading_blend(step_len: f32, axes: MoveAxes, aim: Vec2, right: Vec2) -> Vec2 {
    let forward_axis = axes.forward.max(0.0);
    let backward_axis = -(axes.forward.min(0.0));
    let along_aim = aim * forward_axis - aim * backward_axis;
    (along_aim + right * axes.lateral) * step_len
}

/// Measure the pose against the wall class into a retail-shaped answer: the
/// horizontal pair is penetration depth along each axis of the nearest face
/// (zero when clear); vertical belongs to accumulation semantics and a wall
/// probe answers nothing there.
fn probe_answer(walls: &AuthoredWalls<'_>, center: Vec3, feet_y: f32, r: f32) -> ResultVector {
    match walls.nearest_wall(center, r, feet_y) {
        Some((d_sq, n)) => {
            let intr = (r - d_sq.sqrt()).max(0.0);
            ResultVector {
                lateral_x: intr * n.x.abs(),
                vertical: 0.0,
                lateral_z: intr * n.z.abs(),
                word4_unconsumed: 0.0,
            }
        }
        None => ResultVector::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Quat;
    use bevy::prelude::Transform;
    use kuluu_render::dat_mzb::{
        build_collision_geometry, MzbCollisionBlock, MzbInstance, MzbSubMesh, ZONE_SLOT_MAIN,
    };

    fn geom_with(floors: &[(f32, f32)], cx: f32, cz: f32) -> MzbCollisionGeometry {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let mut tri_normals = Vec::new();
        for (y, half) in floors {
            let base = positions.len() as u32;
            positions.extend_from_slice(&[
                Vec3::new(cx - half, *y, cz - half),
                Vec3::new(cx + half, *y, cz - half),
                Vec3::new(cx + half, *y, cz + half),
                Vec3::new(cx - half, *y, cz + half),
            ]);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            tri_normals.extend([Vec3::Y; 2]);
        }
        let mut block = MzbCollisionBlock::default();
        block.positions = positions;
        block.indices = indices;
        block.tri_normals = tri_normals;
        MzbCollisionGeometry::from_block(block)
    }

    fn tick_once(geom: &MzbCollisionGeometry, st: &mut Walker, pos_z_up: f32) -> StepResult {
        let obstacles = ObstacleSet::default();
        step(
            geom,
            &obstacles,
            st,
            0.0,
            0.0,
            -pos_z_up,
            0.0,
            0.0,
            1.0,
            1.0 / 60.0,
            false,
            true,
        )
    }

    #[test]
    fn fall_step_is_the_recorded_frame_penalty() {
        // "The penalties" + "Lean ladder, closed" (collision record): primary
        // gate minus one point three world units per movement frame at neutral
        // lean magnitude exactly one; alternate carried as sourced data.
        assert_eq!(FALL_STEP_FRAME_PROBE, -1.3);
        assert_eq!(FALL_PENALTY_ALT_YAMLS_PER_FRAME, -0.5);
        assert_eq!(LEAN_DEFAULT_NEUTRAL, 1.0);
        // "Step reach and actor shape": the plus-two construction seed.
        assert_eq!(SHAPE_EXTENT_SEED_YAMLS, 2.0);
    }

    #[test]
    fn fall_from_a_ledge_lands_on_the_floor() {
        // Feet at +3 over a floor at 0: each airborne frame steps exactly one
        // penalty; landing comes on the frame contact enters reach and lands at
        // the measured height, never snapped further.
        let geom = geom_with(&[(0.0, 40.0)], 0.0, 0.0);
        let mut st = Walker {
            mode: WalkMode::Airborne {
                lean: LEAN_DEFAULT_NEUTRAL,
            },
            ..Default::default()
        }; // ledge exit already taken (support missed below)
        let mut z_wire = -3.0_f32; // y_up = +3
        let mut airborne_steps = 0usize;
        for frame in 0..16 {
            let obstacles = ObstacleSet::default();
            let prev_up = -z_wire;
            let res = step(
                &geom,
                &obstacles,
                &mut st,
                0.0,
                0.0,
                z_wire,
                0.0,
                0.0,
                1.0,
                1.0 / 60.0,
                false,
                true,
            );
            let up_now = -res.feet_z;
            match res.decision {
                VerticalDecision::Airborne { delta_y, .. } => {
                    airborne_steps += 1;
                    assert!(
                        (delta_y + 1.3).abs() < 1e-5,
                        "frame {frame}: expected one fall step of -1.3, got {delta_y}"
                    );
                    // No terminal speed: identical step deep into the drop, and
                    // the position actually moved by it.
                    assert!((up_now - prev_up - delta_y).abs() < 1e-5);
                }
                VerticalDecision::Landed => {
                    assert!(
                        up_now.abs() < 1e-4,
                        "landed at height {up_now}, expected the measured floor at 0"
                    );
                    break;
                }
                other => panic!("unexpected decision during fall: {:?}", other),
            }
            z_wire = res.feet_z;
        }
        // From +3: contact ends outside the window (2 frames of fall), landing
        // frame measures in: ceil(3/1.3) = 3 total frames under gravity law.
        assert_eq!(airborne_steps, 2);
        assert!(matches!(st.mode, WalkMode::Walking));
    }

    /// A staircase rising along bevy +x: tread i has its top at `y = rise*i`
    /// spanning `x in [depth*(i-1), depth*i]`, and the riser between tread i
    /// and tread i+1 stands at plane `x = depth*i` from `rise*i` to `rise*(i+1)`.
    /// Faces carry authored normals; the wall query's band rule decides which
    /// risers block.
    fn stair_geom(rise: f32, depth: f32, treads: usize) -> MzbCollisionGeometry {
        let mut block = MzbCollisionBlock::default();
        let mut quad = |p: [[f32; 3]; 4], n: Vec3| {
            let base = block.positions.len() as u32;
            for v in p {
                block.positions.push(Vec3::from_array(v));
            }
            block
                .indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            block.tri_normals.extend([n, n]);
        };
        let (z_lo, z_hi) = (-4.0f32, 4.0f32);
        for i in 0..treads {
            let y = rise * i as f32;
            let x_a = depth * (i as f32 - 1.0);
            let x_b = depth * i as f32;
            quad(
                [
                    [x_a, y, z_lo],
                    [x_b, y, z_lo],
                    [x_b, y, z_hi],
                    [x_a, y, z_hi],
                ],
                Vec3::Y,
            );
        }
        for i in 0..treads.saturating_sub(1) {
            let x = depth * i as f32;
            let y_lo = rise * i as f32;
            let y_hi = rise * (i + 1) as f32;
            quad(
                [
                    [x, y_lo, z_lo],
                    [x, y_lo, z_hi],
                    [x, y_hi, z_hi],
                    [x, y_hi, z_lo],
                ],
                Vec3::new(-1.0, 0.0, 0.0),
            );
        }
        MzbCollisionGeometry::from_block(block)
    }

    #[test]
    fn climbs_short_riser_stairs_like_ffxi_default_steps() {
        // Fine steps (~0.3): terrain risers under the follow-word band, ground
        // follow lifts while walking.
        let geom = stair_geom(0.3, 1.2, 4);
        let mut st = Walker::default();
        let (mut x, mut up) = (-1.0f32, 0.0f32);
        let mut held = false;
        for _tick in 0..160 {
            let obstacles = ObstacleSet::default();
            let res = step(
                &geom,
                &obstacles,
                &mut st,
                x,
                0.0,
                -up,
                0.12,
                0.0,
                6.0,
                1.0 / 60.0,
                false,
                true,
            );
            up = -res.feet_z;
            x += res.dx;
            held |= matches!(res.outcome, HorizontalOutcome::WallHold { .. });
            if x > 3.2 {
                break; // reached the last tread's middle; stop before its edge
            }
        }
        assert!(!held, "a band-suppressed riser blocked the sweep");
        assert!(
            up > 0.9 - 1e-2,
            "short-riser stair not climbed: height {up}, last x {x:.2}"
        );
    }

    #[test]
    fn walks_up_a_tall_riser_end_to_end() {
        // A riser well above the shipped staircase class (measured 0.5 - see
        // `walks_up_the_measured_shipped_sandoria_riser`): terrain must walk up it at the
        // rate bound, not standoff-block it, because the wall band suppresses any face
        // within one follow word. Swept horizontally AND lifted vertically through `step`,
        // unlike `rise_above_the_kuluu_band...` which probes only the contact path.
        let geom = stair_geom(0.9, 1.2, 3);
        let mut st = Walker::default();
        // Wire frame: bevy +x is wire +x; a body at height `up` sits at wire
        // z = -up.
        let (mut x, mut up) = (-1.0f32, 0.0f32);
        let mut trace = Vec::new();
        for tick in 0..160 {
            let obstacles = ObstacleSet::default();
            let res = step(
                &geom,
                &obstacles,
                &mut st,
                x,
                0.0,
                -up,
                0.12,
                0.0,
                6.0,
                1.0 / 60.0,
                false,
                true,
            );
            up = -res.feet_z;
            x += res.dx;
            trace.push((tick, x, up, res.outcome));
            if up > 1.7 {
                break; // two treads climbed
            }
        }
        let reached_top = up > 1.7 - 1e-3;
        if !reached_top {
            for (tick, x, up, o) in trace.iter().skip(trace.len().saturating_sub(30)) {
                println!("tick {tick}: x={x:.3} y_up={up:.3} outcome={o:?}");
            }
        }
        assert!(
            reached_top,
            "walker stalled on a 0.9-riser stair (last x {:.2}, height {:.2})",
            x, up
        );
    }

    #[test]
    fn walks_up_the_measured_shipped_sandoria_riser() {
        // Census of the shipped Northern San d'Oria collision (zone 231, record
        // `.agents/skills/retail-observe/references/2026-10-25-collision-query.md`, spec
        // section "Shipped-zone step classes"): three parallel flights near geom x=-200..-192,
        // z=-74.67/-78.67/-81.33, risers exactly 0.5 yalm tall on a 1.0-yalm run - the
        // staircase class the stall report was filed against, and taller than the old
        // 0.4 wall band that caused it. The replica must walk up without ever holding.
        let geom = stair_geom(0.5, 1.0, 4);
        let mut st = Walker::default();
        let (mut x, mut up) = (-1.0f32, 0.0f32);
        let mut held = false;
        for _tick in 0..240 {
            let obstacles = ObstacleSet::default();
            let res = step(
                &geom,
                &obstacles,
                &mut st,
                x,
                0.0,
                -up,
                0.12,
                0.0,
                6.0,
                1.0 / 60.0,
                false,
                true,
            );
            up = -res.feet_z;
            x += res.dx;
            held |= matches!(res.outcome, HorizontalOutcome::WallHold { .. });
            if x > 2.9 {
                break; // three risers crossed
            }
        }
        assert!(!held, "the shipped 0.5-riser class standoff-blocked");
        assert!(
            (up - 1.5).abs() < 1e-2,
            "measured-class stair not climbed: height {up} at x {x:.2}"
        );
    }

    #[test]
    fn published_measurement_is_what_height_moved_by() {
        // The measurement-feed rule: the tick publishes exactly what height
        // did, in every vertical branch (fall, landing, ground follow, flat).
        let geom = geom_with(&[(0.0, 40.0)], 0.0, 0.0);
        let mut st = Walker {
            mode: WalkMode::Airborne {
                lean: LEAN_DEFAULT_NEUTRAL,
            },
            ..Default::default()
        };
        let mut z_wire = -2.6_f32; // y_up = +2.6
        for frame in 0..16 {
            let obstacles = ObstacleSet::default();
            let prev_up = -z_wire;
            let res = step(
                &geom,
                &obstacles,
                &mut st,
                0.0,
                0.0,
                z_wire,
                0.0,
                0.0,
                1.0,
                1.0 / 60.0,
                false,
                true,
            );
            let moved = (-res.feet_z) - prev_up;
            assert!(
                (st.probe_measurement_y - moved).abs() < 1e-4,
                "frame {frame}: published {} but height moved {moved}",
                st.probe_measurement_y
            );
            if matches!(res.decision, VerticalDecision::Landed) {
                break;
            }
            z_wire = res.feet_z;
        }
        // Settled on the floor: a flat grounded tick publishes zero.
        let obstacles = ObstacleSet::default();
        let res = step(
            &geom,
            &obstacles,
            &mut st,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            1.0 / 60.0,
            false,
            true,
        );
        assert!(matches!(res.decision, VerticalDecision::Flat));
        let _moved = -res.feet_z;
        assert!(st.probe_measurement_y.abs() < 1e-4);
    }

    #[test]
    fn rise_above_the_kuluu_band_within_one_follow_word_is_accepted() {
        // "Step acceptance is a rate bound ... no maximum-rise cap": a riser of
        // 0.8 - double kuluu's authored 0.4 band - within one follow word is
        // walked up at <= FOLLOW_WORD per frame, never refused.
        assert!((STEP_MAX - 0.4).abs() < 1e-6);
        let geom = geom_with(&[(0.9, 40.0)], 0.0, 0.0);
        let mut st = Walker::default();
        let res = tick_once(&geom, &mut st, 0.1);
        match res.decision {
            VerticalDecision::SingleStep { up } => assert!(up),
            other => panic!(
                "rise of 0.8 above the {:.1} band refused: {:?}",
                STEP_MAX, other
            ),
        }
        let after = -res.feet_z;
        // Column-ray hit heights carry ~1e-4 noise by construction
        // (`dat_mzb STEP_UP_REACH_EPSILON` rationale).
        assert!(
            (after - 0.9).abs() < 1e-3,
            "expected merge to the riser top: feet_y=0.1 h=0.9, landed at {after}"
        );
    }

    #[test]
    fn vertical_change_never_exceeds_one_follow_word_per_frame() {
        // Floor far above: outside the containment window, no contact, no
        // teleport. Just inside the window: one frame closes at most FOLLOW_WORD.
        assert_eq!(FOLLOW_WORD_YAMLS_PER_FRAME, 1.3);
        let geom = geom_with(&[(10.0, 40.0)], 0.0, 0.0);
        let mut st = Walker::default();
        let res = tick_once(&geom, &mut st, 0.0);
        assert!(matches!(res.decision, VerticalDecision::Airborne { .. }));
        let mut st2 = Walker::default();
        let start_up = 10.0 - FOLLOW_WORD_YAMLS_PER_FRAME + 0.05;
        let res2 = tick_once(&geom, &mut st2, start_up);
        match res2.decision {
            VerticalDecision::SingleStep { up } => assert!(up),
            other => panic!("rise within window refused: {:?}", other),
        }
        let moved = (-res2.feet_z) - start_up;
        assert!((moved - 1.25).abs() < 1e-4);
    }

    #[test]
    fn wall_height_blocks_horizontally_through_the_authored_class() {
        // A tall vertical face (authored blocker class) stops the horizontal
        // move; terrain faces never do this to a walker.
        // Two-triangle rectangle so every body-chain height sees full wall.
        let positions = vec![
            Vec3::new(-20.0, -2.0, -20.0),
            Vec3::new(20.0, -2.0, -20.0),
            Vec3::new(20.0, 8.0, -20.0),
            Vec3::new(-20.0, 8.0, -20.0),
        ];
        let indices = vec![0u32, 1, 2, 0, 2, 3];
        let tri_normals = vec![Vec3::new(0.0, 0.0, 1.0); 2];
        let mut block = MzbCollisionBlock::default();
        block.positions = positions;
        block.indices = indices;
        block.tri_normals = tri_normals;
        let geom = MzbCollisionGeometry::from_block(block);
        let mut st = Walker::default();
        let obstacles = ObstacleSet::default();
        // Body already within reach of the face (feet_xz=(x,-y): wire y=+19.8
        // sits at bevy z=-19.8, 0.2 from the plane at -20, inside the body
        // radius). Pushing 2.0 into it (+dy -> negative bevy z) must never
        // report a clean move.
        let res = step(
            &geom,
            &obstacles,
            &mut st,
            0.0,
            19.8,
            0.0,
            0.0,
            2.0,
            1.0,
            1.0 / 60.0,
            false,
            true,
        );
        match res.outcome {
            HorizontalOutcome::Slid { .. } | HorizontalOutcome::WallHold { .. } => {}
            other => panic!("wall did not block: {:?}", other),
        }
    }

    #[test]
    fn slope_hold_tracks_ground_at_the_follow_rate() {
        // Stair-as-slope: measured floor rises 0.3/tick; feet must track every
        // tick within the rate bound and never flick airborne (slope hold).
        let mut st = Walker::default();
        for i in 0..12 {
            let y = i as f32 * 0.3;
            let geom = geom_with(&[(y + 0.3, 40.0)], 0.0, 0.0);
            let res = tick_once(&geom, &mut st, y);
            assert!(
                !matches!(res.decision, VerticalDecision::Airborne { .. }),
                "tick {i} flicked airborne on a floor rising to {}",
                y + 0.3
            );
            let moved = (-res.feet_z) - y;
            assert!((moved - 0.3).abs() < 1e-4);
        }
    }

    #[test]
    fn margin_uses_the_recorded_scale_and_skips_vertical() {
        // "the margin wrapper skips the accumulated vertical word ...":
        // nought point eight times the length of the horizontal pair only.
        let a = ResultVector {
            lateral_x: 0.5,
            vertical: -99.0,
            lateral_z: 0.25,
            word4_unconsumed: 7.0,
        };
        assert!((a.margin() - 0.8 * (0.5_f32.powi(2) + 0.25f32.powi(2)).sqrt()).abs() < 1e-6);
        assert_eq!(PROBE_MARGIN_SCALE, 0.8);
    }

    #[test]
    fn camera_aim_height_adds_the_vertical_word_directly() {
        // "Word two - vertical accumulation; moves both player and camera":
        // straight add into the unit's own accumulator.
        let a = ResultVector {
            lateral_x: 0.0,
            vertical: -1.3,
            lateral_z: 0.0,
            word4_unconsumed: 0.0,
        };
        assert!((a.apply_to_aim_height(5.0) - 3.7).abs() < 1e-6);
    }

    #[test]
    fn no_geometry_holds_instead_of_falling() {
        // Zone geometry absent: gravity off (same guard as the column walker).
        let geom = MzbCollisionGeometry::default();
        let mut st = Walker::default();
        let obstacles = ObstacleSet::default();
        let res = step(
            &geom,
            &obstacles,
            &mut st,
            0.0,
            0.0,
            -3.0,
            0.0,
            0.0,
            1.0,
            1.0 / 60.0,
            false,
            false,
        );
        assert!(matches!(res.decision, VerticalDecision::NoGeometry));
        assert!((res.feet_z + 3.0).abs() < 1e-9);
    }

    /// Single up-facing quad at local y=0 spanning ±half in xz.
    fn floor_quad_submesh(half: f32) -> MzbSubMesh {
        MzbSubMesh {
            positions: vec![
                [-half, 0.0, -half],
                [half, 0.0, -half],
                [half, 0.0, half],
                [-half, 0.0, half],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            tri_terrain: vec![0, 0],
            tri_normal: vec![[0.0, 1.0, 0.0]; 2],
            tri_camera_transparent: vec![false; 2],
            flags: 1,
        }
    }

    /// Axis-aligned box with its footprint centered on the local origin and
    /// its base in y=0. No underside: every wall probe's chain center sits at
    /// least BODY_RADIUS above the floor, so the riser-band rule and the
    /// column filter answer for it without an authored face.
    fn box_submesh(w: f32, h: f32, d: f32) -> MzbSubMesh {
        let (x, z) = (w / 2.0, d / 2.0);
        let v: [[f32; 3]; 8] = [
            [-x, 0.0, -z],
            [x, 0.0, -z],
            [x, 0.0, z],
            [-x, 0.0, z],
            [-x, h, -z],
            [x, h, -z],
            [x, h, z],
            [-x, h, z],
        ];
        let positions = v.to_vec();
        let indices = vec![
            0, 1, 5, 0, 5, 4, // -z side
            2, 3, 7, 2, 7, 6, // +z side
            3, 0, 4, 3, 4, 7, // -x side
            1, 2, 6, 1, 6, 5, // +x side
            4, 5, 6, 4, 6, 7, // top
        ];
        let tri_normal = vec![
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        MzbSubMesh {
            positions,
            indices,
            tri_terrain: vec![0; 10],
            tri_normal,
            tri_camera_transparent: vec![false; 10],
            flags: 1,
        }
    }

    fn placed(submesh_idx: usize, translation: [f32; 3], yaw: Option<f32>) -> MzbInstance {
        let mut t = Transform::from_xyz(translation[0], translation[1], translation[2]);
        if let Some(y) = yaw {
            t.rotation = Quat::from_rotation_y(y);
        }
        MzbInstance {
            submesh_idx,
            bevy_transform: t,
            water_height_bevy: None,
            lighting: None,
            sub_area_link: 0,
        }
    }

    fn baked(submeshes: &[MzbSubMesh], instances: &[MzbInstance]) -> MzbCollisionGeometry {
        MzbCollisionGeometry::from_block(build_collision_geometry(submeshes, instances, None))
    }

    /// One tick with positions expressed directly in bevy space (feet point,
    /// horizontal displacement), mapping through the wire convention exactly
    /// like dispatch does.
    fn tick_bevy(
        geom: &MzbCollisionGeometry,
        obstacles: &ObstacleSet,
        st: &mut Walker,
        feet: Vec3,
        disp: Vec2,
    ) -> StepResult {
        step(
            geom,
            obstacles,
            st,
            feet.x,
            -feet.z,
            -feet.y,
            disp.x,
            -disp.y,
            1.0,
            1.0 / 60.0,
            false,
            true,
        )
    }

    #[test]
    fn closed_door_floor_grounds_the_walker() {
        // The door floor source carried over from the pre-retail tick: a
        // closed door's up-facing deck is a floor for the column probe (a
        // drawbridge; an elevator platform), not only a wall.
        let geom = MzbCollisionGeometry::default();
        let deck_y = 1.2;
        let a = Vec3::new(-0.6, deck_y, -0.6);
        let b = Vec3::new(0.6, deck_y, -0.6);
        let c = Vec3::new(0.6, deck_y, 0.6);
        let e = Vec3::new(-0.6, deck_y, 0.6);
        let mut obstacles = ObstacleSet::default();
        obstacles.doors.push(DoorObstacle {
            tris: vec![([a, b, c], Vec3::Y), ([a, c, e], Vec3::Y)],
            min: Vec3::new(-0.6, deck_y - 0.1, -0.6),
            max: Vec3::new(0.6, deck_y, 0.6),
        });
        let mut st = Walker::default();
        let res = tick_bevy(
            &geom,
            &obstacles,
            &mut st,
            Vec3::new(0.0, 0.9, 0.0),
            Vec2::ZERO,
        );
        match res.decision {
            VerticalDecision::SingleStep { up } => assert!(up),
            other => panic!("deck floor not answered: {other:?}"),
        }
        let after = -res.feet_z;
        assert!(
            (after - deck_y).abs() < 1e-3,
            "expected merge onto the posed deck at {deck_y}, got {after}"
        );
    }

    #[test]
    fn actor_contact_drops_the_move_until_its_budget_expires() {
        // ControllableActor::CheckContactActor through Walker's ported budget:
        // first overlapping tick drops the move; sustained input walks through
        // after CONTACT_BLOCK_TICKS, and a fresh approach gets a fresh budget.
        let floor = floor_quad_submesh(20.0);
        let geom = baked(&[floor], &[placed(0, [0.0, 0.0, 0.0], None)]);
        let mut obstacles = ObstacleSet::default();
        obstacles.mobs.push(MobObstacle {
            id: 7,
            center: Vec2::new(2.0, 0.0),
            radius: 0.5,
        });
        let mut st = Walker::default();
        // Projected (1.6, z0) sits inside the mob circle through BODY_RADIUS.
        let first = tick_bevy(
            &geom,
            &obstacles,
            &mut st,
            Vec3::new(1.2, 0.0, 0.0),
            Vec2::new(0.4, 0.0),
        );
        match first.outcome {
            HorizontalOutcome::ActorContact { mob, .. } => assert_eq!(mob, 7),
            other => panic!("overlap must be ActorContact, got {other:?}"),
        }
        // The move was dropped: dx is the zeroed displacement.
        assert!((first.dx).abs() < 1e-6 && first.dy.abs() < 1e-6);
        let mut allowed = None;
        for tick in 0..240 {
            let res = tick_bevy(
                &geom,
                &obstacles,
                &mut st,
                Vec3::new(1.2, 0.0, 0.0),
                Vec2::new(0.4, 0.0),
            );
            if !matches!(res.outcome, HorizontalOutcome::ActorContact { .. }) {
                allowed = Some(tick);
                break;
            }
        }
        let Some(tick) = allowed else {
            panic!("budget never expired: still blocked after 240 ticks");
        };
        // CONTACT_BLOCK_TICKS = 30 decremented at 60/s -> release near tick 30.
        assert!((15..=45).contains(&tick), "released at tick {tick}");
    }

    #[test]
    fn planter_blocks_walk_move() {
        // A dedicated placed blocker object stays solid through the bake even though its
        // side faces stand within one follow word of the feet: the prop identity from
        // `build_collision_geometry` (record spec sections "Face classification" and
        // "Blocking belongs to other candidates") keeps it answering wall queries while
        // terrain-law risers of that height never do.
        let floor = floor_quad_submesh(20.0);
        let planter = box_submesh(0.8, 0.7, 0.8);
        let geom = baked(
            &[floor, planter],
            &[
                placed(0, [0.0, 0.0, 0.0], None),
                placed(1, [4.0, 0.0, 0.0], None),
            ],
        );
        let flags = &geom.block(ZONE_SLOT_MAIN).tri_prop_blocker;
        assert_eq!(flags.len(), 12);
        assert!(
            !flags[..2].iter().any(|f| *f) && flags[2..].iter().all(|f| *f),
            "the bake lost the planter's blocker identity (or flagged the ground)"
        );
        let obstacles = ObstacleSet::default();
        let mut st = Walker::default();
        let res = tick_bevy(
            &geom,
            &obstacles,
            &mut st,
            Vec3::new(2.3, 0.0, 0.0),
            Vec2::new(1.6, 0.0),
        );
        match res.outcome {
            HorizontalOutcome::Slid { .. } | HorizontalOutcome::WallHold { .. } => {}
            other => panic!("planter did not block: {other:?}"),
        }
        let stop_x = 2.3 + res.dx;
        assert!(
            stop_x > 3.0,
            "stopped at {stop_x}, never approached the face"
        );
        // Front face x=3.6 minus the body standoff (BODY_RADIUS): never closer.
        assert!(
            stop_x < 3.6 - BODY_RADIUS + 0.05,
            "walked into the planter: {stop_x}"
        );
    }

    #[test]
    fn rotated_fence_blocks_on_its_rotated_face() {
        // The placement pose reaches the queries: probes and blocks follow the
        // fence's rotated faces, not its AABB or unrotated footprint.
        let yaw = std::f32::consts::FRAC_PI_4;
        let rot = Quat::from_rotation_y(yaw);
        let floor = floor_quad_submesh(20.0);
        let fence = box_submesh(6.0, 1.4, 0.2);
        let geom = baked(
            &[floor, fence],
            &[
                placed(0, [0.0, 0.0, 0.0], None),
                placed(1, [10.0, 0.0, 10.0], Some(yaw)),
            ],
        );
        // Probe 0.4 along the rotated -z normal: a wall contact with exactly
        // that face's rotated authored normal.
        let c1_off = rot * Vec3::new(0.0, 0.7, -0.4);
        let hit = geom.nearest_wall_contact(Vec3::new(10.0, 0.7, 10.0) + c1_off, BODY_RADIUS, 0.0);
        let Some((d2, n)) = hit else {
            panic!(
                "no contact against the rotated fence face at ({:?})",
                Vec3::new(10.0, 0.7, 10.0) + c1_off
            );
        };
        assert!(
            ((d2.sqrt() - 0.3).abs()) < 0.05,
            "contact distance {:.4}, expected ~0.3",
            d2.sqrt()
        );
        let want = rot * Vec3::new(0.0, 0.0, -1.0);
        assert!(
            n.dot(want) > 0.99,
            "face normal {n:?}, expected rotated {want:?}"
        );
        // Along the long axis the slab is thin: a point 2.8 from center on
        // that side sees no wall at all (the unrotated box would have had one).
        assert!(
            geom.nearest_wall_contact(Vec3::new(12.8, 0.7, 10.0), BODY_RADIUS, 0.0)
                .is_none(),
            "phantom wall off the fence's long axis"
        );
        // End to end: walking +x through the fence line is clipped by the
        // diagonal face and the body stays outside the posed slab.
        let obstacles = ObstacleSet::default();
        let mut st = Walker::default();
        let res = tick_bevy(
            &geom,
            &obstacles,
            &mut st,
            Vec3::new(7.6, 0.0, 10.0),
            Vec2::new(3.0, 0.0),
        );
        match res.outcome {
            HorizontalOutcome::Slid { .. } | HorizontalOutcome::WallHold { .. } => {}
            other => panic!("rotated fence did not block: {other:?}"),
        }
        assert!(
            res.dx > 0.8 && res.dx < 2.4,
            "clip distance {:.3} off the diagonal stop band",
            res.dx
        );
        let final_xz = Vec2::new(7.6 + res.dx, 10.0 - res.dy);
        let local: Vec3 = rot.inverse() * Vec3::new(final_xz.x - 10.0, 0.0, final_xz.y - 10.0);
        assert!(
            local.z.abs() > 0.42,
            "body inside the posed slab: local {local:?}"
        );
    }

    #[test]
    fn crate_top_is_standable_at_its_posed_height() {
        // A placed object's top grounds the feet at its COMPOSED height -
        // translation plus local extent, read from the same bake the renderer
        // and every column query use.
        let floor = floor_quad_submesh(20.0);
        let crate_ = box_submesh(1.2, 0.7, 1.2);
        let geom = baked(
            &[floor, crate_],
            &[
                placed(0, [0.0, 0.0, 0.0], None),
                // Local top y=0.7 composed with translation y=0.3: standable
                // at exactly 1.0 - not the local 0.7 an unbaked record would give.
                placed(1, [6.0, 0.3, 6.0], None),
            ],
        );
        let obstacles = ObstacleSet::default();
        let mut st = Walker::default();
        let res = tick_bevy(
            &geom,
            &obstacles,
            &mut st,
            Vec3::new(6.0, 0.9, 6.0),
            Vec2::ZERO,
        );
        match res.decision {
            VerticalDecision::SingleStep { up } => assert!(up),
            other => panic!("crate top not standable: {other:?}"),
        }
        let after = -res.feet_z;
        assert!(
            (after - 1.0).abs() < 1e-3,
            "expected the posed crate top at 1.0, got {after}"
        );
        // The camera BVH is built through `trimesh_data` of the same block
        // (collision_bvh.rs), so the pull-in stops at props by construction:
        // the posed face is in that mirror too.
        let (positions, _) = geom.trimesh_data();
        assert!(
            positions
                .iter()
                .any(|p| (p.y - 1.0).abs() < 1e-3 && p.x > 5.4),
            "posed crate top missing from the mirrored triangle set"
        );
    }

    /// A floor quad at y=0.5 under the ray: hit; a vertical wall face: miss.
    #[test]
    fn tri_hits_column_finds_horizontal_faces_only() {
        let floor = [
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(1.0, 0.5, 0.0),
            Vec3::new(0.0, 0.5, 1.0),
        ];
        assert_eq!(tri_hits_column(floor, Vec2::new(0.4, 0.4)), Some(0.5));
        assert_eq!(tri_hits_column(floor, Vec2::new(2.0, 0.4)), None);

        let wall = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
        ];
        assert_eq!(tri_hits_column(wall, Vec2::new(1.0, 0.5)), None);
    }

    #[test]
    fn tri_hits_column_interpolates_sloped_faces_in_either_winding() {
        let triangle = [
            Vec3::new(4.0, 10.0, 6.0),
            Vec3::new(6.0, 12.0, 6.0),
            Vec3::new(4.0, 14.0, 8.0),
        ];
        let query = Vec2::new(4.5, 6.5);
        assert_eq!(tri_hits_column(triangle, query), Some(11.5));
        assert_eq!(
            tri_hits_column([triangle[0], triangle[2], triangle[1]], query),
            Some(11.5)
        );
        assert_eq!(tri_hits_column(triangle, Vec2::new(6.0, 8.0)), None);
    }

    /// A slab crossing (feet+BODY_HEIGHT, y_new+BODY_HEIGHT] holds; one
    /// above the band or a descent does not. The positive case is a door leaf
    /// with a horizontal lintel at y=2.0 over the column.
    #[test]
    fn ceiling_slab_rejects_only_rises_into_geometry() {
        let doors: &[DoorObstacle] = &[];
        assert!(!doors_in_column_slab(doors, Vec2::ZERO, 1.8, 2.1));

        let lintel = [
            Vec3::new(-1.0, 2.0, -1.0),
            Vec3::new(1.0, 2.0, -1.0),
            Vec3::new(0.0, 2.0, 1.0),
        ];
        let n = (lintel[1] - lintel[0])
            .cross(lintel[2] - lintel[0])
            .normalize();
        let door = DoorObstacle {
            tris: vec![(lintel, n)],
            min: Vec3::new(-1.0, 2.0, -1.0),
            max: Vec3::new(1.0, 2.0, 1.0),
        };
        let doors = [door];
        assert!(doors_in_column_slab(&doors, Vec2::ZERO, 1.8, 2.1));
        assert!(!doors_in_column_slab(&doors, Vec2::ZERO, 2.1, 2.4));
    }

    /// The blend's three terms at pure values: full forward walks the aim,
    /// a backward axis displaces along the reversed aim (heading minus a half
    /// turn), and lateral runs across it; each term carries this tick's step.
    #[test]
    fn heading_blend_places_each_axis_on_its_own_heading_term() {
        let aim = Vec2::new(1.0, 0.0);
        let right = Vec2::new(0.0, 1.0);

        let d = heading_blend(
            0.5,
            MoveAxes {
                forward: 1.0,
                lateral: 0.0,
            },
            aim,
            right,
        );
        assert!((d - Vec2::new(0.5, 0.0)).length() < 1e-6);

        let d = heading_blend(
            0.4,
            MoveAxes {
                forward: -1.0,
                lateral: 0.0,
            },
            aim,
            right,
        );
        assert!((d - Vec2::new(-0.4, 0.0)).length() < 1e-6);

        let d = heading_blend(
            1.0,
            MoveAxes {
                forward: 0.0,
                lateral: 0.6,
            },
            aim,
            right,
        );
        assert!((d - Vec2::new(0.0, 0.6)).length() < 1e-6);

        let s = std::f32::consts::FRAC_1_SQRT_2;
        let d = heading_blend(
            0.5,
            MoveAxes {
                forward: s,
                lateral: -s,
            },
            aim,
            right,
        );
        assert!((d.length() - 0.5).abs() < 1e-6);
        assert!((d - Vec2::new(0.5 * s, -0.5 * s)).length() < 1e-6);
    }
}
