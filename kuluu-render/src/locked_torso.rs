//! Locked on, a side step keeps the upper body pointed at the target through the look-at, inside its limits.
//!
//! The walker aims the body at the locked target on every moving tick (`kuluu/src/view_native/input.rs`, the
//! `locked_bearing` re-aim), but the side-step clips turn the body off that aim: the lower set keys joint 2, which
//! the legs and the upper body both hang off, and the upper set twists the spine on top of it. Two things go wrong
//! with that on screen, and this module owns both answers:
//!
//! * The look-at measures from the root's forward, so a target the walker keeps dead ahead of the root reads as
//!   "straight on" and the bend does nothing while the chest and head face somewhere else. [`bend_reference`] hands
//!   the bend the chest's real facing instead, so the authored neck and chest records turn the head and shoulders
//!   back toward the target, each stopping at its own ellipse limit.
//! * On the A/D changeover a spine joint whose two side-step records sit past a right angle on either side blends
//!   the short way, which is through its back. [`merge_through_front`] takes the other arc for exactly that case, so
//!   the torso comes round through the target side.
//! * Both answers above only live inside a blend window or inside the authored neck/chest records, so a side step
//!   *held* on one key ends with the chest further off the target than the hips carrying it. [`UpperBody::steer_onto_aim`]
//!   turns the torso toward the walker's aim and no further from where its hips point than the authored chest record
//!   allows - holding at that limit while the target stays beyond it, rather than pinning the chest dead ahead past
//!   what the spine was built to bend.

use bevy::math::{Mat4, Quat, Vec3};
use ffxi_actor::animation::{interpolate_kf, SkeletonAnimationCoordinator};
use ffxi_actor::skeleton_instance::{composed_subtree, pose_world, RootTransform};
use ffxi_dat::skel::{standard_position, Skeleton};
use ffxi_dat::skel_anim::KeyFrameTransform;

/// The body's forward in pose space before the actor's facing is applied (every humanoid rig faces `+X`).
const POSE_FORWARD: Vec3 = ffxi_actor::look_bend::POSE_FORWARD;

/// What the pass needs from one rig: how to read the chest's facing, which joints turn the torso round, and how
/// far the authored skeleton lets the chest turn away from where the hips point.
#[derive(Debug, Clone, PartialEq)]
pub struct UpperBody {
    /// The chest reference's joint.
    pub chest: usize,
    /// The chest joint's own axis that points along the body's forward in the bind pose.
    pub chest_forward: Vec3,
    /// The neck reference's joint and its ancestors down to the joint the legs branch from, inclusive: every joint
    /// whose turn swings the torso round.
    pub spine: Vec<usize>,
    hip: usize,
    hip_forward: Vec3,
    /// The authored chest record's yaw semi-axis as an angle - how far this skeleton may turn its chest from its
    /// hips at all. `None` when there is no second bend record, so nothing bounds a torso turn.
    torso_yaw_cap: Option<f32>,
}

impl UpperBody {
    /// `None` for a rig without chest, neck and feet references, or whose spine never meets the legs.
    pub fn of(skeleton: &Skeleton) -> Option<Self> {
        let chest = skeleton.reference_at(standard_position::CHEST)?.index;
        let neck = skeleton.reference_at(standard_position::NECK)?.index;
        let leg_ancestors: Vec<usize> =
            [standard_position::RIGHT_FOOT, standard_position::LEFT_FOOT]
                .into_iter()
                .filter_map(|slot| skeleton.reference_at(slot).map(|r| r.index))
                .flat_map(|foot| ancestors(skeleton, foot))
                .collect();
        let neck_chain = ancestors(skeleton, neck);
        let branch_at = neck_chain
            .iter()
            .position(|joint| leg_ancestors.contains(joint))?;
        let spine = neck_chain[..=branch_at].to_vec();

        let bind = pose_world(skeleton, |_| None, RootTransform::identity(), &[]);
        let (_, chest_bind, _) = bind.get(chest)?.to_scale_rotation_translation();
        let hip = *spine.last()?;
        let (_, hip_bind, _) = bind.get(hip)?.to_scale_rotation_translation();
        let torso_yaw_cap = skeleton
            .look_at_limits
            .get(ffxi_actor::look_bend::CHEST_RECORD)
            .map(|l| (l.x_limit / l.scale).atan());
        Some(Self {
            chest,
            chest_forward: chest_bind.inverse() * POSE_FORWARD,
            hip,
            hip_forward: hip_bind.inverse() * POSE_FORWARD,
            spine,
            torso_yaw_cap,
        })
    }

