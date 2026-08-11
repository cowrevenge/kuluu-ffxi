#![cfg(not(target_arch = "wasm32"))]

use bevy::prelude::*;
use ffxi_dat::particle_gen::{AttachType, ParticleGeneratorDef};
use ffxi_dat::weather::WeatherTypeId;
use ffxi_dat::ChunkKind;
use ffxi_dat::DatRoot;

use crate::particle_sim::{
    spawn_zone_particle_generator, CelestialClock, ParticleSimulator, ZoneGeneratorOptions,
};
use crate::scheduler_runtime::{parse_action_bytes, ActionAssets};
use crate::snapshot::{effective_zone_file_id, SceneState};
use crate::sun_moon::{moon_phase_frame, sun_direction, vana_day_index, DatCelestials, VanaSky};

// research/xim EnvironmentManager.kt:235 `sunMoonDistance = 900f // Measured in E. Saru`.
// The celestial billboards ride a sphere of this radius centred on the camera, so their
// on-screen size is set purely by the generator's own scale and mesh — there is no
// disc-radius constant on our side.
pub const CELESTIAL_DISTANCE: f32 = 900.0;

// The directory that holds a zone's per-weather environment subtrees. Generators for the
// sun, moon, lunar halo and lens-flare chain live under weat/<weather-type>/…, so the
// active weather selects which celestial set is drawn.
const WEAT_DIR: WeatherTypeId = *b"weat";

// Opt-in until kuluu-b98u lands. The DAT weather tree supplies the sun/moon glow dome
// (`suns`/`moon`, BillBoardType::Camera), the moon sprite sheet and the lunar halo — but not
// necessarily the sun disc: in f_ro the disc comes from the screen-space lens-flare chain, while
// other zones author a small-scale third `suns` generator for it (file 104 `weat/fine/sun2`,
// init_scale 2.0). So retiring the hand-authored discs is a per-zone coverage question, not a
// flip of this gate.
const DAT_CELESTIALS_ENV: &str = "FFXI_DAT_CELESTIALS";

fn dat_celestials_enabled() -> bool {
    std::env::var_os(DAT_CELESTIALS_ENV).is_some()
}

#[derive(Resource, Default)]
pub struct CelestialParticles {
    loaded: Option<(Option<u32>, WeatherTypeId)>,
    entities: Vec<Entity>,
}

// Collect every Sun/Moon-attached generator under weat/<weather>/, at any depth: retail nests
// the moon's own set one level further (weat/<weather>/moon/{moon,kasa}) than the sun's
// (weat/<weather>/sun1).
fn collect_celestial_defs(
    bytes: &[u8],
    weather: WeatherTypeId,
) -> Vec<([u8; 4], ParticleGeneratorDef)> {
    fn walk(
        node: &ffxi_dat::chunk::ChunkNode<'_>,
        in_weather: bool,
        weather: WeatherTypeId,
        out: &mut Vec<([u8; 4], ParticleGeneratorDef)>,
    ) {
        for child in &node.children {
            let c = &child.chunk;
            if !child.children.is_empty() || c.kind == ChunkKind::Rmp as u8 {
                let descend = if in_weather {
                    true
                } else if c.name == WEAT_DIR {
                    // The weather ids sit directly under `weat`; recurse with the gate armed
                    // so only the matching subtree is harvested.
                    for sub in &child.children {
                        if sub.chunk.name == weather {
                            walk(sub, true, weather, out);
                        }
                    }
                    continue;
                } else {
                    false
                };
                walk(child, descend, weather, out);
                continue;
            }
            if !in_weather
                || ffxi_dat::kind::ChunkKind::from_u8(c.kind)
                    != Some(ffxi_dat::kind::ChunkKind::Generator)
            {
                continue;
            }
            // A lens-flare generator is Sun-attached too, but it is drawn in screen space
            // from the projected sun position (research/xim ZoneDrawer.kt:219-245) and is
            // owned by lens_flare.rs, not by this world-space path.
            if let Ok(Some(def)) = ParticleGeneratorDef::parse(c.data) {
                if matches!(def.attach_type, AttachType::Sun | AttachType::Moon) {
                    out.push((c.name, def));
                }
            }
        }
    }

    let mut out = Vec::new();
    walk(&ffxi_dat::chunk::walk_tree(bytes), false, weather, &mut out);
    out
}

