//! Where the fishing poses actually live, against retail DATs.
//!
//! `fsh0`..`fsh6` are *routine* names, not animation ids: retail enqueues them as
//! model routines (research/xim Actor.kt `updateFishingState`) and each routine's
//! first Motion stage names the real `fh0?`..`fhd?` clip. Both halves were wrong
//! before — the DAT holding them was never loaded, and the selector looked the
//! routine id up among animations, where it can never match — so no fishing pose
//! ever played, for the local player or anyone in view.

use ffxi_dat::main_dll::{MainDll, ACTION_ANIM_FISHING_OFFSET};
use ffxi_dat::resource_dir::ResourceDir;
use ffxi_dat::DatRoot;

/// The macro-state phases `ffxi_actor::fishing_clip` maps: cast/wait, fighting,
/// then the five resolutions.
const PHASES: [u8; 7] = [0, 1, 2, 3, 4, 5, 6];

/// Every playable race index `skeleton_file_id_for_race` accepts.
const RACES: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

fn install() -> Option<DatRoot> {
    DatRoot::from_env_or_default().ok()
}

fn fishing_dir(root: &DatRoot, race: u8) -> Option<ResourceDir> {
    let dll = MainDll::load(root.root()).ok()?;
    let base = dll.base_action_animation_index(race)?;
    let loc = root
        .resolve(u32::from(base + ACTION_ANIM_FISHING_OFFSET))
        .ok()?;
    let bytes = std::fs::read(loc.path_under(root)).ok()?;
    Some(ResourceDir::from_bytes(bytes))
}

/// Each race's fishing DAT must carry a routine for every phase, and each of
/// those must name a Motion clip that is present in the same DAT. A break
/// anywhere in that chain is a silently missing pose.
#[test]
fn every_race_resolves_every_fishing_phase_to_a_real_clip() {
    let Some(root) = install() else {
        eprintln!("no FFXI install; skipping");
        return;
    };

    for race in RACES {
        let Some(dir) = fishing_dir(&root, race) else {
            panic!("race {race}: no fishing DAT at action base + {ACTION_ANIM_FISHING_OFFSET}");
        };
        let routines = dir.collect_schedulers();
        let clips: Vec<String> = dir
            .collect_animations()
            .iter()
            .map(|a| a.id.as_str().to_string())
            .collect();

        for phase in PHASES {
            let fc = ffxi_actor::actor_state::fishing_clip(phase)
                .unwrap_or_else(|| panic!("phase {phase} has no clip mapping"));
            let want = fc.id.as_str();

            let routine = routines
                .iter()
                .find(|s| String::from_utf8_lossy(&s.name).trim_end() == want)
                .unwrap_or_else(|| panic!("race {race}: no `{want}` routine in the fishing DAT"));

            let motion = routine
                .stages
                .iter()
                .find(|t| t.stage.kind == ffxi_dat::scheduler::StageKind::Motion)
                .unwrap_or_else(|| panic!("race {race} `{want}`: routine has no Motion stage"));
            let motion_id = String::from_utf8_lossy(&motion.stage.id)
                .trim_end()
                .to_string();

            // Retail clip ids end in a `?` wildcard standing for the slot digit,
            // so the concrete animations are `<stem>0`, `<stem>1`, …
            let stem = motion_id.trim_end_matches('?');
            assert!(
                clips.iter().any(|c| c.starts_with(stem)),
                "race {race} `{want}` -> Motion `{motion_id}`, but no clip starts with `{stem}`; \
                 have {clips:?}"
            );
        }
    }
}

/// The sweat routines s2c 0x038 SCHEDULOR names during a fight ride in the same
/// DAT, so loading it is what makes that packet actionable too (kuluu-t5ru).
#[test]
fn the_fishing_dat_also_carries_the_sweat_routines() {
    let Some(root) = install() else { return };
    let Some(dir) = fishing_dir(&root, 1) else {
        panic!("race 1: no fishing DAT")
    };
    let names: Vec<String> = dir
        .collect_schedulers()
        .iter()
        .map(|s| String::from_utf8_lossy(&s.name).trim_end().to_string())
        .collect();
    for want in ["hits", "hitl"] {
        assert!(
            names.contains(&want.to_string()),
            "no `{want}` in {names:?}"
        );
    }
}

/// The offset is a measurement, not a guess: no other DAT near the action base
/// carries `fsh*`, so a wrong offset would silently load a DAT with no poses
/// rather than fail.
#[test]
fn the_fishing_offset_is_the_only_one_that_carries_fsh_routines() {
    let Some(root) = install() else { return };
    let Ok(dll) = MainDll::load(root.root()) else {
        return;
    };
    let Some(base) = dll.base_action_animation_index(1) else {
        return;
    };

    let carries_fsh = |offset: u16| -> bool {
        let Ok(loc) = root.resolve(u32::from(base + offset)) else {
            return false;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
            return false;
        };
        ResourceDir::from_bytes(bytes)
            .collect_schedulers()
            .iter()
            .any(|s| String::from_utf8_lossy(&s.name).starts_with("fsh"))
    };

    assert!(carries_fsh(ACTION_ANIM_FISHING_OFFSET));
    let others: Vec<u16> = (0..8u16)
        .filter(|o| *o != ACTION_ANIM_FISHING_OFFSET && carries_fsh(*o))
        .collect();
    // +2 is the rod model's own copy of fsh0..fsh6; it poses the rod, not the
    // angler, so the actor must not pick it up by mistake.
    assert_eq!(
        others,
        vec![ACTION_ANIM_FISHING_OFFSET + 1],
        "unexpected DATs carrying fsh* near the action base"
    );
}