    /// The chest's facing projected on the ground plane (pose space is `-Y` up).
    pub fn chest_facing(&self, pose: &[Mat4]) -> Option<Vec3> {
        let (_, rotation, _) = pose.get(self.chest)?.to_scale_rotation_translation();
        ground(rotation * self.chest_forward)
    }

    /// Where the hips point - the line every torso turn is measured from.
    fn hip_facing(&self, pose: &[Mat4]) -> Option<Vec3> {
        let (_, rotation, _) = pose.get(self.hip)?.to_scale_rotation_translation();
        ground(rotation * self.hip_forward)
    }

    /// Turn the upper body toward `aim` about the joint where it leaves the hips, for as long as the hold lasts,
    /// and no further from where its hips point than the authored chest record allows. An unbounded turn pins the
    /// chest dead ahead however the side-step clips sit the pelvis, bending the spine past what the rig authored -
    /// it reads on screen as breaking its back. Bounded instead: hold `hip facing + clamp(need, +/- the authored
    /// yaw limit)`, which stays at that limit (never snapping back) while the target is beyond it. Only the subtree
    /// rooted *above* the hips moves; every leg joint hangs off the hip itself, so stepping is untouched. A rig
    /// with no second bend record authors no torso turn, so nothing bounds one and none is made here.
    /// Returns whether anything was turned.
    pub fn steer_onto_aim(
        &self,
        skeleton: &Skeleton,
        parent_overrides: &[(usize, usize)],
        pose: &mut [Mat4],
        aim: Vec3,
        weight: f32,
    ) -> bool {
        if weight <= 0.0 || self.spine.len() < 2 {
            return false;
        }
        // `spine` runs neck first and ends at the joint the legs branch from, so the entry above the hips roots
        // everything the turn may carry.
        let torso_root = self.spine[self.spine.len() - 2];
        let (Some(facing), Some(aim_ground), Some(hip_facing), Some(cap)) = (
            self.chest_facing(pose),
            ground(aim),
            self.hip_facing(pose),
            self.torso_yaw_cap,
        ) else {
            return false;
        };
        let deviation_now = signed_yaw(hip_facing, facing);
        let yaw_needed = signed_yaw(hip_facing, aim_ground).clamp(-cap, cap);
        let yaw = (yaw_needed - deviation_now) * weight.clamp(0.0, 1.0);
        if yaw.abs() <= f32::EPSILON {
            return false;
        }
        let Some(mat) = pose.get(torso_root).copied() else {
            return false;
        };
        // A rotation about a point: the pivot itself must not travel, so translate onto it, turn, translate back.
        let pivot = mat.w_axis.truncate();
        let about_pivot = Mat4::from_translation(pivot)
            * Mat4::from_rotation_y(yaw)
            * Mat4::from_translation(-pivot);
        for joint in composed_subtree(skeleton, parent_overrides, torso_root) {
            if let Some(m) = pose.get_mut(joint) {
                *m = about_pivot * *m;
            }
        }
        true
    }

