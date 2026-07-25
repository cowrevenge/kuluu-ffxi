use std::collections::HashMap;

use bevy::prelude::*;
use ffxi_dat::chunk::walk;
use ffxi_dat::generator::Generator;
use ffxi_dat::kind::ChunkKind;
use ffxi_dat::scheduler::{Scheduler, StageKind, TimedStage};
use ffxi_dat::sep::Sep;

pub const FFXI_FPS: f32 = 30.0;

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

    fn flatten(lookup: &RoutineLookup, name: &[u8; 4], motion: MotionStages) -> Option<Self> {
        lookup.get(name)?;
        let mut stages = Vec::new();
        flatten_routine(lookup, name, 0, 0, motion, &mut stages);
        stages.sort_by_key(|t| t.frame);
        Some(Self {
            stages,
            elapsed: 0.0,
            cursor: 0,
            name: *name,
        })
    }

    pub fn finished(&self) -> bool {
        self.cursor >= self.stages.len()
    }

    pub fn current_frame(&self) -> u32 {
        (self.elapsed * FFXI_FPS) as u32
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
            let finish_secs = sched.last_frame() as f32 / FFXI_FPS;
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
    pub brightness: [f32; 3],
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
    pub keyframes: HashMap<[u8; 4], ffxi_dat::particle_gen::KeyFrameTrack>,
}

const MAX_SUBROUTINE_DEPTH: u8 = 6;

