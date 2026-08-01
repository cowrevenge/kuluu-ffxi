use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use ffxi_dat::particle_gen::{KeyFrameTrack, ParticleGeneratorDef, ParticleMeshKind};
use ffxi_dat::sprite_sheet::ParticleSpriteSheet;

use crate::camera::OperatorCamera;
use crate::components::InGameEntity;
use crate::dat_d3m::{d3m_material, decoded_texture_to_image, D3mBlendMode};
use crate::scheduler_runtime::{
    assets_holding, ActionAssets, GlobalEffectDir, MmbSpriteMesh, SchedulerStageEvent, ROUTINE_FPS,
};
use ffxi_dat::scheduler::StageKind;

// CPU particle simulation. research/xim ParticleGenerator + Particle: a Particle stage (0x02)
// spawns a `LiveGenerator` that streams billboard particles over its window, each integrating
// velocity and following per-particle keyframe tracks (scale/alpha) by life progress. One retained
// mesh entity per generator is rebuilt each frame from its live particles — not an entity per
// particle.
#[derive(Resource, Default)]
pub struct ParticleSimulator {
    generators: Vec<LiveGenerator>,
    clock: CelestialClock,
}

// The Vana'diel clock inputs the celestial particle opcodes read. research/xim
// ParticleUpdaters.kt: ClockValueUpdater samples its keyframe curve at
// EnvironmentManager.getFullDayInterpolation() (the fraction of the Vana'diel day, NOT the
// particle's life progress); DayOfWeekColorUpdater / MoonPhaseColorUpdater /
// MoonPhaseSpriteSheetUpdater index their tables by the elemental weekday and moon phase.
#[derive(Clone, Copy, Debug, Default)]
pub struct CelestialClock {
    pub day_fraction: f32,
    pub day_of_week: usize,
    pub moon_phase: usize,
}

impl ParticleSimulator {
    pub fn drain_entities(&mut self) -> Vec<Entity> {
        self.generators.drain(..).map(|g| g.entity).collect()
    }

    pub fn set_celestial_clock(&mut self, clock: CelestialClock) {
        self.clock = clock;
    }

    // research/xim ParticleGeneratorAttachment / cexi-viewer particle/runtime.js:517-524 —
    // a Sun/Moon-attached generator's associated position is the celestial body's position
    // offset by the camera, refreshed every frame so the sky rides with the viewer.
    pub fn set_celestial_origins(&mut self, sun: Vec3, moon: Vec3) {
        use ffxi_dat::particle_gen::AttachType;
        for g in &mut self.generators {
            g.origin = match g.def.attach_type {
                AttachType::Sun => sun,
                AttachType::Moon => moon,
                _ => continue,
            };
        }
    }

    // research/xim EffectRoutineParser.kt:253-258 StopParticleGeneratorRoutine — emission ceases
    // but the already-live particles play out their lifetime.
    pub fn stop_generator(&mut self, owner: Entity, gen_id: [u8; 4]) {
        self.stop_where(|o| o.owner == owner && o.gen_id == gen_id);
    }

    pub fn stop_routine(&mut self, owner: Entity, routine: [u8; 4]) {
        self.stop_where(|o| o.owner == owner && o.routine == routine);
    }

    // A caster that despawns mid-cast (zone-out, death, out of range) never ends its cast pose,
    // so the aura's authored emit window would keep emitting at its last position without this.
    pub fn stop_generators_of_dead_owners(&mut self, alive: impl Fn(Entity) -> bool) {
        self.stop_where(|o| !alive(o.owner));
    }

    fn stop_where(&mut self, pred: impl Fn(&RoutineOrigin) -> bool) {
        for g in &mut self.generators {
            if g.origin_routine.is_some_and(|o| pred(&o)) {
                g.stopped = true;
            }
        }
    }
}

// Routine-spawned generators are addressable so a later StopParticle stage (or an interrupted
// cast) can end them: `owner` is the tracked entity the routine ran on, `gen_id` the generator
// chunk id, `routine` the top-level routine the stage was flattened from.
#[derive(Clone, Copy)]
struct RoutineOrigin {
    owner: Entity,
    gen_id: [u8; 4],
    routine: [u8; 4],
}

#[derive(Clone)]
struct SpriteTemplate {
    positions: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    brightness: Vec3,
    vert_alpha: f32,
}

// research/XIClient/src/XIClient/source/Resource/Derived/CMoD3m.cpp:16-104 — the D3m texture-stage
// tables, with D = diffuse/vertex, T = texture, F = TEXTUREFACTOR (the generator's particle
// colour). NonZeroTwoTSS is the textured default: stage 0 is MODULATE2X(D,T) for both channels,
// stage 1 MODULATE2X(CURRENT,F) for rgb and MODULATE4X(CURRENT,F) for alpha — totals 4 and 8.
// NonZeroOneTSS (renderStateFlags 0x1000) replaces stage 0's alpha with SELECTARG1(D.a), halving
// the alpha total to 4. The MMB-mesh branch
// (research/XIClient/src/XIClient/source/Rendering/ZoneRenderer.cpp:1396-1433 DoD3mDraw) reaches
// the same per-stage ops, so every template kind goes through `d3m_stage_chain`.
const D3M_STAGE1_RGB_GAIN: f32 = 2.0;
const D3M_STAGE1_ALPHA_GAIN: f32 = 4.0;
// Stage 0's MODULATE2X is already folded into `brightness`/`vert_alpha` by the /128 vertex-colour
// normalise (ffxi_dat::d3m::VERTEX_COLOR_DIVISOR). NonZeroOneTSS's SELECTARG1 does not double, so
// the ignore-texture-alpha table divides it back out.
const D3M_VERTEX_BAKED_GAIN: f32 = 2.0;
// D3D saturates every texture-stage result. Stage 0's texture argument is only available in the
// sampler, so the CPU-side stage-0 value saturates without it and the shader's later multiply by
// a texel <= 1 keeps the drawn colour inside retail's ceiling.
const D3M_STAGE_CLAMP: f32 = 1.0;

// research/XIClient/src/XIClient/source/World/Generator/Effects/CMoD3mElem.cpp:57-63 — `OnDraw`
// sends the element through `DoMMBDraw` when its link is an MMB and `CMoD3m::Draw` otherwise. The
// two paths share the stage tables but not the blend bytes they honour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum D3mDrawPath {
    D3m,
    Mmb,
}

// CMoD3mElem.cpp:108-112 — DoMMBDraw forces the ignore-texture-alpha table at this blend byte,
// whatever the render-state bit says.
const D3M_MMB_FORCE_IGNORE_TEXTURE_ALPHA_BLEND_BYTE: u8 = 0x64;
// CMoD3m.cpp:345-349 — at blend byte 0x44 a TEXTUREFACTOR alpha at or above 0x7F is promoted to
// 0xFF before the stage math. DoMMBDraw carries no such promotion.
const D3M_TFACTOR_PROMOTE_BLEND_BYTE: u8 = 0x44;
const D3M_TFACTOR_PROMOTE_MIN: f32 = 0x7F as f32 / u8::MAX as f32;
const D3M_TFACTOR_PROMOTED: f32 = 1.0;

fn ignores_texture_alpha(def: &ParticleGeneratorDef, path: D3mDrawPath) -> bool {
    def.ignore_texture_alpha
        || (path == D3mDrawPath::Mmb
            && def.blend_byte == D3M_MMB_FORCE_IGNORE_TEXTURE_ALPHA_BLEND_BYTE)
}

fn tfactor_alpha(def: &ParticleGeneratorDef, path: D3mDrawPath, alpha: f32) -> f32 {
    if path == D3mDrawPath::D3m
        && def.blend_byte == D3M_TFACTOR_PROMOTE_BLEND_BYTE
        && alpha >= D3M_TFACTOR_PROMOTE_MIN
    {
        D3M_TFACTOR_PROMOTED
    } else {
        alpha
    }
}

// Resolve the generator's 0x60..0x63 time-of-day colour curves against the DAT's keyframe
// chunks. Absent on everything but the celestial billboards.
fn resolve_tod_tracks(
    def: &ParticleGeneratorDef,
    assets: &ActionAssets,
) -> [Option<KeyFrameTrack>; ffxi_dat::particle_gen::TOD_COLOR_CHANNELS] {
    def.tod_color_tracks
        .map(|id| id.and_then(|i| assets.keyframes.get(&i).cloned()))
}

// research/xim Particle.kt:217-218 — the day-of-week / moon-phase tints are applied with
// Color.modulateInPlace(c, 2f), a 2x modulate.
const CELESTIAL_MODULATE: f32 = 2.0;
// Index of the alpha channel in the 0x60..0x63 time-of-day track array (0x63 -> 0x3F).
const TOD_ALPHA_CHANNEL: usize = 3;

fn d3m_stage_chain(
    vertex_rgb: Vec3,
    vertex_alpha: f32,
    f_rgb: Vec3,
    f_alpha: f32,
    ignore_texture_alpha: bool,
) -> (Vec3, f32) {
    let clamp = Vec3::splat(D3M_STAGE_CLAMP);
    let stage0_rgb = vertex_rgb.min(clamp);
    let stage0_alpha = if ignore_texture_alpha {
        vertex_alpha / D3M_VERTEX_BAKED_GAIN
    } else {
        vertex_alpha.min(D3M_STAGE_CLAMP)
    };
    (
        (stage0_rgb * f_rgb * D3M_STAGE1_RGB_GAIN).min(clamp),
        (stage0_alpha * f_alpha * D3M_STAGE1_ALPHA_GAIN).min(D3M_STAGE_CLAMP),
    )
}

struct LiveGenerator {
    def: ParticleGeneratorDef,
    template: SpriteTemplate,
    draw_path: D3mDrawPath,
    // SpriteSheet (0x0E) flipbook frames; empty for a StaticMesh (0x0B) generator. When
    // non-empty each particle picks a frame by life progress in rebuild_mesh (research/xim
    // Particle.kt:72 spriteSheetIndex advanced over life).
    sprite_frames: Vec<SpriteTemplate>,
    scale_x: Option<KeyFrameTrack>,
    scale_y: Option<KeyFrameTrack>,
    alpha: Option<KeyFrameTrack>,
    // The 0x60..0x63 time-of-day RGBA curves, resolved against the DAT's keyframe chunks.
    // Sampled at the Vana'diel day fraction, so unlike `alpha` above they do not advance
    // with the particle's own life.
    tod_color: [Option<KeyFrameTrack>; ffxi_dat::particle_gen::TOD_COLOR_CHANNELS],
    origin: Vec3,
    particles: Vec<Particle>,
    emit_accum: f32,
    age_frames: f32,
    emit_window_frames: f32,
    mesh: Handle<Mesh>,
    entity: Entity,
    // research/xim ParticleGenerator.kt:56 — auto-run generators never finish
    // emitting; they live until their mesh entity (a child of the actor root)
    // is despawned.
    auto_run: bool,
    // Fixed particle orientation (init_rotation); None = camera billboard.
    orientation: Option<Quat>,
    // The mesh entity is a child of the actor root, so vertex positions are
    // built in the actor's FFXI-local frame instead of world space.
    actor_local: bool,
    // Accumulated UV-translate (def.uv_scroll integrated over life) added to every
    // template UV so a scrolling water sheet/cascade slides its texture.
    tex_translate: Vec2,
    // Per-axis sign applied to init_velocity/accel. Actor-local generators integrate
    // in the DAT frame (ONE); world-space zone generators build positions directly in
    // Bevy space, so velocity gets the same mzb->bevy basis (x,-y,-z) as the origin.
    vel_basis: Vec3,
    origin_routine: Option<RoutineOrigin>,
    stopped: bool,
    // Key of the last BUILT mesh (spawn writes `empty_mesh`, hence `MeshKey::Empty`), so
    // quantization error is bounded by one quantum and never accumulates across skipped frames.
    built_key: MeshKey,
}