    /// This frame's record for each spine joint whose owning layer is crossfading, merged along
    /// [`merge_through_front`]; every other joint keeps what the coordinator sampled.
    pub fn steered_spine(
        &self,
        coordinator: &SkeletonAnimationCoordinator,
    ) -> Vec<(usize, KeyFrameTransform)> {
        let mut steered = Vec::new();
        for &joint in &self.spine {
            let mask = coordinator.mask_at(joint);
            if !mask.writable() || !mask.accepts_blend_layers() {
                continue;
            }
            let Some(owner) = coordinator
                .animations
                .iter()
                .flatten()
                .find(|layer| layer.get_joint_transform(joint).is_some())
            else {
                continue;
            };
            let Some(transition) = owner.transition.as_ref() else {
                continue;
            };
            let (Some(outgoing), Some(incoming), t) = transition.sides(joint) else {
                continue;
            };
            let mut merged = interpolate_kf(&outgoing, &incoming, t);
            merged.rotation = merge_through_front(outgoing.rotation, incoming.rotation, t);
            steered.push((joint, merged));
        }
        steered
    }
}

/// The forward the look-at bend measures from: the body's aim turned toward where the chest really faces, by
/// `weight` of the way. At weight 0 it is the aim itself, which is what the bend has always been given.
pub fn bend_reference(aim: Vec3, chest_facing: Option<Vec3>, weight: f32) -> Vec3 {
    let (Some(aim_ground), Some(facing)) = (ground(aim), chest_facing) else {
        return aim;
    };
    if weight <= 0.0 {
        return aim;
    }
    Quat::from_rotation_y(signed_yaw(aim_ground, facing) * weight.min(1.0)) * aim
}

/// The layer merge (`FFXiMain.dll retail-2026-09` RVA 0x33220: copy at `t == 1`, otherwise the shortest arc and
/// `a·(1−t) + b·t` stored raw), except where the shortest arc passes further from the bind orientation than both
/// of its ends. That arc swings the joint round behind both poses it is blending between; the other arc comes
/// through the bind orientation, which for a spine joint is upright and facing forward. A pair that is not past a
/// right angle on either side never meets the condition, so it merges exactly as retail does.
pub fn merge_through_front(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    if t == 1.0 {
        return b;
    }
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let short = if dot < 0.0 { neg(b) } else { b };
    let long = neg(short);
    let behind_both_ends =
        nearness_to_bind(add(a, short)) < nearness_to_bind(a).min(nearness_to_bind(b));
    let through_bind = nearness_to_bind(add(a, long)) > nearness_to_bind(add(a, short));
    let other = if behind_both_ends && through_bind {
        long
    } else {
        short
    };
    let inv = 1.0 - t;
    [
        a[0] * inv + other[0] * t,
        a[1] * inv + other[1] * t,
        a[2] * inv + other[2] * t,
        a[3] * inv + other[3] * t,
    ]
}

/// `|w|` of the normalised quaternion: 1 at the bind orientation, 0 half a turn away. A sum with no length has no
/// orientation at all and counts as furthest.
fn nearness_to_bind(q: [f32; 4]) -> f32 {
    let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if length <= f32::EPSILON {
        0.0
    } else {
        q[3].abs() / length
    }
}

fn add(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

fn neg(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], -q[3]]
}

/// The rotation about `+Y` that carries ground direction `from` onto `to`, in `(-PI, PI]`.
fn signed_yaw(from: Vec3, to: Vec3) -> f32 {
    from.cross(to).y.atan2(from.dot(to))
}

fn ground(v: Vec3) -> Option<Vec3> {
    Vec3::new(v.x, 0.0, v.z).try_normalize()
}