fn spawn_celestial_set(
    assets: &ActionAssets,
    defs: &[([u8; 4], ParticleGeneratorDef)],
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<crate::ffxi_particle_material::FfxiParticleMaterial>,
    images: &mut Assets<Image>,
    sim: &mut ParticleSimulator,
    commands: &mut Commands,
) -> Vec<Entity> {
    defs.iter()
        .filter_map(|(name, def)| {
            let entity = spawn_zone_particle_generator(
                *def,
                assets,
                None,
                // Placeholder: track_celestial_bodies rewrites this from the camera before
                // the first mesh rebuild.
                Vec3::ZERO,
                ZoneGeneratorOptions::default(),
                meshes,
                mats,
                images,
                sim,
                commands,
            );
            if entity.is_none() {
                debug!(
                    "celestial: {} has no resolvable mesh {}",
                    String::from_utf8_lossy(name),
                    String::from_utf8_lossy(&def.mesh_id)
                );
            }
            entity
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn sync_celestial_particles(
    scene_state: Res<SceneState>,
    zone_weather: Res<crate::weather::ZoneWeather>,
    mut store: ResMut<CelestialParticles>,
    mut dat_celestials: ResMut<DatCelestials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<crate::ffxi_particle_material::FfxiParticleMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sim: ResMut<ParticleSimulator>,
    mut commands: Commands,
) {
    if !dat_celestials_enabled() {
        return;
    }
    let file_id = effective_zone_file_id(&scene_state.snapshot);
    let weather = zone_weather
        .active_weather_type()
        .unwrap_or(ffxi_dat::weather::WEATHER_TYPE_FALLBACK);
    if store.loaded == Some((file_id, weather)) {
        return;
    }
    store.loaded = Some((file_id, weather));

    // OnExit(InGame) does not fire on a zone warp, and a weather change swaps the whole
    // celestial set, so the previous set is despawned explicitly here.
    for e in store.entities.drain(..) {
        commands.entity(e).try_despawn();
    }
    dat_celestials.active = false;

    let Some(bytes) = file_id
        .zip(DatRoot::from_env_or_default().ok())
        .and_then(|(id, root)| {
            root.resolve(id)
                .ok()
                .and_then(|loc| std::fs::read(loc.path_under(&root)).ok())
        })
    else {
        return;
    };

    let defs = collect_celestial_defs(&bytes, weather);
    if defs.is_empty() {
        info!(
            "celestial: DAT {file_id:?} weather {} ships no sun/moon generators",
            String::from_utf8_lossy(&weather)
        );
        return;
    }

    let (_schedulers, assets) = parse_action_bytes(&bytes);
    store.entities = spawn_celestial_set(
        &assets,
        &defs,
        &mut meshes,
        &mut mats,
        &mut images,
        &mut sim,
        &mut commands,
    );
    dat_celestials.active = !store.entities.is_empty();
    info!(
        "celestial: DAT {file_id:?} weather {} → {}/{} generator(s)",
        String::from_utf8_lossy(&weather),
        store.entities.len(),
        defs.len(),
    );
}

// research/cexi-viewer ui/js/particle/runtime.js:517-524 (xim ParticleGeneratorAttachment):
// a Sun/Moon-attached generator's position is the body's position plus the camera's, so the
// sky stays a fixed distance ahead however far the player walks.
fn track_celestial_bodies(
    sky: Res<VanaSky>,
    vana_clock: Res<crate::vana_time::VanaClock>,
    cam: Query<&GlobalTransform, With<crate::camera::OperatorCamera>>,
    mut sim: ResMut<ParticleSimulator>,
) {
    let day = vana_day_index(&vana_clock);
    sim.set_celestial_clock(CelestialClock {
        // research/xim EnvironmentManager getFullDayInterpolation: the fraction of the
        // Vana'diel day the ClockValueUpdater curves are sampled at.
        day_fraction: (sky.hour / 24.0).rem_euclid(1.0),
        day_of_week: (day % ffxi_dat::particle_gen::DAYS_OF_WEEK as u64) as usize,
        moon_phase: moon_phase_frame(sky.moon_phase),
    });

    let Some(cam) = cam.iter().next() else {
        return;
    };
    let cam_pos = cam.translation();
    let sun_dir = sun_direction(sky.hour);
    sim.set_celestial_origins(
        cam_pos + sun_dir * CELESTIAL_DISTANCE,
        cam_pos - sun_dir * CELESTIAL_DISTANCE,
    );
}

pub struct CelestialParticlesPlugin;

impl Plugin for CelestialParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CelestialParticles>()
            .init_resource::<DatCelestials>()
            .add_systems(
                Update,
                (sync_celestial_particles, track_celestial_bodies)
                    .chain()
                    .after(crate::sun_moon::sun_moon_system)
                    // The simulator bakes each generator's colour into its mesh, so the
                    // celestial clock and camera-relative origins have to be in place before
                    // it runs. Unordered, frame 0 rebuilds with a Default clock — day
                    // fraction 0, where the moon's authored alpha curve is 0 — and the
                    // billboard bakes black.
                    .before(crate::particle_sim::sync_particle_meshes),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // research/xim EnvironmentManager.kt:371-382 — the moon rides the sun's circle offset by
    // pi. track_celestial_bodies derives the moon origin as the sun direction negated, which
    // only equals retail's `Vector3f(sin(a+pi), cos(a+pi), 0)` while the sun arc itself stays
    // in the XY plane (no z tilt) and unit-length.
    #[test]
    fn negating_the_sun_matches_retails_moon_arc() {
        for hour in 0..24 {
            let hour = hour as f32;
            let a = (hour / 24.0) * std::f32::consts::TAU;
            let moon_ffxi = Vec3::new(
                (a + std::f32::consts::PI).sin(),
                (a + std::f32::consts::PI).cos(),
                0.0,
            );
            // FFXI -> Bevy for a direction: (x, -y, -z), same mapping as scene::mzb_to_bevy.
            let expected = Vec3::new(moon_ffxi.x, -moon_ffxi.y, -moon_ffxi.z);
            let derived = -sun_direction(hour);
            assert!(
                derived.distance(expected) < 1e-5,
                "hour {hour}: derived {derived:?} != retail {expected:?}"
            );
        }
    }
}