// Auto-run particle generators embedded in an actor DAT (research/xim
// Actor.kt:724-734 startAutoRunParticles), attached at actor spawn by
// ffxi_actor_render and started by `spawn_actor_auto_run_particles`.
#[derive(Component)]
pub struct ActorAutoRunEffects {
    pub assets: std::sync::Arc<ActionAssets>,
}

struct Particle {
    pos: Vec3,
    vel: Vec3,
    age_frames: f32,
    life_frames: f32,
    rgb: Vec3,
    scale: Vec2,
}

pub fn spawn_particle_generators(
    mut events: MessageReader<SchedulerStageEvent>,
    q_actors: Query<(&Transform, Option<&ActionAssets>)>,
    q_action_target: Query<&crate::scheduler_runtime::ActionTarget>,
    q_xf: Query<&Transform>,
    global: Option<Res<GlobalEffectDir>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sim: ResMut<ParticleSimulator>,
    mut commands: Commands,
) {
    for ev in events.read() {
        if ev.stage.stage.kind != StageKind::Particle {
            continue;
        }
        let Ok((actor_xf, local_assets)) = q_actors.get(ev.actor) else {
            continue;
        };
        // A cast routine's generators ship in the global effect dir, never in the caster's own
        // ActionAssets, so the def resolves against whichever tier actually holds it.
        let local_dir = ev.stage.stage.local_dir;
        let Some(assets) = assets_holding(local_assets, global.as_ref().map(|g| &g.assets), |a| {
            a.particle_def(local_dir, &ev.stage.stage.id).is_some()
        }) else {
            continue;
        };
        let Some(def) = assets.particle_def(local_dir, &ev.stage.stage.id).copied() else {
            continue;
        };
        let Some((template, sprite_frames, tex)) = resolve_mesh(assets, &def, &mut images) else {
            continue;
        };
        let origin_entity = crate::scheduler_runtime::particle_origin_entity(
            def.attach_type,
            ev.actor,
            q_action_target.get(ev.actor).ok().and_then(|t| t.0),
        );
        let origin_xf = if origin_entity == ev.actor {
            actor_xf
        } else {
            q_xf.get(origin_entity).unwrap_or(actor_xf)
        };
        let blend = match def.blend {
            ffxi_dat::particle_gen::ParticleBlend::Additive => D3mBlendMode::Additive,
            ffxi_dat::particle_gen::ParticleBlend::Blend => D3mBlendMode::Blended,
            ffxi_dat::particle_gen::ParticleBlend::Subtract => D3mBlendMode::Subtractive,
        };
        let mat = mats.add(d3m_material(blend, tex));
        let mesh = meshes.add(empty_mesh());

        let entity = commands
            .spawn((
                InGameEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                Transform::IDENTITY,
                Visibility::default(),
                // The mesh is rebuilt in place every frame; Bevy computes a frustum-culling Aabb
                // once from the initially-empty mesh and never recomputes it, so the entity would
                // be culled forever. Opt out of culling instead.
                bevy::camera::visibility::NoFrustumCulling,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ))
            .id();

        debug!(
            "spawned particle generator {} mesh {} life {}",
            String::from_utf8_lossy(&ev.stage.stage.id),
            String::from_utf8_lossy(&def.mesh_id),
            def.max_life_frames
        );

        let resolve = |id: Option<[u8; 4]>| -> Option<KeyFrameTrack> {
            id.and_then(|i| assets.keyframes.get(&i).cloned())
        };

        let emit_window_frames = ev.stage.stage.duration_frames as f32;
        sim.generators.push(LiveGenerator {
            scale_x: resolve(def.scale_x_track),
            scale_y: resolve(def.scale_y_track),
            alpha: resolve(def.alpha_track),
            tod_color: resolve_tod_tracks(&def, assets),
            template,
            draw_path: D3mDrawPath::D3m,
            sprite_frames,
            def,
            origin: origin_xf.translation + Vec3::Y * def.base_position[1],
            particles: Vec::new(),
            emit_accum: 0.0,
            age_frames: 0.0,
            emit_window_frames,
            mesh,
            entity,
            auto_run: false,
            orientation: None,
            actor_local: false,
            tex_translate: Vec2::ZERO,
            vel_basis: Vec3::ONE,
            origin_routine: Some(RoutineOrigin {
                owner: ev.actor,
                gen_id: ev.stage.stage.id,
                routine: ev.scheduler,
            }),
            stopped: false,
            built_key: MeshKey::Empty,
        });
    }
}

// research/xim Actor.kt:127,724-734 — at model-ready, every generator in the
// actor DAT flagged auto-run starts immediately and emits forever. The mesh
// entity is a child of the actor root (which carries the FFXI->Bevy basis), so
// particle math stays in the DAT's own FFXI-local frame and the effect follows
// and despawns with the actor.
pub fn spawn_actor_auto_run_particles(
    q_added: Query<(Entity, &ActorAutoRunEffects), Added<ActorAutoRunEffects>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sim: ResMut<ParticleSimulator>,
    mut commands: Commands,
) {
    for (actor_root, fx) in &q_added {
        for (name, def) in fx.assets.particle_defs.iter() {
            if !def.auto_run {
                continue;
            }
            let def = *def;
            let Some((template, sprite_frames, tex)) = resolve_mesh(&fx.assets, &def, &mut images)
            else {
                continue;
            };
            let blend = match def.blend {
                ffxi_dat::particle_gen::ParticleBlend::Additive => D3mBlendMode::Additive,
                ffxi_dat::particle_gen::ParticleBlend::Blend => D3mBlendMode::Blended,
                ffxi_dat::particle_gen::ParticleBlend::Subtract => D3mBlendMode::Subtractive,
            };
            let mat = mats.add(d3m_material(blend, tex));
            let mesh = meshes.add(empty_mesh());

            let entity = commands
                .spawn((
                    InGameEntity,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat),
                    Transform::IDENTITY,
                    ChildOf(actor_root),
                    bevy::camera::visibility::NoFrustumCulling,
                    bevy::light::NotShadowCaster,
                    bevy::light::NotShadowReceiver,
                ))
                .id();

            debug!(
                "auto-run particle generator {} mesh {} blend {:?}",
                String::from_utf8_lossy(name),
                String::from_utf8_lossy(&def.mesh_id),
                def.blend,
            );

            let resolve = |id: Option<[u8; 4]>| -> Option<KeyFrameTrack> {
                id.and_then(|i| fx.assets.keyframes.get(&i).cloned())
            };
            let rot = def.init_rotation;
            sim.generators.push(LiveGenerator {
                scale_x: resolve(def.scale_x_track),
                scale_y: resolve(def.scale_y_track),
                alpha: resolve(def.alpha_track),
                tod_color: resolve_tod_tracks(&def, &fx.assets),
                template,
                draw_path: D3mDrawPath::D3m,
                sprite_frames,
                origin: Vec3::from_array(def.base_position),
                particles: Vec::new(),
                emit_accum: 0.0,
                age_frames: 0.0,
                emit_window_frames: 0.0,
                mesh,
                entity,
                auto_run: true,
                orientation: (!def.camera_billboard)
                    .then(|| Quat::from_euler(EulerRot::XYZ, rot[0], rot[1], rot[2])),
                actor_local: true,
                tex_translate: Vec2::ZERO,
                vel_basis: Vec3::ONE,
                origin_routine: None,
                stopped: false,
                built_key: MeshKey::Empty,
                def,
            });
        }
    }
}

// research/xim EnvironmentManager zone-static Generator: an auto-run particle
// generator embedded in the zone MZB DAT (Bastok Mines pump spray), placed in
// world space rather than parented to an actor. `origin` is already mzb->bevy;
// velocity/accel take the same basis so the spray arcs in Bevy space.
pub fn spawn_zone_particle_generator(
    def: ParticleGeneratorDef,
    assets: &ActionAssets,
    origin: Vec3,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    sim: &mut ParticleSimulator,
    commands: &mut Commands,
) -> Option<Entity> {
    // Zone sprays link a D3M billboard, an MMB mesh, or a SpriteSheet by DatId (e.g. Bastok
    // "abuk", Port Windurst "rivsea"); the MMB/SpriteSheet texture resolves by internal name.
    let (template, sprite_frames, tex, draw_path) =
        if let Some((template, frames, tex)) = resolve_mesh(assets, &def, images) {
            (template, frames, tex, D3mDrawPath::D3m)
        } else {
            let mmb = assets.mmbs.get(&def.mesh_id)?;
            let template = mmb_sprite_template(mmb)?;
            let tex = assets
                .images_by_name
                .get(&mmb.texture_name)
                .map(|t| images.add(decoded_texture_to_image(t)));
            (template, Vec::new(), tex, D3mDrawPath::Mmb)
        };
    let blend = match def.blend {
        ffxi_dat::particle_gen::ParticleBlend::Additive => D3mBlendMode::Additive,
        ffxi_dat::particle_gen::ParticleBlend::Blend => D3mBlendMode::Blended,
        ffxi_dat::particle_gen::ParticleBlend::Subtract => D3mBlendMode::Subtractive,
    };
    let mat = mats.add(d3m_material(blend, tex));
    let mesh = meshes.add(empty_mesh());

    let entity = commands
        .spawn((
            InGameEntity,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat),
            Transform::IDENTITY,
            Visibility::default(),
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ))
        .id();

    let resolve = |id: Option<[u8; 4]>| -> Option<KeyFrameTrack> {
        id.and_then(|i| assets.keyframes.get(&i).cloned())
    };
    let rot = def.init_rotation;
    sim.generators.push(LiveGenerator {
        scale_x: resolve(def.scale_x_track),
        scale_y: resolve(def.scale_y_track),
        alpha: resolve(def.alpha_track),
        tod_color: resolve_tod_tracks(&def, assets),
        template,
        draw_path,
        sprite_frames,
        origin,
        particles: Vec::new(),
        emit_accum: 0.0,
        age_frames: 0.0,
        emit_window_frames: 0.0,
        mesh,
        entity,
        auto_run: true,
        orientation: (!def.camera_billboard)
            .then(|| Quat::from_euler(EulerRot::XYZ, rot[0], rot[1], rot[2])),
        actor_local: false,
        tex_translate: Vec2::ZERO,
        vel_basis: Vec3::new(1.0, -1.0, -1.0),
        origin_routine: None,
        stopped: false,
        built_key: MeshKey::Empty,
        def,
    });
    Some(entity)
}

