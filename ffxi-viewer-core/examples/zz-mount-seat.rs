use bevy::math::Vec3;
use ffxi_actor::skeleton_instance::{pose_world, standard_joint_world_position, RootTransform};
use ffxi_viewer_core::ffxi_actor_render::{load_mount_race, load_pc, LoadedActor};

fn skin_bounds(a: &LoadedActor) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for sm in &a.skel_meshes {
        for m in &sm.meshes {
            for v in &m.vertices {
                let p = Vec3::from(v.p0);
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
    }
    (lo, hi)
}

fn posed_bounds(a: &LoadedActor, pose: &[bevy::math::Mat4]) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for sm in &a.skel_meshes {
        for m in &sm.meshes {
            for v in &m.vertices {
                let j0 = usize::from(v.joint_index0);
                let Some(m0) = pose.get(j0) else { continue };
                let p = m0.transform_point3(Vec3::from(v.p0));
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
    }
    (lo, hi)
}

fn main() {
    let race: u8 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);
    let rider_race: u8 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);

    let mount = load_mount_race(race).expect("load_mount_race");
    let skel = &mount.skeleton;
    let pose = pose_world(skel, |_| None, RootTransform::identity(), &[]);

    println!(
        "mount race {race}: joints={} references={} meshes={}",
        skel.joints.len(),
        skel.references.len(),
        mount.skel_meshes.len()
    );
    let (lo, hi) = skin_bounds(&mount);
    println!("  bind skin bounds  lo={lo:?} hi={hi:?}");
    let (lo, hi) = posed_bounds(&mount, &pose);
    println!("  posed skin bounds lo={lo:?} hi={hi:?}");
    for bb in &skel.bounding_boxes {
        println!("  bbox {bb:?}");
    }

    println!("--- standard joints 40..64 (raw) ---");
    for i in 40..64.min(skel.references.len()) {
        let r = &skel.references[i];
        let p = standard_joint_world_position(&pose, skel, i);
        println!(
            "  std {i:3}  joint {:3}  offset {:?}  world {p:?}",
            r.index, r.position_offset
        );
    }
    println!("--- standard joints with a non-origin world position ---");
    for i in 0..skel.references.len() {
        let Some(p) = standard_joint_world_position(&pose, skel, i) else {
            continue;
        };
        let r = &skel.references[i];
        if p == Vec3::ZERO && r.index == 0 {
            continue;
        }
        println!(
            "  std {i:3}  joint {:3}  offset {:?}  world ({:.3}, {:.3}, {:.3})",
            r.index, r.position_offset, p.x, p.y, p.z
        );
    }

    let rider = load_pc(rider_race, true, &[], None, None, None).expect("load_pc");
    let rpose = pose_world(&rider.skeleton, |_| None, RootTransform::identity(), &[]);
    let (lo, hi) = posed_bounds(&rider, &rpose);
    println!("--- rider race {rider_race} (mounted load) ---");
    println!(
        "  joints={} posed lo={lo:?} hi={hi:?}",
        rider.skeleton.joints.len()
    );
    for i in [0usize, 1, 2, 3] {
        let t = rpose.get(i).map(|m| m.to_scale_rotation_translation().2);
        println!("  joint {i} pose translation {t:?}");
    }
    for i in [2usize, 8, 9] {
        println!(
            "  std {i} world {:?}",
            standard_joint_world_position(&rpose, &rider.skeleton, i)
        );
    }

    let root = ffxi_dat::DatRoot::from_env_or_default().expect("DatRoot");
    let dll = ffxi_dat::main_dll::MainDll::load(root.root()).expect("FFXiMain.dll");
    let base = dll
        .base_action_animation_index(rider_race)
        .expect("action anim base");
    let id = u32::from(base + ffxi_dat::main_dll::ACTION_ANIM_MOUNT_OFFSET);
    let loc = root.resolve(id).expect("resolve mount pose dat");
    let bytes = std::fs::read(loc.path_under(&root)).expect("read mount pose dat");
    let clips = ffxi_dat::resource_dir::ResourceDir::from_bytes(bytes).collect_animations();
    println!("--- rider mount-pose DAT {id}: {} clips ---", clips.len());
    let mut ids: Vec<String> = clips.iter().map(|c| c.id.as_str()).collect();
    ids.sort();
    println!("  {ids:?}");

    let mskel_id = u32::from(dll.base_race_config_index(race).expect("mount race config"));
    let mloc = root.resolve(mskel_id).expect("resolve mount race dat");
    let mbytes = std::fs::read(mloc.path_under(&root)).expect("read mount race dat");
    let mclips = ffxi_dat::resource_dir::ResourceDir::from_bytes(mbytes).collect_animations();
    let mut mids: Vec<String> = mclips.iter().map(|c| c.id.as_str()).collect();
    mids.sort();
    println!("--- mount race DAT {mskel_id}: {} clips ---", mclips.len());
    println!("  {mids:?}");

    let layered = |clips: &[ffxi_dat::skel_anim::SkeletonAnimation], names: &[&str]| {
        let picked: Vec<ffxi_dat::skel_anim::SkeletonAnimation> = names
            .iter()
            .filter_map(|n| clips.iter().find(|c| c.id.as_str() == *n).cloned())
            .collect();
        move |joint: usize| {
            picked
                .iter()
                .find_map(|c| c.get_joint_transform(joint as u32, 0.0))
        }
    };

    for names in [&["chi0", "chi1"][..], &["run0", "run1"][..]] {
        let rposed = pose_world(
            &rider.skeleton,
            layered(&clips, names),
            RootTransform::identity(),
            &[],
        );
        let (lo, hi) = posed_bounds(&rider, &rposed);
        let hips = rposed[2].to_scale_rotation_translation().2;
        println!("rider clip {names:?} unmounted-pose: hips={hips:?} lo={lo:?} hi={hi:?}");
    }

    let mposed = pose_world(
        &mount.skeleton,
        layered(&mclips, &["chi0"]),
        RootTransform::identity(),
        &[],
    );
    let (lo, hi) = posed_bounds(&mount, &mposed);
    println!("mount clip chi0: lo={lo:?} hi={hi:?}");
    for sm in &mount.skel_meshes {
        let mut plo = Vec3::splat(f32::INFINITY);
        let mut phi = Vec3::splat(f32::NEG_INFINITY);
        for m in &sm.meshes {
            for v in &m.vertices {
                let Some(m0) = mposed.get(usize::from(v.joint_index0)) else {
                    continue;
                };
                let p = m0.transform_point3(Vec3::from(v.p0));
                plo = plo.min(p);
                phi = phi.max(p);
            }
        }
        println!("  part {} lo={plo:?} hi={phi:?}", sm.id.as_str());
    }
    for i in [21usize, 43, 48, 50] {
        println!(
            "  mount std {i} world {:?}",
            standard_joint_world_position(&mposed, &mount.skeleton, i)
        );
    }
}
