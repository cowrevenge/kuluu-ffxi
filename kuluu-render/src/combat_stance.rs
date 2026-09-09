use std::collections::{HashMap, VecDeque};
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};

use bevy::prelude::*;
use ffxi_dat::anim::Mo2Animation;
use ffxi_dat::{walk, ChunkKind, DatRoot};

use crate::components::{IsSelf, WorldEntity};
use crate::snapshot::SceneState;
use kuluu_snapshot::EntityKind;

pub fn motion_dat_for_skel(skel_file_id: u32) -> Option<u32> {
    match skel_file_id {
        7072 => Some(9672),
        10248 => Some(12848),
        13424 => Some(16024),
        16600 => Some(19200),
        19776 => Some(22376),
        23176 => Some(25776),
        26352 => Some(28952),
        _ => None,
    }
}

static BATTLE_IDLE_ANIMS: OnceLock<Mutex<HashMap<u32, Option<Arc<Mo2Animation>>>>> =
    OnceLock::new();

static RUN_ANIMS: OnceLock<Mutex<HashMap<u32, Option<Arc<Mo2Animation>>>>> = OnceLock::new();

static SIT_ANIMS: OnceLock<Mutex<HashMap<u32, Option<Arc<Mo2Animation>>>>> = OnceLock::new();
static HEAL_ANIMS: OnceLock<Mutex<HashMap<u32, Option<Arc<Mo2Animation>>>>> = OnceLock::new();

static COMBAT_RUN_ANIMS: OnceLock<Mutex<HashMap<u32, Option<Arc<Mo2Animation>>>>> = OnceLock::new();

static DIRECTIONAL_ANIMS: OnceLock<Mutex<HashMap<(u32, [u8; 3]), Option<Arc<Mo2Animation>>>>> =
    OnceLock::new();

const BATTLE_IDLE_PREFIX: &[u8; 3] = b"btl";

pub fn battle_idle_anim_for_skel(skel_file_id: u32) -> Option<Arc<Mo2Animation>> {
    let motion_dat = motion_dat_for_skel(skel_file_id)?;
    let map = BATTLE_IDLE_ANIMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().ok()?;
    if let Some(entry) = guard.get(&motion_dat) {
        return entry.clone();
    }
    let loaded = load_battle_idle(motion_dat).map(Arc::new);
    guard.insert(motion_dat, loaded.clone());
    loaded
}

fn load_battle_idle(motion_dat_id: u32) -> Option<Mo2Animation> {
    load_anim_with_prefix(motion_dat_id, BATTLE_IDLE_PREFIX)
}

pub fn run_anim_for_skel(skel_file_id: u32) -> Option<Arc<Mo2Animation>> {
    let map = RUN_ANIMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().ok()?;
    if let Some(entry) = guard.get(&skel_file_id) {
        return entry.clone();
    }
    let loaded = load_anim_with_prefix(skel_file_id, b"run").map(Arc::new);
    guard.insert(skel_file_id, loaded.clone());
    loaded
}

pub fn combat_run_anim_for_skel(skel_file_id: u32) -> Option<Arc<Mo2Animation>> {
    let motion_dat = motion_dat_for_skel(skel_file_id)?;
    let map = COMBAT_RUN_ANIMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().ok()?;
    if let Some(entry) = guard.get(&motion_dat) {
        return entry.clone();
    }
    let loaded = load_anim_with_prefix(motion_dat, b"run").map(Arc::new);
    guard.insert(motion_dat, loaded.clone());
    loaded
}

pub fn sit_anim_for_skel(skel_file_id: u32) -> Option<Arc<Mo2Animation>> {
    let map = SIT_ANIMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().ok()?;
    if let Some(entry) = guard.get(&skel_file_id) {
        return entry.clone();
    }
    let loaded = load_anim_with_prefix(skel_file_id, b"sit").map(Arc::new);
    guard.insert(skel_file_id, loaded.clone());
    loaded
}

pub fn heal_anim_for_skel(skel_file_id: u32) -> Option<Arc<Mo2Animation>> {
    let map = HEAL_ANIMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().ok()?;
    if let Some(entry) = guard.get(&skel_file_id) {
        return entry.clone();
    }
    let loaded = load_anim_with_prefix(skel_file_id, b"hea").map(Arc::new);
    guard.insert(skel_file_id, loaded.clone());
    loaded
}

#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct RestStance {
    pub kind: RestKind,
    pub exit: RestExit,
}

/// Retail charges you for standing up: cancelling a rest plays the stand-up
/// clip first, and the character only starts moving if the movement keys are
/// still held when it finishes. Movement stays suppressed for the whole phase
/// instead of sliding out from under the animation.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum RestExit {
    #[default]
    Idle,

    Pending {
        grace: f32,
    },

    Playing,
}

impl RestExit {
    /// A self actor with no rest clips — missing DATs, or the actor not spawned
    /// yet — never reaches `Playing`, so the bridge from the input cancel to the
    /// pose machine's Out phase has to expire on its own.
    pub const HANDOFF_SECS: f32 = 0.25;
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub enum RestKind {
    #[default]
    None,

    Sit,

    Heal,
}

impl RestStance {
    pub fn is_resting(&self) -> bool {
        !matches!(self.kind, RestKind::None)
    }

    pub fn begin_exit(&mut self) {
        self.kind = RestKind::None;
        self.exit = RestExit::Pending {
            grace: RestExit::HANDOFF_SECS,
        };
    }

    pub fn exit_blocks_movement(&mut self, dt: f32) -> bool {
        match &mut self.exit {
            RestExit::Idle => false,
            RestExit::Playing => true,
            RestExit::Pending { grace } => {
                *grace -= dt;
                if *grace <= 0.0 {
                    self.exit = RestExit::Idle;
                    false
                } else {
                    true
                }
            }
        }
    }

    pub fn observe_exit_clip(&mut self, playing: bool) {
        self.exit = match (self.exit, playing) {
            (_, true) => RestExit::Playing,
            (RestExit::Playing, false) => RestExit::Idle,
            (other, false) => other,
        };
    }
}

/// Retail's rest is server-owned: it starts and ends on the server's terms
/// (damage, status effects, zoning), and the answer comes back as the 0x037
/// animation byte. The local stance is an optimistic prediction, so it gets
/// this long to be confirmed — the 0x0E8 camp leaves on the client's 200 ms
/// datagram cadence and the server's reply rides the following update — before
/// the server byte wins.
pub const SELF_REST_ACK_SECS: f32 = 1.0;

pub fn reconcile_rest_kind(local: RestKind, server_status: u8) -> Option<RestKind> {
    let server_healing = server_status == ffxi_proto::decode::animation::HEALING;
    match (local, server_healing) {
        (RestKind::Heal, false) => Some(RestKind::None),
        // Sitting never appears here: /sit is a client-side pose that sends no
        // packet, so a 0 byte must not stand the player up.
        (k, true) if k != RestKind::Heal => Some(RestKind::Heal),
        _ => None,
    }
}

pub fn reconcile_self_rest_stance_system(
    time: Res<Time>,
    state: Res<SceneState>,
    mut rest: ResMut<RestStance>,
    mut prev_local: Local<RestKind>,
    mut ack_grace: Local<f32>,
) {
    if *prev_local != rest.kind {
        *prev_local = rest.kind;
        *ack_grace = SELF_REST_ACK_SECS;
        return;
    }
    if *ack_grace > 0.0 {
        *ack_grace -= time.delta_secs();
        return;
    }
    if !matches!(state.snapshot.stage, kuluu_snapshot::Stage::InZone) {
        return;
    }
    if let Some(next) = reconcile_rest_kind(rest.kind, state.snapshot.self_server_status) {
        rest.kind = next;
        *prev_local = next;
    }
}

#[derive(Resource, Default, Debug, Clone, Copy, Eq, PartialEq)]
pub struct WalkMode {
    pub walking: bool,
}

impl WalkMode {
    pub const WALK_SCALE: f32 = 0.25;

    pub fn scale(self) -> f32 {
        if self.walking {
            Self::WALK_SCALE
        } else {
            1.0
        }
    }
}

/// Whether the self character's movement keys are held this tick, written by
/// the client's movement dispatch. While keys are what move the player, the
/// self pose reads this instead of inferring motion from transform deltas:
/// prediction reconcile keeps nudging the rendered transform, so inferred speed
/// can hover above `MOVE_EXIT` and hold the run cycle after the keys are
/// released. Not authoritative while a reactor goal (follow/goto/engage) moves
/// the player with no keys held — the pose falls back to inference there.
///
/// `forward`/`strafe` are character-frame intent components feeding directional
/// gait selection (mvb/mvl/mvr). They are only ever non-(1,0) while locked on —
/// matching retail, where unlocked movement steers the character into the run
/// direction (run/wlk gait only) and directional gait exists only under lock-on.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct SelfMoveIntent {
    pub moving: bool,
    pub forward: f32,
    pub strafe: f32,
}