pub fn stop_generators_for_despawned_owners(
    q_alive: Query<()>,
    mut sim: ResMut<ParticleSimulator>,
) {
    sim.stop_generators_of_dead_owners(|e| q_alive.get(e).is_ok());
}

pub fn tick_particle_simulator(time: Res<Time>, mut sim: ResMut<ParticleSimulator>) {
    let frames = time.delta_secs() * ROUTINE_FPS;
    for g in &mut sim.generators {
        advance_generator(g, frames);
    }
}

fn advance_generator(g: &mut LiveGenerator, frames: f32) {
    g.age_frames += frames;

    // research/xim ParticleGenerator.kt:66 — completed particles are swept
    // before emission, so a continuous singleton re-emits the same tick its
    // predecessor expires.
    g.particles.retain(|p| p.age_frames < p.life_frames);

    // Particles emitted below were born during this tick, so the ageing pass must not charge them
    // the whole frame: at 30 fps retail that error is invisible, but one long frame (the blocking
    // action-DAT read) would otherwise age a freshly emitted short-life particle past its life and
    // sweep it before it ever renders.
    let pre_emit_len = g.particles.len();

    // research/xim: a maxLifeSpan of 0 marks a singleton — emit one particle once.
    let singleton = g.def.is_singleton();
    let emitting = !g.stopped && (g.auto_run || g.age_frames <= g.emit_window_frames.max(1.0));
    if singleton {
        // `age_frames <= frames` already pins this to the first tick, so the emit window must not
        // gate it: a long frame (the blocking action-DAT read precedes these) makes age_frames
        // exceed a dur=0 stage's 1-frame window on that very tick and the singleton never fires.
        if !g.stopped && g.particles.is_empty() && g.age_frames <= frames {
            // research/xim ParticleInitializers.kt:130-131 — a maxLifeSpan of 0 is rewritten
            // to POSITIVE_INFINITY, "used for 'singleton' particles, like the sea and such":
            // the auto-run zone/weather billboards that stand as long as the zone does (the
            // sun, the moon, the sea). A 1-frame life made those vanish on the tick after
            // they spawned. A scheduled generator is NOT that population — its singleton
            // plays out the stage window and is reaped with the effect, so it keeps the
            // bounded life or a dur=0 cast aura would hang in the world forever.
            let bounded = g.emit_window_frames.max(g.def.max_life_frames);
            let life = if g.auto_run && bounded <= 0.0 {
                f32::INFINITY
            } else {
                bounded.max(1.0)
            };
            emit(g, life);
        }
    } else if emitting {
        g.emit_accum += frames;
        while g.emit_accum >= g.def.frames_per_emission {
            // research/xim ParticleGenerator.kt:80 — a continuous-singleton
            // generator holds one live particle and re-emits the moment it
            // expires (the accumulator stays primed, capped to one period).
            if g.def.continuous && !g.particles.is_empty() {
                g.emit_accum = g.def.frames_per_emission;
                break;
            }
            g.emit_accum -= g.def.frames_per_emission;
            for _ in 0..g.def.particles_per_emission {
                emit(g, g.def.max_life_frames);
                if g.def.continuous {
                    break;
                }
            }
        }
    }

    // research/xim ParticleUpdaters TextureCoordinateUpdater: scroll velocity is
    // per-generator (frames of life advance the shared UV offset), not per-particle.
    g.tex_translate += Vec2::from_array(g.def.uv_scroll) * frames;

    let accel = g
        .def
        .accel
        .map(|a| Vec3::from_array(a) * g.vel_basis * frames);
    for p in g.particles.iter_mut().take(pre_emit_len) {
        p.age_frames += frames;
        if let Some(a) = accel {
            p.vel += a;
        }
        p.pos += p.vel * frames;
    }
    g.particles.retain(|p| p.age_frames < p.life_frames);

    // A continuous generator re-emits "the moment its particle expires"
    // (research/xim ParticleGenerator.kt:80). The aging above can push the lone
    // particle past its life within this same tick, after the pre-emit sweep
    // already ran — replace it now so the mesh is never empty at render and the
    // body does not blink out for a frame.
    if g.def.continuous && g.particles.is_empty() && continuous_active(g) {
        emit(g, g.def.max_life_frames);
    }
}

fn continuous_active(g: &LiveGenerator) -> bool {
    !g.stopped && (g.auto_run || g.age_frames <= g.emit_window_frames.max(1.0))
}

fn emit(g: &mut LiveGenerator, life_frames: f32) {
    g.particles.push(Particle {
        pos: Vec3::ZERO,
        vel: Vec3::from_array(g.def.init_velocity) * g.vel_basis,
        age_frames: 0.0,
        life_frames: life_frames.max(1.0),
        rgb: Vec3::from_slice(&g.def.init_color[..3]),
        scale: Vec2::new(g.def.init_scale[0], g.def.init_scale[1]),
    });
}

pub fn sync_particle_meshes(
    cam: Query<&GlobalTransform, With<OperatorCamera>>,
    q_mesh_xf: Query<&GlobalTransform, With<Mesh3d>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sim: ResMut<ParticleSimulator>,
    mut commands: Commands,
) {
    let cam_rot = cam.iter().next().map(|t| t.rotation()).unwrap_or_default();
    let clock = sim.clock;
    let trace_celestial = std::env::var_os("FFXI_TRACE_CELESTIAL").is_some();

    // (index, despawn-needed); indices ascending so the reverse sweep below can
    // swap_remove safely.
    let mut reap: Vec<(usize, bool)> = Vec::new();
    for (i, g) in sim.generators.iter_mut().enumerate() {
        // The mesh entity despawns with its actor (auto-run generators are
        // children of the actor root); reap the simulator entry when it's gone.
        let Ok(entity_xf) = q_mesh_xf.get(g.entity) else {
            reap.push((i, false));
            continue;
        };
        // In the actor-local frame a billboard must cancel the parent's
        // FFXI->Bevy basis: parent_rot * rot == cam_rot. Fixed-orientation
        // meshes use their DAT rotation directly in the local frame.
        let rot = match (g.orientation, g.actor_local) {
            (Some(q), _) => q,
            (None, true) => entity_xf.rotation().inverse() * cam_rot,
            (None, false) => cam_rot,
        };
        // The tracked get_mut marks the mesh Modified and forces a full GPU re-upload, so it
        // only runs when the rebuilt vertex output would differ from the last built mesh
        // (kuluu-b5nt).
        // The celestial billboards are the one particle population with no on-screen
        // debug affordance — they are 900 units away and often below the horizon, so a
        // wrong colour curve or sprite frame is indistinguishable from "not drawing".
        if trace_celestial
            && matches!(
                g.def.attach_type,
                ffxi_dat::particle_gen::AttachType::Sun | ffxi_dat::particle_gen::AttachType::Moon
            )
        {
            let draw = g.particles.first().map(|p| particle_draw(g, p, &clock));
            info!(
                mesh = %String::from_utf8_lossy(&g.def.mesh_id),
                verts = g.template.positions.len(),
                live = g.particles.len(),
                origin = ?g.origin,
                scale = ?draw.as_ref().map(|d| d.scale),
                rgb = ?draw.as_ref().map(|d| d.rgb),
                frame = ?draw.as_ref().map(|d| d.flipbook_frame),
                "{:?} billboard",
                g.def.attach_type,
            );
        }
        let key = mesh_key(g, rot, &clock);
        if needs_rebuild(&g.built_key, &key) {
            if let Some(mut mesh) = meshes.get_mut(&g.mesh) {
                rebuild_mesh(g, rot, &clock, &mut mesh);
                g.built_key = key;
            }
        }
        let window_over =
            g.stopped || (!g.auto_run && g.age_frames > g.emit_window_frames.max(1.0));
        let done = window_over && g.particles.is_empty();
        if done {
            reap.push((i, true));
        }
    }

    for &(i, despawn) in reap.iter().rev() {
        let g = sim.generators.swap_remove(i);
        if despawn {
            commands.entity(g.entity).despawn();
        }
    }
}

struct ParticleDraw {
    flipbook_frame: usize,
    scale: Vec2,
    rgb: Vec3,
    vert_alpha: f32,
    world: Vec3,
}

fn particle_draw(g: &LiveGenerator, p: &Particle, clock: &CelestialClock) -> ParticleDraw {
    let progress = (p.age_frames / p.life_frames).clamp(0.0, 1.0);
    // A SpriteSheet particle flipbooks its frames over life (research/xim Particle.kt:72
    // spriteSheetIndex), except under MoonPhaseSpriteSheetUpdater (0x45), which pins the
    // frame to the moon phase; a StaticMesh particle keeps its single template.
    let flipbook_frame = if g.def.moon_phase_sprite {
        clock
            .moon_phase
            .min(g.sprite_frames.len().saturating_sub(1))
    } else {
        flipbook_index(g, progress)
    };
    let tpl = flipbook_template(g, flipbook_frame);
    let sx = g
        .scale_x
        .as_ref()
        .map(|t| t.sample_from(progress, Some(p.scale.x)))
        .unwrap_or(p.scale.x);
    let sy = g
        .scale_y
        .as_ref()
        .map(|t| t.sample_from(progress, Some(p.scale.y)))
        .unwrap_or(p.scale.y);
    // Additive blend ignores alpha, so the alpha track drives brightness. With
    // no track, a transient spray fades linearly to nothing over life; a
    // continuous generator (one particle re-emitted on expiry — the steady
    // crystal body) holds full opacity, or each re-emit cycle would fade the
    // single particle out and strobe the whole model transparent.
    let alpha = g
        .alpha
        .as_ref()
        .map(|t| t.sample_from(progress, Some(g.def.init_color[3])))
        .unwrap_or(if g.def.continuous {
            1.0
        } else {
            1.0 - progress
        });
    // research/xim ParticleGeneratorParser.kt:431-434 ClockValueUpdater — 0x3C/0x3D/0x3E
    // assign the particle's colour channel from a time-of-day curve, 0x3F multiplies alpha.
    // This is the sun's authored dawn/noon/dusk ramp: the disc is not tinted by a formula.
    let mut rgb = p.rgb;
    let mut alpha = alpha;
    for (channel, track) in g.tod_color.iter().enumerate() {
        let Some(track) = track.as_ref().filter(|_| g.def.tod_color_driven[channel]) else {
            continue;
        };
        let v = track.sample(clock.day_fraction);
        match channel {
            TOD_ALPHA_CHANNEL => alpha *= v,
            _ => rgb[channel] = v,
        }
    }
    // research/xim Particle.kt:217-218 getColor() — the day-of-week tint is applied first,
    // then the moon-phase tint, each as a 2x modulate (out = min(1, out * 2 * c)).
    for table in [
        g.def
            .day_of_week_color
            .map(|t| t[clock.day_of_week % ffxi_dat::particle_gen::DAYS_OF_WEEK]),
        g.def
            .moon_phase_color
            .map(|t| t[clock.moon_phase % ffxi_dat::particle_gen::MOON_PHASES]),
    ]
    .into_iter()
    .flatten()
    {
        rgb = (rgb * Vec3::from_slice(&table[..3]) * CELESTIAL_MODULATE).min(Vec3::ONE);
    }

    let (stage_rgb, stage_alpha) = d3m_stage_chain(
        tpl.brightness,
        tpl.vert_alpha,
        rgb,
        tfactor_alpha(&g.def, g.draw_path, alpha),
        ignores_texture_alpha(&g.def, g.draw_path),
    );
    // Additive/subtract ignore the alpha channel, so the alpha curve modulates brightness;
    // alpha-blended particles keep full-brightness colour and use the alpha channel. That
    // fold is a brightness proxy rather than a blend factor, so it takes the raw life curve
    // instead of the saturating stage-1 alpha.
    let (rgb, vert_alpha) = match g.def.blend {
        ffxi_dat::particle_gen::ParticleBlend::Blend => (stage_rgb, stage_alpha),
        _ => (stage_rgb * alpha, 1.0),
    };
    ParticleDraw {
        flipbook_frame,
        scale: Vec2::new(sx, sy),
        rgb,
        vert_alpha,
        world: g.origin + p.pos,
    }
}

