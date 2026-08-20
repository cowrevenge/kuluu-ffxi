use bevy::math::{Mat4, Vec3};
use ffxi_actor::skeleton_instance::{pose_world, RootTransform};
use ffxi_dat::skel_anim::SkeletonAnimation;
use kuluu_render::ffxi_actor_render::{load_mount_race, load_pc, LoadedActor};

fn layered<'a>(
    clips: &'a [SkeletonAnimation],
    prefix: &str,
) -> impl Fn(usize) -> Option<ffxi_dat::skel_anim::KeyFrameTransform> + Clone + 'a {
    let picked: Vec<&SkeletonAnimation> = clips
        .iter()
        .filter(|c| c.id.as_str().starts_with(prefix))
        .collect();
    move |joint: usize| {
        picked
            .iter()
            .find_map(|c| c.get_joint_transform(joint as u32, 0.0))
    }
}

fn layered_at<'a>(
    clips: &'a [SkeletonAnimation],
    prefix: &str,
    t: f32,
) -> impl Fn(usize) -> Option<ffxi_dat::skel_anim::KeyFrameTransform> + Clone + 'a {
    let picked: Vec<&SkeletonAnimation> = clips
        .iter()
        .filter(|c| c.id.as_str().starts_with(prefix))
        .collect();
    move |joint: usize| {
        picked
            .iter()
            .find_map(|c| c.get_joint_transform(joint as u32, t))
    }
}

fn part_joint_histogram(a: &LoadedActor) {
    for sm in &a.skel_meshes {
        let mut counts: std::collections::BTreeMap<u16, usize> = Default::default();
        for m in &sm.meshes {
            for v in &m.vertices {
                *counts.entry(v.joint_index0).or_default() += 1;
            }
        }
        let mut top: Vec<(u16, usize)> = counts.into_iter().collect();
        top.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
        top.truncate(8);
        println!("  part {:6} joints {:?}", sm.id.as_str(), top);
    }
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
    let rider = load_pc(rider_race, true, &[], None, None, None).expect("load_pc");

    println!("--- mount parts: vertex counts per bound joint ---");
    part_joint_histogram(&mount);

    let idle = pose_world(
        &mount.skeleton,
        layered(&mount.animations, "chi"),
        RootTransform::identity(),
        &[],
    );
    println!("--- mount joints in the chi? carrying pose (skeleton space, Y-down) ---");
    for (i, m) in idle.iter().enumerate() {
        let t = m.to_scale_rotation_translation().2;
        println!("  joint {i:3}  ({:7.3}, {:7.3}, {:7.3})", t.x, t.y, t.z);
    }

    println!("--- vertical travel of each mount joint over run? ---");
    let samples: Vec<Vec<Mat4>> = (0..8)
        .map(|k| {
            pose_world(
                &mount.skeleton,
                layered_at(&mount.animations, "run", k as f32 * 4.0),
                RootTransform::identity(),
                &[],
            )
        })
        .collect();
    for i in 0..mount.skeleton.joints.len() {
        let ys: Vec<f32> = samples
            .iter()
            .map(|p| p[i].to_scale_rotation_translation().2.y)
            .collect();
        let lo = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if hi - lo > 0.01 {
            println!("  joint {i:3}  y {lo:7.3}..{hi:7.3}  travel {:.3}", hi - lo);
        }
    }

    println!("--- root (joint 0) animated translation in the rider's mounted clips ---");
    for c in rider.animations.iter() {
        if let Some(t) = c.get_joint_transform(0, 0.0) {
            println!("  {:5} root t={:?}", c.id.as_str(), t.translation);
        }
    }
    println!("--- root animated translation in the mount's own clips ---");
    for c in mount.animations.iter() {
        if let Some(t) = c.get_joint_transform(0, 0.0) {
            println!("  {:5} root t={:?}", c.id.as_str(), t.translation);
        }
    }

    for prefix in ["chi", "run"] {
        let pose = pose_world(
            &rider.skeleton,
            layered(&rider.animations, prefix),
            RootTransform::identity(),
            &[],
        );
        let hip = pose[ffxi_actor::skeleton_instance::HIP_JOINT]
            .to_scale_rotation_translation()
            .2;
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for sm in &rider.skel_meshes {
            for m in &sm.meshes {
                for v in &m.vertices {
                    if let Some(j) = pose.get(usize::from(v.joint_index0)) {
                        let p = j.transform_point3(Vec3::from(v.p0));
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                }
            }
        }
        println!(
            "rider {prefix}?: hip={hip:?} body y {:.3}..{:.3}",
            lo.y, hi.y
        );
    }
}
