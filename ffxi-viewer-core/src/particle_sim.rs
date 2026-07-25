use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use ffxi_dat::particle_gen::{KeyFrameTrack, ParticleGeneratorDef, ParticleMeshKind};
use ffxi_dat::sprite_sheet::ParticleSpriteSheet;

use crate::camera::OperatorCamera;
use crate::components::InGameEntity;
use crate::dat_d3m::{d3m_material, decoded_texture_to_image, D3mBlendMode};
use crate::scheduler_runtime::{
    assets_holding, ActionAssets, GlobalEffectDir, MmbSpriteMesh, SchedulerStageEvent, FFXI_FPS,
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
}

impl ParticleSimulator {
    pub fn drain_entities(&mut self) -> Vec<Entity> {
        self.generators.drain(..).map(|g| g.entity).collect()
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
}

struct LiveGenerator {
    def: ParticleGeneratorDef,
    template: SpriteTemplate,
    // SpriteSheet (0x0E) flipbook frames; empty for a StaticMesh (0x0B) generator. When
    // non-empty each particle picks a frame by life progress in rebuild_mesh (research/xim
    // Particle.kt:72 spriteSheetIndex advanced over life).
    sprite_frames: Vec<SpriteTemplate>,
    scale_x: Option<KeyFrameTrack>,
    scale_y: Option<KeyFrameTrack>,
    alpha: Option<KeyFrameTrack>,
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
        let Some(assets) = assets_holding(local_assets, global.as_ref().map(|g| &g.assets), |a| {
            a.particle_defs.contains_key(&ev.stage.stage.id)
        }) else {
            continue;
        };
        let Some(def) = assets.particle_defs.get(&ev.stage.stage.id).copied() else {
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
            template,
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
                template,
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
    let (template, sprite_frames, tex) = if let Some(triple) = resolve_mesh(assets, &def, images) {
        triple
    } else {
        let mmb = assets.mmbs.get(&def.mesh_id)?;
        let template = mmb_sprite_template(mmb)?;
        let tex = assets
            .images_by_name
            .get(&mmb.texture_name)
            .map(|t| images.add(decoded_texture_to_image(t)));
        (template, Vec::new(), tex)
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
        template,
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
    let frames = time.delta_secs() * FFXI_FPS;
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
            emit(g, g.emit_window_frames.max(g.def.max_life_frames).max(1.0));
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

    // (index, despawn-needed); indices ascending so the reverse sweep below can
    // swap_remove safely.
    let mut reap: Vec<(usize, bool)> = Vec::new();
    for (i, g) in sim.generators.iter().enumerate() {
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
        if let Some(mut mesh) = meshes.get_mut(&g.mesh) {
            rebuild_mesh(g, rot, &mut mesh);
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

fn rebuild_mesh(g: &LiveGenerator, rot: Quat, mesh: &mut Mesh) {
    let verts_per = g.template.positions.len();
    let n = g.particles.len();
    let mut positions = Vec::with_capacity(n * verts_per);
    let mut uvs = Vec::with_capacity(n * verts_per);
    let mut colors = Vec::with_capacity(n * verts_per);
    let mut indices = Vec::with_capacity(n * g.template.indices.len());

    for p in &g.particles {
        let progress = (p.age_frames / p.life_frames).clamp(0.0, 1.0);
        // A SpriteSheet particle flipbooks its frames over life (research/xim Particle.kt:72
        // spriteSheetIndex); a StaticMesh particle keeps its single template.
        let tpl = flipbook_frame(g, progress);
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
        // Additive/subtract ignore the alpha channel, so the alpha curve modulates brightness;
        // alpha-blended particles keep full-brightness colour and use the alpha channel.
        let (rgb, vert_a) = match g.def.blend {
            ffxi_dat::particle_gen::ParticleBlend::Blend => (tpl.brightness * p.rgb, alpha),
            _ => (tpl.brightness * p.rgb * alpha, 1.0),
        };
        let world = g.origin + p.pos;

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
            let local = Vec3::new(tp.x * sx, tp.y * sy, tp.z * sz);
            let oriented = rot * local;
            let oriented = if world_basis {
                oriented * g.vel_basis
            } else {
                oriented
            };
            positions.push((world + oriented).to_array());
            uvs.push([uv[0] + g.tex_translate.x, uv[1] + g.tex_translate.y]);
            colors.push([rgb.x, rgb.y, rgb.z, vert_a]);
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
    })
}

// Resolve a generator's mesh_id to (frame-0 template, flipbook frames, texture). A StaticMesh
// (0x0B) def binds a D3M and has no flipbook frames; a SpriteSheet (0x0E) def binds a 0x21
// sheet whose texture resolves by qualified name with a local-name fallback. Returns None when
// the referenced mesh isn't present (leaving zone callers to fall back to an MMB mesh).
fn resolve_mesh(
    assets: &ActionAssets,
    def: &ParticleGeneratorDef,
    images: &mut Assets<Image>,
) -> Option<(SpriteTemplate, Vec<SpriteTemplate>, Option<Handle<Image>>)> {
    match def.mesh_kind {
        ParticleMeshKind::StaticMesh => {
            let d3m = assets.d3ms.get(&def.mesh_id)?;
            let template = sprite_template(d3m)?;
            let tex = d3m.texture_name[8..12]
                .try_into()
                .ok()
                .and_then(|name: [u8; 4]| assets.images.get(&name))
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
                brightness: Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 128.0,
            })
        })
        .collect()
}

// research/xim Particle.kt:72 — the spriteSheetIndex advances the flipbook across the
// particle's lifetime. StaticMesh particles carry no frames and use the single template.
fn flipbook_frame(g: &LiveGenerator, progress: f32) -> &SpriteTemplate {
    if g.sprite_frames.is_empty() {
        return &g.template;
    }
    let n = g.sprite_frames.len();
    let idx = ((progress * n as f32) as usize).min(n - 1);
    &g.sprite_frames[idx]
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
            attach_joint_source: 0,
            attach_joint_target: 0,
            attach_source_oriented: false,
            init_scale: [0.1, 0.1, 1.0],
            init_color: [0.2, 0.2, 0.6, 0.5],
            init_velocity: [0.0, 0.01, 0.0],
            init_rotation: [0.0; 3],
            blend: ffxi_dat::particle_gen::ParticleBlend::Additive,
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
            },
            sprite_frames: Vec::new(),
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
        }
    }

    // Drive the emission math directly (no Bevy world), one tick's worth of frames per call.
    fn advance(g: &mut LiveGenerator, frames: f32) {
        advance_generator(g, frames);
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
        rebuild_mesh(&g, Quat::IDENTITY, &mut mesh);
        assert!(count(&mesh) > 0, "empty rebuild must not be zero-length");
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
        rebuild_mesh(&g, Quat::IDENTITY, &mut mesh);
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
        rebuild_mesh(&g, Quat::IDENTITY, &mut mesh);
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

        let mut cont = live(base, 1.0);
        cont.def.continuous = true;
        let mut spray = live(base, 1.0);

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
            rebuild_mesh(g, Quat::IDENTITY, &mut mesh);
            match mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap() {
                bevy::mesh::VertexAttributeValues::Float32x4(c) => c[0][3],
                _ => panic!("expected Float32x4 colours"),
            }
        };

        assert!(
            (alpha_of(&cont) - 1.0).abs() < 1e-4,
            "continuous body stays fully opaque, not the life fade"
        );
        assert!(
            (alpha_of(&spray) - 0.25).abs() < 1e-4,
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
}