// One step is invisible on screen: 1/1024 world unit is sub-pixel at any playable camera
// distance, and the same step on a quat component (~0.11 deg) or a UV offset (sub-texel on
// retail sprite sheets) moves a vertex/texel by less than that.
const MESH_KEY_SPATIAL_QUANTUM: f32 = 1.0 / 1024.0;
// One 8-bit render-target step; a smaller colour delta cannot change the drawn pixel.
const MESH_KEY_COLOR_QUANTUM: f32 = 1.0 / 256.0;

// Quantized snapshot of every dynamic input rebuild_mesh consumes (via particle_draw, plus the
// billboard rotation and UV scroll it reads directly). Zero live particles rebuild to the same
// hidden primitive whatever those inputs are, hence the input-free Empty variant.
#[derive(PartialEq, Eq, Debug)]
enum MeshKey {
    Empty,
    Live {
        rot: [i32; 4],
        uv_scroll: [i32; 2],
        particles: Vec<ParticleKey>,
    },
}

#[derive(PartialEq, Eq, Debug)]
struct ParticleKey {
    world: [i32; 3],
    flipbook_frame: usize,
    scale: [i32; 2],
    color: [i32; 4],
}

fn quantized(v: f32, quantum: f32) -> i32 {
    (v / quantum).round() as i32
}

fn mesh_key(g: &LiveGenerator, rot: Quat, clock: &CelestialClock) -> MeshKey {
    if g.particles.is_empty() {
        return MeshKey::Empty;
    }
    let spatial = |v: f32| quantized(v, MESH_KEY_SPATIAL_QUANTUM);
    let color = |v: f32| quantized(v, MESH_KEY_COLOR_QUANTUM);
    MeshKey::Live {
        rot: rot.to_array().map(spatial),
        uv_scroll: [spatial(g.tex_translate.x), spatial(g.tex_translate.y)],
        particles: g
            .particles
            .iter()
            .map(|p| {
                let draw = particle_draw(g, p, clock);
                ParticleKey {
                    world: draw.world.to_array().map(spatial),
                    flipbook_frame: draw.flipbook_frame,
                    scale: [spatial(draw.scale.x), spatial(draw.scale.y)],
                    color: [
                        color(draw.rgb.x),
                        color(draw.rgb.y),
                        color(draw.rgb.z),
                        color(draw.vert_alpha),
                    ],
                }
            })
            .collect(),
    }
}

fn needs_rebuild(built: &MeshKey, next: &MeshKey) -> bool {
    built != next
}

fn rebuild_mesh(g: &LiveGenerator, rot: Quat, clock: &CelestialClock, mesh: &mut Mesh) {
    let verts_per = g.template.positions.len();
    let n = g.particles.len();
    let mut positions = Vec::with_capacity(n * verts_per);
    let mut uvs = Vec::with_capacity(n * verts_per);
    let mut colors = Vec::with_capacity(n * verts_per);
    let mut indices = Vec::with_capacity(n * g.template.indices.len());

    for p in &g.particles {
        let draw = particle_draw(g, p, clock);
        let tpl = flipbook_template(g, draw.flipbook_frame);

        // Billboard sprites are flat (z unused); a fixed-orientation 3D particle
        // mesh keeps its DAT depth axis scaled by the untracked init z-scale.
        let sz = if g.orientation.is_some() {
            g.def.init_scale[2]
        } else {
            1.0
        };
        // Fixed-orientation zone sheets carry raw FFXI-frame geometry; apply the
        // generator's FFXI->Bevy basis (the same flip on origin/velocity, matching
        // dat_mzb.rs to_bevy) so a falling water sheet hangs down into the basin
        // instead of standing up above the emitter (kuluu-czc6). Camera billboards
        // orient in Bevy already; actor-local generators integrate in the actor frame.
        let world_basis = g.orientation.is_some() && !g.actor_local;
        let base = positions.len() as u32;
        for (tp, uv) in tpl.positions.iter().zip(&tpl.uvs) {
            let local = Vec3::new(tp.x * draw.scale.x, tp.y * draw.scale.y, tp.z * sz);
            let oriented = rot * local;
            let oriented = if world_basis {
                oriented * g.vel_basis
            } else {
                oriented
            };
            positions.push((draw.world + oriented).to_array());
            uvs.push([uv[0] + g.tex_translate.x, uv[1] + g.tex_translate.y]);
            colors.push([draw.rgb.x, draw.rgb.y, draw.rgb.z, draw.vert_alpha]);
        }
        indices.extend(tpl.indices.iter().map(|&idx| base + idx));
    }

    if positions.is_empty() {
        push_hidden_primitive(&mut positions, &mut uvs, &mut colors, &mut indices);
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
}

// A generator with zero live particles (on spawn, and in the gaps between emit
// windows) would otherwise rebuild an empty mesh. Bevy's MeshAllocator skips the
// slab allocation for a zero-length vertex buffer but still runs the upload copy,
// logging "Use-after-free: attempted to copy element data for an unallocated key"
// (bevy_render slab_allocator.rs) every such frame. Keep the buffer non-empty with
// one zero-area, fully-transparent triangle so it uploads cleanly and draws nothing.
fn push_hidden_primitive(
    positions: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
) {
    let base = positions.len() as u32;
    for _ in 0..3 {
        positions.push([0.0, 0.0, 0.0]);
        uvs.push([0.0, 0.0]);
        colors.push([0.0, 0.0, 0.0, 0.0]);
    }
    indices.extend([base, base + 1, base + 2]);
}

fn sprite_template(d3m: &ffxi_dat::d3m::D3m) -> Option<SpriteTemplate> {
    if d3m.vertices.is_empty() {
        return None;
    }
    let positions = d3m
        .vertices
        .iter()
        .map(|v| Vec3::from_array(v.pos))
        .collect();
    let uvs = d3m.vertices.iter().map(|v| v.uv).collect();
    let indices = (0..d3m.vertices.len() as u32).collect();
    let c = d3m.vertices[0].color;
    Some(SpriteTemplate {
        positions,
        uvs,
        indices,
        brightness: Vec3::new(c[0], c[1], c[2]),
        vert_alpha: c[3],
    })
}

// None when the referenced mesh isn't present, which leaves zone callers to fall back to an
// MMB mesh.
fn resolve_mesh(
    assets: &ActionAssets,
    def: &ParticleGeneratorDef,
    images: &mut Assets<Image>,
) -> Option<(SpriteTemplate, Vec<SpriteTemplate>, Option<Handle<Image>>)> {
    match def.mesh_kind {
        ParticleMeshKind::StaticMesh => {
            let d3m = assets.d3ms.get(&def.mesh_id)?;
            let template = sprite_template(d3m)?;
            let (namespace, local) = d3m.texture_name_tokens();
            // research/xim DatResource.kt:488-493 — qualified (namespace, local) match, then
            // local-only. The truncated DatId stays as a last tier: a few meshes name a
            // texture whose local token outruns the Img chunk id (`kumori` vs `kumo`) and
            // resolve only that way.
            let by_name = (!local.is_empty()).then(|| {
                assets
                    .images_by_qualified_name
                    .get(&(namespace, local.clone()))
                    .or_else(|| assets.images_by_name.get(&local))
            });
            let tex = by_name
                .flatten()
                .or_else(|| assets.images.get(&d3m.texture_dat_id()))
                .map(|t| images.add(decoded_texture_to_image(t)));
            Some((template, Vec::new(), tex))
        }
        ParticleMeshKind::SpriteSheet => {
            let ss = assets.sprite_sheets.get(&def.mesh_id)?;
            let frames = sprite_sheet_templates(ss);
            let first = frames.first().cloned()?;
            // research/xim DatResource.kt:483-493 — try the qualified (namespace, local) pair
            // first, then fall back to a local-name-only match.
            let tex = assets
                .images_by_qualified_name
                .get(&(ss.category.clone(), ss.id.clone()))
                .or_else(|| assets.images_by_name.get(&ss.id))
                .map(|t| images.add(decoded_texture_to_image(t)));
            Some((first, frames, tex))
        }
    }
}

fn sprite_sheet_templates(ss: &ParticleSpriteSheet) -> Vec<SpriteTemplate> {
    ss.frames
        .iter()
        .filter_map(|f| {
            if f.positions.is_empty() {
                return None;
            }
            let c = f.colors[0];
            Some(SpriteTemplate {
                positions: f.positions.iter().map(|p| Vec3::from_array(*p)).collect(),
                uvs: f.uvs.clone(),
                indices: (0..f.positions.len() as u32).collect(),
                // FFXI vertex colors are 2x-overbright (see d3m.rs color parse); the venom-cloud
                // tint is then modulated by the generator's init_color in rebuild_mesh.
                brightness: Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32)
                    / ffxi_dat::d3m::VERTEX_COLOR_DIVISOR,
                vert_alpha: c[3] as f32 / ffxi_dat::d3m::VERTEX_COLOR_DIVISOR,
            })
        })
        .collect()
}

// research/xim Particle.kt:72 — the spriteSheetIndex advances the flipbook across the
// particle's lifetime. StaticMesh particles carry no frames and use the single template.
fn flipbook_index(g: &LiveGenerator, progress: f32) -> usize {
    let n = g.sprite_frames.len();
    if n == 0 {
        return 0;
    }
    ((progress * n as f32) as usize).min(n - 1)
}

fn flipbook_template(g: &LiveGenerator, idx: usize) -> &SpriteTemplate {
    g.sprite_frames.get(idx).unwrap_or(&g.template)
}

fn mmb_sprite_template(mmb: &MmbSpriteMesh) -> Option<SpriteTemplate> {
    if mmb.positions.is_empty() || mmb.indices.is_empty() {
        return None;
    }
    Some(SpriteTemplate {
        positions: mmb.positions.iter().map(|p| Vec3::from_array(*p)).collect(),
        uvs: mmb.uvs.clone(),
        indices: mmb.indices.clone(),
        brightness: Vec3::from_array(mmb.brightness),
        vert_alpha: mmb.vert_alpha,
    })
}