fn flatten_routine(
    lookup: &RoutineLookup,
    name: &[u8; 4],
    base_frame: u32,
    depth: u8,
    motion: MotionStages,
    out: &mut Vec<TimedStage>,
) {
    if depth > MAX_SUBROUTINE_DEPTH {
        return;
    }
    let Some(s) = lookup.get(name) else {
        return;
    };
    for t in &s.stages {
        let frame = base_frame + t.frame;
        match t.stage.kind {
            StageKind::SubRoutine => {
                flatten_routine(lookup, &t.stage.id, frame, depth + 1, motion, out)
            }
            StageKind::Motion if motion == MotionStages::Suppress => {}
            _ => out.push(TimedStage {
                frame,
                stage: t.stage,
            }),
        }
    }
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

pub fn parse_action_bytes(bytes: &[u8]) -> (Vec<Scheduler>, ActionAssets) {
    let mut schedulers = Vec::new();
    let mut assets = ActionAssets::default();
    for c in walk(bytes).flatten() {
        let Some(kind) = ChunkKind::from_u8(c.kind) else {
            continue;
        };
        match kind {
            ChunkKind::Scheduler => {
                if let Ok(s) = Scheduler::parse(c.name, c.data) {
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
    }
    (schedulers, assets)
}

#[cfg(not(target_arch = "wasm32"))]
fn mmb_sprite_mesh(data: &[u8]) -> Option<MmbSpriteMesh> {
    let dec = ffxi_dat::mmb::decrypt(data).ok()?;
    let models = ffxi_dat::mmb::parse_models(&dec);
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
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
    let c = models
        .iter()
        .find(|m| !m.vertices.is_empty())
        .map(|m| m.vertices[0].rgba)
        .unwrap_or([128, 128, 128, 128]);
    Some(MmbSpriteMesh {
        positions,
        uvs,
        indices,
        brightness: [
            c[0] as f32 / 128.0,
            c[1] as f32 / 128.0,
            c[2] as f32 / 128.0,
        ],
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
                std::fs::read(loc.path_under(root.root())).ok()
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
pub fn dispatch_sound_stages(
    mut events: MessageReader<SchedulerStageEvent>,
    q_actors: Query<&ActionAssets>,
    global: Option<Res<GlobalEffectDir>>,
    mut sfx_writer: MessageWriter<crate::audio::SfxEvent>,
) {
    for ev in events.read() {
        let kind = ev.stage.stage.kind;
        if !matches!(kind, StageKind::SoundOnCaster | StageKind::SoundOnTarget) {
            continue;
        }
        let Some(assets) = assets_holding(
            q_actors.get(ev.actor).ok(),
            global.as_ref().map(|g| &g.assets),
            |a| {
                ffxi_dat::action::resolve_stage_to_se(
                    &ev.stage.stage.id,
                    kind,
                    &a.generators,
                    &a.seps,
                )
                .is_some()
            },
        ) else {
            continue;
        };

        let Some((se_id, _on_caster)) = ffxi_dat::action::resolve_stage_to_se(
            &ev.stage.stage.id,
            kind,
            &assets.generators,
            &assets.seps,
        ) else {
            continue;
        };

        sfx_writer.write(crate::audio::SfxEvent::new(se_id));
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
    action_kind: u8,
    race: Option<u8>,
    main_dll: Option<&ffxi_dat::main_dll::MainDll>,
) -> Option<u32> {
    // research/xim EffectDisplayer.displaySkill: the completion effect routine for a
    // skill lives in the file-table DAT keyed by the skill's animation index. Only the
    // "finish" action categories carry that completed skill — start categories drive the
    // caster's cast-loop motion instead (see ffxi_actor_render::action_routine).
    // vendor/server map/utils/battleutils action categories: 3 = weaponskill finish,
    // 4 = magic finish, 6 = job-ability finish.
    match action_kind {
        3 => weapon_skill_file_id(action_id, race?, main_dll?),
        4 => ffxi_proto::action_anim::spell_file_id(action_id),
        6 => ffxi_proto::action_anim::ability_file_id(action_id),
        _ => None,
    }
}

// research/xim AbilityTable.kt:103 — WS file id = race base (FFXiMain.dll) + per-skill index.
// `race` is the FFXI look race byte (HumeM=1..Galka=8), which is XIM's RaceGenderConfig.index.
fn weapon_skill_file_id(
    weapon_skill_id: u32,
    race: u8,
    main_dll: &ffxi_dat::main_dll::MainDll,
) -> Option<u32> {
    let index = ffxi_proto::action_anim::weapon_skill_animation_index(weapon_skill_id)?;
    let base = main_dll.base_weapon_skill_index(race)?;
    Some(base as u32 + index as u32)
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
        let Some(file_id) =
            action_dat_file_id(action_id, action_kind, race, dll_cache.dll.as_ref())
        else {
            continue;
        };

        let Ok(root) = ffxi_dat::DatRoot::from_env_or_default() else {
            continue;
        };
        let Ok(loc) = root.resolve(file_id) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            continue;
        };
        let (schedulers, assets) = parse_action_bytes(&bytes);

        // A spell DAT's `main` links the caster's own finish routine (0x3C `shbk`), which in turn
        // links global-dir routines — so the flatten must span all three tiers.
        let actor_routines = actor_render_routines(actor_entity, &q_children, &q_render);
        let mut lookup = RoutineLookup::new().with_dat(&schedulers);
        if let Some(r) = actor_routines {
            lookup = lookup.with_actor(r);
        }
        if let Some(g) = global.as_ref() {
            lookup = lookup.with_dat(&g.schedulers);
        }

        let active = ActiveScheduler::from_routine(&lookup, b"main")
            .or_else(|| schedulers.first().map(ActiveScheduler::from_scheduler));
        let Some(active) = active else { continue };

        commands
            .entity(actor_entity)
            .try_insert(active)
            .try_insert(assets)
            .try_insert(ActionTarget(target_entity));
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
#[derive(Component, Debug, Clone, Copy)]
pub struct CastRoutine(pub [u8; 4]);

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
        let suffix = spell_suffix.suffix(action_id);
        let Some((routine, _looping)) =
            crate::ffxi_actor_render::action_routine(action_kind, suffix)
        else {
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
        let name = routine.0;
        let Some(active) = ActiveScheduler::effects_only(&lookup, &name) else {
            continue;
        };
        commands
            .entity(actor_entity)
            .try_insert(active)
            .try_insert(CastRoutine(name))
            .try_insert(ActionTarget(
                target_id.and_then(|id| tracked.by_id.get(&id).copied()),
            ));
    }
}

// Belt-and-braces stop for the cases retail's 0x2D StopParticle stages cannot reach: an
// interrupted cast never runs the spell DAT's `main`, and an observed caster's interrupt carries
// no wire signal — both end the cast pose, which is what this watches.
#[cfg(not(target_arch = "wasm32"))]
pub fn stop_cast_effects_when_cast_ends(
    q_cast: Query<(Entity, &CastRoutine, &Children)>,
    q_render: Query<&crate::ffxi_actor_render::FfxiRenderActor>,
    mut sim: ResMut<crate::particle_sim::ParticleSimulator>,
    mut commands: Commands,
) {
    for (entity, cast, children) in &q_cast {
        let Some(actor) = children.iter().find_map(|c| q_render.get(c).ok()) else {
            continue;
        };
        if actor.cast_posing() {
            continue;
        }
        sim.stop_routine(entity, cast.0);
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
                if let Some(active) = load_emote_scheduler(file_id, &routine) {
                    commands
                        .entity(actor_entity)
                        .try_insert(active.0)
                        .try_insert(active.1)
                        .try_insert(ActionTarget(tracked.by_id.get(&target_id).copied()));
                    continue;
                }
            }
        }

        // NPC casters (lua sendEmote) and PCs whose emote DAT failed to load:
        // play the actor's own em0N clip when it has one; silent no-op
        // otherwise (XIM findLocalAnimationRoutine, Actor.kt:695-697).
        let clip = ffxi_dat::datid::DatId::from_name(&routine);
        let Ok(children) = q_children.get(actor_entity) else {
            continue;
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
}

#[cfg(not(target_arch = "wasm32"))]
fn load_emote_scheduler(
    file_id: u32,
    routine: &[u8; 4],
) -> Option<(ActiveScheduler, ActionAssets)> {
    let root = ffxi_dat::DatRoot::from_env_or_default().ok()?;
    let loc = root.resolve(file_id).ok()?;
    let bytes = std::fs::read(loc.path_under(root.root())).ok()?;
    let (schedulers, assets) = parse_action_bytes(&bytes);
    let active = ActiveScheduler::from_main(&schedulers, routine)?;
    Some((active, assets))
}

pub struct SchedulerRuntimePlugin;

impl Plugin for SchedulerRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SchedulerStageEvent>()
            .add_systems(Update, tick_active_schedulers);

        #[cfg(not(target_arch = "wasm32"))]
        {
            app.init_resource::<crate::particle_sim::ParticleSimulator>();
            app.add_systems(Startup, load_global_effect_dir);
            app.add_systems(
                Update,
                (
                    poll_global_effect_dir,
                    dispatch_action_started,
                    dispatch_cast_routine_started,
                    dispatch_entity_emoted,
                    crate::particle_sim::spawn_actor_auto_run_particles,
                    crate::particle_sim::spawn_particle_generators,
                    dispatch_stop_particle_stages,
                    crate::particle_sim::stop_generators_for_despawned_owners,
                    crate::particle_sim::tick_particle_simulator,
                    crate::particle_sim::sync_particle_meshes,
                    dispatch_sound_stages,
                    dispatch_motion_stages,
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
            },
        }
    }

    fn make_scheduler(name: [u8; 4], stages: Vec<TimedStage>) -> Scheduler {
        Scheduler { name, stages }
    }

    #[test]
    fn current_frame_advances_by_fps() {
        let sched = make_scheduler(*b"main", vec![]);
        let mut a = ActiveScheduler::from_scheduler(&sched);
        a.elapsed = 0.5;
        assert_eq!(a.current_frame(), 15);
        a.elapsed = 1.0;
        assert_eq!(a.current_frame(), 30);
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
        let Ok(root) = ffxi_dat::DatRoot::from_env_or_default() else {
            return;
        };
        let Ok(dll) = ffxi_dat::main_dll::MainDll::load(root.root()) else {
            return;
        };
        let base = dll.base_emote_index(1).expect("HumeM emote base") as u32;
        let (offset, routine) = emote_routine(1, 0).expect("bow is mapped");
        let loc = root.resolve(base + offset).expect("emote file resolves");
        let bytes = std::fs::read(loc.path_under(root.root())).expect("emote DAT readable");
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

        let Ok(root) = ffxi_dat::DatRoot::from_env_or_default() else {
            return;
        };
        let Ok(loc) = root.resolve(POISON_FILE) else {
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
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
        let Ok(cure_bytes) = std::fs::read(cure_loc.path_under(root.root())) else {
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

        let Ok(root) = ffxi_dat::DatRoot::from_env_or_default() else {
            return;
        };
        let Ok(loc) = root.resolve(POISON_FILE) else {
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
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
        let root = ffxi_dat::DatRoot::from_env_or_default().ok()?;
        let loc = root.resolve(file_id).ok()?;
        std::fs::read(loc.path_under(root.root())).ok()
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

        assert!(
            ActiveScheduler::effects_only(
                &RoutineLookup::new().with_dat(&actor_scheds),
                &CAST_ROUTINE
            )
            .expect("cabk exists")
            .stages
            .is_empty(),
            "without the global tier the aura sub-routines resolve to nothing — the original bug"
        );
    }
}