pub fn directional_anim_for_skel(skel_file_id: u32, prefix: &[u8; 3]) -> Option<Arc<Mo2Animation>> {
    let map = DIRECTIONAL_ANIMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().ok()?;
    let key = (skel_file_id, *prefix);
    if let Some(entry) = guard.get(&key) {
        return entry.clone();
    }
    let loaded = load_anim_with_prefix(skel_file_id, prefix).map(Arc::new);
    guard.insert(key, loaded.clone());
    loaded
}

pub fn load_anim_with_prefix(file_id: u32, prefix: &[u8; 3]) -> Option<Mo2Animation> {
    let root = DatRoot::from_env_or_default().ok()?;
    let loc = root.resolve(file_id).ok()?;
    let bytes = fs::read(loc.path_under(&root)).ok()?;
    for chunk in walk(&bytes).filter_map(Result::ok) {
        if ChunkKind::from_u8(chunk.kind) != Some(ChunkKind::AnimMo2) {
            continue;
        }
        let name_prefix = &chunk.name[..3];
        if name_prefix.eq_ignore_ascii_case(prefix) {
            if let Ok(anim) = ffxi_dat::anim::parse_mo2(chunk.data, &chunk.name) {
                return Some(anim);
            }
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ClipId {
    Idle,
    BattleIdle,

    Run,

    CombatRun,

    Backpedal,

    StrafeLeft,
    StrafeRight,

    TurnInPlace,

    Walk,
}

#[derive(Clone, Copy, Debug)]
pub struct AnimationBlend {
    pub from_clip: ClipId,
    pub to_clip: ClipId,

    pub t: f32,

    pub duration: f32,
}

#[derive(Resource, Default)]
pub struct AnimationBlends {
    pub by_id: HashMap<u32, AnimationBlend>,
}

impl AnimationBlends {
    pub const DEFAULT_DURATION: f32 = 0.15;

    pub fn update(&mut self, id: u32, current: ClipId, dt: f32) {
        match self.by_id.get_mut(&id) {
            None => {
                self.by_id.insert(
                    id,
                    AnimationBlend {
                        from_clip: current,
                        to_clip: current,
                        t: 1.0,
                        duration: Self::DEFAULT_DURATION,
                    },
                );
            }
            Some(blend) => {
                if blend.to_clip != current {
                    blend.from_clip = blend.to_clip;
                    blend.to_clip = current;
                    blend.t = 0.0;
                    blend.duration = Self::DEFAULT_DURATION;
                } else if blend.t < 1.0 {
                    blend.t = (blend.t + dt / blend.duration.max(1e-4)).min(1.0);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MotionSample {
    pub last_pos: Vec3,

    pub speed: f32,

    pub forward_component: f32,

    pub strafe_component: f32,

    pub last_heading_rad: f32,

    pub heading_rate: f32,

    pub smooth_vx: f32,
    pub smooth_vz: f32,

    pub moving: bool,
}

#[derive(Resource, Default)]
pub struct EntityMotion {
    pub by_id: HashMap<u32, MotionSample>,
}

/// KULUU_MOTION_LOG=1 gated probe for the kuluu-df9t Part B locomotion diagnosis.
///
/// Measures, per entity: server-update spacing (seconds between 0x0E position
/// updates), jump distance and which branch `advance_prediction` took (snap vs
/// blend), remaining chase distance when a packet lands, idle frames between
/// packets, moving-toggle rate on the transform-delta fallback path (the chase
/// model's entities toggle without hysteresis: reached target or not), and
/// heading-vs-travel-direction mismatch events. Prints one line per event plus
/// a rolling summary every few seconds; redirect stdout to a file when capturing
/// a roaming-area log. Observation only - no constant changes.
#[derive(Resource)]
pub struct MotionProbe {
    enabled: bool,
    per_id: HashMap<u32, ProbeEntity>,
    /// Every server-update spacing seen, for the global median/p90 in summaries.
    all_intervals: VecDeque<f32>,
    last_summary_at: f32,
}

#[derive(Default)]
struct ProbeEntity {
    kind_name: &'static str,
    updates: u64,
    normals: u64,
    stretches: u64,
    pops: u64,
    toggles: u64,
    mismatch_events: u64,
    in_mismatch: bool,
}

fn entity_kind_name(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Mob => "Mob",
        EntityKind::Pc => "Pc",
        EntityKind::Pet => "Pet",
        EntityKind::Npc => "Npc",
        _ => "Other",
    }
}

impl MotionProbe {
    pub const SUMMARY_EVERY_SECS: f32 = 5.0;

    /// Travel direction this far from the heading counts as playing sideways.
    pub const MISMATCH_THRESHOLD_RAD: f32 = std::f32::consts::FRAC_PI_4;

    /// Below this dead-reckoned speed the travel direction is noise, not intent.
    const MIN_MEANINGFUL_SPEED_SQ: f32 = 0.01; // (0.1 yps)^2

    /// Test-only: an always-enabled probe (init() reads KULUU_MOTION_LOG).
    #[cfg(test)]
    pub fn enabled_for_test() -> Self {
        let mut p = Self::init();
        p.enabled = true;
        p
    }

    pub fn init() -> Self {
        let enabled = matches!(
            std::env::var("KULUU_MOTION_LOG").as_deref(),
            Ok(v) if !v.is_empty() && v != "0"
        );
        Self {
            enabled,
            per_id: HashMap::new(),
            all_intervals: VecDeque::new(),
            last_summary_at: 0.0,
        }
    }

    fn entry(&mut self, id: u32, kind: EntityKind) -> &mut ProbeEntity {
        self.per_id.entry(id).or_insert_with(|| ProbeEntity {
            kind_name: entity_kind_name(kind),
            ..Default::default()
        })
    }

    /// One server update was consumed by `advance_prediction` this frame.
    pub fn record_update(&mut self, id: u32, kind: EntityKind, u: UpdateOutcome) {
        if !self.enabled {
            return;
        }
        self.all_intervals.push_back(u.dt_server);
        if self.all_intervals.len() > 65_536 {
            self.all_intervals.pop_front();
        }
        let e = self.entry(id, kind);
        e.updates += 1;
        match u.band {
            SnapBand::Normal => e.normals += 1,
            SnapBand::Stretch => e.stretches += 1,
            SnapBand::Pop => e.pops += 1,
        }
        let jump = u.jump_sq.sqrt();
        // ratio is jump/step (the band's defining quantity); guard the zero-step case (a
        // stationary speed byte) so a real move still prints as an unbounded ratio, not NaN.
        let ratio = if u.step_yalms > 0.0 {
            jump / u.step_yalms
        } else {
            f32::INFINITY
        };
        println!(
            "MOTION_UPD id={id:#x} kind={} band={} dt_srv={:.3}s jump={:.2}y step={:.2} ratio={:.2} speed_pkt={} speed_base={} idle_fr={} rem={:.2}",
            e.kind_name,
            match u.band {
                SnapBand::Normal => "Normal",
                SnapBand::Stretch => "Stretch",
                SnapBand::Pop => "Pop",
            },
            u.dt_server,
            jump,
            u.step_yalms,
            ratio,
            u.speed,
            u.speed_base,
            u.idle_frames,
            u.rem_dist
        );
    }

    /// The MOVE_ENTER/MOVE_EXIT hysteresis flipped for this entity.
    pub fn record_toggle(&mut self, id: u32, kind: EntityKind, now_moving: bool, speed: f32) {
        if !self.enabled {
            return;
        }
        let e = self.entry(id, kind);
        e.toggles += 1;
        println!(
            "MOTION_TGL id={id:#x} kind={} moving={} speed={:.2}",
            e.kind_name, now_moving, speed
        );
    }

    /// Rising-edge detector: one event per sideways episode, not per frame.
    pub fn record_heading_mismatch(
        &mut self,
        id: u32,
        kind: EntityKind,
        heading_rad: f32,
        vel: Vec3,
    ) {
        if !self.enabled || vel.length_squared() < Self::MIN_MEANINGFUL_SPEED_SQ {
            return;
        }
        // heading_to_rad convention: forward = (sin h, -cos h), so a travel
        // vector (vx, vz) corresponds to the angle atan2(vx, -vz).
        let travel = vel.x.atan2(-vel.z);
        let mut diff = (heading_rad - travel).rem_euclid(std::f32::consts::TAU);
        if diff > std::f32::consts::PI {
            diff -= std::f32::consts::TAU;
        }
        let sideways = diff.abs() >= Self::MISMATCH_THRESHOLD_RAD;
        let e = self.entry(id, kind);
        if sideways && !e.in_mismatch {
            e.mismatch_events += 1;
            println!(
                "MOTION_MIS id={id:#x} heading={:.0}deg travel={:.0}deg diff={:.0}deg speed={:.2}",
                heading_rad.to_degrees(),
                travel.to_degrees(),
                diff.abs().to_degrees(),
                vel.length()
            );
        }
        e.in_mismatch = sideways;
    }

    /// Rolling summary; called once per frame from predict_entities_system.
    pub fn maybe_summary(&mut self, now_secs: f32) {
        if !self.enabled || now_secs - self.last_summary_at < Self::SUMMARY_EVERY_SECS {
            return;
        }
        self.last_summary_at = now_secs;
        let (mut upd, mut norm, mut stretch, mut pop, mut tgl, mut mis) =
            (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
        for e in self.per_id.values() {
            upd += e.updates;
            norm += e.normals;
            stretch += e.stretches;
            pop += e.pops;
            tgl += e.toggles;
            mis += e.mismatch_events;
        }
        let (median, p90) = quantiles(&self.all_intervals);
        println!(
            "MOTION_SUM t={:.1}s ents={} upd={} normal={} stretch={} pop={} tgl={} mis={} med_dt_srv={} p90_dt_srv={}",
            now_secs,
            self.per_id.len(),
            upd,
            norm,
            stretch,
            pop,
            tgl,
            mis,
            fmt_opt(median),
            fmt_opt(p90)
        );
    }
}

fn quantiles(samples: &VecDeque<f32>) -> (Option<f32>, Option<f32>) {
    if samples.is_empty() {
        return (None, None);
    }
    let mut v: Vec<f32> = samples.iter().copied().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = |q: f32| {
        let idx = (q * (v.len() - 1) as f32).round() as usize;
        Some(v[idx.min(v.len() - 1)])
    };
    (pick(0.5), pick(0.9))
}

fn fmt_opt(x: Option<f32>) -> String {
    match x {
        Some(v) => format!("{v:.3}s"),
        None => "-".to_string(),
    }
}

impl EntityMotion {
    pub fn is_moving(&self, id: u32) -> bool {
        self.by_id.get(&id).is_some_and(|s| s.moving)
    }

    pub fn sample(&self, id: u32) -> Option<MotionSample> {
        self.by_id.get(&id).copied()
    }

    pub fn apply_move_hysteresis(prev_moving: bool, speed: f32) -> bool {
        if speed >= Self::MOVE_ENTER {
            true
        } else if speed <= Self::MOVE_EXIT {
            false
        } else {
            prev_moving
        }
    }

    pub const MOVE_THRESHOLD: f32 = 0.5;

    pub const MOVE_ENTER: f32 = 0.8;

    pub const MOVE_EXIT: f32 = 0.35;

    pub const TURN_THRESHOLD_RAD_PER_SEC: f32 = 0.5;
}

pub fn track_entity_motion_system(
    time: Res<Time>,
    state: Res<SceneState>,
    prediction: Res<EntityPrediction>,
    mut motion: ResMut<EntityMotion>,
    mut probe: ResMut<MotionProbe>,
    q: Query<(&WorldEntity, &Transform)>,
    mut heading_by_id: Local<std::collections::HashMap<u32, u8>>,
) {
    let dt = time.delta_secs().max(1e-4);

    // Headings only change with a snapshot; rebuilding the map every frame was
    // pure per-frame churn in the crowd scene.
    if state.dirty {
        heading_by_id.clear();
        heading_by_id.extend(state.snapshot.entities.iter().map(|e| (e.id, e.heading)));
    }
    for (world, transform) in &q {
        let pos = transform.translation;

        // Entities the chase model owns get their motion sample from the chase state itself:
        // moving is "has not reached target yet", speed is the wire-driven chase rate, and the
        // direction components are where the target still is relative to the rendered heading.
        // No velocity smoothing or enter/exit hysteresis on this path (research/XiPackets
        // world/server/0x000E: walk toward the target, stop on arrival).
        if let Some(chase) = prediction.by_id.get(&world.id) {
            let to_target = Vec3::new(chase.server_pos.x - pos.x, 0.0, chase.server_pos.z - pos.z);
            let chasing = chase.is_chasing();
            let heading_rad = chase.rendered_heading_rad;
            // heading_forward convention: forward = (sin h, -cos h).
            let fwd = Vec3::new(heading_rad.sin(), 0.0, -heading_rad.cos());
            let right = Vec3::new(fwd.z, 0.0, -fwd.x);
            let prev = motion
                .by_id
                .get(&world.id)
                .copied()
                .unwrap_or(MotionSample {
                    last_pos: pos,
                    last_heading_rad: heading_rad,
                    ..Default::default()
                });
            let mut dh = heading_rad - prev.last_heading_rad;
            if dh > std::f32::consts::PI {
                dh -= std::f32::consts::TAU;
            } else if dh < -std::f32::consts::PI {
                dh += std::f32::consts::TAU;
            }
            let heading_rate = dh / dt;
            motion.by_id.insert(
                world.id,
                MotionSample {
                    last_pos: pos,
                    speed: if chasing { chase.chase_rate } else { 0.0 },
                    forward_component: to_target.dot(fwd),
                    strafe_component: to_target.dot(right),
                    last_heading_rad: heading_rad,
                    heading_rate,
                    smooth_vx: 0.0,
                    smooth_vz: 0.0,
                    moving: chasing,
                },
            );
            continue;
        }

        let heading_u8 = heading_by_id.get(&world.id).copied().unwrap_or(0);
        let heading_rad = heading_to_rad(heading_u8);

        let fwd = heading_forward(heading_u8);
        let (fwd_x, fwd_z) = (fwd.x, fwd.z);

        let right_x = fwd_z;
        let right_z = -fwd_x;

        let prev = motion
            .by_id
            .get(&world.id)
            .copied()
            .unwrap_or(MotionSample {
                last_pos: pos,
                last_heading_rad: heading_rad,
                ..Default::default()
            });
        let dx = pos.x - prev.last_pos.x;
        let dz = pos.z - prev.last_pos.z;

        const VEL_TAU: f32 = 0.25;
        let alpha = 1.0 - (-dt / VEL_TAU).exp();
        let smooth_vx = prev.smooth_vx + alpha * (dx / dt - prev.smooth_vx);
        let smooth_vz = prev.smooth_vz + alpha * (dz / dt - prev.smooth_vz);
        let speed = (smooth_vx * smooth_vx + smooth_vz * smooth_vz).sqrt();
        let forward_component = smooth_vx * fwd_x + smooth_vz * fwd_z;
        let strafe_component = smooth_vx * right_x + smooth_vz * right_z;

        let mut dh = heading_rad - prev.last_heading_rad;
        if dh > std::f32::consts::PI {
            dh -= std::f32::consts::TAU;
        } else if dh < -std::f32::consts::PI {
            dh += std::f32::consts::TAU;
        }
        let heading_rate = dh / dt;

        let moving = EntityMotion::apply_move_hysteresis(prev.moving, speed);
        if probe.enabled && moving != prev.moving {
            probe.record_toggle(world.id, world.kind, moving, speed);
        }
        motion.by_id.insert(
            world.id,
            MotionSample {
                last_pos: pos,
                speed,
                forward_component,
                strafe_component,
                last_heading_rad: heading_rad,
                heading_rate,
                smooth_vx,
                smooth_vz,
                moving,
            },
        );
    }
}

/// Which snap band a POS update fell into (see `EntityPrediction::SNAP_*_RATIO`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapBand {
    /// jump <= 1.0 * step: an ordinary tick; chase.
    Normal,

    /// 1.0*step < jump <= 2.0*step: plausible (path re-eval or a late tick); chase, no snap.
    Stretch,

    /// jump > 2.0*step, or the sample is older than DT_SERVER_CEIL: pop rendered XZ onto the server.
    Pop,
}

/// What `advance_prediction` learned when it consumed a server update this
/// frame; read by the KULUU_MOTION_LOG probe in predict_entities_system.
#[derive(Clone, Copy, Debug)]
pub struct UpdateOutcome {
    /// Seconds since the previous server update for this entity (the LSB
    /// position-update cadence).
    pub dt_server: f32,

    /// Squared distance from the rendered position to the new server position.
    pub jump_sq: f32,

    /// The snap band this update fell into; Pop is the only band that snaps XZ onto the server.
    pub band: SnapBand,

    /// expected_step_yalms for this update's wire bytes: the LSB per-tick step distance the bands
    /// are measured against (the ratio jump/step is what the probe prints).
    pub step_yalms: f32,

    /// The 0x0E speed byte this update carried (retail decodes it as yalms/sec * 10).
    pub speed: u8,

    /// The 0x0E animationSpeed byte (LSB `animationSpeed`, never multiplied by the
    /// run factor; battleentity.cpp UpdateSpeed writes `speed` only). Feeds the
    /// gait rule and the clip playback-rate scale.
    pub speed_base: u8,

    /// Rendered frames that elapsed between this update and the previous one:
    /// how long the client chased on its own before the wire caught up.
    pub idle_frames: u32,

    /// Distance from the rendered position to the new server target after this
    /// frame's advance: what is left of the chase when the packet lands.
    pub rem_dist: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PredictSample {
    pub rendered_pos: Vec3,

    pub server_pos: Vec3,

    pub target_heading: u8,

    /// Chase rate in yalms per second, paced to the packet cadence (B): expected_step_yalms /
    /// dt_server_smoothed. Recomputed on every POS update; the rendered position walks toward
    /// `server_pos` at this rate and clamps exactly on arrival, so it lands as the next update is
    /// due rather than early.
    pub chase_rate: f32,

    /// Last 0x0E speed byte observed for this entity (retail decodes it as yalms/sec * 10).
    pub packet_speed: u8,

    /// Last 0x0E animationSpeed byte observed for this entity (LSB `animationSpeed`; the gait
    /// rule compares it against `packet_speed`).
    pub packet_speed_base: u8,

    /// Rendered frames since the last consumed POS update. Reset on an update, incremented every
    /// frame without one: how long the client chased unaided before the wire caught up.
    pub idle_frames: u32,

    pub rendered_heading_rad: f32,

    pub secs_since_update: f32,

    /// Running mean of the measured inter-update interval, clamped to [DT_SERVER_FLOOR,
    /// DT_SERVER_CEIL]. Paces the chase (B) and sets the hold-until grace window; a single late or
    /// early packet can only move it halfway toward itself (DT_SMOOTH_ALPHA).
    pub dt_server_smoothed: f32,

    /// Monotonic per-sample time base in seconds, advanced by `dt` every frame. Compared against
    /// `hold_until` to keep the moving flag up across a late packet.
    pub clock: f32,

    /// Clock value until which the entity still counts as chasing even after reaching its target:
    /// last update's clock + dt_server_smoothed (B). A late packet then holds the gait instead of
    /// dropping to idle between updates.
    pub hold_until: f32,

    pub sample_dirty: bool,

    pub initialized: bool,

    /// Set by the most recent `advance_prediction` call that consumed a server update; cleared on
    /// frames without one.
    pub last_update: Option<UpdateOutcome>,
}

impl PredictSample {
    fn seed(server_pos: Vec3, heading: u8, speed: u8, speed_base: u8, _mounted: bool) -> Self {
        // The step model paces from the wire speed byte and the measured inter-update cadence,
        // not mount state (see expected_step_yalms), so `_mounted` is retained only to keep the
        // observe() call-site contract stable.
        let dt_server_smoothed = EntityPrediction::DT_SERVER_CEIL;
        PredictSample {
            rendered_pos: server_pos,
            server_pos,
            target_heading: heading,
            chase_rate: expected_step_yalms(speed, speed_base) / dt_server_smoothed,
            packet_speed: speed,
            packet_speed_base: speed_base,
            idle_frames: 0,
            rendered_heading_rad: heading_to_rad(heading),
            secs_since_update: 0.0,
            dt_server_smoothed,
            clock: 0.0,
            hold_until: 0.0,
            sample_dirty: false,
            initialized: true,
            last_update: None,
        }
    }

    /// Whether the entity still counts as moving. This is the chase model's moving flag (retail
    /// stops on arrival, research/XiPackets world/server/0x000E): no speed hysteresis, no velocity
    /// smoothing. XZ only: Y is fully server-resolved (LSB stepTowards grounds it and we assign it
    /// directly on each update), so it never holds "moving" up after the XZ chase has arrived.
    ///
    /// Once the rendered position reaches the wire target, the flag stays up for one expected
    /// interval past the last update (B): `clock < hold_until`. A late packet then keeps the gait
    /// running instead of dropping to idle between updates and restarting the walk clip from frame 0.
    pub fn is_chasing(&self) -> bool {
        let dx = self.server_pos.x - self.rendered_pos.x;
        let dz = self.server_pos.z - self.rendered_pos.z;
        if dx * dx + dz * dz > EntityPrediction::ARRIVAL_EPS_SQ {
            return true;
        }
        self.clock < self.hold_until
    }
}

#[derive(Resource, Default)]
pub struct EntityPrediction {
    pub by_id: HashMap<u32, PredictSample>,
}

impl EntityPrediction {
    /// Per-AI-tick step distance a moving mob advances, in yalms, from the wire speed byte.
    /// vendor/server/src/map/ai/helpers/pathfind.cpp CPathFind::StepTo:
    /// `stepDistance = speed / (run ? 50.0 : 40.0)`, then markPositionDirty() -> exactly one POS
    /// update per tick. The run flag comes from PATHFLAG_RUN, which mob_controller.cpp sets for
    /// chase/follow/return-home and leaves clear while roaming: roam = walk (/40), engaged = run
    /// (/50). The client's gait signal is already `run = speed > speed_base` (wire bytes), so the
    /// divisor is chosen from that same comparison. These two divisors are LSB constants; they are
    /// the only literals this step model may use.
    pub const LSB_RUN_STEP_DIVISOR: f32 = 50.0;

    pub const LSB_WALK_STEP_DIVISOR: f32 = 40.0;

    /// Snap-band multipliers, as a RATIO TO THE EXPECTED STEP (not distances). A jump within one
    /// step is an ordinary tick; up to two steps is plausible (a path re-eval or a late tick) and
    /// still chases without snapping; beyond that the sample is stale/teleported and we pop.
    pub const SNAP_NORMAL_RATIO: f32 = 1.0;

    pub const SNAP_STRETCH_RATIO: f32 = 2.0;

    pub const HEADING_TAU: f32 = 0.10;

    /// A sample older than this is stale even for a small jump: pop it onto the server position.
    /// Also the ceiling of the smoothed inter-update interval used to pace the chase (B).
    pub const DT_SERVER_CEIL: f32 = 1.0;

    /// Floor on the smoothed inter-update interval. Below this, packets arrive faster than the
    /// ~400 ms AI tick cadence can justify; clamping here stops a burst of rapid updates from
    /// spiking chase_rate to an absurd value. A guard against pathological timing, not a movement
    /// constant.
    pub const DT_SERVER_FLOOR: f32 = 0.1;

    /// EMA weight for the smoothed inter-update interval: each new measured interval moves the mean
    /// halfway toward itself, so one late/early packet can shift the chase pace by at most half its
    /// deviation and the mean tracks within ~two ticks.
    pub const DT_SMOOTH_ALPHA: f32 = 0.5;

    /// Squared XZ distance at which the chase counts as arrived on target. The clamp below lands
    /// exactly on `server_pos`, so this only has to clear float noise, not a real gap.
    pub const ARRIVAL_EPS_SQ: f32 = 1e-6; // (0.001 yalms)^2

    const SAMPLE_EPSILON_SQ: f32 = 1e-4;

    pub fn observe(
        &mut self,
        id: u32,
        server_pos: Vec3,
        heading: u8,
        speed: u8,
        speed_base: u8,
        mounted: bool,
    ) {
        match self.by_id.get_mut(&id) {
            None => {
                self.by_id.insert(
                    id,
                    PredictSample::seed(server_pos, heading, speed, speed_base, mounted),
                );
            }
            Some(e) => {
                if e.server_pos.distance_squared(server_pos) > Self::SAMPLE_EPSILON_SQ {
                    e.server_pos = server_pos;
                    e.sample_dirty = true;
                }
                e.target_heading = heading;
                e.packet_speed = speed;
                e.packet_speed_base = speed_base;
                // The wire speed byte is per-mob and per-moment (LSB UpdateSpeed). The chase rate
                // itself is paced in advance_prediction when this update is consumed, from the step
                // model and the measured inter-update cadence; observe only records the bytes.
            }
        }
    }
}

#[inline]
fn heading_to_rad(heading: u8) -> f32 {
    (heading as f32) * std::f32::consts::TAU / 256.0
}

/// World-space direction an entity with this heading faces. The one place the
/// `(sin, -cos)` pairing lives — the motion basis below and the fishing water
/// probe both come through here rather than re-deriving it.
#[inline]
pub fn heading_forward(heading: u8) -> Vec3 {
    let rad = heading_to_rad(heading);
    Vec3::new(rad.sin(), 0.0, -rad.cos())
}

/// Per-AI-tick step distance in yalms for the incoming wire speed bytes.
///
/// vendor/server/src/map/ai/helpers/pathfind.cpp CPathFind::StepTo advances a moving mob by
/// `speed / (run ? 50.0 : 40.0)` each tick; entity_path_owner.cpp updateSpeed(run) multiplies only
/// the movement speed, never animationSpeed, so the run/walk split is read straight off the wire as
/// `speed > speed_base` (the same comparison the gait rule uses). No mount or retail-yps factor:
/// this is the server's own per-tick budget.
pub fn expected_step_yalms(speed: u8, speed_base: u8) -> f32 {
    let divisor = if speed > speed_base {
        EntityPrediction::LSB_RUN_STEP_DIVISOR
    } else {
        EntityPrediction::LSB_WALK_STEP_DIVISOR
    };
    speed as f32 / divisor
}

fn advance_prediction(s: &mut PredictSample, dt: f32) -> (Vec3, f32) {
    use std::f32::consts::{PI, TAU};

    // Advance the per-sample time base first so `clock` and `hold_until` stay in lockstep.
    s.clock += dt;

    let mut outcome: Option<UpdateOutcome> = None;
    if s.sample_dirty {
        s.sample_dirty = false;
        let dt_server = s.secs_since_update;

        // B: pace the chase to the packet cadence, not a yps constant. Update the running mean of
        // the measured inter-update interval (clamped so one late/early packet cannot spike it),
        // then set the rate so one smoothed interval covers exactly one LSB step.
        let measured = dt_server.clamp(
            EntityPrediction::DT_SERVER_FLOOR,
            EntityPrediction::DT_SERVER_CEIL,
        );
        s.dt_server_smoothed = (s.dt_server_smoothed
            + (measured - s.dt_server_smoothed) * EntityPrediction::DT_SMOOTH_ALPHA)
            .clamp(
                EntityPrediction::DT_SERVER_FLOOR,
                EntityPrediction::DT_SERVER_CEIL,
            );
        let step = expected_step_yalms(s.packet_speed, s.packet_speed_base);
        s.chase_rate = step / s.dt_server_smoothed;

        // A: step-relative snap band. jump is the XZ distance the server moved this tick relative
        // to where we are rendering it; step is what LSB actually advanced (StepToInternal). The
        // bands are a ratio to that step, never a flat distance, so a fast mob's legitimate per-tick
        // move no longer reads as a teleport. XZ only: Y is assigned directly below and must not
        // inflate the jump with a floor-height change.
        let dxj = s.server_pos.x - s.rendered_pos.x;
        let dzj = s.server_pos.z - s.rendered_pos.z;
        let jump_sq = dxj * dxj + dzj * dzj;
        let jump = jump_sq.sqrt();
        let band = if dt_server > EntityPrediction::DT_SERVER_CEIL
            || jump > EntityPrediction::SNAP_STRETCH_RATIO * step
        {
            SnapBand::Pop
        } else if jump > EntityPrediction::SNAP_NORMAL_RATIO * step {
            SnapBand::Stretch
        } else {
            SnapBand::Normal
        };

        // B: hold the moving flag for one expected interval past this update so a late packet keeps
        // the gait running instead of dropping to idle between updates.
        s.hold_until = s.clock + s.dt_server_smoothed;

        if band == SnapBand::Pop {
            // Teleport, zone-in, or a stale sample: snap XZ onto the server position.
            s.rendered_pos.x = s.server_pos.x;
            s.rendered_pos.z = s.server_pos.z;
        }
        // A: Y is fully server-resolved (LSB stepTowards walks it toward target.y and snaps on
        // arrival), so assign it directly on every update. No smoothing, no ground probe, no gravity.
        s.rendered_pos.y = s.server_pos.y;

        outcome = Some(UpdateOutcome {
            dt_server,
            jump_sq,
            band,
            step_yalms: step,
            speed: s.packet_speed,
            speed_base: s.packet_speed_base,
            idle_frames: s.idle_frames,
            rem_dist: 0.0, // filled below, once this frame's advance has run
        });
        s.idle_frames = 0;
        s.secs_since_update = 0.0;
    }

    // Retail target-chase (research/XiPackets world/server/0x000E): the client walks
    // LocalPosition toward Movement.Move and stops on arrival. There is no velocity state, so a stop
    // can never overshoot and slide back: between packets the rendered position keeps walking to the
    // last target and clamps exactly on it.
    let dx = s.server_pos.x - s.rendered_pos.x;
    let dz = s.server_pos.z - s.rendered_pos.z;
    if dx * dx + dz * dz > EntityPrediction::ARRIVAL_EPS_SQ {
        let dist = (dx * dx + dz * dz).sqrt();
        // Clamp at the target, never past it: a slow frame must not run on.
        let t = (s.chase_rate * dt / dist).min(1.0);
        s.rendered_pos.x += dx * t;
        s.rendered_pos.z += dz * t;
    }

    if let Some(u) = &mut outcome {
        let rx = s.server_pos.x - s.rendered_pos.x;
        let rz = s.server_pos.z - s.rendered_pos.z;
        u.rem_dist = (rx * rx + rz * rz).sqrt();
    }

    s.secs_since_update += dt;
    // Frames without a consumed update: the client is chasing on its own.
    if outcome.is_none() {
        s.idle_frames = s.idle_frames.saturating_add(1);
    }

    let target = heading_to_rad(s.target_heading);
    let mut dh = target - s.rendered_heading_rad;
    dh = dh.rem_euclid(TAU);
    if dh > PI {
        dh -= TAU;
    }
    let alpha_h = 1.0 - (-dt / EntityPrediction::HEADING_TAU).exp();
    s.rendered_heading_rad += dh * alpha_h;

    s.last_update = outcome;

    (s.rendered_pos, s.rendered_heading_rad)
}

pub fn predict_entities_system(
    time: Res<Time>,
    mut prediction: ResMut<EntityPrediction>,
    mut probe: ResMut<MotionProbe>,
    mut q: Query<(&WorldEntity, &mut Transform), Without<IsSelf>>,
) {
    let dt = time.delta_secs().max(1e-4);
    for (world, mut transform) in &mut q {
        if !matches!(
            world.kind,
            EntityKind::Mob | EntityKind::Pc | EntityKind::Pet | EntityKind::Npc
        ) {
            continue;
        }
        let Some(sample) = prediction.by_id.get_mut(&world.id) else {
            continue;
        };
        if !sample.initialized {
            continue;
        }
        let (pos, heading_rad) = advance_prediction(sample, dt);
        if probe.enabled {
            if let Some(u) = sample.last_update {
                probe.record_update(world.id, world.kind, u);
            }
            // The chase model's travel direction: toward the target at the wire rate while
            // chasing, still otherwise.
            let to_target = Vec3::new(
                sample.server_pos.x - pos.x,
                0.0,
                sample.server_pos.z - pos.z,
            );
            let vel = if sample.is_chasing() {
                to_target.normalize() * sample.chase_rate
            } else {
                Vec3::ZERO
            };
            probe.record_heading_mismatch(world.id, world.kind, heading_rad, vel);
        }
        transform.translation = pos;
        transform.rotation = Quat::from_rotation_y(-heading_rad);
    }
    probe.maybe_summary(time.elapsed_secs());
}

#[derive(Resource, Debug, Clone)]
pub struct ModelViewerClipOverride {
    pub clip_name: String,
}

impl ModelViewerClipOverride {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            clip_name: name.into(),
        }
    }
}

pub fn enumerate_clips_for_skel(skel_file_id: u32) -> Vec<(String, Arc<Mo2Animation>)> {
    let mut out = Vec::new();
    let mut sources: Vec<u32> = vec![skel_file_id];
    if let Some(motion) = motion_dat_for_skel(skel_file_id) {
        sources.push(motion);
    }
    let mut seen = std::collections::HashSet::<String>::new();
    for file_id in sources {
        for_each_anim_chunk_in_dat(file_id, |name, anim| {
            if seen.insert(name.clone()) {
                out.push((name, Arc::new(anim)));
            }
        });
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn override_anim_for_skel(skel_file_id: u32, prefix: &[u8; 3]) -> Option<Arc<Mo2Animation>> {
    if let Some(a) = load_anim_with_prefix(skel_file_id, prefix) {
        return Some(Arc::new(a));
    }
    let motion = motion_dat_for_skel(skel_file_id)?;
    load_anim_with_prefix(motion, prefix).map(Arc::new)
}

fn for_each_anim_chunk_in_dat(file_id: u32, mut f: impl FnMut(String, Mo2Animation)) {
    let Ok(root) = DatRoot::from_env_or_default() else {
        return;
    };
    let Ok(loc) = root.resolve(file_id) else {
        return;
    };
    let Ok(bytes) = fs::read(loc.path_under(&root)) else {
        return;
    };
    for chunk in walk(&bytes).filter_map(Result::ok) {
        if ChunkKind::from_u8(chunk.kind) != Some(ChunkKind::AnimMo2) {
            continue;
        }
        if let Ok(anim) = ffxi_dat::anim::parse_mo2(chunk.data, &chunk.name) {
            let name = String::from_utf8_lossy(&chunk.name)
                .trim_end_matches('\0')
                .trim_end()
                .to_string();
            f(name, anim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_dat_resolves_for_each_pc_race() {
        let pairs = [
            (7072, 9672),
            (10248, 12848),
            (13424, 16024),
            (16600, 19200),
            (19776, 22376),
            (23176, 25776),
            (26352, 28952),
        ];
        for (skel, motion) in pairs {
            assert_eq!(
                motion_dat_for_skel(skel),
                Some(motion),
                "skel {skel} should map to motion {motion}"
            );
        }
    }

    #[test]
    fn motion_dat_returns_none_for_non_pc_skel() {
        assert_eq!(motion_dat_for_skel(0), None);
        assert_eq!(motion_dat_for_skel(7000), None);
        assert_eq!(motion_dat_for_skel(50000), None);
    }

    #[test]
    fn motion_dat_offset_is_consistent() {
        for skel in [7072u32, 10248, 13424, 16600, 19776, 23176, 26352] {
            let motion = motion_dat_for_skel(skel).expect("PC race");
            assert_eq!(
                motion - skel,
                2600,
                "skel {skel} → motion {motion}: offset must be +2600"
            );
        }
    }

    #[test]
    fn battle_idle_resolves_for_every_pc_race_when_dats_available() {
        if DatRoot::from_env_or_default().is_err() {
            eprintln!("skipping: no retail DAT root");
            return;
        }
        for skel in [7072u32, 10248, 13424, 16600, 19776, 23176, 26352] {
            let anim = battle_idle_anim_for_skel(skel).expect("battle-idle MO2 missing for skel");
            assert!(
                anim.frames > 0,
                "skel {skel}: btl MO2 has zero frames — parse drift?"
            );
        }
    }

    #[test]
    fn run_anim_resolves_for_every_pc_race_when_dats_available() {
        if DatRoot::from_env_or_default().is_err() {
            eprintln!("skipping: no retail DAT root");
            return;
        }
        for skel in [7072u32, 10248, 13424, 16600, 19776, 23176, 26352] {
            let anim = run_anim_for_skel(skel).expect("casual run MO2 missing for skel");
            assert!(anim.frames > 0, "skel {skel}: run MO2 has zero frames");
        }
    }

    #[test]
    fn is_moving_reads_latch_not_raw_speed() {
        let mut m = EntityMotion::default();
        m.by_id.insert(
            1,
            MotionSample {
                moving: true,
                speed: 0.0,
                ..Default::default()
            },
        );
        m.by_id.insert(
            2,
            MotionSample {
                moving: false,
                speed: 9.0,
                ..Default::default()
            },
        );
        assert!(
            m.is_moving(1),
            "latched-moving animates even at instant speed 0"
        );
        assert!(
            !m.is_moving(2),
            "latched-idle stays idle even at instant speed 9"
        );
        assert!(!m.is_moving(99), "unknown id should not animate");
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn move_hysteresis_enter_exit_and_hold() {
        assert!(
            EntityMotion::MOVE_EXIT < EntityMotion::MOVE_ENTER,
            "there must be a genuine hold band"
        );
        let mid = 0.5 * (EntityMotion::MOVE_EXIT + EntityMotion::MOVE_ENTER);

        assert!(!EntityMotion::apply_move_hysteresis(
            false,
            EntityMotion::MOVE_EXIT
        ));
        assert!(
            !EntityMotion::apply_move_hysteresis(false, mid),
            "idle holds in band"
        );
        assert!(EntityMotion::apply_move_hysteresis(
            false,
            EntityMotion::MOVE_ENTER + 0.1
        ));

        assert!(
            EntityMotion::apply_move_hysteresis(true, mid),
            "moving holds in band"
        );
        assert!(EntityMotion::apply_move_hysteresis(
            true,
            EntityMotion::MOVE_ENTER + 0.1
        ));
        assert!(!EntityMotion::apply_move_hysteresis(
            true,
            EntityMotion::MOVE_EXIT - 0.01
        ));
        assert!(!EntityMotion::apply_move_hysteresis(true, 0.0));
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn walk_run_boundary_is_sane() {
        use crate::ffxi_actor_render::{infers_walk_gait, WALK_RUN_BOUNDARY};
        assert!(EntityMotion::MOVE_EXIT < WALK_RUN_BOUNDARY);
        assert!(
            WALK_RUN_BOUNDARY < 5.0,
            "a base-run actor must NOT be classed as walking"
        );
        assert!(!infers_walk_gait(0.0), "stationary is not walking");
        assert!(infers_walk_gait(1.5), "slow mover walks");
        assert!(!infers_walk_gait(6.0), "runner runs, not walks");
    }

    /// A dirty chase sample: rendered at `rendered`, wire target at `server`, the update aged
    /// `age` seconds, and a speed byte (speed_base set equal so the walk divisor applies).
    fn chase_sample(server: Vec3, rendered: Vec3, age: f32, speed_byte: u8) -> PredictSample {
        let mut s = PredictSample::seed(rendered, 0, speed_byte, speed_byte, false);
        s.server_pos = server;
        s.secs_since_update = age;
        s.sample_dirty = true;
        s
    }

    /// A dirty sample with distinct speed/speed_base bytes so the run/walk divisor is chosen by
    /// `speed > speed_base` (the gait rule), not by a fixed walk assumption.
    fn chase_sample_gait(
        server: Vec3,
        rendered: Vec3,
        age: f32,
        speed_byte: u8,
        base_byte: u8,
    ) -> PredictSample {
        let mut s = PredictSample::seed(rendered, 0, speed_byte, base_byte, false);
        s.server_pos = server;
        s.secs_since_update = age;
        s.sample_dirty = true;
        s
    }

    #[test]
    fn expected_step_uses_the_lsb_divisors() {
        // walk (speed <= speed_base): /40; run (speed > speed_base): /50. StepToInternal.
        assert!(
            (expected_step_yalms(40, 40) - 1.0).abs() < 1e-6,
            "walk step = 40/40"
        );
        assert!(
            (expected_step_yalms(40, 39) - 0.8).abs() < 1e-6,
            "run step = 40/50"
        );
        assert!(
            expected_step_yalms(120, 120) > expected_step_yalms(120, 119),
            "a faster byte steps further per tick"
        );
    }

    #[test]
    fn prediction_band_normal_within_one_step() {
        // jump (1.0) == step (40/40): within one step -> Normal, no snap; the chase runs on.
        let mut s = chase_sample(Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        advance_prediction(&mut s, 1.0 / 60.0);
        assert_eq!(s.last_update.unwrap().band, SnapBand::Normal);
        // Not snapped: the rendered position is still short of the server target.
        assert!(
            (s.rendered_pos.x - 1.0).abs() > 1e-3,
            "a Normal band chases rather than snapping: {}",
            s.rendered_pos.x
        );
    }

    #[test]
    fn prediction_band_stretch_between_one_and_two_steps() {
        // jump (1.5) is between one step (1.0) and two steps (2.0): Stretch, still no snap.
        let mut s = chase_sample(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        advance_prediction(&mut s, 1.0 / 60.0);
        assert_eq!(s.last_update.unwrap().band, SnapBand::Stretch);
        assert!(
            (s.rendered_pos.x - 1.5).abs() > 1e-3,
            "a Stretch band chases rather than snapping: {}",
            s.rendered_pos.x
        );
    }

    #[test]
    fn prediction_band_pop_beyond_two_steps_snaps_xz() {
        // jump (3.0) exceeds two steps (2.0): Pop, XZ snaps onto the server position.
        let mut s = chase_sample(Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        let (pos, _) = advance_prediction(&mut s, 1.0 / 60.0);
        assert_eq!(s.last_update.unwrap().band, SnapBand::Pop);
        assert_eq!(pos.x, 3.0, "a Pop band snaps XZ onto the server position");
    }

    #[test]
    fn prediction_band_pop_on_stale_sample() {
        // A sample older than DT_SERVER_CEIL is stale even for a small jump: Pop it.
        let mut s = chase_sample(Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, 2.0, 40);
        advance_prediction(&mut s, 1.0 / 60.0);
        assert_eq!(s.last_update.unwrap().band, SnapBand::Pop);
        assert_eq!(
            s.rendered_pos.x, 0.5,
            "a stale sample pops onto the server position"
        );
    }

    #[test]
    fn prediction_y_assigns_server_directly() {
        // Y is fully server-resolved (LSB stepTowards): it lands on server.y in one update with no
        // exp smoothing. A floor-height change must not inflate the XZ jump into a Pop either.
        let mut s = chase_sample(Vec3::new(1.0, 5.0, 0.0), Vec3::new(0.0, 0.0, 0.0), 0.4, 40);
        advance_prediction(&mut s, 1.0 / 60.0);
        assert_eq!(
            s.rendered_pos.y, 5.0,
            "Y is assigned directly from the server"
        );
        // The XZ jump (1.0) is within one step, so the floor-height change did not force a Pop.
        assert_eq!(s.last_update.unwrap().band, SnapBand::Normal);
    }

    #[test]
    fn prediction_chase_paces_to_packet_cadence() {
        // B: chase_rate = expected_step_yalms / dt_server_smoothed. With one measured interval of
        // 0.4 s the smoothed mean is halfway between its seed (DT_SERVER_CEIL) and 0.4, so the rate
        // is step/that-mean -- paced to the tick, not a flat yps constant.
        let mut s = chase_sample(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        advance_prediction(&mut s, 1.0 / 60.0);
        let step = expected_step_yalms(40, 40); // walk: 1.0
        let smoothed = (EntityPrediction::DT_SERVER_CEIL
            + (0.4 - EntityPrediction::DT_SERVER_CEIL) * EntityPrediction::DT_SMOOTH_ALPHA)
            .clamp(
                EntityPrediction::DT_SERVER_FLOOR,
                EntityPrediction::DT_SERVER_CEIL,
            );
        assert!(
            (s.dt_server_smoothed - smoothed).abs() < 1e-6,
            "smoothed interval: {}",
            s.dt_server_smoothed
        );
        assert!(
            (s.chase_rate - step / smoothed).abs() < 1e-6,
            "chase rate paces one step per smoothed interval: {} vs {}",
            s.chase_rate,
            step / smoothed
        );
    }

    #[test]
    fn prediction_run_gait_paces_with_the_run_step() {
        // B + gait: a running entity (speed > speed_base) paces off the /50 run step, not the walk
        // step. Same wire speed byte, different divisor -> a slower per-tick pace.
        let mut s = chase_sample_gait(Vec3::new(1.6, 0.0, 0.0), Vec3::ZERO, 0.4, 40, 39);
        advance_prediction(&mut s, 1.0 / 60.0);
        let run_step = expected_step_yalms(40, 39); // 40/50 = 0.8
        assert!((run_step - 0.8).abs() < 1e-6);
        let smoothed = (EntityPrediction::DT_SERVER_CEIL
            + (0.4 - EntityPrediction::DT_SERVER_CEIL) * EntityPrediction::DT_SMOOTH_ALPHA)
            .clamp(
                EntityPrediction::DT_SERVER_FLOOR,
                EntityPrediction::DT_SERVER_CEIL,
            );
        assert!(
            (s.chase_rate - run_step / smoothed).abs() < 1e-6,
            "run gait paces off the /50 step: {} vs {}",
            s.chase_rate,
            run_step / smoothed
        );
    }

    #[test]
    fn prediction_chase_never_overshoots_the_target() {
        // Target 1.5 yalms ahead (a Stretch band, so it chases rather than pops): the clamp must
        // hold on every single frame and land exactly on target.
        let mut s = chase_sample(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        for _ in 0..600 {
            advance_prediction(&mut s, 1.0 / 60.0);
            assert!(
                s.rendered_pos.x <= 1.5 + 1e-6,
                "chase must not run past the target: {}",
                s.rendered_pos.x
            );
        }
    }

    #[test]
    fn prediction_chase_arrives_exactly_and_reports_idle() {
        let mut s = chase_sample(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            if !s.is_chasing() {
                break;
            }
            advance_prediction(&mut s, dt);
        }
        assert!(
            !s.is_chasing(),
            "the chase reports idle once it reaches the target"
        );
        let arrived = s.rendered_pos.x;
        assert!(
            (arrived - 1.5).abs() < 1e-3,
            "idle means on target: {arrived}"
        );
        // Idle holds: no slide back or forward for a second of frames.
        for _ in 0..60 {
            advance_prediction(&mut s, dt);
        }
        assert!(!s.is_chasing());
        assert!(
            (s.rendered_pos.x - arrived).abs() < 1e-3,
            "stays put: {arrived} -> {}",
            s.rendered_pos.x
        );
    }

    #[test]
    fn prediction_keeps_chasing_between_updates() {
        // No new POS update lands for the whole window; the rendered position must keep walking to
        // the last target at the paced rate instead of coasting on a velocity.
        let mut s = chase_sample(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        advance_prediction(&mut s, 1.0 / 60.0); // consumes the dirty update
        let rate = s.chase_rate;
        let after_first = s.rendered_pos.x;
        for _ in 0..59 {
            advance_prediction(&mut s, 1.0 / 60.0);
        }
        assert_eq!(s.idle_frames, 59, "frames without an update count");
        let expected = (after_first + rate * 59.0 / 60.0).min(1.5);
        assert!(
            (s.rendered_pos.x - expected).abs() < 0.02,
            "walks toward the target between updates: {} vs {expected}",
            s.rendered_pos.x
        );
    }

    #[test]
    fn is_chasing_holds_across_the_grace_window() {
        // B: once the rendered position reaches the wire target, the moving flag must stay up for one
        // expected interval past the last update so a late packet does not drop the gait to idle and
        // restart the walk clip from frame 0. A short hop (Normal band) is reached in well under one
        // smoothed interval, so the entity sits on target inside the grace window.
        let mut s = chase_sample(Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, 0.4, 40);
        let dt = 1.0 / 60.0;
        advance_prediction(&mut s, dt); // consumes the update; hold_until is set here
        let hold_until = s.hold_until;
        assert!(
            hold_until > s.clock,
            "the grace window extends past this frame"
        );
        // Chase forward and stop on the first frame where it is both on target AND still inside the
        // grace window: that is exactly when a late packet would otherwise have dropped the gait.
        let mut held = false;
        for _ in 0..240 {
            advance_prediction(&mut s, dt);
            let on_target = (s.rendered_pos.x - 0.5).abs() < 1e-3;
            if on_target && s.clock < hold_until {
                held = true;
                break;
            }
        }
        assert!(held, "the entity sat on target inside the grace window");
        // On target but inside the grace window: is_chasing must still be true (no idle drop).
        assert!(s.is_chasing(), "the moving flag holds across a late packet");
        // Past the grace window with no new update, it finally reports idle.
        for _ in 0..240 {
            advance_prediction(&mut s, dt);
            if !s.is_chasing() {
                break;
            }
        }
        assert!(!s.is_chasing(), "idle once the grace window lapses");
    }

    #[test]
    fn prediction_heading_eases_toward_the_target() {
        // A heading change on an update must ease in over frames (HEADING_TAU), not snap: the
        // first frame moves partway toward the target and the rest converges on it.
        let mut s = PredictSample::seed(Vec3::ZERO, 0, 40, 40, false);
        let start = s.rendered_heading_rad;
        s.target_heading = 16; // a quarter turn from the seeded heading
        let target = heading_to_rad(16);
        assert!(
            (start - target).abs() > f32::EPSILON,
            "the test needs a real heading change"
        );
        let dist = |h: f32| {
            let mut d = (target - h).rem_euclid(std::f32::consts::TAU);
            if d > std::f32::consts::PI {
                d -= std::f32::consts::TAU;
            }
            d.abs()
        };
        advance_prediction(&mut s, 1.0 / 60.0);
        assert_ne!(
            s.rendered_heading_rad, target,
            "the first frame must not snap the heading"
        );
        assert!(
            dist(s.rendered_heading_rad) < dist(start),
            "heading moves toward the target"
        );
        for _ in 0..600 {
            advance_prediction(&mut s, 1.0 / 60.0);
        }
        assert!(
            dist(s.rendered_heading_rad) < 1e-3,
            "converges on the target heading"
        );
    }

    #[test]
    fn prediction_static_actor_does_not_drift() {
        let anchor = Vec3::new(3.0, 1.0, 2.0);
        let mut s = PredictSample::seed(anchor, 64, 0, 0, false);
        for _ in 0..60 {
            advance_prediction(&mut s, 1.0 / 30.0);
        }
        assert!(
            (s.rendered_pos - anchor).length() < 0.05,
            "stays put: {:?}",
            s.rendered_pos
        );
    }

    #[test]
    fn observe_seeds_then_flags_only_on_real_move() {
        let mut p = EntityPrediction::default();
        p.observe(7, Vec3::new(1.0, 0.0, 0.0), 10, 25, 40, false);
        let s = p.by_id[&7];
        assert!(
            s.initialized && !s.sample_dirty,
            "first sight seeds, not dirty"
        );
        assert_eq!(s.rendered_pos, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(s.packet_speed, 25);
        assert_eq!(s.packet_speed_base, 40);
        assert!(!s.is_chasing(), "a fresh sample is already on target");

        p.observe(7, Vec3::new(1.0, 0.0, 0.0), 10, 25, 40, false);
        assert!(
            !p.by_id[&7].sample_dirty,
            "unchanged position must not re-ingest"
        );

        p.observe(7, Vec3::new(2.0, 0.0, 0.0), 20, 40, 50, false);
        assert!(p.by_id[&7].sample_dirty, "moved position raises dirty");
        assert_eq!(p.by_id[&7].target_heading, 20);
        assert_eq!(p.by_id[&7].packet_speed, 40, "speed byte tracks the update");
        assert_eq!(
            p.by_id[&7].packet_speed_base, 50,
            "animationSpeed byte tracks the update"
        );

        // Mount state no longer feeds the pace: the step model derives it from the wire speed byte
        // and the measured inter-update cadence (advance_prediction), so observe() leaves chase_rate
        // at its seeded value regardless of mount.
        let seeded_rate = expected_step_yalms(25, 40) / EntityPrediction::DT_SERVER_CEIL;
        p.observe(7, Vec3::new(2.0, 0.0, 0.0), 20, 40, 50, true);
        assert!(
            (p.by_id[&7].chase_rate - seeded_rate).abs() < 1e-6,
            "observe does not re-pace the chase: {} vs seeded {}",
            p.by_id[&7].chase_rate,
            seeded_rate
        );
    }

    #[test]
    fn rest_stance_is_resting_matches_kind() {
        let mut s = RestStance::default();
        assert!(!s.is_resting());
        s.kind = RestKind::Sit;
        assert!(s.is_resting());
        s.kind = RestKind::Heal;
        assert!(s.is_resting());
        s.kind = RestKind::None;
        assert!(!s.is_resting());
    }

    #[test]
    fn reconcile_adopts_the_server_rest_state() {
        use ffxi_proto::decode::animation;

        assert_eq!(
            reconcile_rest_kind(RestKind::Heal, animation::NONE),
            Some(RestKind::None),
            "the server ended the rest (damage, effect wearing) — stand up"
        );
        assert_eq!(
            reconcile_rest_kind(RestKind::None, animation::HEALING),
            Some(RestKind::Heal),
            "the server has us resting — adopt it"
        );
        assert_eq!(
            reconcile_rest_kind(RestKind::Heal, animation::HEALING),
            None,
            "agreement needs no correction"
        );
        assert_eq!(reconcile_rest_kind(RestKind::None, animation::NONE), None);
    }

    #[test]
    fn reconcile_never_stands_up_a_client_side_sit() {
        use ffxi_proto::decode::animation;

        assert_eq!(
            reconcile_rest_kind(RestKind::Sit, animation::NONE),
            None,
            "/sit sends no packet, so a 0 byte must not cancel it"
        );
        assert_eq!(
            reconcile_rest_kind(RestKind::Sit, animation::HEALING),
            Some(RestKind::Heal),
            "a server-side rest still wins over the local sit"
        );
    }

    #[test]
    fn rest_exit_blocks_movement_until_the_stand_up_clip_ends() {
        let mut s = RestStance {
            kind: RestKind::Heal,
            ..Default::default()
        };
        s.begin_exit();
        assert_eq!(s.kind, RestKind::None, "the stance is released immediately");

        let dt = 1.0 / 30.0;
        assert!(s.exit_blocks_movement(dt), "pending bridge holds movement");

        s.observe_exit_clip(true);
        for _ in 0..60 {
            assert!(
                s.exit_blocks_movement(dt),
                "the Out clip outlasts the pending grace"
            );
        }

        s.observe_exit_clip(false);
        assert!(!s.exit_blocks_movement(dt), "movement resumes once it ends");
    }

    #[test]
    fn rest_exit_pending_expires_without_a_stand_up_clip() {
        let mut s = RestStance::default();
        s.begin_exit();
        let dt = 1.0 / 30.0;
        let mut ticks = 0;
        while s.exit_blocks_movement(dt) {
            ticks += 1;
            assert!(ticks < 1000, "pending must not block movement forever");
        }
        assert!(
            (ticks as f32) * dt >= RestExit::HANDOFF_SECS - dt,
            "the grace should run out roughly at HANDOFF_SECS, not instantly"
        );
        assert_eq!(s.exit, RestExit::Idle);
    }

    #[test]
    fn resting_again_clears_a_finished_exit() {
        let mut s = RestStance::default();
        s.begin_exit();
        s.observe_exit_clip(true);
        s.kind = RestKind::Heal;
        s.observe_exit_clip(false);
        assert!(!s.exit_blocks_movement(1.0 / 30.0));
    }

    // KULUU_MOTION_LOG probe: snap-band counting, toggle counting, and rising-edge mismatch
    // detection.
    #[test]
    fn motion_probe_counts_snap_bands() {
        let mut p = MotionProbe::enabled_for_test();
        for band in [SnapBand::Normal, SnapBand::Stretch, SnapBand::Pop] {
            p.record_update(
                1,
                EntityKind::Mob,
                UpdateOutcome {
                    dt_server: 0.4,
                    jump_sq: 1.0,
                    band,
                    step_yalms: 1.0,
                    speed: 40,
                    speed_base: 40,
                    idle_frames: 12,
                    rem_dist: 0.0,
                },
            );
        }
        let e = &p.per_id[&1];
        assert_eq!((e.updates, e.normals, e.stretches, e.pops), (3, 1, 1, 1));
        assert_eq!(p.all_intervals.len(), 3);
    }

    #[test]
    fn motion_probe_counts_moving_toggles() {
        let mut p = MotionProbe::enabled_for_test();
        p.record_toggle(9, EntityKind::Mob, true, 0.9);
        p.record_toggle(9, EntityKind::Mob, false, 0.3);
        assert_eq!(p.per_id[&9].toggles, 2);
    }

    #[test]
    fn motion_probe_mismatch_is_rising_edge() {
        let mut p = MotionProbe::enabled_for_test();
        // heading 0 faces -z; travelling +x is a full quarter turn sideways.
        p.record_heading_mismatch(3, EntityKind::Mob, 0.0, Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(p.per_id[&3].mismatch_events, 1);
        // still sideways: no second event for the same episode
        p.record_heading_mismatch(3, EntityKind::Mob, 0.05, Vec3::new(4.9, 0.0, 0.6));
        assert_eq!(p.per_id[&3].mismatch_events, 1);
        // aligned: the episode ends without an event
        p.record_heading_mismatch(3, EntityKind::Mob, 0.0, Vec3::new(0.0, 0.0, -5.0));
        assert_eq!(p.per_id[&3].mismatch_events, 1);
        // a new sideways episode counts again
        p.record_heading_mismatch(3, EntityKind::Mob, 0.0, Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(p.per_id[&3].mismatch_events, 2);
    }

    #[test]
    fn motion_probe_ignores_crawl_speed_for_mismatches() {
        let mut p = MotionProbe::enabled_for_test();
        // below MIN_MEANINGFUL_SPEED_SQ the direction is noise: no event and
        // not even an entry (the early return precedes first-sight bookkeeping)
        p.record_heading_mismatch(4, EntityKind::Mob, 0.0, Vec3::new(0.05, 0.0, 0.0));
        assert!(!p.per_id.contains_key(&4), "crawl speed records nothing");
    }

    #[test]
    fn motion_probe_quantiles() {
        let mut samples = std::collections::VecDeque::new();
        for v in [1.0f32, 0.5, 0.75, 0.25, 2.0] {
            samples.push_back(v);
        }
        let (med, p90) = quantiles(&samples);
        assert_eq!(med, Some(0.75));
        assert_eq!(p90, Some(2.0), "five samples: p90 rounds to the top");
    }

    #[test]
    fn combat_run_resolves_with_higher_bone_count_than_casual() {
        if DatRoot::from_env_or_default().is_err() {
            eprintln!("skipping: no retail DAT root");
            return;
        }
        for skel in [7072u32, 10248, 13424, 16600, 19776, 23176, 26352] {
            let casual = run_anim_for_skel(skel).expect("casual run");
            let combat = combat_run_anim_for_skel(skel).expect("combat run");
            assert!(
                combat.per_bone.len() >= casual.per_bone.len(),
                "skel {skel}: combat run ({}) should have ≥ bones than casual ({})",
                combat.per_bone.len(),
                casual.per_bone.len()
            );
        }
    }
}