fn empty_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let (mut positions, mut uvs, mut colors, mut indices) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    push_hidden_primitive(&mut positions, &mut uvs, &mut colors, &mut indices);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffxi_dat::particle_gen::ParticleGeneratorDef;

    fn def(life: f32, fpe: f32, ppe: u32) -> ParticleGeneratorDef {
        ParticleGeneratorDef {
            frames_per_emission: fpe,
            particles_per_emission: ppe,
            emission_variance: 0.0,
            mesh_id: *b"gr  ",
            mesh_kind: ffxi_dat::particle_gen::ParticleMeshKind::StaticMesh,
            base_position: [0.0, 0.5, 0.0],
            max_life_frames: life,
            camera_billboard: true,
            continuous: false,
            auto_run: false,
            attach_type: ffxi_dat::particle_gen::AttachType::SourceActor,
            tod_color_tracks: [None; ffxi_dat::particle_gen::TOD_COLOR_CHANNELS],
            tod_color_driven: [false; ffxi_dat::particle_gen::TOD_COLOR_CHANNELS],
            moon_phase_sprite: false,
            attach_joint_source: 0,
            attach_joint_target: 0,
            attach_source_oriented: false,
            init_scale: [0.1, 0.1, 1.0],
            init_color: [0.2, 0.2, 0.6, 0.5],
            init_velocity: [0.0, 0.01, 0.0],
            init_rotation: [0.0; 3],
            blend: ffxi_dat::particle_gen::ParticleBlend::Additive,
            blend_byte: 0x48,
            ignore_texture_alpha: false,
            scale_x_track: None,
            scale_y_track: None,
            alpha_track: None,
            day_of_week_color: None,
            moon_phase_color: None,
            uv_scroll: [0.0, 0.0],
            accel: None,
        }
    }

    fn live(def: ParticleGeneratorDef, window: f32) -> LiveGenerator {
        LiveGenerator {
            def,
            template: SpriteTemplate {
                positions: vec![Vec3::ZERO; 3],
                uvs: vec![[0.0, 0.0]; 3],
                indices: vec![0, 1, 2],
                brightness: Vec3::ONE,
                vert_alpha: 1.0,
            },
            draw_path: D3mDrawPath::D3m,
            sprite_frames: Vec::new(),
            tod_color: [None, None, None, None],
            scale_x: None,
            scale_y: None,
            alpha: None,
            origin: Vec3::ZERO,
            particles: Vec::new(),
            emit_accum: 0.0,
            age_frames: 0.0,
            emit_window_frames: window,
            mesh: Handle::default(),
            entity: Entity::PLACEHOLDER,
            auto_run: false,
            orientation: None,
            actor_local: false,
            tex_translate: Vec2::ZERO,
            vel_basis: Vec3::ONE,
            origin_routine: None,
            stopped: false,
            built_key: MeshKey::Empty,
        }
    }

    // Drive the emission math directly (no Bevy world), one tick's worth of frames per call.
    fn advance(g: &mut LiveGenerator, frames: f32) {
        advance_generator(g, frames);
    }

    // A generator stage's duration is authored in 60 fps frames (research/xim util/Fps.kt:9),
    // so a 30-frame emit window is half a second of wall time, not a whole one.
    #[test]
    fn emit_window_is_duration_frames_at_60fps() {
        const WINDOW_FRAMES: f32 = 30.0;
        const TICK_SECS: f32 = 1.0 / 120.0;

        let mut g = live(def(600.0, 1.0, 1), WINDOW_FRAMES);
        let run_for = |g: &mut LiveGenerator, secs: f32| {
            let mut t = 0.0;
            while t < secs {
                advance(g, TICK_SECS * ROUTINE_FPS);
                t += TICK_SECS;
            }
            g.particles.len()
        };
        let after_window = run_for(&mut g, 0.55);
        let half_second_later = run_for(&mut g, 0.5);
        assert!(after_window > 0, "the generator emitted inside its window");
        assert_eq!(
            after_window, half_second_later,
            "emission stops half a second in, not a whole one"
        );
    }

    // research/XIClient/src/XIClient/source/Resource/Derived/CMoD3m.cpp:16-104. `brightness` and
    // `vert_alpha` already carry stage 0's MODULATE2X (the /128 normalise), so an input of 0.25
    // here stands for a retail D of 0.125.
    mod stage_chain {
        use super::*;

        // NonZeroTwoTSS: rgb = 4*D*T*F, alpha = 8*D.a*T.a*F.a, with T left to the sampler.
        #[test]
        fn textured_default_reaches_the_retail_totals_below_saturation() {
            let (rgb, alpha) =
                d3m_stage_chain(Vec3::splat(0.25), 0.25, Vec3::splat(0.25), 0.25, false);
            assert_eq!(rgb, Vec3::splat(4.0 * 0.125 * 0.25));
            assert_eq!(alpha, 8.0 * 0.125 * 0.25);
        }

        // NonZeroOneTSS (renderStateFlags 0x1000): stage 0 selects D.a instead of modulating it
        // with the texture alpha, so the total is 4*D.a*F.a — half the default, rgb untouched.
        #[test]
        fn ignoring_texture_alpha_halves_the_alpha_total() {
            let two = d3m_stage_chain(Vec3::splat(0.25), 0.25, Vec3::splat(0.25), 0.25, false);
            let one = d3m_stage_chain(Vec3::splat(0.25), 0.25, Vec3::splat(0.25), 0.25, true);
            assert_eq!(one.1, two.1 / 2.0);
            assert_eq!(one.0, two.0);
        }

        // D3D saturates each stage on its own: a 0xFF vertex byte clips at stage 0, so stage 1's
        // MODULATE4X starts from 1.0 instead of carrying the excess through it.
        #[test]
        fn stage_zero_saturates_before_the_stage_one_gain() {
            let vert = u8::MAX as f32 / ffxi_dat::d3m::VERTEX_COLOR_DIVISOR;
            let (rgb, alpha) = d3m_stage_chain(Vec3::splat(vert), vert, Vec3::ONE, 0.15, false);
            assert_eq!(alpha, 0.6);
            assert_eq!(rgb, Vec3::splat(D3M_STAGE_CLAMP));
        }

        #[test]
        fn a_full_brightness_particle_never_exceeds_the_stage_clamp() {
            let vert = u8::MAX as f32 / ffxi_dat::d3m::VERTEX_COLOR_DIVISOR;
            let (rgb, alpha) = d3m_stage_chain(Vec3::splat(vert), vert, Vec3::ONE, 1.0, false);
            assert_eq!(rgb, Vec3::splat(D3M_STAGE_CLAMP));
            assert_eq!(alpha, D3M_STAGE_CLAMP);
        }

        // CMoD3mElem.cpp:108-112 — DoMMBDraw forces the ignore-texture-alpha table at blend byte
        // 0x64; CMoD3m::Draw has no such override.
        #[test]
        fn blend_byte_64_forces_the_one_tss_table_on_the_mmb_path_only() {
            let mut d = def(1.0, 1.0, 1);
            d.blend_byte = D3M_MMB_FORCE_IGNORE_TEXTURE_ALPHA_BLEND_BYTE;
            assert!(ignores_texture_alpha(&d, D3mDrawPath::Mmb));
            assert!(!ignores_texture_alpha(&d, D3mDrawPath::D3m));
            d.blend_byte = 0x03;
            assert!(!ignores_texture_alpha(&d, D3mDrawPath::Mmb));
            d.ignore_texture_alpha = true;
            assert!(ignores_texture_alpha(&d, D3mDrawPath::D3m));
        }

        // CMoD3m.cpp:345-349 — blend byte 0x44 only, and only on the CMoD3m::Draw path.
        #[test]
        fn tfactor_alpha_promotes_at_half_only_for_blend_byte_44() {
            let promote = |byte: u8, path: D3mDrawPath, a: f32| {
                let mut d = def(1.0, 1.0, 1);
                d.blend_byte = byte;
                tfactor_alpha(&d, path, a)
            };
            let just_under = 0x7E as f32 / u8::MAX as f32;
            let at_threshold = 0x7F as f32 / u8::MAX as f32;
            assert_eq!(promote(0x44, D3mDrawPath::D3m, at_threshold), 1.0);
            assert_eq!(promote(0x44, D3mDrawPath::D3m, just_under), just_under);
            assert_eq!(promote(0x44, D3mDrawPath::Mmb, at_threshold), at_threshold);
            assert_eq!(promote(0x03, D3mDrawPath::D3m, at_threshold), at_threshold);
        }

        fn vertex_colors(g: &LiveGenerator) -> Vec<[f32; 4]> {
            let mut mesh = empty_mesh();
            rebuild_mesh(g, Quat::IDENTITY, &CelestialClock::default(), &mut mesh);
            match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
                Some(bevy::mesh::VertexAttributeValues::Float32x4(v)) => v.clone(),
                _ => panic!("expected Float32x4 vertex colours"),
            }
        }

        // One particle at half life, where the untracked alpha curve gives F.a = 0.5.
        fn half_life_gen(blend: ffxi_dat::particle_gen::ParticleBlend, byte: u8) -> LiveGenerator {
            let mut d = def(100.0, 1.0, 1);
            d.blend = blend;
            d.blend_byte = byte;
            d.init_color = [1.0, 1.0, 1.0, 1.0];
            let mut g = live(d, 100.0);
            g.template.vert_alpha = 0.5;
            g.particles.push(Particle {
                pos: Vec3::ZERO,
                vel: Vec3::ZERO,
                age_frames: 50.0,
                life_frames: 100.0,
                rgb: Vec3::ONE,
                scale: Vec2::ONE,
            });
            g
        }

        #[test]
        fn blended_particle_carries_the_stage_one_rgb_gain() {
            let mut g = half_life_gen(ffxi_dat::particle_gen::ParticleBlend::Blend, 0x03);
            g.template.brightness = Vec3::splat(0.25);
            for c in vertex_colors(&g) {
                assert_eq!([c[0], c[1], c[2]], [0.5, 0.5, 0.5]);
            }
        }

        #[test]
        fn blended_particle_alpha_scales_with_vertex_alpha() {
            let mut g = half_life_gen(ffxi_dat::particle_gen::ParticleBlend::Blend, 0x03);
            g.template.vert_alpha = 0.25;
            for c in vertex_colors(&g) {
                assert_eq!(c[3], 0.5);
            }
        }

        // The 0x44 promotion lifts F.a 0.5 -> 1.0 before the stage math.
        #[test]
        fn blend_byte_44_promotes_the_particle_alpha() {
            let mut g = half_life_gen(ffxi_dat::particle_gen::ParticleBlend::Blend, 0x44);
            g.template.vert_alpha = 0.125;
            let promoted = vertex_colors(&g)[0][3];
            g.def.blend_byte = 0x03;
            let unpromoted = vertex_colors(&g)[0][3];
            assert_eq!(promoted, 0.5);
            assert_eq!(unpromoted, 0.25);
        }

        #[test]
        fn additive_particle_folds_the_life_curve_into_rgb() {
            let mut g = half_life_gen(ffxi_dat::particle_gen::ParticleBlend::Additive, 0x48);
            g.template.brightness = Vec3::splat(0.25);
            for c in vertex_colors(&g) {
                assert_eq!([c[0], c[1], c[2]], [0.25, 0.25, 0.25]);
                assert_eq!(c[3], 1.0);
            }
        }

        // The fold is a brightness proxy, not a blend factor: the saturating stage-1 alpha would
        // hold an additive spray at full brightness until the last quarter of its life.
        #[test]
        fn additive_brightness_still_fades_late_in_life() {
            let mut g = half_life_gen(ffxi_dat::particle_gen::ParticleBlend::Additive, 0x48);
            g.particles[0].age_frames = 90.0;
            let late = vertex_colors(&g)[0][0];
            assert!((late - (1.0 - 0.9f32)).abs() < 1e-6, "{late}");
        }
    }

    #[test]
    fn mesh_is_never_zero_length() {
        // Bevy's MeshAllocator errors on a zero-length vertex buffer, so an
        // empty generator (fresh spawn / between emit windows) must still
        // upload a non-empty mesh. Covers empty_mesh() and the empty rebuild.
        let count = |m: &Mesh| m.count_vertices();
        assert!(
            count(&empty_mesh()) > 0,
            "empty_mesh must not be zero-length"
        );

        let g = live(def(2.0, 1.0, 1), 3.0);
        assert!(g.particles.is_empty());
        let mut mesh = empty_mesh();
        rebuild_mesh(&g, Quat::IDENTITY, &CelestialClock::default(), &mut mesh);
        assert!(count(&mesh) > 0, "empty rebuild must not be zero-length");
    }

    // kuluu-b5nt: rebuild_mesh only fires when its quantized inputs differ from the last BUILT
    // mesh, so a tracked get_mut (AssetEvent::Modified, a full GPU re-upload) stops scaling
    // with fps.
    mod rebuild_skip {
        use super::*;

        fn one_particle_gen() -> LiveGenerator {
            let mut g = live(def(100.0, 1.0, 1), 100.0);
            g.particles.push(Particle {
                pos: Vec3::new(1.0, 2.0, 3.0),
                vel: Vec3::ZERO,
                age_frames: 50.0,
                life_frames: 100.0,
                rgb: Vec3::ONE,
                scale: Vec2::ONE,
            });
            g
        }

        #[test]
        fn idle_generator_never_rebuilds_whatever_the_camera_does() {
            let mut g = live(def(100.0, 1.0, 1), 100.0);
            assert!(g.particles.is_empty());
            g.tex_translate = Vec2::new(3.7, -1.2);
            for rot in [
                Quat::IDENTITY,
                Quat::from_rotation_y(1.3),
                Quat::from_rotation_x(-0.4),
            ] {
                assert!(!needs_rebuild(
                    &g.built_key,
                    &mesh_key(&g, rot, &CelestialClock::default())
                ));
            }
        }

        #[test]
        fn sub_quantum_motion_skips() {
            let mut g = one_particle_gen();
            let built = mesh_key(&g, Quat::IDENTITY, &CelestialClock::default());
            g.particles[0].pos.x += MESH_KEY_SPATIAL_QUANTUM * 0.25;
            assert!(!needs_rebuild(
                &built,
                &mesh_key(&g, Quat::IDENTITY, &CelestialClock::default())
            ));
        }

        #[test]
        fn super_quantum_motion_rebuilds() {
            let mut g = one_particle_gen();
            let built = mesh_key(&g, Quat::IDENTITY, &CelestialClock::default());
            g.particles[0].pos.x += MESH_KEY_SPATIAL_QUANTUM * 2.0;
            assert!(needs_rebuild(
                &built,
                &mesh_key(&g, Quat::IDENTITY, &CelestialClock::default())
            ));
        }

        // Ageing feeds the untracked additive life curve through tfactor_alpha and the D3m
        // stage chain into the key's colour, so an alpha change alone dirties the mesh.
        #[test]
        fn alpha_stage_change_rebuilds() {
            let mut g = one_particle_gen();
            let built = mesh_key(&g, Quat::IDENTITY, &CelestialClock::default());
            g.particles[0].age_frames = 90.0;
            assert!(needs_rebuild(
                &built,
                &mesh_key(&g, Quat::IDENTITY, &CelestialClock::default())
            ));
        }

        #[test]
        fn camera_rotation_rebuilds_a_live_billboard() {
            let g = one_particle_gen();
            let built = mesh_key(&g, Quat::IDENTITY, &CelestialClock::default());
            assert!(needs_rebuild(
                &built,
                &mesh_key(&g, Quat::from_rotation_y(0.5), &CelestialClock::default())
            ));
        }

        #[test]
        fn uv_scroll_change_rebuilds() {
            let mut g = one_particle_gen();
            let built = mesh_key(&g, Quat::IDENTITY, &CelestialClock::default());
            g.tex_translate.x += MESH_KEY_SPATIAL_QUANTUM * 2.0;
            assert!(needs_rebuild(
                &built,
                &mesh_key(&g, Quat::IDENTITY, &CelestialClock::default())
            ));
        }
    }

    // kuluu-czc6: a fixed-orientation zone sheet (e.g. the Lower Jeuno fountain
    // "sibj" cascade) carries raw FFXI-frame geometry extending local +Y (FFXI
    // down). rebuild_mesh must flip it through the generator's mzb->bevy vel_basis
    // so the sheet hangs DOWN from the emitter (Bevy -Y), not up above it. A camera
    // billboard (orientation None) must NOT be flipped — it orients in Bevy already.
    fn sheet_gen(orientation: Option<Quat>) -> LiveGenerator {
        let mut d = def(100.0, 1.0, 1);
        d.camera_billboard = orientation.is_none();
        d.init_scale = [1.0, 1.0, 1.0];
        let mut g = live(d, 5.0);
        // Flat quad extending local +Y (FFXI down), like the sibj water sheet.
        g.template.positions = vec![
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 4.0, 0.0),
        ];
        g.template.uvs = vec![[0.0, 0.0]; 3];
        g.template.indices = vec![0, 1, 2];
        g.origin = Vec3::new(0.0, 10.0, 0.0);
        g.orientation = orientation;
        g.actor_local = false;
        g.vel_basis = Vec3::new(1.0, -1.0, -1.0);
        emit(&mut g, 100.0);
        g
    }

    fn max_sheet_y(mesh: &Mesh) -> f32 {
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        // Ignore the far-below hidden primitive push_hidden_primitive leaves when needed.
        pos.iter()
            .map(|p| p[1])
            .filter(|y| *y > -1.0e6)
            .fold(f32::MIN, f32::max)
    }

    #[test]
    fn fixed_orientation_sheet_hangs_below_emitter() {
        let g = sheet_gen(Some(Quat::IDENTITY));
        let mut mesh = empty_mesh();
        rebuild_mesh(&g, Quat::IDENTITY, &CelestialClock::default(), &mut mesh);
        // Local +Y (0..4) flipped through vel_basis -> Bevy -Y, so every sheet vertex
        // sits at or below the emit origin (y=10); none stand above it.
        assert!(
            max_sheet_y(&mesh) <= 10.0 + 1.0e-4,
            "fixed sheet vertices must not rise above the emitter (kuluu-czc6)"
        );
    }

    #[test]
    fn camera_billboard_sheet_not_flipped() {
        let g = sheet_gen(None);
        let mut mesh = empty_mesh();
        rebuild_mesh(&g, Quat::IDENTITY, &CelestialClock::default(), &mut mesh);
        // Billboard: no basis flip, so the same +Y geometry rises above the emitter.
        assert!(
            max_sheet_y(&mesh) > 10.0 + 1.0,
            "camera billboards must keep their unflipped local frame"
        );
    }

    #[test]
    fn emits_one_per_period_over_window() {
        let mut g = live(def(100.0, 5.0, 1), 20.0);
        // 20 frames at 1/frame, period 5 -> 4 emits within window (the emit at accum reset).
        for _ in 0..20 {
            advance(&mut g, 1.0);
        }
        assert_eq!(g.particles.len(), 4);
    }

    #[test]
    fn stops_emitting_after_window() {
        let mut g = live(def(2.0, 1.0, 1), 3.0);
        for _ in 0..10 {
            advance(&mut g, 1.0);
        }
        // window 3 -> ~3 emitted, each lives 2 frames, all expired by frame 10.
        assert!(g.particles.is_empty());
    }

    // research/xim EffectRoutineParser.kt:253-258 StopParticleGeneratorRoutine: the cast aura's
    // authored emit window is 1800 frames (60 s), so retail's 0x2D stop is what ends it at the
    // end of the cast — emission ceases at once, live particles still play out their life.
    #[test]
    fn stopped_generator_ceases_emission_but_keeps_live_particles() {
        const LIFE_FRAMES: f32 = 10.0;
        const LONG_WINDOW_FRAMES: f32 = 1800.0;

        let mut sim = ParticleSimulator::default();
        let owner = Entity::from_raw_u32(7).unwrap();
        let mut g = live(def(LIFE_FRAMES, 1.0, 1), LONG_WINDOW_FRAMES);
        g.origin_routine = Some(RoutineOrigin {
            owner,
            gen_id: *b"gn10",
            routine: *b"cabk",
        });
        sim.generators.push(g);

        for _ in 0..5 {
            advance_generator(&mut sim.generators[0], 1.0);
        }
        let live_at_stop = sim.generators[0].particles.len();
        assert!(live_at_stop > 0, "generator emits inside its window");

        sim.stop_generator(owner, *b"gn10");
        advance_generator(&mut sim.generators[0], 1.0);
        assert_eq!(
            sim.generators[0].particles.len(),
            live_at_stop,
            "a stopped generator emits nothing new"
        );
        assert!(
            sim.generators[0].particles[0].age_frames > 0.0,
            "already-live particles keep ageing"
        );

        for _ in 0..LIFE_FRAMES as u32 {
            advance_generator(&mut sim.generators[0], 1.0);
        }
        assert!(
            sim.generators[0].particles.is_empty(),
            "live particles finish their lifetime and none replace them"
        );
    }

    // The cast aura's own generators sit on dur=0 Particle stages (global-dir `ner1`: gn1s dur=0;
    // `eis3`: ge3s/ge31 dur=0), giving a 1-frame emit window, and the frame that spawns them
    // carries a blocking action-DAT read. A singleton must still fire on its first tick however
    // long that frame ran, or the aura never appears at all.
    #[test]
    fn singleton_emits_on_a_first_frame_longer_than_its_emit_window() {
        const SINGLETON_LIFE: f32 = 0.0;
        const ZERO_DURATION_WINDOW: f32 = 0.0;
        const LONG_FRAME: f32 = 9.0;

        let mut g = live(def(SINGLETON_LIFE, 1.0, 1), ZERO_DURATION_WINDOW);
        assert!(g.def.is_singleton());
        advance(&mut g, LONG_FRAME);
        assert_eq!(
            g.particles.len(),
            1,
            "a long spawn frame must not swallow the singleton's only emission"
        );

        advance(&mut g, LONG_FRAME);
        assert!(
            g.particles.is_empty(),
            "it lives out its window and is not re-emitted"
        );
    }

    // research/xim ParticleInitializers.kt:130-131 — maxLifeSpan 0 means POSITIVE_INFINITY
    // for the auto-run zone billboards ("the sea and such"): the sun, the moon and the sea
    // must stand for as long as the zone does. The counterpart above pins that a SCHEDULED
    // dur=0 singleton still expires, so the two populations cannot be collapsed.
    #[test]
    fn auto_run_singleton_is_the_persistent_kind() {
        let mut g = live(def(0.0, 1.0, 1), 0.0);
        g.auto_run = true;
        assert!(g.def.is_singleton());

        advance(&mut g, 9.0);
        assert_eq!(g.particles.len(), 1);
        assert!(g.particles[0].life_frames.is_infinite());

        // Whatever the elapsed time, it neither expires nor re-emits.
        for _ in 0..100 {
            advance(&mut g, 60.0);
        }
        assert_eq!(
            g.particles.len(),
            1,
            "the zone billboard neither expires nor duplicates"
        );
        // An infinite life pins life progress at 0, which is what keeps a keyframe-tracked
        // channel on the curve's opening value instead of racing to its end.
        let draw = particle_draw(&g, &g.particles[0], &CelestialClock::default());
        assert!(draw.rgb.is_finite(), "infinite life must not poison the draw");
    }

    #[test]
    fn stopped_singleton_never_emits() {
        let mut g = live(def(0.0, 1.0, 1), 0.0);
        g.stopped = true;
        advance(&mut g, 9.0);
        assert!(g.particles.is_empty());
    }

    #[test]
    fn stop_routine_ends_every_generator_the_routine_spawned() {
        let mut sim = ParticleSimulator::default();
        let owner = Entity::from_raw_u32(7).unwrap();
        let other = Entity::from_raw_u32(8).unwrap();
        for (o, gen_id) in [(owner, b"gn10"), (owner, b"gn11"), (other, b"gn12")] {
            let mut g = live(def(4.0, 1.0, 1), 600.0);
            g.origin_routine = Some(RoutineOrigin {
                owner: o,
                gen_id: *gen_id,
                routine: *b"cabk",
            });
            sim.generators.push(g);
        }
        sim.generators.push(live(def(4.0, 1.0, 1), 600.0));

        sim.stop_routine(owner, *b"cabk");
        let stopped: Vec<bool> = sim.generators.iter().map(|g| g.stopped).collect();
        assert_eq!(stopped, vec![true, true, false, false]);

        sim.stop_generators_of_dead_owners(|e| e == owner);
        let stopped: Vec<bool> = sim.generators.iter().map(|g| g.stopped).collect();
        assert_eq!(
            stopped,
            vec![true, true, true, false],
            "a despawned caster's aura stops; a zone/auto-run generator is untouched"
        );
    }

    #[test]
    fn singleton_emits_once() {
        let mut g = live(def(0.0, 1.0, 1), 30.0);
        for _ in 0..5 {
            advance(&mut g, 1.0);
        }
        assert_eq!(g.particles.len(), 1, "singleton emits exactly once");
        assert!(g.particles[0].pos.y > 0.0, "velocity integrated");
    }

    #[test]
    fn auto_run_keeps_emitting_past_window() {
        let mut g = live(def(2.0, 1.0, 1), 3.0);
        g.auto_run = true;
        for _ in 0..30 {
            advance(&mut g, 1.0);
        }
        assert!(
            !g.particles.is_empty(),
            "auto-run generators never stop emitting"
        );
    }

    // A celestial billboard: continuous singleton, additive, one live particle whose colour
    // is what the sun/moon opcodes drive.
    fn celestial(def: ParticleGeneratorDef) -> LiveGenerator {
        let mut g = live(def, 1.0);
        g.auto_run = true;
        g.particles.push(Particle {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age_frames: 0.0,
            life_frames: 1.0,
            rgb: Vec3::from_slice(&g.def.init_color[..3]),
            scale: Vec2::ONE,
        });
        g
    }

    fn ramp(from: f32, to: f32) -> KeyFrameTrack {
        KeyFrameTrack {
            points: vec![(0.0, from), (1.0, to)],
        }
    }

    // research/xim ParticleGeneratorParser.kt:431-434 — the ClockValueUpdater curves are
    // sampled at the Vana'diel day fraction, so a celestial particle's colour tracks the
    // clock, NOT its own life progress. This is the sun's authored dawn/noon/dusk ramp;
    // sampling it by life would freeze the disc at the curve's opening value forever, since
    // a continuous singleton is re-emitted at progress 0 every frame.
    #[test]
    fn time_of_day_curves_sample_the_clock_not_particle_life() {
        let mut def = def(1.0, 1.0, 1);
        def.blend = ffxi_dat::particle_gen::ParticleBlend::Blend;
        def.init_color = [1.0, 1.0, 1.0, 1.0];
        def.tod_color_driven = [true, false, false, false];
        let mut g = celestial(def);
        g.tod_color[0] = Some(ramp(0.0, 1.0));

        // The particle never ages (life_frames == 1, age 0), so any change here is the clock.
        let at = |day_fraction: f32| {
            particle_draw(
                &g,
                &g.particles[0],
                &CelestialClock {
                    day_fraction,
                    ..Default::default()
                },
            )
            .rgb
            .x
        };
        let (dawn, dusk) = (at(0.25), at(0.75));
        assert!(
            dusk > dawn,
            "red channel must follow the day fraction: {dawn} -> {dusk}"
        );
    }

    // research/xim Particle.kt:217-218 — day-of-week first, then moon phase, each a 2x
    // modulate that saturates at 1. Order matters because the modulate clamps: applying the
    // brighter table second cannot recover what the first one crushed.
    #[test]
    fn celestial_tints_apply_day_of_week_then_moon_phase_at_2x() {
        let mut def = def(1.0, 1.0, 1);
        def.blend = ffxi_dat::particle_gen::ParticleBlend::Blend;
        // Low enough that the D3M stage-1 2x gain does not saturate the channel and hide
        // the tint (a 0.5 base already clamps to 1.0 untinted).
        def.init_color = [0.2, 0.2, 0.2, 1.0];
        // A 2x modulate makes 0.5 the identity entry, so 0.25 is the one that halves.
        // Weekday 3 halves red, phase 6 halves it again: 0.2 * 0.5 * 0.5 = 0.05.
        def.day_of_week_color = Some(halves_red_at(3));
        def.moon_phase_color = Some(halves_red_at(6));
        let g = celestial(def);
        let clock = CelestialClock {
            day_fraction: 0.5,
            day_of_week: 3,
            moon_phase: 6,
        };
        let untinted = celestial(blended_celestial_def());
        let plain = particle_draw(&untinted, &untinted.particles[0], &clock)
            .rgb
            .x;
        let tinted = particle_draw(&g, &g.particles[0], &clock).rgb.x;
        assert!(
            (tinted - plain * 0.25).abs() < 1e-5,
            "two halving tables at 2x modulate should quarter the channel: {tinted} vs {plain}"
        );
    }

    // A tint table that is the identity everywhere except `target`, where it halves red.
    fn halves_red_at<const N: usize>(target: usize) -> [[f32; 4]; N] {
        std::array::from_fn(|i| {
            let red = if i == target { 0.25 } else { 0.5 };
            [red, 0.5, 0.5, 1.0]
        })
    }

    fn blended_celestial_def() -> ParticleGeneratorDef {
        let mut def = def(1.0, 1.0, 1);
        def.blend = ffxi_dat::particle_gen::ParticleBlend::Blend;
        def.init_color = [0.2, 0.2, 0.2, 1.0];
        def
    }

    // research/xim ParticleGeneratorParser.kt:444 MoonPhaseSpriteSheetUpdater — the moon's
    // sheet frame is the phase index, so it must NOT flipbook over the particle's life the
    // way every other sprite-sheet particle does.
    #[test]
    fn moon_phase_pins_the_sprite_frame() {
        let mut def = def(1.0, 1.0, 1);
        def.moon_phase_sprite = true;
        let mut g = celestial(def);
        g.sprite_frames = (0..ffxi_dat::particle_gen::MOON_PHASES)
            .map(|_| g.template.clone())
            .collect();

        for phase in 0..ffxi_dat::particle_gen::MOON_PHASES {
            let draw = particle_draw(
                &g,
                &g.particles[0],
                &CelestialClock {
                    moon_phase: phase,
                    ..Default::default()
                },
            );
            assert_eq!(draw.flipbook_frame, phase);
        }

        // Out-of-range phases clamp instead of indexing past the sheet.
        let draw = particle_draw(
            &g,
            &g.particles[0],
            &CelestialClock {
                moon_phase: 99,
                ..Default::default()
            },
        );
        assert_eq!(draw.flipbook_frame, ffxi_dat::particle_gen::MOON_PHASES - 1);
    }

    #[test]
    fn continuous_singleton_holds_one_particle_and_replaces_on_expiry() {
        let mut d = def(4.0, 1.0, 3);
        d.continuous = true;
        let mut g = live(d, 1.0);
        g.auto_run = true;
        let mut max_alive = 0usize;
        let mut empty_streak = 0usize;
        let mut max_empty_streak = 0usize;
        for _ in 0..20 {
            advance(&mut g, 1.0);
            max_alive = max_alive.max(g.particles.len());
            if g.particles.is_empty() {
                empty_streak += 1;
                max_empty_streak = max_empty_streak.max(empty_streak);
            } else {
                empty_streak = 0;
            }
        }
        assert_eq!(
            max_alive, 1,
            "continuous singleton caps at one live particle"
        );
        assert_eq!(
            max_empty_streak, 0,
            "a continuous generator is never empty at render — the expired particle \
             is replaced the same tick, so the body never blinks out for a frame"
        );
    }

    #[test]
    fn continuous_trackless_generator_holds_constant_alpha() {
        // A continuous generator holds one particle re-emitted on expiry (the
        // steady crystal body). Track-less, it must stay fully opaque — if it fell
        // back to the 1.0-progress spray fade, the single particle would fade out
        // each cycle and strobe the whole model transparent.
        use ffxi_dat::particle_gen::ParticleBlend;
        let mut base = def(4.0, 1.0, 1);
        base.blend = ParticleBlend::Blend;
        base.init_color = [1.0, 1.0, 1.0, 0.8];

        // Vertex alpha well under the D3m stage clamp, so the two curves stay distinguishable
        // after the 4x TEXTUREFACTOR alpha gain instead of both saturating at 1.
        const VERT_ALPHA: f32 = 0.125;
        let mut cont = live(base, 1.0);
        cont.def.continuous = true;
        cont.template.vert_alpha = VERT_ALPHA;
        let mut spray = live(base, 1.0);
        spray.template.vert_alpha = VERT_ALPHA;

        let particle = |age: f32| Particle {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age_frames: age,
            life_frames: 4.0,
            rgb: Vec3::ONE,
            scale: Vec2::splat(0.1),
        };
        cont.particles = vec![particle(3.0)];
        spray.particles = vec![particle(3.0)];

        let alpha_of = |g: &LiveGenerator| -> f32 {
            let mut mesh = empty_mesh();
            rebuild_mesh(g, Quat::IDENTITY, &CelestialClock::default(), &mut mesh);
            match mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap() {
                bevy::mesh::VertexAttributeValues::Float32x4(c) => c[0][3],
                _ => panic!("expected Float32x4 colours"),
            }
        };

        let expected = |curve: f32| VERT_ALPHA * curve * D3M_STAGE1_ALPHA_GAIN;
        assert!(
            (alpha_of(&cont) - expected(1.0)).abs() < 1e-4,
            "continuous body stays fully opaque, not the life fade"
        );
        assert!(
            (alpha_of(&spray) - expected(0.25)).abs() < 1e-4,
            "a transient spray still fades 1.0-progress over life"
        );
    }

    #[test]
    fn particle_expires_at_life() {
        let mut g = live(def(3.0, 1.0, 1), 1.0);
        advance(&mut g, 1.0); // emit one at age 0
        assert_eq!(g.particles.len(), 1);
        advance(&mut g, 5.0); // past life
        assert!(g.particles.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod sheet_texture {
        use super::*;
        use ffxi_dat::sprite_sheet::{ParticleSpriteSheet, SpriteFrame};
        use ffxi_dat::texture::{DecodedTexture, TexFormat};

        const SHEET_ID: [u8; 4] = *b"fir ";
        const CATEGORY: &str = "venom1";
        const LOCAL: &str = "fir";

        fn one_pixel() -> DecodedTexture {
            DecodedTexture {
                width: 1,
                height: 1,
                format_tag: TexFormat::Bgra32,
                rgba: vec![255, 255, 255, 255],
            }
        }

        fn sheet_assets(qualified: bool, local: bool, namespace_only: bool) -> ActionAssets {
            let mut assets = ActionAssets::default();
            assets.sprite_sheets.insert(
                SHEET_ID,
                ParticleSpriteSheet {
                    frames: vec![SpriteFrame {
                        positions: vec![[0.0; 3]; 3],
                        uvs: vec![[0.0, 0.0]; 3],
                        colors: vec![[128, 128, 128, 128]; 3],
                    }],
                    category: CATEGORY.to_string(),
                    id: LOCAL.to_string(),
                },
            );
            if qualified {
                assets
                    .images_by_qualified_name
                    .insert((CATEGORY.to_string(), LOCAL.to_string()), one_pixel());
            }
            if local {
                assets.images_by_name.insert(LOCAL.to_string(), one_pixel());
            }
            if namespace_only {
                assets
                    .images_by_name
                    .insert(CATEGORY.to_string(), one_pixel());
            }
            assets
        }

        fn sheet_def() -> ParticleGeneratorDef {
            let mut d = def(30.0, 1.0, 1);
            d.mesh_id = SHEET_ID;
            d.mesh_kind = ffxi_dat::particle_gen::ParticleMeshKind::SpriteSheet;
            d
        }

        fn resolved_texture(assets: &ActionAssets) -> Option<Handle<Image>> {
            let mut images = Assets::<Image>::default();
            resolve_mesh(assets, &sheet_def(), &mut images)
                .expect("sheet mesh resolves")
                .2
        }

        // research/xim DatResource.kt:483-493 — qualified (namespace, local) match first.
        #[test]
        fn sprite_sheet_texture_resolves_by_qualified_name() {
            assert!(resolved_texture(&sheet_assets(true, false, false)).is_some());
        }

        #[test]
        fn sprite_sheet_texture_falls_back_to_local_name() {
            assert!(resolved_texture(&sheet_assets(false, true, false)).is_some());
        }

        // The kuluu-7jpq regression: the Img was only ever looked up under the sheet's
        // NAMESPACE token, which is not how any tier resolves, so the cloud drew untextured.
        #[test]
        fn sprite_sheet_texture_does_not_resolve_by_namespace_alone() {
            assert!(resolved_texture(&sheet_assets(false, false, true)).is_none());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod static_mesh_texture {
        use super::*;
        use ffxi_dat::texture::{DecodedTexture, TexFormat};

        const MESH_ID: [u8; 4] = *b"pou1";
        // ROM/97/59.DAT (`ele_ice`): the d3m names texture `pou`, whose backing Img chunk id is
        // `pou1`, so the truncated-DatId key and the name key disagree.
        const QUALIFIED: &[u8; 16] = b"ele_ice pou     ";
        const NAMESPACE: &str = "ele_ice";
        const LOCAL: &str = "pou";
        const IMG_DAT_ID: [u8; 4] = *b"pou1";

        fn one_pixel() -> DecodedTexture {
            DecodedTexture {
                width: 1,
                height: 1,
                format_tag: TexFormat::Bgra32,
                rgba: vec![255, 255, 255, 255],
            }
        }

        fn mesh_assets(qualified: bool, local: bool, dat_id: bool) -> ActionAssets {
            let mut assets = ActionAssets::default();
            let mut texture_name = [0u8; 16];
            texture_name.copy_from_slice(QUALIFIED);
            assets.d3ms.insert(
                MESH_ID,
                ffxi_dat::d3m::D3m {
                    name: MESH_ID,
                    num_triangles: 1,
                    texture_name,
                    vertices: vec![
                        ffxi_dat::d3m::D3mVertex {
                            pos: [0.0; 3],
                            normal: [0.0, 1.0, 0.0],
                            color: [1.0; 4],
                            uv: [0.0, 0.0],
                        };
                        3
                    ],
                },
            );
            if qualified {
                assets
                    .images_by_qualified_name
                    .insert((NAMESPACE.to_string(), LOCAL.to_string()), one_pixel());
            }
            if local {
                assets.images_by_name.insert(LOCAL.to_string(), one_pixel());
            }
            if dat_id {
                assets.images.insert(IMG_DAT_ID, one_pixel());
            }
            assets
        }

        fn mesh_def() -> ParticleGeneratorDef {
            let mut d = def(30.0, 1.0, 1);
            d.mesh_id = MESH_ID;
            d.mesh_kind = ffxi_dat::particle_gen::ParticleMeshKind::StaticMesh;
            d
        }

        fn resolved_texture(assets: &ActionAssets) -> Option<Handle<Image>> {
            let mut images = Assets::<Image>::default();
            resolve_mesh(assets, &mesh_def(), &mut images)
                .expect("static mesh resolves")
                .2
        }

        // research/xim DatResource.kt:488-493 — qualified (namespace, local) match first.
        #[test]
        fn static_mesh_texture_resolves_by_qualified_name() {
            assert!(resolved_texture(&mesh_assets(true, false, false)).is_some());
        }

        #[test]
        fn static_mesh_texture_falls_back_to_local_name() {
            assert!(resolved_texture(&mesh_assets(false, true, false)).is_some());
        }

        // The bug: `pou` truncated to the 4-byte key `pou ` never matched the `pou1` chunk id,
        // so the ice mesh drew untextured even though its Img was loaded.
        #[test]
        fn static_mesh_texture_does_not_need_the_name_to_equal_the_chunk_dat_id() {
            assert!(resolved_texture(&mesh_assets(false, false, true)).is_none());
            assert!(resolved_texture(&mesh_assets(true, false, true)).is_some());
        }

        // ROM file 173 (`cld1`/`clo1`, `kumori`) only ever resolves through the truncated id.
        #[test]
        fn static_mesh_texture_keeps_the_dat_id_as_a_last_tier() {
            let mut assets = mesh_assets(false, false, false);
            let mut texture_name = [0u8; 16];
            texture_name.copy_from_slice(b"cld1    kumori  ");
            assets.d3ms.get_mut(&MESH_ID).unwrap().texture_name = texture_name;
            assets.images.insert(*b"kumo", one_pixel());
            assert!(resolved_texture(&assets).is_some());
        }

        #[test]
        fn static_mesh_texture_is_none_when_no_tier_matches() {
            assert!(resolved_texture(&mesh_assets(false, false, false)).is_none());
        }

        // A mesh that names no texture must not claim the blank key: 44 d3ms in this install
        // carry an all-blank qualified name, and a single blank-keyed Img would give every one
        // of them the same wrong texture.
        #[test]
        fn static_mesh_texture_ignores_the_name_tiers_when_the_name_is_blank() {
            let mut assets = mesh_assets(false, false, false);
            assets.d3ms.get_mut(&MESH_ID).unwrap().texture_name = [b' '; 16];
            assets
                .images_by_qualified_name
                .insert((String::new(), String::new()), one_pixel());
            assets.images_by_name.insert(String::new(), one_pixel());

            assert!(resolved_texture(&assets).is_none());
        }

        fn retail_assets(file_id: u32) -> Option<ActionAssets> {
            let root = ffxi_dat::archive::open_test_install()?;
            let loc = match root.resolve(file_id) {
                Ok(loc) => loc,
                Err(err) => {
                    eprintln!("skipping: file {file_id} is not in this install ({err})");
                    return None;
                }
            };
            let path = loc.path_under(root.root());
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("skipping: {} unreadable ({err})", path.display());
                    return None;
                }
            };
            Some(crate::scheduler_runtime::parse_action_bytes(&bytes).1)
        }

        fn texture_for(assets: &ActionAssets, def: &ParticleGeneratorDef) -> Option<Handle<Image>> {
            let mut images = Assets::<Image>::default();
            resolve_mesh(assets, def, &mut images)
                .expect("mesh resolves")
                .2
        }

        // ROM/97/59.DAT `ele_ice`: the d3m names texture `pou` while the Img chunk id is `pou1`,
        // so the truncated-DatId key left the ice mesh untextured.
        #[test]
        fn real_dat_static_mesh_resolves_a_texture_its_chunk_id_does_not_name() {
            const ELE_ICE_FILE_ID: u32 = 1309;
            let Some(assets) = retail_assets(ELE_ICE_FILE_ID) else {
                return;
            };
            let d3m = assets.d3ms.get(&MESH_ID).expect("ele_ice ships a pou1 d3m");
            assert_eq!(
                d3m.texture_name_tokens(),
                (NAMESPACE.to_string(), LOCAL.to_string())
            );
            assert!(!assets.images.contains_key(&d3m.texture_dat_id()));

            let mut def = mesh_def();
            def.mesh_id = MESH_ID;
            assert!(texture_for(&assets, &def).is_some());
        }

        // ROM3/0/0.DAT: sheet `lf01` is backed by a palettised 0xB1 Img, which never entered the
        // name-keyed maps while extract_texture_tokens accepted 0xA1 alone. The sheet tier has no
        // DatId fallback, so the leaf drew untextured.
        #[test]
        fn real_dat_sprite_sheet_resolves_a_palettised_texture() {
            const ENVIRONMENT_FILE_ID: u32 = 101;
            const LEAF_SHEET_ID: [u8; 4] = *b"lf01";
            let Some(assets) = retail_assets(ENVIRONMENT_FILE_ID) else {
                return;
            };
            let sheet = assets
                .sprite_sheets
                .get(&LEAF_SHEET_ID)
                .expect("environment dat ships an lf01 sheet");
            assert!(assets
                .images_by_qualified_name
                .contains_key(&(sheet.category.clone(), sheet.id.clone())));

            let mut def = mesh_def();
            def.mesh_id = LEAF_SHEET_ID;
            def.mesh_kind = ffxi_dat::particle_gen::ParticleMeshKind::SpriteSheet;
            assert!(texture_for(&assets, &def).is_some());
        }
    }
}
