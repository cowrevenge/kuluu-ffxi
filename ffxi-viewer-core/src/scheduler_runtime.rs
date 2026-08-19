use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use bevy::prelude::*;
use ffxi_dat::generator::Generator;
use ffxi_dat::kind::ChunkKind;
use ffxi_dat::scheduler::{Scheduler, StageKind, TimedStage};
use ffxi_dat::sep::Sep;

// research/xim util/Fps.kt:9 — `internalFps = 60.0` is the clock every effect routine and
// particle generator is authored against (poc/MainTool.kt:118 feeds the raw elapsed frames to
// EffectManager). Only the skeleton domain is halved: poc/ActorManager.kt:59-62 "In game,
// skeletal animations are only updated every other frame" — see SKELETON_FRAME_DIVISOR.
pub const ROUTINE_FPS: f32 = 60.0;

// research/xim poc/ActorManager.kt:59-62 — `elapsedFrames / 2f` into updateAnimation.
pub const SKELETON_FRAME_DIVISOR: f32 = 2.0;

// The rate the retail/vanilla client renders at, distinct from the 60 fps routine clock above.
// Anything authored per *rendered* frame — cloud texture-coordinate velocities, the targeted
// nameplate pulse — advances on this one.
pub const RETAIL_FPS: f32 = 30.0;

const POST_FINISH_TTL_SECS: f32 = 2.0;

// The entity the currently-running routine is aimed at. Written in the same commands chain as
// `ActiveScheduler` by every routine dispatcher, so it can never outlive the routine that set it.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct ActionTarget(pub Option<Entity>);

// research/xim ParticleGeneratorAttachment.kt:64-111 — Target*/TargetToSourceBasis read the
// primary target's position, every other attach type the source actor's. `None` falls back to
// the caster so an untracked target never drops the routine.
pub fn particle_origin_entity(
    attach: ffxi_dat::particle_gen::AttachType,
    caster: Entity,
    target: Option<Entity>,
) -> Entity {
    use ffxi_dat::particle_gen::AttachType;
    match attach {
        AttachType::TargetActor
        | AttachType::TargetActorSourceFacing
        | AttachType::TargetToSourceBasis => target.unwrap_or(caster),
        _ => caster,
    }
}

// ffxi-dat/src/action.rs::resolve_stage_to_se yields `on_caster` straight from the stage kind:
// a 0x0A/0x53 SoundOnCaster emits at the source actor, a 0x0B SoundOnTarget at the primary
// target. `None` falls back to the caster so an untracked target never silences the SE.
pub fn sound_origin_entity(on_caster: bool, caster: Entity, target: Option<Entity>) -> Entity {
    if on_caster {
        caster
    } else {
        target.unwrap_or(caster)
    }
}

// research/xim poc/MainTool.kt:250 — ROM/0/0.DAT is loaded as XIM's `GlobalDirectory`, the
// system-effect resource dir every routine falls back to (the cast aura `ner1` and its `stbk`
// stop live there, not in the caster's DAT). `DatRoot::resolve(0)` yields exactly that file.
pub const GLOBAL_EFFECT_DIR_FILE_ID: u32 = 0;

#[derive(Resource, Default)]
pub struct GlobalEffectDir {
    pub schedulers: Vec<Scheduler>,
    pub assets: ActionAssets,
}

// research/xim EffectRoutineInstance.kt:418-431 findResource — a routine id resolves against the
// routine's own DAT, then the actor's own dirs, then the global dir.
pub enum RoutineSource<'a> {
    Dat(&'a [Scheduler]),
    Actor(&'a HashMap<ffxi_dat::datid::DatId, Scheduler>),
}

#[derive(Default)]
pub struct RoutineLookup<'a> {
    tiers: Vec<RoutineSource<'a>>,
}

impl<'a> RoutineLookup<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_dat(mut self, schedulers: &'a [Scheduler]) -> Self {
        self.tiers.push(RoutineSource::Dat(schedulers));
        self
    }

    pub fn with_actor(mut self, routines: &'a HashMap<ffxi_dat::datid::DatId, Scheduler>) -> Self {
        self.tiers.push(RoutineSource::Actor(routines));
        self
    }

    pub fn get(&self, name: &[u8; 4]) -> Option<&'a Scheduler> {
        self.tiers.iter().find_map(|tier| match tier {
            RoutineSource::Dat(list) => list.iter().find(|s| &s.name == name),
            RoutineSource::Actor(map) => map.get(&ffxi_dat::datid::DatId::from_name(name)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionStages {
    Play,

    // The caster's looping cast pose is owned by ffxi_actor_render::dispatch_action_overlay; a
    // cast routine's own Motion stage would replace it with a one-shot.
    Suppress,
}

#[derive(Component, Debug, Clone)]
pub struct ActiveScheduler {
    pub stages: Vec<TimedStage>,

    pub elapsed: f32,

    pub cursor: usize,

    pub name: [u8; 4],
}

impl ActiveScheduler {
    pub fn from_scheduler(s: &Scheduler) -> Self {
        let mut stages = s.stages.clone();
        stages.sort_by_key(|t| t.frame);
        Self {
            stages,
            elapsed: 0.0,
            cursor: 0,
            name: s.name,
        }
    }

    // A retail effect routine's "main" scheduler delegates to sub-routines via 0x03 stages
    // (id = sub-scheduler name) — e.g. Cure's main calls tgt0, which holds the particle
    // spawns. Inline them at their call frame into one flat timeline.
    pub fn from_main(schedulers: &[Scheduler], name: &[u8; 4]) -> Option<Self> {
        Self::from_routine(&RoutineLookup::new().with_dat(schedulers), name)
    }

    pub fn from_routine(lookup: &RoutineLookup, name: &[u8; 4]) -> Option<Self> {
        Self::flatten(lookup, name, MotionStages::Play)
    }

    pub fn effects_only(lookup: &RoutineLookup, name: &[u8; 4]) -> Option<Self> {
        Self::flatten(lookup, name, MotionStages::Suppress)
    }

    // research/xim Actor.kt:864-903 — one swing enqueues TWO routines on the attacker: the
    // self-targeted voice routine (`atk0`) and the weapon swing (`ati0`/`bti0`/…). A single-slot
    // ActiveScheduler component cannot hold two, so their timelines are merged.
    pub fn effects_only_merged(lookup: &RoutineLookup, names: &[[u8; 4]]) -> Option<Self> {
        let first = *names.iter().find(|n| lookup.get(n).is_some())?;
        let mut stages = Vec::new();
        for name in names {
            let mut path = Vec::new();
            flatten_routine(
                lookup,
                name,
                0,
                MotionStages::Suppress,
                &mut path,
                &mut stages,
            );
        }
        stages.sort_by_key(|t| t.frame);
        Some(Self {
            stages,
            elapsed: 0.0,
            cursor: 0,
            name: first,
        })
    }

    fn flatten(lookup: &RoutineLookup, name: &[u8; 4], motion: MotionStages) -> Option<Self> {
        lookup.get(name)?;
        let mut stages = Vec::new();
        let mut path = Vec::new();
        flatten_routine(lookup, name, 0, motion, &mut path, &mut stages);
        stages.sort_by_key(|t| t.frame);
        Some(Self {
            stages,
            elapsed: 0.0,
            cursor: 0,
            name: *name,
        })
    }

    pub fn name(&self) -> [u8; 4] {
        self.name
    }

    pub fn finished(&self) -> bool {
        self.cursor >= self.stages.len()
    }

    pub fn current_frame(&self) -> u32 {
        (self.elapsed * ROUTINE_FPS) as u32
    }

    pub fn last_frame(&self) -> u32 {
        self.stages.last().map(|t| t.frame).unwrap_or(0)
    }
}

#[derive(Message, Debug, Clone, Copy)]
pub struct SchedulerStageEvent {
    pub actor: Entity,

    pub stage: TimedStage,

    pub scheduler: [u8; 4],
}

pub fn tick_active_schedulers(
    time: Res<Time>,
    mut q: Query<(Entity, &mut ActiveScheduler)>,
    mut writer: MessageWriter<SchedulerStageEvent>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut sched) in &mut q {
        sched.elapsed += dt;
        let frame_now = sched.current_frame();

        let scheduler_name = sched.name;
        while sched.cursor < sched.stages.len() {
            let next = sched.stages[sched.cursor];
            if next.frame > frame_now {
                break;
            }
            writer.write(SchedulerStageEvent {
                actor: entity,
                stage: next,
                scheduler: scheduler_name,
            });
            sched.cursor += 1;
        }

        if sched.finished() {
            let finish_secs = sched.last_frame() as f32 / ROUTINE_FPS;
            if sched.elapsed >= finish_secs + POST_FINISH_TTL_SECS {
                commands.entity(entity).remove::<ActiveScheduler>();
            }
        }
    }
}

// A zone-spray generator (e.g. Bastok "abuk", Port Windurst "rivsea") links an MMB
// mesh by its 4-byte DatId, not a D3M. Flattened here to sprite geometry so the
// particle sim can build a SpriteTemplate without re-parsing the MMB.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default)]
pub struct MmbSpriteMesh {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    // Stage 0's D argument per `positions` entry, /128-normalised like a D3m vertex colour so
    // both particle mesh sources feed `SpriteTemplate::colors` on the same scale.
    pub colors: Vec<[f32; 4]>,
    pub texture_name: String,
}

#[derive(Component, Debug, Clone, Default)]
pub struct ActionAssets {
    pub generators: HashMap<[u8; 4], Generator>,
    #[cfg(not(target_arch = "wasm32"))]
    pub d3ms: HashMap<[u8; 4], ffxi_dat::d3m::D3m>,
    #[cfg(not(target_arch = "wasm32"))]
    pub mmbs: HashMap<[u8; 4], MmbSpriteMesh>,
    // SpriteSheet (0x0E) particle meshes, keyed by the 0x21 chunk DatId a generator's
    // mesh_id references (e.g. Poison's `fir ` → 0x21 `fir`).
    #[cfg(not(target_arch = "wasm32"))]
    pub sprite_sheets: HashMap<[u8; 4], ffxi_dat::sprite_sheet::ParticleSpriteSheet>,
    pub seps: HashMap<[u8; 4], Sep>,
    pub animations: Vec<ffxi_dat::skel_anim::SkeletonAnimation>,
    #[cfg(not(target_arch = "wasm32"))]
    pub images: HashMap<[u8; 4], ffxi_dat::texture::DecodedTexture>,
    // Img chunks keyed by their INTERNAL name (bytes 0x09..0x11), which is what an
    // MMB model's texture_name references — distinct from the Img chunk's DatId.
    #[cfg(not(target_arch = "wasm32"))]
    pub images_by_name: HashMap<String, ffxi_dat::texture::DecodedTexture>,
    // Img chunks keyed by their fully qualified (namespace, local) name pair — the tier a
    // 0x21 sprite sheet's own 16-byte name field resolves against.
    #[cfg(not(target_arch = "wasm32"))]
    pub images_by_qualified_name: HashMap<(String, String), ffxi_dat::texture::DecodedTexture>,
    pub emitters: HashMap<[u8; 4], ffxi_dat::generator::ParticleEmitter>,
    pub particle_defs: HashMap<[u8; 4], ffxi_dat::particle_gen::ParticleGeneratorDef>,
    // The same defs keyed by (containing directory, name). ROM/0/0.DAT defines four different
    // generators called `g010`, one per effect directory; the flat map keeps only the last.
    pub particle_defs_by_dir:
        HashMap<([u8; 4], [u8; 4]), ffxi_dat::particle_gen::ParticleGeneratorDef>,
    pub keyframes: HashMap<[u8; 4], ffxi_dat::particle_gen::KeyFrameTrack>,
}

impl ActionAssets {
    // research/xim EffectRoutineInstance.kt:418-431 — `resource.localDir` first, wider scopes
    // after. `local_dir` is the directory of the routine the stage was authored in, carried on
    // the stage because flattening merges routines from several directories into one timeline.
    pub fn particle_def(
        &self,
        local_dir: [u8; 4],
        id: &[u8; 4],
    ) -> Option<&ffxi_dat::particle_gen::ParticleGeneratorDef> {
        self.particle_defs_by_dir
            .get(&(local_dir, *id))
            .or_else(|| self.particle_defs.get(id))
    }
}

const MAX_SUBROUTINE_DEPTH: usize = 6;

// Knuth's MMIX LCG. Every DAT-driven choice the format leaves unauthored (random routine
// branches, particle spawn spread, sound-emitter jitter) advances this same recurrence, so the
// pair lives here once rather than being retyped per consumer.
pub const LCG_MULTIPLIER: u64 = 6364136223846793005;
pub const LCG_INCREMENT: u64 = 1442695040888963407;

pub fn lcg_next(state: u64) -> u64 {
    state
        .wrapping_mul(LCG_MULTIPLIER)
        .wrapping_add(LCG_INCREMENT)
}

// research/xim EffectRoutineParser.kt:275-285 — a random block runs exactly one of its children
// per activation, and which one is not authored in the DAT.
static RANDOM_PICK_STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_random_pick(len: usize) -> usize {
    use std::sync::atomic::Ordering;
    let next = RANDOM_PICK_STATE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |s| Some(lcg_next(s)))
        .unwrap_or(1);
    if len == 0 {
        0
    } else {
        ((next >> 33) as usize) % len
    }
}

fn flatten_routine(
    lookup: &RoutineLookup,
    name: &[u8; 4],
    base_frame: u32,
    motion: MotionStages,
    path: &mut Vec<[u8; 4]>,
    out: &mut Vec<TimedStage>,
) {
    if path.len() > MAX_SUBROUTINE_DEPTH || path.contains(name) {
        return;
    }
    let Some(s) = lookup.get(name) else {
        return;
    };
    let mut chosen: HashMap<u16, usize> = HashMap::new();
    let mut seen_in_group: HashMap<u16, usize> = HashMap::new();
    for t in &s.stages {
        if let Some(g) = t.stage.random_group {
            *seen_in_group.entry(g).or_insert(0) += 1;
        }
    }
    for (&g, &count) in &seen_in_group {
        chosen.insert(g, next_random_pick(count));
    }
    let mut index_in_group: HashMap<u16, usize> = HashMap::new();

    path.push(*name);
    for t in &s.stages {
        if let Some(g) = t.stage.random_group {
            let i = index_in_group.entry(g).or_insert(0);
            let is_pick = chosen.get(&g) == Some(i);
            *i += 1;
            if !is_pick {
                continue;
            }
        }
        let frame = base_frame + t.frame;
        match t.stage.kind {
            StageKind::SubRoutine | StageKind::BlockingSubRoutine => {
                // A control-flow routine is a switch we cannot evaluate (`dada` tail-calls
                // `dam0`, ten mutually exclusive additional-effect branches); inlining it would
                // run every branch. Callers that know the condition dispatch the branch itself.
                if lookup
                    .get(&t.stage.id)
                    .is_some_and(|c| c.has_control_flow())
                {
                    continue;
                }
                flatten_routine(lookup, &t.stage.id, frame, motion, path, out)
            }
            StageKind::Motion if motion == MotionStages::Suppress => {}
            _ => out.push(TimedStage {
                frame,
                stage: t.stage,
            }),
        }
    }
    path.pop();
}

// A generator and the mesh/sheet/texture it references always ship in the same DAT, so a stage
// resolves against whichever single ActionAssets actually holds it — the routine's own (on the
// tracked entity) or the global effect dir's.
pub fn assets_holding<'a>(
    local: Option<&'a ActionAssets>,
    global: Option<&'a ActionAssets>,
    has: impl Fn(&ActionAssets) -> bool,
) -> Option<&'a ActionAssets> {
    local
        .filter(|a| has(a))
        .or_else(|| global.filter(|a| has(a)))
}

