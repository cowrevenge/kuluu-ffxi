#![cfg(not(target_arch = "wasm32"))]

use bevy::prelude::*;
use ffxi_dat::particle_gen::ParticleGeneratorDef;
use ffxi_dat::DatRoot;
use ffxi_viewer_wire::Vec3 as WireVec3;

use crate::particle_sim::{spawn_zone_particle_generator, ParticleSimulator};
use crate::scene::mzb_to_bevy;
use crate::scheduler_runtime::parse_action_bytes;
use crate::snapshot::{effective_zone_file_id, SceneState};

#[derive(Resource, Default)]
pub struct ZoneParticles {
    pub file_id: Option<u32>,
    entities: Vec<Entity>,
}

// Water sprays are placed, timed emitters (life > 0) whose MMB mesh carries a water
// texture. No DAT field flags a generator as water, so the surface is identified by
// its mesh's texture (or mesh DatId) — cloud/star/window/flower/lamp sheets share
// the placed+auto-run shape but are life==0 or non-water-textured.
const WATER_MESH_PREFIXES: [&str; 11] = [
    "sea", "riv", "taki", "wat", "muzu", "mz0", "abuk", "sib", "spl", "spr", "oomi",
];

fn is_water_name(name: &str) -> bool {
    let n = name.trim_end().to_ascii_lowercase();
    WATER_MESH_PREFIXES.iter().any(|p| n.starts_with(p)) || n.contains("water")
}

// Campfire/brazier/hearth flame (and its smoke) is a zone-static auto-run
// particle generator, NOT an NPC or zone-lighting effect — e.g. West Ronfaure
// (file 201) fir1..fir5 [mesh hi12, additive] and the Mog House hearth (file
// 200) fir1..fir7/fih1. Like water, no DAT field flags a generator as fire, so
// the surface is identified by the generator's chunk name, which follows FFXI's
// fir*/fih* (flame) and sma*/smk*/smf*/kas* (smoke) naming convention. Only the
// timed, placed emitters (life > 0, base_position set) are these ambient sprays;
// life==0 persistent billboards (clouds/sun/stars) belong to the Particle VM.
const FIRE_NAME_PREFIXES: [&str; 6] = ["fir", "fih", "sma", "smk", "smf", "kas"];

fn is_fire_generator(name: &[u8; 4], def: &ParticleGeneratorDef) -> bool {
    if !def.auto_run || def.base_position == [0.0, 0.0, 0.0] || def.max_life_frames <= 0.0 {
        return false;
    }
    let n = String::from_utf8_lossy(name)
        .trim_end()
        .to_ascii_lowercase();
    FIRE_NAME_PREFIXES.iter().any(|p| n.starts_with(p))
}

fn water_spray_mesh<'a>(
    def: &ParticleGeneratorDef,
    assets: &'a crate::scheduler_runtime::ActionAssets,
) -> Option<&'a crate::scheduler_runtime::MmbSpriteMesh> {
    if !def.auto_run || def.base_position == [0.0, 0.0, 0.0] || def.max_life_frames <= 0.0 {
        return None;
    }
    let mmb = assets.mmbs.get(&def.mesh_id)?;
    let mesh_id = String::from_utf8_lossy(&def.mesh_id);
    (is_water_name(&mmb.texture_name) || is_water_name(&mesh_id)).then_some(mmb)
}

fn sync_zone_particles(
    scene_state: Res<SceneState>,
    mut store: ResMut<ZoneParticles>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sim: ResMut<ParticleSimulator>,
    mut commands: Commands,
) {
    let current = effective_zone_file_id(&scene_state.snapshot);
    if current == store.file_id {
        return;
    }
    store.file_id = current;

    // OnExit(InGame) does not fire on a zone warp, so despawn the previous zone's
    // generator entities explicitly here; the simulator self-reaps the dangling
    // LiveGenerators once their mesh entity is gone (sync_particle_meshes).
    for e in store.entities.drain(..) {
        commands.entity(e).try_despawn();
    }

    let Some(file_id) = current else {
        return;
    };
    let Ok(root) = DatRoot::from_env_or_default() else {
        return;
    };
    let Ok(loc) = root.resolve(file_id) else {
        return;
    };
    let path = loc.path_under(root.root());
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };

    let (_schedulers, assets) = parse_action_bytes(&bytes);
    let mut spawned = 0usize;
    for (name, def) in assets.particle_defs.iter() {
        if water_spray_mesh(def, &assets).is_none() && !is_fire_generator(name, def) {
            continue;
        }
        let bp = def.base_position;
        let origin = mzb_to_bevy(WireVec3 {
            x: bp[0],
            y: bp[1],
            z: bp[2],
        });
        if let Some(entity) = spawn_zone_particle_generator(
            *def,
            &assets,
            origin,
            &mut meshes,
            &mut mats,
            &mut images,
            &mut sim,
            &mut commands,
        ) {
            store.entities.push(entity);
            spawned += 1;
        }
    }

    info!("zone_particles: DAT {file_id} → {spawned} zone-static particle generator(s)");
}

pub struct ZoneParticlesPlugin;

impl Plugin for ZoneParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZoneParticles>()
            .add_systems(Update, sync_zone_particles);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fire_def(name: &[u8; 4]) -> ([u8; 4], ParticleGeneratorDef) {
        (
            *name,
            ParticleGeneratorDef {
                auto_run: true,
                base_position: [10.0, 0.0, 20.0],
                max_life_frames: 60.0,
                ..Default::default()
            },
        )
    }

    #[test]
    fn fire_and_smoke_generators_classified() {
        // West Ronfaure (201) / Mog House (200) campfire + hearth emitter names.
        for name in [
            b"fir1", b"fir7", b"fih1", b"sma2", b"smk1", b"smf1", b"kas1",
        ] {
            let (n, def) = fire_def(name);
            assert!(
                is_fire_generator(&n, &def),
                "{} should be a fire/smoke emitter",
                String::from_utf8_lossy(name)
            );
        }
    }

    #[test]
    fn non_fire_timed_emitters_excluded() {
        for name in [b"gl01", b"li01", b"cry1", b"Rp01", b"aww1"] {
            let (n, def) = fire_def(name);
            assert!(
                !is_fire_generator(&n, &def),
                "{} is not fire",
                String::from_utf8_lossy(name)
            );
        }
    }

    #[test]
    fn fire_requires_placed_timed_auto_run() {
        let (n, base) = fire_def(b"fir1");
        assert!(is_fire_generator(&n, &base));

        let not_auto = ParticleGeneratorDef {
            auto_run: false,
            ..base
        };
        assert!(
            !is_fire_generator(&n, &not_auto),
            "manual-run fire excluded"
        );

        let unplaced = ParticleGeneratorDef {
            base_position: [0.0, 0.0, 0.0],
            ..base
        };
        assert!(!is_fire_generator(&n, &unplaced), "unplaced fire excluded");

        let persistent = ParticleGeneratorDef {
            max_life_frames: 0.0,
            ..base
        };
        assert!(
            !is_fire_generator(&n, &persistent),
            "life==0 persistent billboard excluded (Particle VM scope)"
        );
    }
}