/// `joint` first, then each parent up to the root.
fn ancestors(skeleton: &Skeleton, joint: usize) -> Vec<usize> {
    let mut chain = vec![joint];
    let mut current = skeleton.joints.get(joint).and_then(|j| j.parent);
    while let Some(parent) = current {
        if chain.contains(&parent) {
            break;
        }
        chain.push(parent);
        current = skeleton.joints.get(parent).and_then(|j| j.parent);
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffxi_dat::datid::DatId;
    use ffxi_dat::skel::{Joint, JointReference};

    const IDENTITY_QUAT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn joint(parent: Option<usize>, translation: [f32; 3]) -> Joint {
        Joint {
            parent,
            rotation: IDENTITY_QUAT,
            translation,
        }
    }

    fn reference(index: usize) -> JointReference {
        JointReference {
            index,
            rotation: [0.0; 3],
            position_offset: [0.0; 3],
        }
    }

    const NECK_CAP_DEG: f32 = 15.0;
    const TORSO_CAP_DEG: f32 = 30.0;

    /// One authored record yaw-capable of `yaw_deg`, with the minor axis and scale left nominal.
    fn bend_record(yaw_deg: f32) -> ffxi_dat::skel::LookAtLimit {
        let x_limit = yaw_deg.to_radians().tan();
        ffxi_dat::skel::LookAtLimit {
            x_limit,
            y_limit: x_limit * 0.4,
            scale: 1.0,
        }
    }

    /// root(0) -> hips(1) -> { spine(2) -> chest(3) -> neck(4), leg(5) -> foot(6) }.
    fn rig() -> Skeleton {
        let mut references = vec![reference(0); standard_position::LEFT_FOOT + 1];
        references[standard_position::NECK] = reference(4);
        references[standard_position::CHEST] = reference(3);
        references[standard_position::RIGHT_FOOT] = reference(6);
        references[standard_position::LEFT_FOOT] = reference(6);
        Skeleton {
            id: DatId::from_str("test"),
            joints: vec![
                joint(None, [0.0, 0.0, 0.0]),
                joint(Some(0), [0.0, -1.0, 0.0]),
                joint(Some(1), [0.0, -0.3, 0.0]),
                joint(Some(2), [0.0, -0.3, 0.0]),
                joint(Some(3), [0.0, -0.3, 0.0]),
                joint(Some(1), [0.0, 0.5, 0.0]),
                joint(Some(5), [0.0, 0.5, 0.0]),
            ],
            references,
            bounding_boxes: Vec::new(),
            look_at_limits: vec![bend_record(NECK_CAP_DEG), bend_record(TORSO_CAP_DEG)],
        }
    }

    fn yaw(deg: f32) -> [f32; 4] {
        let q = Quat::from_rotation_y(deg.to_radians());
        [q.x, q.y, q.z, q.w]
    }

    fn angle_of(q: [f32; 4]) -> f32 {
        Quat::from_xyzw(q[0], q[1], q[2], q[3])
            .normalize()
            .to_axis_angle()
            .1
            .to_degrees()
    }

    #[test]
    fn the_spine_runs_from_the_neck_down_to_where_the_legs_branch() {
        let upper = UpperBody::of(&rig()).expect("the rig has chest, neck and feet");
        assert_eq!(upper.chest, 3);
        assert_eq!(
            upper.spine,
            vec![4, 3, 2, 1],
            "the hips are where the leg chain meets the spine"
        );
    }

    /// Two side-step records 100 degrees either side of forward: the shortest arc runs through half a turn, the
    /// merge here runs through forward instead.
    #[test]
    fn a_pair_past_a_right_angle_each_side_comes_round_through_the_front() {
        let mid = merge_through_front(yaw(100.0), yaw(-100.0), 0.5);
        assert!(
            angle_of(mid).abs() < 1e-3,
            "mid-blend sits at {} deg from forward",
            angle_of(mid)
        );
        let retail = ffxi_actor::animation::merge_layer_rotation(yaw(100.0), yaw(-100.0), 0.5);
        assert!(
            (angle_of(retail) - 180.0).abs() < 1e-3,
            "the shortest arc is the one through the back ({} deg)",
            angle_of(retail)
        );
    }

    /// Anything the shortest arc already brings through the front, and anything that is not a turn past a right
    /// angle, merges exactly as retail's layer merge does.
    #[test]
    fn every_other_pair_merges_as_retail_does() {
        let pairs = [
            (yaw(45.0), yaw(-45.0)),
            (yaw(80.0), yaw(-80.0)),
            (yaw(100.0), yaw(-60.0)),
            (yaw(170.0), yaw(160.0)),
            (yaw(-30.0), [0.0, -0.3826834, 0.0, -0.9238795]),
        ];
        for (a, b) in pairs {
            for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                assert_eq!(
                    merge_through_front(a, b, t),
                    ffxi_actor::animation::merge_layer_rotation(a, b, t),
                    "{a:?} -> {b:?} at {t}"
                );
            }
        }
    }

    #[test]
    fn the_reference_turns_from_the_aim_to_the_chest_by_the_weight() {
        let chest = Some(Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(bend_reference(POSE_FORWARD, chest, 0.0), POSE_FORWARD);
        let full = bend_reference(POSE_FORWARD, chest, 1.0);
        assert!(
            full.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-5),
            "{full:?}"
        );
        let half = bend_reference(POSE_FORWARD, chest, 0.5);
        assert!(
            half.angle_between(POSE_FORWARD) > 0.7 && half.angle_between(POSE_FORWARD) < 0.9,
            "{half:?}"
        );
        assert_eq!(bend_reference(POSE_FORWARD, None, 1.0), POSE_FORWARD);
    }

    /// A torso wound up by a side-step clip must come back onto the aim, and the turn must stop above the hips: the
    /// legs hang off the hip itself, so nothing that drives stepping may move.
    #[test]
    fn a_steered_torso_faces_the_aim_and_the_legs_stay_put() {
        let skel = rig();
        let upper = UpperBody::of(&skel).expect("the rig has chest, neck and feet");
        let mut pose = pose_world(&skel, |_| None, RootTransform::identity(), &[]);
        let torso_root = upper.spine[upper.spine.len() - 2];
        let subtree = composed_subtree(&skel, &[], torso_root);
        // Wind everything above the hips 70 degrees away from the aim, which is what one side-step clip does.
        let pivot = pose[torso_root].w_axis.truncate();
        let twist = Mat4::from_translation(pivot)
            * Mat4::from_rotation_y(70.0_f32.to_radians())
            * Mat4::from_translation(-pivot);
        for joint in &subtree {
            pose[*joint] = twist * pose[*joint];
        }
        let hips = *upper.spine.last().expect("the spine ends at the hips");
        let off_torso: Vec<usize> = (0..skel.joints.len())
            .filter(|joint| !subtree.contains(joint))
            .collect();
        let off_torso_before: Vec<Mat4> = off_torso.iter().map(|&joint| pose[joint]).collect();

        assert!(upper.steer_onto_aim(&skel, &[], &mut pose, POSE_FORWARD, 1.0));

        let facing = upper
            .chest_facing(&pose)
            .expect("the steered chest still has a ground facing");
        assert!(
            facing.abs_diff_eq(POSE_FORWARD, 1e-4),
            "steered chest faces {facing:?}, expected the aim {POSE_FORWARD:?}"
        );
        for (index, &joint) in off_torso.iter().enumerate() {
            assert_eq!(
                pose[joint], off_torso_before[index],
                "leg-side joint {joint} moved"
            );
        }
        assert_ne!(
            pose[hips], pose[torso_root],
            "the torso did turn relative to its hips"
        );
    }

    /// Zero weight (unlocked, or a side step that never ramped in) must not touch the pose at all.
    #[test]
    fn an_unweighted_steer_leaves_the_pose_alone() {
        let skel = rig();
        let upper = UpperBody::of(&skel).expect("the rig has chest, neck and feet");
        let bind = pose_world(&skel, |_| None, RootTransform::identity(), &[]);
        let mut pose = bind;
        assert!(!upper.steer_onto_aim(&skel, &[], &mut pose, POSE_FORWARD, 0.0));
    }

    /// With the hips themselves wound off the aim - which is what a side-step clip does to joint 2 - the torso may
    /// only close that gap by as far as its authored chest record reaches: it holds at that limit toward the target
    /// instead of being pinned onto the aim, and holding there means nothing left for a repeat steer to do.
    #[test]
    fn a_torso_beyond_its_authored_yaw_holds_at_the_limit_not_the_target() {
        let skel = rig();
        let upper = UpperBody::of(&skel).expect("the rig has chest, neck and feet");
        let mut pose = pose_world(&skel, |_| None, RootTransform::identity(), &[]);
        let hips = *upper.spine.last().expect("the spine ends at the hips");
        let torso_root = upper.spine[upper.spine.len() - 2];
        let pivot = pose[hips].w_axis.truncate();
        let wind = Mat4::from_translation(pivot)
            * Mat4::from_rotation_y(45.0_f32.to_radians())
            * Mat4::from_translation(-pivot);
        for joint in composed_subtree(&skel, &[], hips) {
            pose[joint] = wind * pose[joint];
        }
        let legs: Vec<usize> = {
            let all = composed_subtree(&skel, &[], hips);
            let torso: std::collections::HashSet<usize> = composed_subtree(&skel, &[], torso_root)
                .into_iter()
                .collect();
            all.into_iter().filter(|j| !torso.contains(j)).collect()
        };
        let legs_before: Vec<Mat4> = legs.iter().map(|&joint| pose[joint]).collect();

        assert!(upper.steer_onto_aim(&skel, &[], &mut pose, POSE_FORWARD, 1.0));

        let hip_facing = upper.hip_facing(&pose).expect("hips face somewhere");
        let chest_facing = upper
            .chest_facing(&pose)
            .expect("the chest faces somewhere");
        let deviation = signed_yaw(hip_facing, chest_facing).to_degrees();
        assert!(
            (deviation.abs() - TORSO_CAP_DEG).abs() < 1e-3,
            "chest sits {deviation:.2} deg from its hips, expected the {TORSO_CAP_DEG} deg record"
        );
        // ...and toward the target: nearer it than the hips are, by exactly that record.
        let hip_off = signed_yaw(hip_facing, Vec3::X).abs().to_degrees();
        let chest_off = signed_yaw(chest_facing, Vec3::X).abs().to_degrees();
        assert!(
            (chest_off + 1e-2 < hip_off) && (hip_off - TORSO_CAP_DEG - chest_off).abs() < 1e-2,
            "hips {hip_off:.2} deg off the aim, chest left at {chest_off:.2}"
        );

        for (index, &joint) in legs.iter().enumerate() {
            assert_eq!(pose[joint], legs_before[index], "leg joint {joint} moved");
        }
        assert!(
            !upper.steer_onto_aim(&skel, &[], &mut pose, POSE_FORWARD, 1.0),
            "holding at the limit leaves nothing to turn"
        );
    }

    /// A rig with no second bend record authors no torso turn, so there is nothing to bound it by and none is taken.
    #[test]
    fn a_rig_without_a_chest_record_takes_no_torso_turn() {
        let mut skel = rig();
        skel.look_at_limits.truncate(1);
        let upper = UpperBody::of(&skel).expect("the rig has chest, neck and feet");
        let mut pose = pose_world(&skel, |_| None, RootTransform::identity(), &[]);
        let torso_root = upper.spine[upper.spine.len() - 2];
        let pivot = pose[torso_root].w_axis.truncate();
        let twist = Mat4::from_translation(pivot)
            * Mat4::from_rotation_y(70.0_f32.to_radians())
            * Mat4::from_translation(-pivot);
        for joint in composed_subtree(&skel, &[], torso_root) {
            pose[joint] = twist * pose[joint];
        }
        let before: Vec<Mat4> = pose.clone();
        assert!(!upper.steer_onto_aim(&skel, &[], &mut pose, POSE_FORWARD, 1.0));
        assert_eq!(pose, before, "nothing in the pose moved");
    }
}