// A routine and the generators it names share a chunk directory, and those names are only unique
// within it, so the walk carries the enclosing directory alongside each chunk.
fn walk_with_dirs(
    node: &ffxi_dat::chunk::ChunkNode<'_>,
    visit: &mut dyn FnMut([u8; 4], &ffxi_dat::chunk::Chunk<'_>),
) {
    fn rec<'a>(
        node: &ffxi_dat::chunk::ChunkNode<'a>,
        dir: [u8; 4],
        visit: &mut dyn FnMut([u8; 4], &ffxi_dat::chunk::Chunk<'a>),
    ) {
        let dir = if node.chunk.kind == ChunkKind::Rmp as u8 {
            node.chunk.name
        } else {
            visit(dir, &node.chunk);
            dir
        };
        for child in &node.children {
            rec(child, dir, visit);
        }
    }
    rec(node, ffxi_dat::scheduler::NO_LOCAL_DIR, visit);
}

pub fn parse_action_bytes(bytes: &[u8]) -> (Vec<Scheduler>, ActionAssets) {
    parse_action_tree(&ffxi_dat::chunk::walk_tree(bytes))
}

// Chunk ids are only unique within a directory, and a zone DAT repeats them across weat/ subtrees
// (zone 123 carries `clod` and `hm01..hm15` under both weat/rain and weat/squl), so a consumer
// that owns one subtree must build its assets from that subtree alone or it binds the wrong
// mesh/texture/keyframe.
pub fn parse_action_tree(node: &ffxi_dat::chunk::ChunkNode<'_>) -> (Vec<Scheduler>, ActionAssets) {
    let mut schedulers = Vec::new();
    let mut assets = ActionAssets::default();
    walk_with_dirs(node, &mut |dir, c| {
        let Some(kind) = ChunkKind::from_u8(c.kind) else {
            return;
        };
        match kind {
            ChunkKind::Scheduler => {
                if let Ok(s) = Scheduler::parse_in_dir(dir, c.name, c.data) {
                    schedulers.push(s);
                }
            }
            ChunkKind::Generator => {
                if let Ok(Some(g)) = Generator::parse(c.name, c.data) {
                    assets.generators.insert(c.name, g);
                }
                if let Ok(Some(e)) = Generator::parse_particle_emitter(c.data) {
                    assets.emitters.insert(c.name, e);
                }
                if let Ok(Some(d)) = ffxi_dat::particle_gen::ParticleGeneratorDef::parse(c.data) {
                    assets.particle_defs.insert(c.name, d);
                    assets.particle_defs_by_dir.insert((dir, c.name), d);
                }
            }
            ChunkKind::KeyFrame => {
                assets
                    .keyframes
                    .insert(c.name, ffxi_dat::particle_gen::KeyFrameTrack::parse(c.data));
            }
            #[cfg(not(target_arch = "wasm32"))]
            ChunkKind::D3m => {
                if let Ok(d) = ffxi_dat::d3m::D3m::parse(c.name, c.data) {
                    assets.d3ms.insert(c.name, d);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            ChunkKind::Mmb => {
                if let Some(mesh) = mmb_sprite_mesh(c.data) {
                    assets.mmbs.insert(c.name, mesh);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            ChunkKind::SpriteSheet => {
                if let Some(ss) = ffxi_dat::sprite_sheet::ParticleSpriteSheet::parse(c.data) {
                    assets.sprite_sheets.insert(c.name, ss);
                }
            }
            ChunkKind::Sep => {
                if let Ok(s) = Sep::parse(c.name, c.data) {
                    assets.seps.insert(c.name, s);
                }
            }
            ChunkKind::AnimMo2 => {
                let id = ffxi_dat::datid::DatId::from_name(&c.name);
                assets
                    .animations
                    .push(ffxi_dat::skel_anim::parse(id, c.data));
            }
            #[cfg(not(target_arch = "wasm32"))]
            ChunkKind::Img => {
                if let Ok(tex) = ffxi_dat::texture::decode_texture(c.data) {
                    if let Some((category, id)) = ffxi_dat::texture::extract_texture_tokens(c.data)
                    {
                        assets
                            .images_by_qualified_name
                            .insert((category, id.clone()), tex.clone());
                        assets.images_by_name.insert(id, tex.clone());
                    }
                    assets.images.insert(c.name, tex);
                }
            }
            _ => {}
        }
    });
    (schedulers, assets)
}

#[cfg(not(target_arch = "wasm32"))]
fn mmb_sprite_mesh(data: &[u8]) -> Option<MmbSpriteMesh> {
    let dec = ffxi_dat::mmb::decrypt(data).ok()?;
    let models = ffxi_dat::mmb::parse_models(&dec);
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let mut texture_name = String::new();
    for m in &models {
        if m.vertices.is_empty() || m.indices.is_empty() {
            continue;
        }
        if texture_name.is_empty() && !m.texture_name.is_empty() {
            texture_name = m.texture_name.clone();
        }
        let base = positions.len() as u32;
        let vert_count = m.vertices.len() as u16;
        for v in &m.vertices {
            positions.push(v.pos);
            uvs.push(v.uv);
            colors.push(
                v.rgba
                    .map(|c| c as f32 / ffxi_dat::d3m::VERTEX_COLOR_DIVISOR),
            );
        }
        for tri in m.indices.chunks_exact(3) {
            if tri.iter().all(|&i| i < vert_count) {
                indices.extend(tri.iter().map(|&i| base + i as u32));
            }
        }
    }
    if positions.is_empty() || indices.is_empty() {
        return None;
    }
    Some(MmbSpriteMesh {
        positions,
        uvs,
        colors,
        indices,
        texture_name,
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub(crate) struct GlobalEffectDirTask(bevy::tasks::Task<(Vec<Scheduler>, ActionAssets)>);

// ROM/0/0.DAT is ~540 KB of ~1000 chunks including many Img decodes; parsing it on the render
// thread reproduces the actor-load hitch, so it loads once off-thread and every lookup falls
// back to the pre-global behaviour until it lands.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load_global_effect_dir(mut commands: Commands) {
    let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let bytes = ffxi_dat::DatRoot::from_env_or_default()
            .ok()
            .and_then(|root| {
                let loc = root.resolve(GLOBAL_EFFECT_DIR_FILE_ID).ok()?;
                std::fs::read(loc.path_under(&root)).ok()
            })
            .unwrap_or_default();
        parse_action_bytes(&bytes)
    });
    commands.insert_resource(GlobalEffectDirTask(task));
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn poll_global_effect_dir(
    task: Option<ResMut<GlobalEffectDirTask>>,
    mut commands: Commands,
) {
    use bevy::tasks::futures_lite::future;
    let Some(mut task) = task else { return };
    let Some((schedulers, assets)) = future::block_on(future::poll_once(&mut task.0)) else {
        return;
    };
    commands.remove_resource::<GlobalEffectDirTask>();
    commands.insert_resource(GlobalEffectDir { schedulers, assets });
}

#[cfg(not(target_arch = "wasm32"))]
pub struct ParsedActionDat {
    pub schedulers: Vec<Scheduler>,
    pub assets: ActionAssets,
}

// Populated Jeuno fires several casts/WS per second and each re-visits a handful of files, so a
// small window over the recently seen action DATs already turns repeat casts into pure hits.
#[cfg(not(target_arch = "wasm32"))]
const ACTION_DAT_CACHE_CAP: usize = 32;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) struct ActionDatLru {
    map: HashMap<u32, Arc<ParsedActionDat>>,
    order: std::collections::VecDeque<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ActionDatLru {
    pub(crate) fn get_and_promote(&mut self, file_id: u32) -> Option<Arc<ParsedActionDat>> {
        let hit = self.map.get(&file_id).cloned()?;
        self.order.retain(|k| *k != file_id);
        self.order.push_back(file_id);
        Some(hit)
    }

    pub(crate) fn insert(&mut self, file_id: u32, parsed: Arc<ParsedActionDat>) {
        if self.map.insert(file_id, parsed).is_some() {
            self.order.retain(|k| *k != file_id);
        }
        self.order.push_back(file_id);
        while self.map.len() > ACTION_DAT_CACHE_CAP {
            let Some(evict) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&evict);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum PendingActionDispatch {
    Action {
        actor_id: u32,
        target_id: Option<u32>,
    },
    Emote {
        actor_id: u32,
        target_id: u32,
        routine: [u8; 4],
    },
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource, Default)]
pub struct ActionDatCache {
    lru: ActionDatLru,
    tasks: HashMap<u32, bevy::tasks::Task<ParsedActionDat>>,
    pending: Vec<(u32, PendingActionDispatch)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ActionDatCache {
    fn request(&mut self, file_id: u32) {
        if self.tasks.contains_key(&file_id) {
            return;
        }
        let task =
            bevy::tasks::AsyncComputeTaskPool::get().spawn(async move { load_action_dat(file_id) });
        self.tasks.insert(file_id, task);
    }

    fn defer(&mut self, file_id: u32, dispatch: PendingActionDispatch) {
        self.request(file_id);
        self.pending.push((file_id, dispatch));
    }
}

// An unresolvable/unreadable file caches as an empty parse, so a broken DAT path degrades to the
// pre-existing "no effect" behaviour instead of re-spawning a load per cast.
#[cfg(not(target_arch = "wasm32"))]
fn load_action_dat(file_id: u32) -> ParsedActionDat {
    let bytes = ffxi_dat::DatRoot::from_env_or_default()
        .ok()
        .and_then(|root| {
            let loc = root.resolve(file_id).ok()?;
            std::fs::read(loc.path_under(&root)).ok()
        })
        .unwrap_or_default();
    let (schedulers, assets) = parse_action_bytes(&bytes);
    ParsedActionDat { schedulers, assets }
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_action_dispatch(
    parsed: &ParsedActionDat,
    actor_routines: Option<&HashMap<ffxi_dat::datid::DatId, Scheduler>>,
    global: Option<&GlobalEffectDir>,
    actor_entity: Entity,
    target_entity: Option<Entity>,
    commands: &mut Commands,
) {
    // A spell DAT's `main` links the caster's own finish routine (0x3C `shbk`), which in turn
    // links global-dir routines — so the flatten must span all three tiers.
    let mut lookup = RoutineLookup::new().with_dat(&parsed.schedulers);
    if let Some(r) = actor_routines {
        lookup = lookup.with_actor(r);
    }
    if let Some(g) = global {
        lookup = lookup.with_dat(&g.schedulers);
    }
    let active = ActiveScheduler::from_routine(&lookup, b"main").or_else(|| {
        parsed
            .schedulers
            .first()
            .map(ActiveScheduler::from_scheduler)
    });
    let Some(active) = active else { return };
    commands
        .entity(actor_entity)
        .try_insert(active)
        .try_insert(parsed.assets.clone())
        .try_insert(ActionTarget(target_entity));
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_emote_dispatch(
    parsed: &ParsedActionDat,
    routine: &[u8; 4],
    actor_entity: Entity,
    target_entity: Option<Entity>,
    commands: &mut Commands,
) -> bool {
    let Some(active) = ActiveScheduler::from_main(&parsed.schedulers, routine) else {
        return false;
    };
    commands
        .entity(actor_entity)
        .try_insert(active)
        .try_insert(parsed.assets.clone())
        .try_insert(ActionTarget(target_entity));
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn actor_routines_via_mut<'a>(
    entity: Entity,
    q_children: &Query<&Children>,
    q_actors: &'a Query<&mut crate::ffxi_actor_render::FfxiRenderActor>,
) -> Option<&'a HashMap<ffxi_dat::datid::DatId, Scheduler>> {
    q_children
        .get(entity)
        .ok()?
        .iter()
        .find_map(|child| q_actors.get(child).ok())
        .map(|actor| actor.routines())
}

// Applies dispatches whose action-DAT parse has landed. A cache miss therefore delays the
// completion effect by the load's frames-in-flight instead of stalling the frame it arrived on;
// the routine's internal timeline (motion + particles + SE) shifts as one unit.
#[cfg(not(target_arch = "wasm32"))]
pub fn poll_action_dat_tasks(
    mut cache: ResMut<ActionDatCache>,
    tracked: Res<crate::scene::TrackedEntities>,
    q_children: Query<&Children>,
    mut q_actors: Query<&mut crate::ffxi_actor_render::FfxiRenderActor>,
    global: Option<Res<GlobalEffectDir>>,
    mut commands: Commands,
) {
    use bevy::tasks::futures_lite::future;
    if cache.tasks.is_empty() && cache.pending.is_empty() {
        return;
    }
    let mut landed = Vec::new();
    cache.tasks.retain(
        |file_id, task| match future::block_on(future::poll_once(task)) {
            Some(parsed) => {
                landed.push((*file_id, Arc::new(parsed)));
                false
            }
            None => true,
        },
    );
    for (file_id, parsed) in landed {
        cache.lru.insert(file_id, parsed);
    }
    if cache.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut cache.pending);
    for (file_id, dispatch) in pending {
        let Some(parsed) = cache.lru.get_and_promote(file_id) else {
            // Still in flight — or evicted before this entry drained, in which case re-request.
            cache.defer(file_id, dispatch);
            continue;
        };
        match dispatch {
            PendingActionDispatch::Action {
                actor_id,
                target_id,
            } => {
                let Some(&actor_entity) = tracked.by_id.get(&actor_id) else {
                    continue;
                };
                let target_entity = target_id.and_then(|id| tracked.by_id.get(&id).copied());
                let actor_routines = actor_routines_via_mut(actor_entity, &q_children, &q_actors);
                apply_action_dispatch(
                    &parsed,
                    actor_routines,
                    global.as_deref(),
                    actor_entity,
                    target_entity,
                    &mut commands,
                );
            }
            PendingActionDispatch::Emote {
                actor_id,
                target_id,
                routine,
            } => {
                let Some(&actor_entity) = tracked.by_id.get(&actor_id) else {
                    continue;
                };
                let target_entity = tracked.by_id.get(&target_id).copied();
                if !apply_emote_dispatch(
                    &parsed,
                    &routine,
                    actor_entity,
                    target_entity,
                    &mut commands,
                ) {
                    play_local_emote_clip(&routine, actor_entity, &q_children, &mut q_actors);
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_sound_stages(
    mut events: MessageReader<SchedulerStageEvent>,
    q_actors: Query<&ActionAssets>,
    q_children: Query<&Children>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    q_target: Query<&ActionTarget>,
    // `Transform`, not `GlobalTransform`, for the same reason spawn_particle_generators reads
    // it: world entities are roots, and a frame-0 stage fires on the insert frame, before
    // PostUpdate has propagated anything — a `GlobalTransform` read there is Ok-but-identity,
    // which would place the emitter at the world origin and get it culled to silence.
    q_transform: Query<&Transform>,
    global: Option<Res<GlobalEffectDir>>,
    mut sfx_writer: MessageWriter<crate::audio::SfxEvent>,
) {
    for ev in events.read() {
        let kind = ev.stage.stage.kind;
        if !matches!(
            kind,
            StageKind::SoundOnCaster | StageKind::SoundOnTarget | StageKind::SoundNonPositional
        ) {
            continue;
        }
        // research/xim EffectRoutineInstance.kt:418-431,592-604 — routine DAT, then the actor's
        // own resource dirs (weapon `skaz`, face `atk1..4`), then the global dir.
        let actor_assets = q_children
            .get(ev.actor)
            .ok()
            .and_then(|c| c.iter().find_map(|child| q_render.get(child).ok()))
            .map(|a| a.action_assets());
        let tiers = [
            q_actors.get(ev.actor).ok(),
            actor_assets,
            global.as_ref().map(|g| &g.assets),
        ];
        let Some((se_id, on_caster)) = tiers.into_iter().flatten().find_map(|a| {
            ffxi_dat::action::resolve_stage_to_se(&ev.stage.stage.id, kind, &a.generators, &a.seps)
        }) else {
            continue;
        };

        // A 0x4A/0x60 stage has no world emitter: it mixes dry, like a UI or
        // weather cue, so it must not be sited on an actor and attenuated.
        if kind == StageKind::SoundNonPositional {
            sfx_writer.write(crate::audio::SfxEvent::new(se_id));
            continue;
        }
        let target = q_target.get(ev.actor).ok().and_then(|t| t.0);
        let origin = sound_origin_entity(on_caster, ev.actor, target);
        sfx_writer.write(match q_transform.get(origin) {
            Ok(xf) => crate::audio::SfxEvent::at(se_id, xf.translation),
            Err(_) => crate::audio::SfxEvent::new(se_id),
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_motion_stages(
    mut events: MessageReader<SchedulerStageEvent>,
    q_children: Query<&Children>,
    q_assets: Query<&ActionAssets>,
    global: Option<Res<GlobalEffectDir>>,
    mut q_actors: Query<&mut crate::ffxi_actor_render::FfxiRenderActor>,
) {
    for ev in events.read() {
        if ev.stage.stage.kind != StageKind::Motion {
            continue;
        }
        // research/xim EffectRoutineInterpolatedEffects.kt:49 — a skill's body motion is
        // resolved against the skill DAT's own clips first, then the caster's animation
        // directories. ActionAssets lives on the tracked entity the scheduler runs on; the
        // render actor is its child.
        let stage = ev.stage.stage;
        let clip = ffxi_dat::datid::DatId::from_name(&stage.id);
        let local_clips: &[ffxi_dat::skel_anim::SkeletonAnimation] = assets_holding(
            q_assets.get(ev.actor).ok(),
            global.as_ref().map(|g| &g.assets),
            |a| {
                a.animations
                    .iter()
                    .any(|an| an.id.parameterized_match(&clip))
            },
        )
        .map(|a| a.animations.as_slice())
        .unwrap_or(&[]);
        let Ok(children) = q_children.get(ev.actor) else {
            continue;
        };
        for &child in children {
            if let Ok(mut actor) = q_actors.get_mut(child) {
                actor.begin_completion_motion(
                    clip,
                    crate::ffxi_actor_render::CompletionMotion {
                        local_clips,
                        duration_frames: stage.duration_frames as f32,
                        max_loops: stage.max_loops,
                        transition_in: stage.transition_in,
                        transition_out: stage.transition_out,
                    },
                );
            }
        }
    }
}

pub fn action_dat_file_id(
    action_id: u32,
    animation: Option<u16>,
    action_kind: u8,
    race: Option<u8>,
    main_dll: Option<&ffxi_dat::main_dll::MainDll>,
) -> Option<u32> {
    // research/xim EffectDisplayer.displaySkill: the completion effect routine for a
    // skill lives in the file-table DAT keyed by the skill's animation index, which s2c 0x028
    // carries per result. Only the "finish" action categories carry that completed skill —
    // start categories drive the caster's cast-loop motion instead (see
    // ffxi_actor_render::action_routine). vendor/server enums/action/category.h:
    // 3 = weaponskill finish, 4 = magic finish, 6 = job-ability finish.
    match action_kind {
        3 => weapon_skill_file_id(animation?, race?, main_dll?),
        4 => ffxi_vocab::action_anim::spell_file_id(action_id, animation),
        6 => ffxi_vocab::action_anim::ability_file_id(action_id, animation),
        _ => None,
    }
}

// research/xim AbilityTable.kt:103 — WS file id = race base (FFXiMain.dll) + per-skill index.
// `race` is the FFXI look race byte (HumeM=1..Galka=8), which is XIM's RaceGenderConfig.index.
fn weapon_skill_file_id(
    animation: u16,
    race: u8,
    main_dll: &ffxi_dat::main_dll::MainDll,
) -> Option<u32> {
    let base = main_dll.base_weapon_skill_index(race)?;
    Some(base as u32 + animation as u32)
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub struct MainDllCache {
    loaded: bool,
    dll: Option<ffxi_dat::main_dll::MainDll>,
}

#[cfg(not(target_arch = "wasm32"))]
fn look_race(look: &ffxi_viewer_wire::EntityLook) -> Option<u8> {
    match look {
        ffxi_viewer_wire::EntityLook::Equipped { race, .. } => Some(*race),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_action_started(
    events: Res<crate::snapshot::EventLog>,
    tracked: Res<crate::scene::TrackedEntities>,
    q_look: Query<&crate::components::LookComp>,
    q_children: Query<&Children>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    global: Option<Res<GlobalEffectDir>>,
    mut dll_cache: Local<MainDllCache>,
    mut cache: ResMut<ActionDatCache>,
    mut commands: Commands,
    mut last_seen: Local<u64>,
) {
    let new_count =
        (events.pushed_total.saturating_sub(*last_seen)).min(events.recent.len() as u64) as usize;
    *last_seen = events.pushed_total;
    if new_count == 0 {
        return;
    }
    for ev in events.recent.iter().rev().take(new_count).rev() {
        let ffxi_viewer_wire::ViewerEvent::ActionStarted {
            actor_id,
            action_id,
            action_kind,
            target_id,
            animation,
            ..
        } = *ev
        else {
            continue;
        };
        let Some(&actor_entity) = tracked.by_id.get(&actor_id) else {
            continue;
        };
        let target_entity = target_id.and_then(|id| tracked.by_id.get(&id).copied());
        let race = q_look.get(actor_entity).ok().and_then(|l| look_race(&l.0));
        // FFXiMain.dll is only needed for weaponskill base indices; load it lazily once.
        if action_kind == 3 && !dll_cache.loaded {
            dll_cache.loaded = true;
            if let Ok(root) = ffxi_dat::DatRoot::from_env_or_default() {
                dll_cache.dll = ffxi_dat::main_dll::MainDll::load(root.root()).ok();
            }
        }
        let Some(file_id) = action_dat_file_id(
            action_id,
            animation,
            action_kind,
            race,
            dll_cache.dll.as_ref(),
        ) else {
            continue;
        };

        match cache.lru.get_and_promote(file_id) {
            Some(parsed) => {
                let actor_routines = actor_render_routines(actor_entity, &q_children, &q_render);
                apply_action_dispatch(
                    &parsed,
                    actor_routines,
                    global.as_deref(),
                    actor_entity,
                    target_entity,
                    &mut commands,
                );
            }
            None => cache.defer(
                file_id,
                PendingActionDispatch::Action {
                    actor_id,
                    target_id,
                },
            ),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn actor_render_routines<'a>(
    entity: Entity,
    q_children: &'a Query<&Children>,
    q_render: &'a Query<&crate::ffxi_actor_render::FfxiRenderActor>,
) -> Option<&'a HashMap<ffxi_dat::datid::DatId, Scheduler>> {
    q_children
        .get(entity)
        .ok()?
        .iter()
        .find_map(|child| q_render.get(child).ok())
        .map(|actor| actor.routines())
}

// The routine the caster's cast-start effects were flattened from, so an interrupt can stop the
// generators it spawned. research/xim Actor.kt:263-266 startCasting enqueues the whole model
// routine, not just its Motion stage.
// `posed` latches once the caster is observed in the looping cast pose. Cast routines with no
// Motion stage (retail `caso`/`calg`/`cage`) never set it, so the heuristic teardown below must
// not read "not posing" as "cast over" — for those the 0x2D stops and the interrupt signal are
// the only correct ends.
#[derive(Component, Debug, Clone, Copy)]
pub struct CastRoutine {
    pub routine: [u8; 4],
    pub posed: bool,
}

// research/xim Actor.kt:263-266 — a cast start runs the caster's full `ca<suffix>` model routine
// (the `ner1` aura, its sounds, its sub-routines). Only the magic-start category is routed here:
// the melee `ati0` routine carries its own sub-routines and would change every auto-attack swing.
#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_cast_routine_started(
    events: Res<crate::snapshot::EventLog>,
    tracked: Res<crate::scene::TrackedEntities>,
    q_children: Query<&Children>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    global: Option<Res<GlobalEffectDir>>,
    q_cast: Query<&CastRoutine>,
    mut sim: ResMut<crate::particle_sim::ParticleSimulator>,
    mut spell_suffix: Local<crate::ffxi_actor_render::SpellSuffixCache>,
    mut commands: Commands,
    mut last_seen: Local<u64>,
) {
    let new_count =
        (events.pushed_total.saturating_sub(*last_seen)).min(events.recent.len() as u64) as usize;
    *last_seen = events.pushed_total;
    if new_count == 0 {
        return;
    }
    for ev in events.recent.iter().rev().take(new_count).rev() {
        let ffxi_viewer_wire::ViewerEvent::ActionStarted {
            actor_id,
            action_id,
            action_kind,
            target_id,
            ..
        } = *ev
        else {
            continue;
        };
        if action_kind != crate::ffxi_actor_render::MAGIC_START_CATEGORY {
            continue;
        }
        let Some(&actor_entity) = tracked.by_id.get(&actor_id) else {
            continue;
        };
        // cmd_arg is the routine FourCC, not a spell id (magic_state.cpp:102); an "sp*" FourCC is
        // an interrupt on the same category (interrupts.cpp:268-284) and must tear the cast down.
        let magic = ffxi_vocab::magic::magic_start_routine(action_id);
        if magic.is_some_and(|m| m.interrupt) {
            if let Ok(cast) = q_cast.get(actor_entity) {
                sim.stop_routine(actor_entity, cast.routine);
            }
            commands.entity(actor_entity).remove::<CastRoutine>();
            continue;
        }
        let routine = match magic {
            Some(m) => ffxi_dat::datid::DatId::from_name(&m.id),
            None => {
                let suffix = spell_suffix.suffix(action_id);
                match crate::ffxi_actor_render::action_routine(action_kind, suffix) {
                    Some((routine, _looping)) => routine,
                    None => continue,
                }
            }
        };
        let Some(actor_routines) = actor_render_routines(actor_entity, &q_children, &q_render)
        else {
            continue;
        };
        let mut lookup = RoutineLookup::new().with_actor(actor_routines);
        if let Some(g) = global.as_ref() {
            lookup = lookup.with_dat(&g.schedulers);
        }
        let name = routine.0;
        let Some(active) = ActiveScheduler::effects_only(&lookup, &name) else {
            continue;
        };
        commands
            .entity(actor_entity)
            .try_insert(active)
            .try_insert(CastRoutine {
                routine: name,
                posed: false,
            })
            .try_insert(ActionTarget(
                target_id.and_then(|id| tracked.by_id.get(&id).copied()),
            ));
    }
}

// The victim reaction the attacker's routine will hand off at its 0x2B DamageCallback stage.
// Held on the attacker between the swing dispatch and that stage so the flinch, the impact SE
// and the hurt grunt land on the frame retail invokes the damage callback, not on packet
// arrival (research/xim EffectRoutineInstance.kt:956-959).
#[derive(Component, Debug, Clone, Copy)]
pub struct PendingHitReaction {
    pub routine: [u8; 4],
    // The scheduler whose DamageCallback stage is allowed to fire this reaction. Every completion
    // routine ends in a 0x2B (a spell's `mdam`), so an unqualified pending reaction would be
    // consumed by whichever routine happened to reach its callback first.
    pub armed_by: [u8; 4],
}

// The global effect dir's `dam0` chunk is the MELEE hit-reaction switch (`dada` tail-calls it;
// the ranged chain `ldad` uses `daml` instead). Its cases select on `context.hitTypeFlag`
// (research/xim EffectRoutineInstance.kt:691) and their branch order is byte-for-byte the
// ActionResolution values in vendor/server/src/map/enums/action/resolution.h.
// research/xim leaves the `damh`-vs-`damg` selector (var 0x3B) unhandled
// (EffectRoutineInstance.kt:689-701 warns and defaults to 0), which is the `damg` branch.
pub fn hit_reaction_routine(resolution: ffxi_proto::melee::ActionResolution) -> Option<[u8; 4]> {
    use ffxi_proto::melee::ActionResolution;
    Some(match resolution {
        ActionResolution::Hit => *b"damg",
        ActionResolution::Miss => *b"sway",
        ActionResolution::Guard => *b"gurd",
        ActionResolution::Parry => *b"pary",
        ActionResolution::Block => *b"gur1",
    })
}

// research/xim Actor.kt:864-903 — the swing routine is chosen by which limb struck.
// Direction-of-movement variants (atf0/atb0/atl0/atr0) are not selected here; that needs the
// attacker's locomotion state at swing time.
pub fn swing_routine(animation: ffxi_proto::melee::AttackAnimation) -> Option<[u8; 4]> {
    use ffxi_proto::melee::AttackAnimation;
    Some(match animation {
        AttackAnimation::RightAttack => *b"ati0",
        AttackAnimation::LeftAttack => *b"bti0",
        AttackAnimation::RightKick => *b"cti0",
        AttackAnimation::LeftKick => *b"dti0",
        AttackAnimation::Throw => return None,
    })
}

// vendor/server/src/map/enums/four_cc.h:30 — BasicAttack's FourCC is "atk0", the self-targeted
// voice routine research/xim Actor.kt:866 enqueues alongside the swing.
const MELEE_VOICE_ROUTINE: [u8; 4] = *b"atk0";

// A basic attack's routines live in the attacker's own battle/equipment dirs and the global effect
// dir, keyed by the swing animation rather than by a DAT file id, which is why the category is
// dispatched here rather than through `action_dat_file_id`. BATTLE2 cmd_arg does carry a FourCC —
// vendor/server/src/map/action/action.cpp:111 normalize() sets actionid = FourCC::BasicAttack —
// but it is the same constant for every swing, so it selects nothing.
#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_melee_action_started(
    events: Res<crate::snapshot::EventLog>,
    tracked: Res<crate::scene::TrackedEntities>,
    q_children: Query<&Children>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    global: Option<Res<GlobalEffectDir>>,
    mut commands: Commands,
    mut last_seen: Local<u64>,
) {
    let new_count =
        (events.pushed_total.saturating_sub(*last_seen)).min(events.recent.len() as u64) as usize;
    *last_seen = events.pushed_total;
    if new_count == 0 {
        return;
    }
    for ev in events.recent.iter().rev().take(new_count).rev() {
        let ffxi_viewer_wire::ViewerEvent::ActionStarted {
            actor_id,
            action_kind,
            target_id,
            result,
            ..
        } = *ev
        else {
            continue;
        };
        if action_kind != ffxi_proto::melee::CATEGORY_BASIC_ATTACK {
            continue;
        }
        let Some(&actor_entity) = tracked.by_id.get(&actor_id) else {
            continue;
        };
        let Some(actor_routines) = actor_render_routines(actor_entity, &q_children, &q_render)
        else {
            continue;
        };
        let mut lookup = RoutineLookup::new().with_actor(actor_routines);
        if let Some(g) = global.as_ref() {
            lookup = lookup.with_dat(&g.schedulers);
        }
        // An off-hand/kick routine is absent from some weapon-motion DATs; the main-hand swing
        // is the only routine every armed race base is known to carry.
        let result = result.and_then(|(resolution, animation)| {
            ffxi_proto::melee::MeleeResult::from_wire(resolution, animation)
        });
        let swing = result
            .and_then(|r| swing_routine(r.animation))
            .filter(|r| lookup.get(r).is_some())
            .unwrap_or(*b"ati0");
        let merged = [MELEE_VOICE_ROUTINE, swing];
        let Some(active) = ActiveScheduler::effects_only_merged(&lookup, &merged) else {
            continue;
        };
        let armed_by = active.name();
        let mut entity = commands.entity(actor_entity);
        entity.try_insert(active).try_insert(ActionTarget(
            target_id.and_then(|id| tracked.by_id.get(&id).copied()),
        ));
        match result.and_then(|r| hit_reaction_routine(r.resolution)) {
            Some(routine) => {
                entity.try_insert(PendingHitReaction { routine, armed_by });
            }
            None => {
                entity.remove::<PendingHitReaction>();
            }
        }
    }
}

// research/xim EffectRoutineInstance.kt:956-959 — the 0x2B stage is where retail hands control
// to the damage callback. That is the frame the victim's reaction routine starts, so the flinch
// and impact SE line up with the swing instead of with packet arrival.
#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_damage_callback_stages(
    mut events: MessageReader<SchedulerStageEvent>,
    q_pending: Query<(&PendingHitReaction, &ActionTarget)>,
    q_children: Query<&Children>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    q_active: Query<&ActiveScheduler>,
    global: Option<Res<GlobalEffectDir>>,
    mut commands: Commands,
) {
    for ev in events.read() {
        if ev.stage.stage.kind != StageKind::DamageCallback {
            continue;
        }
        let Ok((pending, target)) = q_pending.get(ev.actor) else {
            continue;
        };
        if pending.armed_by != ev.scheduler {
            continue;
        }
        commands.entity(ev.actor).remove::<PendingHitReaction>();
        let Some(victim) = target.0 else { continue };
        run_routine_on(
            victim,
            &pending.routine,
            Some(ev.actor),
            &q_children,
            &q_render,
            &q_active,
            global.as_deref(),
            &mut commands,
        );
    }
}

// research/xim EffectRoutineParser.kt:136-140 + EffectRoutineInstance.kt:387-394 — a 0x09 link
// runs its child ON the primary target, under a context flipped by `cloneWithOverrideTarget`:
// the parent becomes the child's target. Resource lookup follows that flip
// (EffectRoutineInstance.kt:418-431,592-604 searchAssociatedDir), which is the only reason the
// melee hit chain resolves at all — the victim's `damg` links `chit` back onto the ATTACKER, so
// `ef h` is found in the attacker's equipped-weapon DAT and its `hit1` sparks, being
// AttachType::TargetActor, land on the victim again.
// Returns (entity the child runs on, the child's flipped target). A routine with no target of
// its own keeps its child where it is and never flips a context onto itself.
pub fn target_link_context(actor: Entity, target: Option<Entity>) -> (Entity, Option<Entity>) {
    match target {
        Some(t) if t != actor => (t, Some(actor)),
        _ => (actor, None),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_target_routine_stages(
    mut events: MessageReader<SchedulerStageEvent>,
    q_target: Query<&ActionTarget>,
    q_children: Query<&Children>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    q_active: Query<&ActiveScheduler>,
    global: Option<Res<GlobalEffectDir>>,
    mut commands: Commands,
) {
    for ev in events.read() {
        if ev.stage.stage.kind != StageKind::SubRoutineOnTarget {
            continue;
        }
        let (host, flipped_target) =
            target_link_context(ev.actor, q_target.get(ev.actor).ok().and_then(|t| t.0));
        run_routine_on(
            host,
            &ev.stage.stage.id,
            flipped_target,
            &q_children,
            &q_render,
            &q_active,
            global.as_deref(),
            &mut commands,
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_routine_on(
    entity: Entity,
    routine: &[u8; 4],
    flipped_target: Option<Entity>,
    q_children: &Query<&Children>,
    q_render: &Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    q_active: &Query<&ActiveScheduler>,
    global: Option<&GlobalEffectDir>,
    commands: &mut Commands,
) {
    // ActiveScheduler is single-slot, so writing one onto a victim who is mid-routine would drop
    // the rest of that routine — including the 0x2D StopParticle stages that end its emitters,
    // leaking generators. A victim already running effects keeps them; retail's own reaction is
    // the lower-priority one here.
    if q_active.get(entity).is_ok_and(|a| !a.finished()) {
        return;
    }
    let Some(routines) = actor_render_routines(entity, q_children, q_render) else {
        return;
    };
    let mut lookup = RoutineLookup::new().with_actor(routines);
    if let Some(g) = global {
        lookup = lookup.with_dat(&g.schedulers);
    }
    let Some(active) = ActiveScheduler::from_routine(&lookup, routine) else {
        return;
    };
    let mut entity = commands.entity(entity);
    entity.try_insert(active);
    if let Some(target) = flipped_target {
        entity.try_insert(ActionTarget(Some(target)));
    }
}

// Belt-and-braces stop for the case retail's 0x2D StopParticle stages cannot reach: an
// interrupted cast never runs the spell DAT's `main`. Only a cast that was OBSERVED posing and
// then stopped counts as ended — see CastRoutine::posed.
#[cfg(not(target_arch = "wasm32"))]
pub fn stop_cast_effects_when_cast_ends(
    mut q_cast: Query<(Entity, &mut CastRoutine, &Children)>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    mut sim: ResMut<crate::particle_sim::ParticleSimulator>,
    mut commands: Commands,
) {
    for (entity, mut cast, children) in &mut q_cast {
        let Some(actor) = children.iter().find_map(|c| q_render.get(c).ok()) else {
            continue;
        };
        if actor.cast_posing() {
            cast.posed = true;
            continue;
        }
        if !cast.posed {
            continue;
        }
        sim.stop_routine(entity, cast.routine);
        commands.entity(entity).remove::<CastRoutine>();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_stop_particle_stages(
    mut events: MessageReader<SchedulerStageEvent>,
    mut sim: ResMut<crate::particle_sim::ParticleSimulator>,
) {
    for ev in events.read() {
        if ev.stage.stage.kind != StageKind::StopParticle {
            continue;
        }
        sim.stop_generator(ev.actor, ev.stage.stage.id);
    }
}

pub const EMOTE_ROUTINES_PER_FILE: u16 = 8;

const SALUTE_NATION_MAX: u16 = 2;

fn em_routine(sub: u16) -> [u8; 4] {
    [
        b'e',
        b'm',
        b'0',
        b'0' + (sub % EMOTE_ROUTINES_PER_FILE) as u8,
    ]
}

/// Emote id → (emote-file offset from the FFXiMain.dll race base, `em0N`
/// routine). Derived empirically from the retail HumeM emote DATs (dump:
/// examples/zz-emote-probe.rs; each routine's Motion clip mnemonic names the
/// emote — bow/poi/sl1-3/kne/lau/wee, den/nod/wav/wel/gla/che/clp, …) and
/// pinned to XIM's only known points (Actor.kt:1080-1082 HELM: Logging=(5,0),
/// Mining=(6,0), Harvesting=(7,0) — confirmed by the files' Japanese tool
/// particles: ono0=axe, turu=pickaxe, kama=sickle). Notable non-uniformities
/// the old id/8 hypothesis missed: Point/Bow are swapped in file 0, Salute
/// occupies em02..em04 (one per nation, 0x05A Param = nation), and ids ≥ 6
/// sit at (id+2)/8 only through id 37. Returns None when no body routine
/// exists in the era DATs (face-only emotes, id gaps, unmapped job emotes).
pub fn emote_routine(emote_id: u16, param: u16) -> Option<(u32, [u8; 4])> {
    match emote_id {
        0 => Some((0, *b"em01")),
        1 => Some((0, *b"em00")),
        2 => Some((0, em_routine(2 + param.min(SALUTE_NATION_MAX)))),
        3 => Some((0, *b"em05")),
        4 => Some((0, *b"em06")),
        5 => Some((0, *b"em07")),
        6..=37 => {
            let shifted = emote_id + 2;
            Some((
                (shifted / EMOTE_ROUTINES_PER_FILE) as u32,
                em_routine(shifted % EMOTE_ROUTINES_PER_FILE),
            ))
        }
        // HELM (server-initiated): axe / pickaxe / sickle files.
        40 => Some((5, *b"em00")),
        41 => Some((6, *b"em00")),
        42 => Some((7, *b"em00")),
        // Hurray variants (xe0..xe6) are weapon-keyed; selection unmapped — em00 default.
        43 => Some((8, *b"em00")),
        44 => Some((11, *b"em00")),
        // Dance1-4 (dc0..dc3).
        65..=68 => Some((12, em_routine(emote_id - 65))),
        // Bell-ring motion variants (rx/rs); note→variant selection unmapped.
        73 => Some((10, *b"em00")),
        // Aim variants (ye0..ye6) are ranged-weapon-keyed; selection unmapped — em00 default.
        96 => Some((9, *b"em00")),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_entity_emoted(
    events: Res<crate::snapshot::EventLog>,
    tracked: Res<crate::scene::TrackedEntities>,
    q_look: Query<&crate::components::LookComp>,
    q_children: Query<&Children>,
    mut q_actors: Query<&mut crate::ffxi_actor_render::FfxiRenderActor>,
    mut dll_cache: Local<MainDllCache>,
    mut cache: ResMut<ActionDatCache>,
    mut commands: Commands,
    mut last_seen: Local<u64>,
) {
    use ffxi_proto::map::emote;

    let new_count =
        (events.pushed_total.saturating_sub(*last_seen)).min(events.recent.len() as u64) as usize;
    *last_seen = events.pushed_total;
    if new_count == 0 {
        return;
    }
    for ev in events.recent.iter().rev().take(new_count).rev() {
        let ffxi_viewer_wire::ViewerEvent::EntityEmoted {
            actor_id,
            target_id,
            emote_id,
            param,
            mode,
        } = *ev
        else {
            continue;
        };
        if mode == emote::mode::TEXT {
            continue;
        }
        // Job emotes (MesNum 74..=95) live in a separate per-job file range
        // not yet mapped (bead kuluu-d4u retail_unknowns) — text only for now.
        if (emote::JOB_MESNUM_BASE..=emote::JOB_MESNUM_MAX).contains(&emote_id) {
            continue;
        }
        let Some(&actor_entity) = tracked.by_id.get(&actor_id) else {
            continue;
        };
        let Some((file_offset, routine)) = emote_routine(emote_id, param) else {
            continue;
        };
        let race = q_look.get(actor_entity).ok().and_then(|l| look_race(&l.0));

        if let Some(race) = race {
            if !dll_cache.loaded {
                dll_cache.loaded = true;
                if let Ok(root) = ffxi_dat::DatRoot::from_env_or_default() {
                    dll_cache.dll = ffxi_dat::main_dll::MainDll::load(root.root()).ok();
                }
            }
            let base = dll_cache
                .dll
                .as_ref()
                .and_then(|d| d.base_emote_index(race));
            if let Some(base) = base {
                let file_id = base as u32 + file_offset;
                match cache.lru.get_and_promote(file_id) {
                    Some(parsed) => {
                        if apply_emote_dispatch(
                            &parsed,
                            &routine,
                            actor_entity,
                            tracked.by_id.get(&target_id).copied(),
                            &mut commands,
                        ) {
                            continue;
                        }
                    }
                    // The DAT-vs-local-clip decision needs the parse, so it is deferred with it.
                    None => {
                        cache.defer(
                            file_id,
                            PendingActionDispatch::Emote {
                                actor_id,
                                target_id,
                                routine,
                            },
                        );
                        continue;
                    }
                }
            }
        }

        play_local_emote_clip(&routine, actor_entity, &q_children, &mut q_actors);
    }
}

// NPC casters (lua sendEmote) and PCs whose emote DAT lacks the routine:
// play the actor's own em0N clip when it has one; silent no-op
// otherwise (XIM findLocalAnimationRoutine, Actor.kt:695-697).
#[cfg(not(target_arch = "wasm32"))]
fn play_local_emote_clip(
    routine: &[u8; 4],
    actor_entity: Entity,
    q_children: &Query<&Children>,
    q_actors: &mut Query<&mut crate::ffxi_actor_render::FfxiRenderActor>,
) {
    let clip = ffxi_dat::datid::DatId::from_name(routine);
    let Ok(children) = q_children.get(actor_entity) else {
        return;
    };
    for &child in children {
        if let Ok(mut actor) = q_actors.get_mut(child) {
            actor.begin_completion_motion(
                clip,
                crate::ffxi_actor_render::CompletionMotion {
                    local_clips: &[],
                    duration_frames: 0.0,
                    max_loops: 1,
                    transition_in: 0,
                    transition_out: 0,
                },
            );
        }
    }
}

pub struct SchedulerRuntimePlugin;

impl Plugin for SchedulerRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SchedulerStageEvent>();

        #[cfg(target_arch = "wasm32")]
        app.add_systems(Update, tick_active_schedulers);

        #[cfg(not(target_arch = "wasm32"))]
        {
            app.init_resource::<crate::particle_sim::ParticleSimulator>();
            app.init_resource::<ActionDatCache>();
            app.add_systems(Startup, load_global_effect_dir);
            app.add_systems(
                Update,
                (
                    poll_global_effect_dir,
                    dispatch_action_started,
                    dispatch_cast_routine_started,
                    dispatch_melee_action_started,
                    dispatch_entity_emoted,
                    poll_action_dat_tasks,
                    // Chained between the routine inserters and the stage consumers so a
                    // routine's frame-0 stages fire on the frame it is inserted, and every
                    // stage is consumed the same frame it is written.
                    tick_active_schedulers,
                    crate::particle_sim::spawn_actor_auto_run_particles,
                    crate::particle_sim::spawn_particle_generators,
                    dispatch_stop_particle_stages,
                    crate::particle_sim::stop_generators_for_despawned_owners,
                    crate::particle_sim::tick_particle_simulator,
                    crate::particle_sim::sync_particle_meshes,
                    dispatch_sound_stages,
                    dispatch_motion_stages,
                    dispatch_damage_callback_stages,
                    dispatch_target_routine_stages,
                )
                    .chain()
                    // The overlay and this chain both drain EventLog with private cursors; the
                    // overlay's "no routine for this action" branch clears the looping action, so
                    // it must run before a completion routine's Motion stage begins here.
                    .after(crate::ffxi_actor_render::dispatch_action_overlay),
            );
            app.add_systems(
                Update,
                stop_cast_effects_when_cast_ends
                    .after(crate::ffxi_actor_render::tick_live_ffxi_actors),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffxi_dat::scheduler::{SchedulerStage, StageKind};

    #[test]
    fn particle_origin_entity_routes_by_attach_type() {
        use ffxi_dat::particle_gen::AttachType;
        let caster = Entity::from_raw_u32(1).unwrap();
        let target = Entity::from_raw_u32(2).unwrap();

        for attach in [
            AttachType::None,
            AttachType::SourceActor,
            AttachType::SourceActorWeapon,
            AttachType::SourceActorTargetFacing,
            AttachType::SourceToTargetBasis,
            AttachType::Sun,
        ] {
            assert_eq!(
                particle_origin_entity(attach, caster, Some(target)),
                caster,
                "{attach:?}"
            );
        }

        for attach in [
            AttachType::TargetActor,
            AttachType::TargetActorSourceFacing,
            AttachType::TargetToSourceBasis,
        ] {
            assert_eq!(
                particle_origin_entity(attach, caster, Some(target)),
                target,
                "{attach:?}"
            );
            assert_eq!(
                particle_origin_entity(attach, caster, None),
                caster,
                "{attach:?} falls back to the caster when the target is untracked"
            );
        }
    }

    // A 0x0B SoundOnTarget is the victim's impact, a 0x53 SoundOnCaster the attacker's whoosh;
    // resolve_stage_to_se hands the flag over and the dispatcher must mix them from different
    // world positions.
    #[test]
    fn sound_origin_entity_routes_by_on_caster_flag() {
        let caster = Entity::from_raw_u32(1).unwrap();
        let target = Entity::from_raw_u32(2).unwrap();

        assert_eq!(sound_origin_entity(true, caster, Some(target)), caster);
        assert_eq!(sound_origin_entity(true, caster, None), caster);
        assert_eq!(sound_origin_entity(false, caster, Some(target)), target);
        assert_eq!(
            sound_origin_entity(false, caster, None),
            caster,
            "an untracked target falls back to the caster instead of silencing the SE"
        );
    }

    // The flag the dispatcher routes on comes straight from the stage kind, so a parser change
    // that stopped distinguishing the two opcodes would silently collapse both to the caster.
    #[test]
    fn resolve_stage_to_se_reports_on_caster_from_the_stage_kind() {
        let seps = HashMap::from([(*b"se01", Sep::parse(*b"se01", &[0u8; 12]).unwrap())]);
        let generators = HashMap::new();

        assert_eq!(
            ffxi_dat::action::resolve_stage_to_se(
                b"se01",
                StageKind::SoundOnCaster,
                &generators,
                &seps
            ),
            Some((0, true))
        );
        assert_eq!(
            ffxi_dat::action::resolve_stage_to_se(
                b"se01",
                StageKind::SoundOnTarget,
                &generators,
                &seps
            ),
            Some((0, false))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[derive(Resource, Default)]
    struct CapturedSfx(Vec<crate::audio::SfxEvent>);

    #[cfg(not(target_arch = "wasm32"))]
    fn capture_sfx(
        mut reader: MessageReader<crate::audio::SfxEvent>,
        mut out: ResMut<CapturedSfx>,
    ) {
        out.0.extend(reader.read().copied());
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn sep_assets(stage_id: [u8; 4], se_id: u32) -> ActionAssets {
        let mut body = [0u8; 12];
        body[8..12].copy_from_slice(&se_id.to_le_bytes());
        ActionAssets {
            seps: HashMap::from([(stage_id, Sep::parse(stage_id, &body).unwrap())]),
            ..Default::default()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn run_sound_stage(
        kind: StageKind,
        stage_id: [u8; 4],
        assets: ActionAssets,
        caster_pos: Option<Vec3>,
        target: Option<Vec3>,
    ) -> Vec<crate::audio::SfxEvent> {
        let mut app = App::new();
        app.add_message::<SchedulerStageEvent>()
            .add_message::<crate::audio::SfxEvent>()
            .init_resource::<CapturedSfx>()
            .add_systems(Update, (dispatch_sound_stages, capture_sfx).chain());

        let target_entity =
            target.map(|p| app.world_mut().spawn(Transform::from_translation(p)).id());
        let mut caster = app.world_mut().spawn((assets, ActionTarget(target_entity)));
        if let Some(p) = caster_pos {
            caster.insert(Transform::from_translation(p));
        }
        let caster = caster.id();

        app.world_mut().write_message(SchedulerStageEvent {
            actor: caster,
            stage: stage(0, kind, 0, stage_id),
            scheduler: *b"test",
        });
        app.update();
        std::mem::take(&mut app.world_mut().resource_mut::<CapturedSfx>().0)
    }

    // The whole point of the spatial SE path: a 0x0B impact has to mix from where the victim is
    // standing, not from the attacker, and neither may fall back to a 2D cue.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dispatch_sound_stages_emits_each_stage_kind_from_its_own_actor() {
        const STAGE_ID: [u8; 4] = *b"se01";
        const SE_ID: u32 = 4242;
        let caster_pos = Vec3::new(10.0, 2.0, -40.0);
        let target_pos = Vec3::new(-25.0, 6.0, 120.0);

        for (kind, expected) in [
            (StageKind::SoundOnTarget, target_pos),
            (StageKind::SoundOnCaster, caster_pos),
        ] {
            let got = run_sound_stage(
                kind,
                STAGE_ID,
                sep_assets(STAGE_ID, SE_ID),
                Some(caster_pos),
                Some(target_pos),
            );
            assert_eq!(got.len(), 1, "{kind:?} produced {got:?}");
            assert_eq!(got[0].se_id, SE_ID, "{kind:?}");
            assert_eq!(
                got[0].emitter,
                Some(expected),
                "{kind:?} must mix from {expected:?}"
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dispatch_sound_stages_falls_back_to_the_caster_and_then_to_a_dry_cue() {
        const STAGE_ID: [u8; 4] = *b"se01";
        const SE_ID: u32 = 4242;
        let caster_pos = Vec3::new(10.0, 2.0, -40.0);

        let untracked_target = run_sound_stage(
            StageKind::SoundOnTarget,
            STAGE_ID,
            sep_assets(STAGE_ID, SE_ID),
            Some(caster_pos),
            None,
        );
        assert_eq!(
            untracked_target.first().map(|e| e.emitter),
            Some(Some(caster_pos)),
            "an untracked target falls back to the caster, not to silence"
        );

        let unpositioned = run_sound_stage(
            StageKind::SoundOnCaster,
            STAGE_ID,
            sep_assets(STAGE_ID, SE_ID),
            None,
            None,
        );
        assert_eq!(
            unpositioned.first().map(|e| e.emitter),
            Some(None),
            "an actor with no transform yet mixes dry rather than from the world origin"
        );
    }

    fn stage(frame: u32, kind: StageKind, raw_type: u8, id: [u8; 4]) -> TimedStage {
        TimedStage {
            frame,
            stage: SchedulerStage {
                kind,
                raw_type,
                delay_frames: 0,
                duration_frames: 0,
                id,
                max_loops: 0,
                transition_in: 0,
                transition_out: 0,
                random_group: None,
                local_dir: ffxi_dat::scheduler::NO_LOCAL_DIR,
                model_transform: None,
                screen_color: None,
            },
        }
    }

    fn make_scheduler(name: [u8; 4], stages: Vec<TimedStage>) -> Scheduler {
        Scheduler { name, stages }
    }

    // Every completion routine ends in a 0x2B DamageCallback (a spell's `mdam`), so a melee
    // reaction armed by the swing must not be consumed by an unrelated routine reaching its
    // callback first. The DamageCallback dispatcher gates on this pairing.
    #[test]
    fn pending_hit_reaction_is_bound_to_the_scheduler_that_armed_it() {
        let pending = PendingHitReaction {
            routine: *b"damg",
            armed_by: *b"atk0",
        };
        assert_eq!(pending.armed_by, *b"atk0");
        assert_ne!(
            pending.armed_by, *b"mdam",
            "a spell's mdam must not match a melee-armed reaction"
        );
    }

    // effects_only_merged names the merged timeline after the first routine that resolved, which
    // is what a stage event reports as its scheduler — the value the reaction is armed with.
    #[test]
    fn merged_scheduler_reports_its_first_resolved_routine_as_its_name() {
        let voice = make_scheduler(*b"atk0", vec![stage(0, StageKind::Motion, 0x01, *b"at0?")]);
        let swing = make_scheduler(*b"ati0", vec![stage(2, StageKind::Motion, 0x01, *b"ati?")]);
        let dat = [voice, swing];
        let lookup = RoutineLookup::new().with_dat(&dat);

        let merged = ActiveScheduler::effects_only_merged(&lookup, &[*b"atk0", *b"ati0"]).unwrap();
        assert_eq!(merged.name(), *b"atk0");

        let only_swing =
            ActiveScheduler::effects_only_merged(&lookup, &[*b"zzzz", *b"ati0"]).unwrap();
        assert_eq!(
            only_swing.name(),
            *b"ati0",
            "an absent voice routine leaves the swing as the merged name"
        );
    }

    #[test]
    fn current_frame_advances_by_fps() {
        let sched = make_scheduler(*b"main", vec![]);
        let mut a = ActiveScheduler::from_scheduler(&sched);
        a.elapsed = 0.5;
        assert_eq!(a.current_frame(), 30);
        a.elapsed = 1.0;
        assert_eq!(a.current_frame(), 60);
    }

    // research/xim util/Fps.kt:9 `internalFps = 60.0` is the clock effect routines and particle
    // generators are authored against; poc/ActorManager.kt:59-62 halves it — and only it — for
    // skeletal animation. Neither constant may be "fixed" without the other.
    #[test]
    fn routine_clock_is_double_the_skeleton_clock() {
        assert_eq!(ROUTINE_FPS, 60.0);
        assert_eq!(crate::ffxi_actor_render::FRAME_RATE, 30.0);
        assert_eq!(
            ROUTINE_FPS,
            SKELETON_FRAME_DIVISOR * crate::ffxi_actor_render::FRAME_RATE
        );
    }

    // A stage authored 60 frames after its predecessor lands one second later, not two.
    #[test]
    fn stage_delay_60_fires_after_one_second() {
        const DELAY_FRAMES: u32 = 60;
        let sched = make_scheduler(
            *b"main",
            vec![
                stage(0, StageKind::SoundOnCaster, 0x53, *b"snd0"),
                stage(DELAY_FRAMES, StageKind::SoundOnCaster, 0x53, *b"snd1"),
            ],
        );
        let mut a = ActiveScheduler::from_scheduler(&sched);

        a.elapsed = 0.9;
        assert_eq!(a.current_frame(), 54);
        assert!(a.current_frame() < DELAY_FRAMES);

        a.elapsed = 1.05;
        assert!(a.current_frame() >= DELAY_FRAMES);
    }

    // Retail-byte fixture (skips without an install): Cure's effect DAT (file 2801 = 0xAF1) runs
    // its target routine `tgt0` out to frame 239 — 3.98 s at the authored 60 fps. That frame is
    // the routine's own `totalDelay` header field (research/xim EffectRoutineParser.kt:46), the
    // DAT's independent statement of its length, which the summed stage delays must reproduce.
    #[test]
    fn real_dat_cure_target_routine_completes_in_retail_wall_time() {
        const CURE_FILE: u32 = 2801;
        const TGT0_LAST_FRAME: u32 = 239;
        const TGT0_SECS: f32 = 3.983;

        let Some(root) = ffxi_dat::archive::open_test_install() else {
            return;
        };
        let Ok(loc) = root.resolve(CURE_FILE) else {
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
            return;
        };
        let (schedulers, _) = parse_action_bytes(&bytes);
        let tgt0 = schedulers
            .iter()
            .find(|s| &s.name == b"tgt0")
            .expect("cure DAT has a tgt0 routine");
        let last = tgt0.stages.last().expect("tgt0 has stages").frame;
        assert_eq!(last, TGT0_LAST_FRAME);
        let secs = last as f32 / ROUTINE_FPS;
        assert!(
            (secs - TGT0_SECS).abs() < 0.1,
            "cure tgt0 runs {secs}s, retail authors {TGT0_SECS}s"
        );
    }

    #[test]
    fn from_scheduler_sorts_by_frame() {
        let sched = make_scheduler(
            *b"main",
            vec![
                stage(60, StageKind::Motion, 0x05, *b"mot0"),
                stage(10, StageKind::SoundOnCaster, 0x53, *b"snd0"),
                stage(30, StageKind::Particle, 0x39, *b"prt0"),
            ],
        );
        let a = ActiveScheduler::from_scheduler(&sched);
        assert_eq!(
            a.stages.iter().map(|t| t.frame).collect::<Vec<_>>(),
            vec![10, 30, 60]
        );
    }

    #[test]
    fn finished_only_after_all_stages_emitted() {
        let sched = make_scheduler(
            *b"main",
            vec![stage(5, StageKind::SoundOnCaster, 0x53, *b"snd0")],
        );
        let mut a = ActiveScheduler::from_scheduler(&sched);
        assert!(!a.finished());
        a.cursor = 1;
        assert!(a.finished());
    }

    #[test]
    fn empty_scheduler_is_immediately_finished() {
        let sched = make_scheduler(*b"main", vec![]);
        let a = ActiveScheduler::from_scheduler(&sched);
        assert!(a.finished());
        assert_eq!(a.last_frame(), 0);
    }

    /// End-to-end against the installed retail DATs (skips without them):
    /// /bow on a HumeM resolves to a routine whose Motion fires at frame 0
    /// with the bow? clip, and the file's assets carry the matching clips —
    /// the two defects that made emotes play the wrong clip 5s late.
    #[test]
    fn real_dat_bow_routine_fires_bow_clip_at_frame_zero() {
        let Some(root) = ffxi_dat::archive::open_test_install() else {
            return;
        };
        let Ok(dll) = ffxi_dat::main_dll::MainDll::load(root.root()) else {
            return;
        };
        let base = dll.base_emote_index(1).expect("HumeM emote base") as u32;
        let (offset, routine) = emote_routine(1, 0).expect("bow is mapped");
        let loc = root.resolve(base + offset).expect("emote file resolves");
        let bytes = std::fs::read(loc.path_under(&root)).expect("emote DAT readable");
        let (schedulers, assets) = parse_action_bytes(&bytes);
        let active = ActiveScheduler::from_main(&schedulers, &routine).expect("em00 exists");
        let motion = active
            .stages
            .iter()
            .find(|t| t.stage.kind == StageKind::Motion)
            .expect("bow routine has a Motion stage");
        assert_eq!(motion.frame, 0, "bow motion fires immediately");
        assert_eq!(&motion.stage.id, b"bow?");
        let clip_id = ffxi_dat::datid::DatId::from_name(&motion.stage.id);
        assert!(
            assets
                .animations
                .iter()
                .any(|a| a.id.parameterized_match(&clip_id)),
            "emote file carries bow clips matching the parameterized id"
        );
    }

    /// Pins the empirically-derived emote table against the DAT dump
    /// (examples/zz-emote-probe.rs clip mnemonics) and the XIM HELM points
    /// (Actor.kt:1080-1082). Point/Bow swap and Salute nation variants are
    /// the file-0 irregularities the old id/8 hypothesis got wrong.
    #[test]
    fn emote_table_matches_dat_clip_mnemonics() {
        assert_eq!(emote_routine(1, 0), Some((0, *b"em00")), "bow → bow? clip");
        assert_eq!(
            emote_routine(0, 0),
            Some((0, *b"em01")),
            "point → poi? clip"
        );
        assert_eq!(
            emote_routine(2, 0),
            Some((0, *b"em02")),
            "salute san d'oria → sl1?"
        );
        assert_eq!(
            emote_routine(2, 2),
            Some((0, *b"em04")),
            "salute windurst → sl3?"
        );
        assert_eq!(
            emote_routine(2, 9),
            Some((0, *b"em04")),
            "salute clamps unknown nations"
        );
        assert_eq!(emote_routine(3, 0), Some((0, *b"em05")), "kneel → kne?");
        assert_eq!(emote_routine(5, 0), Some((0, *b"em07")), "cry → wee?");
        assert_eq!(emote_routine(6, 0), Some((1, *b"em00")), "no → den?");
        assert_eq!(emote_routine(8, 0), Some((1, *b"em02")), "wave → wav?");
        assert_eq!(
            emote_routine(9, 0),
            Some((1, *b"em03")),
            "goodbye → wav? (second)"
        );
        assert_eq!(emote_routine(13, 0), Some((1, *b"em07")), "clap → clp?");
        assert_eq!(emote_routine(32, 0), Some((4, *b"em02")), "think → thk?");
        assert_eq!(emote_routine(36, 0), Some((4, *b"em06")), "psych → gut?");
        assert_eq!(emote_routine(37, 0), Some((4, *b"em07")));
        assert_eq!(
            emote_routine(40, 0),
            Some((5, *b"em00")),
            "logging → ono0 axe (XIM 5,0)"
        );
        assert_eq!(
            emote_routine(41, 0),
            Some((6, *b"em00")),
            "excavation → turu pickaxe (XIM 6,0)"
        );
        assert_eq!(
            emote_routine(42, 0),
            Some((7, *b"em00")),
            "harvesting → kama sickle (XIM 7,0)"
        );
        assert_eq!(emote_routine(44, 0), Some((11, *b"em00")), "toss → tos?");
        assert_eq!(emote_routine(65, 0), Some((12, *b"em00")), "dance1 → dc0?");
        assert_eq!(emote_routine(68, 0), Some((12, *b"em03")), "dance4 → dc3?");
        assert_eq!(
            emote_routine(38, 0),
            None,
            "shocked has no body routine in the era DATs"
        );
        assert_eq!(emote_routine(39, 0), None, "id gap");
        assert_eq!(emote_routine(45, 0), None, "id gap");
    }

    #[test]
    fn parse_action_bytes_handles_empty_input() {
        let (scheds, assets) = parse_action_bytes(&[]);
        assert!(scheds.is_empty());
        assert!(assets.generators.is_empty());
        assert!(assets.seps.is_empty());
        #[cfg(not(target_arch = "wasm32"))]
        assert!(assets.d3ms.is_empty());
    }

    // End-to-end against the installed retail DATs (skips without them): Poison's completion
    // effect (file 3020, 'veno') carries a 0x0E SpriteSheet particle cloud backed by a 0x21
    // 'fir' sheet. The fir0/fir1/fir2 generators must parse as SpriteSheet defs whose mesh_id
    // resolves to a retained sprite sheet — the regression that dropped every 0x0E generator so
    // only the neutral pk00/pk01 smoke survived. Cure (file 2801) must be unaffected: its
    // static-mesh (0x0B) particle defs still parse.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn real_dat_poison_renders_sprite_sheet_particles_cure_unaffected() {
        use ffxi_dat::particle_gen::ParticleMeshKind;

        const POISON_FILE: u32 = 3020;
        const CURE_FILE: u32 = 2801;

        let Some(root) = ffxi_dat::archive::open_test_install() else {
            return;
        };
        let Ok(loc) = root.resolve(POISON_FILE) else {
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
            return;
        };
        let (_scheds, assets) = parse_action_bytes(&bytes);

        assert!(
            !assets.sprite_sheets.is_empty(),
            "poison DAT carries at least one 0x21 sprite sheet"
        );
        for name in [b"fir0", b"fir1", b"fir2"] {
            let def = assets
                .particle_defs
                .get(name)
                .unwrap_or_else(|| panic!("{} generator present", String::from_utf8_lossy(name)));
            assert_eq!(
                def.mesh_kind,
                ParticleMeshKind::SpriteSheet,
                "{} is a SpriteSheet particle",
                String::from_utf8_lossy(name)
            );
            assert!(
                assets.sprite_sheets.contains_key(&def.mesh_id),
                "{}'s mesh {} resolves to a retained sprite sheet",
                String::from_utf8_lossy(name),
                String::from_utf8_lossy(&def.mesh_id),
            );
        }

        let Ok(cure_loc) = root.resolve(CURE_FILE) else {
            return;
        };
        let Ok(cure_bytes) = std::fs::read(cure_loc.path_under(&root)) else {
            return;
        };
        let (_s, cure_assets) = parse_action_bytes(&cure_bytes);
        assert!(
            cure_assets
                .particle_defs
                .values()
                .any(|d| d.mesh_kind == ParticleMeshKind::StaticMesh),
            "cure still parses its static-mesh particle generators"
        );
    }

    // Retail-DAT coupling guard (skips without an install): Poison's 0x21 'fir' sheet names its
    // backing Img with the qualified pair ("venom1", "fir"). Looking the Img up by the sheet's
    // namespace token alone misses, which is what rendered the venom cloud as an untextured
    // quad (kuluu-7jpq). research/xim DatResource.kt:483-493 matches qualified, then local.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn real_dat_poison_sheet_name_indexes_its_backing_img() {
        const POISON_FILE: u32 = 3020;

        let Some(root) = ffxi_dat::archive::open_test_install() else {
            return;
        };
        let Ok(loc) = root.resolve(POISON_FILE) else {
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
            return;
        };
        let (_scheds, assets) = parse_action_bytes(&bytes);

        let sheet = assets
            .sprite_sheets
            .values()
            .find(|s| s.id == "fir")
            .expect("poison DAT carries the 'fir' sprite sheet");
        assert_eq!(sheet.category, "venom1");
        assert!(
            assets
                .images_by_qualified_name
                .contains_key(&(sheet.category.clone(), sheet.id.clone())),
            "the sheet's qualified name indexes an Img chunk"
        );
        assert!(
            assets.images_by_name.contains_key(&sheet.id),
            "the local-name fallback tier also indexes it"
        );
        assert!(
            !assets.images_by_name.contains_key(&sheet.category),
            "the namespace token is NOT a local-name key — the original miss"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_dat(file_id: u32) -> Option<Vec<u8>> {
        let root = ffxi_dat::archive::open_test_install()?;
        let loc = root.resolve(file_id).ok()?;
        std::fs::read(loc.path_under(&root)).ok()
    }

    // Retail-DAT guard (skips without an install): the cast aura `ner1` and its `stbk` shutdown
    // live in ROM/0/0.DAT, XIM's GlobalDirectory (research/xim poc/MainTool.kt:250) — not in the
    // caster's own DAT — so a DAT-root or resolver change cannot silently un-resolve them.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn real_dat_global_dir_holds_cast_aura_and_its_stop() {
        const AURA_GENERATORS: [&[u8; 4]; 4] = [b"gn10", b"gn11", b"gn12", b"gn13"];

        let Some(bytes) = read_dat(GLOBAL_EFFECT_DIR_FILE_ID) else {
            return;
        };
        let (schedulers, assets) = parse_action_bytes(&bytes);

        let ner1 = schedulers
            .iter()
            .find(|s| &s.name == b"ner1")
            .expect("global effect dir holds the cast aura routine");
        let spawned: Vec<[u8; 4]> = ner1
            .stages
            .iter()
            .filter(|t| t.stage.kind == StageKind::Particle)
            .map(|t| t.stage.id)
            .collect();
        for gen_id in AURA_GENERATORS {
            assert!(
                spawned.contains(gen_id),
                "ner1 spawns {}",
                String::from_utf8_lossy(gen_id)
            );
            assert!(
                assets.particle_defs.contains_key(gen_id),
                "the global dir also carries {}'s generator def",
                String::from_utf8_lossy(gen_id)
            );
        }

        let stbk = schedulers
            .iter()
            .find(|s| &s.name == b"stbk")
            .expect("global effect dir holds the cast-aura stop routine");
        let stopped: Vec<[u8; 4]> = stbk
            .stages
            .iter()
            .filter(|t| t.stage.kind == StageKind::StopParticle)
            .map(|t| t.stage.id)
            .collect();
        for gen_id in AURA_GENERATORS {
            assert!(
                stopped.contains(gen_id),
                "stbk stops {}",
                String::from_utf8_lossy(gen_id)
            );
        }
    }

    // Retail-DAT guard (skips without an install): HumeM's black-magic cast routine `cabk` is a
    // Motion stage plus two SubRoutine calls that only resolve in the global dir, so the flatten
    // has to span both tiers. `effects_only` drops the Motion because the caster's looping cast
    // pose is owned by ffxi_actor_render::dispatch_action_overlay.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cast_routine_flattens_across_actor_and_global_dirs() {
        const HUME_M_SKELETON_FILE: u32 = 7072;
        const CAST_ROUTINE: [u8; 4] = *b"cabk";

        let (Some(actor_bytes), Some(global_bytes)) = (
            read_dat(HUME_M_SKELETON_FILE),
            read_dat(GLOBAL_EFFECT_DIR_FILE_ID),
        ) else {
            return;
        };
        let (actor_scheds, _) = parse_action_bytes(&actor_bytes);
        let (global_scheds, _) = parse_action_bytes(&global_bytes);
        let lookup = RoutineLookup::new()
            .with_dat(&actor_scheds)
            .with_dat(&global_scheds);

        let full = ActiveScheduler::from_routine(&lookup, &CAST_ROUTINE).expect("cabk exists");
        assert!(
            full.stages
                .iter()
                .any(|t| t.stage.kind == StageKind::Motion && &t.stage.id == b"mb0?"),
            "the full cast routine still carries the mb0? cast motion"
        );

        let effects = ActiveScheduler::effects_only(&lookup, &CAST_ROUTINE).expect("cabk exists");
        assert!(
            effects
                .stages
                .iter()
                .all(|t| t.stage.kind != StageKind::Motion),
            "effects_only suppresses every Motion stage"
        );
        let particles = effects
            .stages
            .iter()
            .filter(|t| t.stage.kind == StageKind::Particle)
            .count();
        assert!(
            particles >= 4,
            "the aura's generators are inlined from the global dir, got {particles}"
        );

        assert_eq!(
            ActiveScheduler::effects_only(
                &RoutineLookup::new().with_dat(&actor_scheds),
                &CAST_ROUTINE
            )
            .expect("cabk exists")
            .stages
            .iter()
            .filter(|t| t.stage.kind != StageKind::Unknown)
            .count(),
            0,
            "without the global tier the aura sub-routines resolve to nothing — the original bug"
        );
    }

    fn tagged_stage(
        frame: u32,
        kind: StageKind,
        raw_type: u8,
        id: [u8; 4],
        random_group: Option<u16>,
    ) -> TimedStage {
        let mut t = stage(frame, kind, raw_type, id);
        t.stage.random_group = random_group;
        t
    }

    // The whole point of the melee path: `ati0` (weapon-motion DAT) links `skaz` with 0x57, and
    // `skaz` resolves in the EQUIPPED WEAPON's DAT — three tiers away from the routine that
    // named it. Flattening must carry the link across and keep the frame the whoosh is authored
    // at (research/xim EffectRoutineParser.kt:371-375).
    #[test]
    fn melee_swing_flattens_to_the_weapon_swing_sound() {
        const SKAZ_FRAME: u32 = 34;
        let weapon_motion = vec![
            make_scheduler(
                *b"ati0",
                vec![
                    stage(0, StageKind::Motion, 0x05, *b"at0?"),
                    stage(SKAZ_FRAME, StageKind::SubRoutine, 0x57, *b"skaz"),
                ],
            ),
            make_scheduler(
                *b"atk0",
                vec![stage(0, StageKind::SubRoutine, 0x57, *b"vatk")],
            ),
        ];
        let weapon_item = vec![make_scheduler(
            *b"skaz",
            vec![stage(0, StageKind::SoundOnCaster, 0x0A, *b"skaz")],
        )];
        let face = vec![make_scheduler(
            *b"vatk",
            vec![tagged_stage(
                0,
                StageKind::SoundOnCaster,
                0x0A,
                *b"atk1",
                Some(0),
            )],
        )];
        let lookup = RoutineLookup::new()
            .with_dat(&weapon_motion)
            .with_dat(&weapon_item)
            .with_dat(&face);

        let active = ActiveScheduler::effects_only_merged(&lookup, &[*b"atk0", *b"ati0"])
            .expect("the swing flattens");
        let whoosh = active
            .stages
            .iter()
            .find(|t| &t.stage.id == b"skaz" && t.stage.kind == StageKind::SoundOnCaster)
            .expect("the weapon swing sound survives the 0x57 link");
        assert_eq!(whoosh.frame, SKAZ_FRAME);
        assert!(
            active
                .stages
                .iter()
                .any(|t| &t.stage.id == b"atk1" && t.stage.kind == StageKind::SoundOnCaster),
            "the merged voice routine contributes its grunt"
        );
        assert!(
            active
                .stages
                .iter()
                .all(|t| t.stage.kind != StageKind::Motion),
            "the body clip stays with dispatch_action_overlay — running both double-fires it"
        );
    }

    // research/xim EffectRoutineParser.kt:275-285 — one alternative per activation. Four
    // simultaneous `vatk` grunts is the regression this guards.
    #[test]
    fn random_block_contributes_exactly_one_member() {
        let dat = vec![make_scheduler(
            *b"vatk",
            (0..4)
                .map(|i| {
                    tagged_stage(
                        0,
                        StageKind::SoundOnCaster,
                        0x0A,
                        [b'a', b't', b'k', b'1' + i as u8],
                        Some(0),
                    )
                })
                .collect(),
        )];
        let lookup = RoutineLookup::new().with_dat(&dat);
        for _ in 0..16 {
            let active = ActiveScheduler::from_routine(&lookup, b"vatk").expect("flattens");
            assert_eq!(active.stages.len(), 1, "exactly one grunt per swing");
            assert!(active.stages[0].stage.id.starts_with(b"atk"));
        }
    }

    // `dada` tail-calls `dam0`, ten mutually exclusive additional-effect branches keyed on a
    // condition we do not evaluate (research/xim EffectRoutineParser.kt:408-427). Inlining it
    // would fire every branch at once.
    #[test]
    fn control_flow_switch_is_not_inlined() {
        let mut switch = make_scheduler(
            *b"dam0",
            vec![
                stage(0, StageKind::SubRoutineOnTarget, 0x09, *b"sb00"),
                stage(0, StageKind::SubRoutineOnTarget, 0x09, *b"sb01"),
            ],
        );
        switch
            .stages
            .push(stage(0, StageKind::Unknown, 0x6B, *b"    "));
        let dat = vec![
            make_scheduler(
                *b"dada",
                vec![
                    stage(0, StageKind::DamageCallback, 0x2B, *b"    "),
                    stage(0, StageKind::SubRoutine, 0x03, *b"dam0"),
                ],
            ),
            switch,
        ];
        let lookup = RoutineLookup::new().with_dat(&dat);
        let active = ActiveScheduler::from_routine(&lookup, b"dada").expect("flattens");
        assert!(
            active.stages.iter().all(|t| !t.stage.id.starts_with(b"sb")),
            "no branch of an unevaluated switch is taken"
        );
        assert!(
            active
                .stages
                .iter()
                .any(|t| t.stage.kind == StageKind::DamageCallback),
            "the damage callback still reaches the runtime"
        );
    }

    // A 0x09 link stays a stage rather than being inlined, so the runtime can start it on the
    // VICTIM and resolve the victim's own `sdam`/`vdam` (EffectRoutineParser.kt:136-140).
    #[test]
    fn target_link_is_not_flattened_into_the_caster_timeline() {
        let dat = vec![
            make_scheduler(
                *b"dcnt",
                vec![stage(0, StageKind::SubRoutineOnTarget, 0x09, *b"damg")],
            ),
            make_scheduler(
                *b"damg",
                vec![stage(0, StageKind::SoundOnCaster, 0x0A, *b"sdam")],
            ),
        ];
        let lookup = RoutineLookup::new().with_dat(&dat);
        let active = ActiveScheduler::from_routine(&lookup, b"dcnt").expect("flattens");
        assert_eq!(active.stages.len(), 1);
        assert_eq!(active.stages[0].stage.kind, StageKind::SubRoutineOnTarget);
        assert_eq!(&active.stages[0].stage.id, b"damg");
    }

    // vendor/server/src/map/enums/action/resolution.h ordering, pinned to the branch order the
    // retail MELEE `dam0` chunk dispatches in (ffxi_dat guard
    // real_dat_dam0_switches_hit_type_to_melee_reaction_routines). `ldam` is the RANGED chain's
    // Hit branch (`ldad` -> `daml`) and links `lhit` -> eflg/selg, which no melee weapon DAT has.
    #[test]
    fn hit_reaction_routines_follow_lsb_resolution_order() {
        use ffxi_proto::melee::ActionResolution;
        let order: Vec<Option<[u8; 4]>> = [
            ActionResolution::Hit,
            ActionResolution::Miss,
            ActionResolution::Guard,
            ActionResolution::Parry,
            ActionResolution::Block,
        ]
        .into_iter()
        .map(hit_reaction_routine)
        .collect();
        assert_eq!(
            order,
            vec![
                Some(*b"damg"),
                Some(*b"sway"),
                Some(*b"gurd"),
                Some(*b"pary"),
                Some(*b"gur1"),
            ]
        );
    }

    // research/xim EffectRoutineInstance.kt:387-394 — createChild for a 0x09 link builds
    // `ActorAssociation(target, context.cloneWithOverrideTarget(actor.id))`: the child runs on the
    // target and its own target is the parent. Without the flip the melee chain dead-ends on the
    // victim and the weapon's `ef h` sparks are never reached.
    #[test]
    fn target_link_flips_the_context_onto_the_parent() {
        let attacker = Entity::from_raw_u32(1).unwrap();
        let victim = Entity::from_raw_u32(2).unwrap();
        assert_eq!(
            target_link_context(attacker, Some(victim)),
            (victim, Some(attacker))
        );
        assert_eq!(
            target_link_context(victim, Some(attacker)),
            (attacker, Some(victim))
        );
        assert_eq!(target_link_context(victim, None), (victim, None));
        assert_eq!(
            target_link_context(victim, Some(victim)),
            (victim, None),
            "a self-targeted link must not make an actor its own target"
        );
    }

    // ROM/32/13.DAT — the HumeM weapon-motion base whose `ati0`/`atk0` the melee dispatcher runs.
    const HUME_M_WEAPON_MOTION_FILE: u32 = 9672;
    // ROM/27/82.DAT `hm_s` — the HumeM skeleton, which carries the reaction routines
    // (`damg`/`chit`/`sway`/`gurd`/`pary`).
    const HUME_M_SKELETON_FILE: u32 = 7072;
    // look_resolver::PC_MODEL_IDS[HumeM][main-hand] base — main-hand weapon model 0.
    const HUME_M_MAIN_WEAPON_FILE: u32 = 8392;

    fn routines_in_file(file_id: u32) -> Option<Vec<Scheduler>> {
        let root = ffxi_dat::archive::open_test_install()?;
        let loc = root.resolve(file_id).ok()?;
        let bytes = std::fs::read(loc.path_under(&root)).ok()?;
        Some(ffxi_dat::resource_dir::ResourceDir::from_bytes(bytes).collect_schedulers())
    }

    fn global_effect_dir() -> Option<(Vec<Scheduler>, ActionAssets)> {
        let root = ffxi_dat::archive::open_test_install()?;
        let loc = root.resolve(GLOBAL_EFFECT_DIR_FILE_ID).ok()?;
        let bytes = std::fs::read(loc.path_under(&root)).ok()?;
        Some(parse_action_bytes(&bytes))
    }

    // Retail-DAT guard (self-skips without an install) for the whole hit-spark chain. `chit`
    // lives in the victim's skeleton but is reached through the 0x09 flip back onto the ATTACKER,
    // which is why it resolves `ef h` in the equipped-weapon DAT; `ef h` links global `hit1`,
    // whose generators are AttachType::TargetActor and therefore land on the victim again
    // (research/xim ParticleGeneratorAttachment.kt:64-111). Every tier must be present for a
    // single spark to appear, so this pins all three at once.
    #[test]
    fn melee_hit_chain_flattens_to_target_attached_sparks() {
        let (Some(skeleton), Some(weapon), Some((global_scheds, global_assets))) = (
            routines_in_file(HUME_M_SKELETON_FILE),
            routines_in_file(HUME_M_MAIN_WEAPON_FILE),
            global_effect_dir(),
        ) else {
            return;
        };
        let lookup = RoutineLookup::new()
            .with_dat(&skeleton)
            .with_dat(&weapon)
            .with_dat(&global_scheds);

        let active = ActiveScheduler::from_routine(&lookup, b"chit")
            .expect("the hit-flash routine flattens across skeleton -> weapon -> global");
        let sparks: Vec<([u8; 4], [u8; 4])> = active
            .stages
            .iter()
            .filter(|t| t.stage.kind == StageKind::Particle)
            .map(|t| (t.stage.local_dir, t.stage.id))
            .collect();
        assert!(
            !sparks.is_empty(),
            "chit -> ef h -> hit1 must reach the spark generators, got {:?}",
            active.stages
        );

        // The reason the lookup has to be directory-scoped at all: ROM/0/0.DAT defines `g010`
        // several times over, and the flat by-name map keeps whichever the walk saw last.
        let g010_dirs = global_assets
            .particle_defs_by_dir
            .keys()
            .filter(|(_, name)| name == b"g010")
            .count();
        assert!(
            g010_dirs > 1,
            "expected duplicate `g010` generators across directories, found {g010_dirs}"
        );

        let attacker = Entity::from_raw_u32(1).unwrap();
        let victim = Entity::from_raw_u32(2).unwrap();
        for (local_dir, id) in &sparks {
            assert_eq!(
                local_dir, b"hit1",
                "the spark generators are authored in the `hit1` directory"
            );
            let def = global_assets
                .particle_def(*local_dir, id)
                .unwrap_or_else(|| panic!("global dir defines {}", String::from_utf8_lossy(id)));
            assert_eq!(
                particle_origin_entity(def.attach_type, attacker, Some(victim)),
                victim,
                "{} spawns on the victim, not the swinger",
                String::from_utf8_lossy(id)
            );
        }
    }

    // The victim's reaction routine must survive the same flatten: `damg` keeps its 0x09 `chit`
    // link as a stage (so the runtime can flip it) rather than inlining it onto the victim.
    #[test]
    fn real_dat_damg_keeps_the_hit_flash_as_a_target_link() {
        let (Some(skeleton), Some((global_scheds, _))) =
            (routines_in_file(HUME_M_SKELETON_FILE), global_effect_dir())
        else {
            return;
        };
        let lookup = RoutineLookup::new()
            .with_dat(&skeleton)
            .with_dat(&global_scheds);
        let active =
            ActiveScheduler::from_routine(&lookup, b"damg").expect("the skeleton has `damg`");
        assert!(
            active.stages.iter().any(|t| {
                t.stage.kind == StageKind::SubRoutineOnTarget && &t.stage.id == b"chit"
            }),
            "got {:?}",
            active.stages
        );
    }

    // The swing routine the melee dispatcher merges must still reach the 0x2B damage callback —
    // that is the frame the reaction (and therefore the spark chain) is handed off on.
    #[test]
    fn real_dat_swing_reaches_the_damage_callback() {
        let (Some(motion), Some((global_scheds, _))) = (
            routines_in_file(HUME_M_WEAPON_MOTION_FILE),
            global_effect_dir(),
        ) else {
            return;
        };
        let lookup = RoutineLookup::new()
            .with_dat(&motion)
            .with_dat(&global_scheds);
        let active = ActiveScheduler::effects_only_merged(&lookup, &[*b"atk0", *b"ati0"])
            .expect("the swing flattens");
        assert!(
            active
                .stages
                .iter()
                .any(|t| t.stage.kind == StageKind::DamageCallback),
            "got {:?}",
            active.stages
        );
    }

    // vendor/server/src/map/attack.h:52-59 AttackAnimation -> the limb routine
    // research/xim Actor.kt:864-903 enqueues.
    #[test]
    fn swing_routines_follow_lsb_attack_animation_order() {
        use ffxi_proto::melee::AttackAnimation;
        assert_eq!(swing_routine(AttackAnimation::RightAttack), Some(*b"ati0"));
        assert_eq!(swing_routine(AttackAnimation::LeftAttack), Some(*b"bti0"));
        assert_eq!(swing_routine(AttackAnimation::RightKick), Some(*b"cti0"));
        assert_eq!(swing_routine(AttackAnimation::LeftKick), Some(*b"dti0"));
        assert_eq!(swing_routine(AttackAnimation::Throw), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn empty_parsed() -> Arc<ParsedActionDat> {
        Arc::new(ParsedActionDat {
            schedulers: Vec::new(),
            assets: ActionAssets::default(),
        })
    }

    // The bead's acceptance criterion: repeated casts of the same spell hit the cache. A hit must
    // also count as a use, or a spammed spell would be the first thing evicted.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn action_dat_lru_evicts_least_recently_used_not_promoted_hits() {
        let mut lru = ActionDatLru::default();
        for id in 0..ACTION_DAT_CACHE_CAP as u32 {
            lru.insert(id, empty_parsed());
        }
        assert!(
            lru.get_and_promote(0).is_some(),
            "filled to cap, no eviction"
        );

        lru.insert(ACTION_DAT_CACHE_CAP as u32, empty_parsed());
        assert!(
            lru.get_and_promote(0).is_some(),
            "the promoted entry survives the over-cap insert"
        );
        assert!(
            lru.get_and_promote(1).is_none(),
            "the least-recently-used entry is the one evicted"
        );
        assert!(lru.get_and_promote(2).is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn action_dat_lru_reinsert_refreshes_recency_without_duplicating() {
        let mut lru = ActionDatLru::default();
        lru.insert(7, empty_parsed());
        for id in 100..100 + (ACTION_DAT_CACHE_CAP as u32 - 1) {
            lru.insert(id, empty_parsed());
        }
        assert_eq!(lru.map.len(), ACTION_DAT_CACHE_CAP);

        lru.insert(7, empty_parsed());
        assert_eq!(
            lru.map.len(),
            ACTION_DAT_CACHE_CAP,
            "re-insert does not double-count"
        );

        lru.insert(999, empty_parsed());
        assert!(
            lru.get_and_promote(100).is_none(),
            "the oldest untouched entry is evicted"
        );
        assert!(
            lru.get_and_promote(7).is_some(),
            "the re-insert refreshed 7's recency"
        );
        assert!(lru.get_and_promote(999).is_some());
    }
}
