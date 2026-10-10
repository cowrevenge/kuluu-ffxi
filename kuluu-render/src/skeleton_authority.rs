//! The one pass with the final say over an actor's skeleton, run once per pose pass after the clips compose.
//!
//! Everything it decides is read from what is loaded and what was requested, so a swapped DAT, another rig or a
//! motion a cutscene opcode asked for runs through it unchanged:
//!
//! - The pose is the clips': the coordinator's records, written the way retail writes them, so a bone a motion keys
//!   is that motion's outright from its first frame.
//! - The correction on top is the look-at, from the skeleton's own records, their limits and the reference joints
//!   they name, with the wire status choosing how many apply. [`Correction::Free`] is retail's law: every record
//!   turned from the actor's forward against one pre-bend frame. [`Correction::Held`], locked on a side step, turns
//!   each record from where the bone it carries faces (where its subtree fans out: the head for the neck record, the
//!   chest itself for the chest record), shallow bone first in the pose the turns above it left. A bone only takes
//!   what is still owed, so a head the side step already turns back toward the target lands on the look point
//!   instead of past it.
//! - Nothing the clips did not author moves a bone faster than the handover that caused it. A bone whose last writer
//!   lets go is given back to rest over the blend of the motion that let it go; a change of correction is given back
//!   over the blend running in the slot that writes the bone, or over the look-at's own opening when none runs. A
//!   handover requested as instant stays instant.

use bevy::math::{Mat4, Quat, Vec3};
use ffxi_actor::animation::{BonePoseScratch, SkeletonAnimationCoordinator};
use ffxi_actor::look_bend::{self, BendFrame, POSE_FORWARD};
use ffxi_actor::skeleton_instance::{
    pose_world, pose_world_mounted_into, MountAttach, PoseScratch, RootTransform,
};
use ffxi_dat::skel::Skeleton;
use ffxi_dat::skel_anim::KeyFrameTransform;

/// The correction the actor's state puts in force this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Correction {
    /// Retail's look-at, opened by its own weight.
    Free,
    /// Locked on a side step: the clip turns the body off the walker's aim, so each record turns from the bone it
    /// carries, whole.
    Held,
}

/// This pass's look-at: the point it turns toward (pose space), how far it is open, how much it opens or closes per
/// skeleton frame, and the record count and anchor drop the wire status allows (`crate::look_at_gates`).
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub point: Vec3,
    pub weight: f32,
    pub pace: f32,
    pub records: usize,
    pub anchor_drop: f32,
}

/// How a pose composes, the same for the clips' pose and the shown one.
#[derive(Debug, Clone, Copy)]
pub struct Composition<'a> {
    pub root: RootTransform,
    pub parent_overrides: &'a [(usize, usize)],
    pub mount: Option<MountAttach>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Law {
    correction: Correction,
    records: usize,
    anchor_drop: f32,
    open: bool,
}

/// One bone as the last pass left it.
#[derive(Debug, Clone, Copy, Default)]
struct Watch {
    /// The slot whose layer wrote it; `None` when nothing did.
    writer: Option<usize>,
    /// The record it was shown with; `None` is its bind data.
    shown: Option<KeyFrameTransform>,
    residue: Option<Residue>,
    /// For the bone log: the correction's turn on it and what its residue still owes, in degrees.
    turned_deg: f32,
    owed_deg: f32,
}

/// What a bone was shown with that nothing asks for any more, given back over `window` skeleton frames.
#[derive(Debug, Clone, Copy)]
struct Residue {
    rotation: Quat,
    translation: Vec3,
    scale: Vec3,
    window: f32,
    elapsed: f32,
}

impl Residue {
    /// The difference between what was shown and the goal, or nothing when the handover asked for none.
    fn between(
        shown: Option<KeyFrameTransform>,
        goal: Option<KeyFrameTransform>,
        window: f32,
    ) -> Option<Self> {
        if window <= 0.0 {
            return None;
        }
        let from = shown.unwrap_or_default();
        let to = goal.unwrap_or_default();
        let to_scale = Vec3::from(to.scale);
        Some(Self {
            rotation: unit(from.rotation) * unit(to.rotation).inverse(),
            translation: Vec3::from(from.translation) - Vec3::from(to.translation),
            scale: Vec3::select(
                to_scale.cmpeq(Vec3::ZERO),
                Vec3::ONE,
                Vec3::from(from.scale) / to_scale,
            ),
            window,
            elapsed: 0.0,
        })
    }

    fn share_left(&self) -> f32 {
        1.0 - (self.elapsed / self.window).clamp(0.0, 1.0)
    }

    fn rotation_left(&self) -> Quat {
        Quat::IDENTITY.slerp(self.rotation, self.share_left())
    }

    /// The goal with what is left of this residue on top.
    fn over(&self, goal: Option<KeyFrameTransform>) -> KeyFrameTransform {
        let base = goal.unwrap_or_default();
        let left = self.share_left();
        KeyFrameTransform {
            rotation: (self.rotation_left() * unit(base.rotation)).to_array(),
            translation: (Vec3::from(base.translation) + self.translation * left).to_array(),
            scale: (Vec3::from(base.scale) * Vec3::ONE.lerp(self.scale, left)).to_array(),
        }
    }
}

/// One joint a look-at record turns, and the bone it faces by.
#[derive(Debug, Clone, Copy)]
struct RecordBone {
    joint: usize,
    /// The end of the joint's unbranched chain: the subtree a record turns points where it fans out, so the neck
    /// record faces by the head the clip carries on it and the chest record by the chest itself.
    facing: usize,
    /// The facing bone's own axis that points along the body's forward in bind.
    axis: Vec3,
}

/// See the module docs.
#[derive(Default)]
pub struct SkeletonAuthority {
    record_bones: Vec<RecordBone>,
    bones: Vec<Watch>,
    /// Each slot's live blend at the last pass, by its length in frames.
    blends: Vec<Option<f32>>,
    law: Option<Law>,
    shown: Vec<Option<KeyFrameTransform>>,
}

impl SkeletonAuthority {
    /// The authority for one rig: which joints its look-at records name, the bone each faces by and how that bone
    /// faces in bind.
    pub fn of(skeleton: &Skeleton) -> Self {
        let bind = pose_world(skeleton, |_| None, RootTransform::identity(), &[]);
        let record_bones = look_bend::authored_bones(skeleton, look_bend::BEND_RECORDS_MAX)
            .into_iter()
            .filter_map(|bone| {
                let facing = chain_end(skeleton, bone.joint);
                let (_, rotation, _) = bind.get(facing)?.to_scale_rotation_translation();
                Some(RecordBone {
                    joint: bone.joint,
                    facing,
                    axis: rotation.inverse() * POSE_FORWARD,
                })
            })
            .collect();
        Self {
            record_bones,
            ..Self::default()
        }
    }

    /// Forget every bone's history (a pose-state reset): the next pass shows the clips as they stand.
    pub fn reset(&mut self) {
        self.bones.clear();
        self.blends.clear();
        self.law = None;
    }

    /// One pose pass. `pose` holds the clips' composition of `clips` on entry and the shown pose on exit.
    #[allow(clippy::too_many_arguments)]
    pub fn settle(
        &mut self,
        pose: &mut Vec<Mat4>,
        work: &mut PoseScratch,
        skeleton: &Skeleton,
        clips: &BonePoseScratch,
        motions: &SkeletonAnimationCoordinator,
        composition: Composition,
        look: Option<Look>,
        correction: Correction,
        elapsed_frames: f32,
    ) {
        let bone_count = skeleton.joints.len();
        let primed = self.bones.len() == bone_count;
        if !primed {
            self.bones.clear();
            self.bones.resize(bone_count, Watch::default());
        }
        // Each slot's live blend: its length and how far it has run.
        let blends: Vec<Option<(f32, f32)>> = motions
            .animations
            .iter()
            .map(|slot| {
                slot.as_ref()
                    .and_then(|animator| animator.transition.as_ref())
                    .map(|blend| (blend.transition_duration, blend.progress()))
            })
            .collect();
        let law = look.map(|look| Law {
            correction,
            records: look.records,
            anchor_drop: look.anchor_drop,
            open: look.weight > 0.0,
        });
        let law_changed = primed && law != self.law;
        let turns = match look {
            Some(look) => record_turns(
                pose,
                skeleton,
                composition.parent_overrides,
                &look,
                correction,
                &self.record_bones,
            ),
            None => Vec::new(),
        };

        self.shown.clear();
        self.shown.resize(bone_count, None);
        let mut recompose = !turns.is_empty();
        for bone in 0..bone_count {
            let clip = clips.get(bone);
            let writer = clips.writer(bone);
            let turn = turns
                .iter()
                .find(|(joint, _)| *joint == bone)
                .map(|(_, turn)| *turn);
            let goal = match turn {
                Some(turn) => {
                    let base = clip.unwrap_or_default();
                    Some(KeyFrameTransform {
                        rotation: (turn * unit(base.rotation)).to_array(),
                        ..base
                    })
                }
                None => clip,
            };
            let watch = &mut self.bones[bone];
            let let_go = primed && watch.writer.is_some() && writer.is_none();
            let recorrected =
                law_changed && self.record_bones.iter().any(|record| record.joint == bone);
            if let_go || recorrected {
                let mut window = 0.0_f32;
                if let_go {
                    window = window.max(let_go_window(watch.writer, &blends, &self.blends));
                }
                if recorrected {
                    window = window.max(recorrect_window(
                        writer.or(watch.writer),
                        &blends,
                        look.map_or(0.0, |look| look.pace),
                    ));
                }
                watch.residue = Residue::between(watch.shown, goal, window);
            } else if writer.is_some() && watch.writer.is_none() {
                // Taken outright: a motion writes a bone it keys from its first frame.
                watch.residue = None;
            } else if let Some(residue) = watch.residue.as_mut() {
                residue.elapsed = (residue.elapsed + elapsed_frames.clamp(0.0, residue.window))
                    .min(residue.window);
            }
            let shown = match &watch.residue {
                Some(residue) => {
                    recompose = true;
                    watch.owed_deg = degrees(residue.rotation_left());
                    Some(residue.over(goal))
                }
                None => {
                    watch.owed_deg = 0.0;
                    goal
                }
            };
            if watch
                .residue
                .is_some_and(|residue| residue.elapsed >= residue.window)
            {
                watch.residue = None;
            }
            watch.turned_deg = turn.map_or(0.0, degrees);
            watch.writer = writer;
            watch.shown = shown;
            self.shown[bone] = shown;
        }
        self.law = law;
        self.blends = blends
            .iter()
            .map(|blend| blend.map(|(length, _)| length))
            .collect();

        if recompose {
            let shown = &self.shown;
            pose_world_mounted_into(
                pose,
                work,
                skeleton,
                |bone| shown.get(bone).copied().flatten(),
                composition.root,
                composition.parent_overrides,
                composition.mount,
            );
        }
    }

    /// Whether `joint` still carries something it is being given back from.
    pub fn owes(&self, joint: usize) -> bool {
        self.bones
            .get(joint)
            .is_some_and(|watch| watch.residue.is_some())
    }

    /// One bone's line for the bone log: the law in force, who wrote it, the correction's turn on it and what it
    /// is still owed.
    pub fn describe(&self, joint: usize) -> String {
        let law = match self.law.map(|law| law.correction) {
            Some(Correction::Free) => "free",
            Some(Correction::Held) => "held",
            None => "none",
        };
        match self.bones.get(joint) {
            Some(watch) => format!(
                "law={law} writer={} turned={:.1} owed={:.1}",
                watch
                    .writer
                    .map_or_else(|| "-".to_string(), |slot| slot.to_string()),
                watch.turned_deg,
                watch.owed_deg
            ),
            None => format!("law={law}"),
        }
    }
}

/// A bone let go is given back over the blend of the motion that let it go: what is left of that slot's blend, the
/// whole of it when it landed this pass, or, for a slot swapped or cleared outright, the blend the request that did
/// it runs in its other slots. Nothing blending means the handover was instant.
fn let_go_window(
    slot: Option<usize>,
    blends: &[Option<(f32, f32)>],
    last_blends: &[Option<f32>],
) -> f32 {
    if let Some(slot) = slot {
        if let Some(Some((length, run))) = blends.get(slot) {
            return (length - run).max(0.0);
        }
        if let Some(Some(length)) = last_blends.get(slot) {
            return *length;
        }
    }
    running_blend(blends).unwrap_or(0.0)
}

/// A change of correction is given back over what is left of the handover that changed it: the blend in the slot
/// that writes the bone, or the one the request runs in its other slots for a bone no clip writes. With no blend
/// running it is a change of the look-at alone, given back over the span the look-at opens or closes in.
fn recorrect_window(slot: Option<usize>, blends: &[Option<(f32, f32)>], pace: f32) -> f32 {
    if let Some(Some((length, run))) = slot.and_then(|slot| blends.get(slot)) {
        return (length - run).max(0.0);
    }
    if let Some(left) = running_blend(blends) {
        return left;
    }
    if pace > 0.0 {
        pace.recip()
    } else {
        0.0
    }
}

/// What is left of the longest blend running in any slot.
fn running_blend(blends: &[Option<(f32, f32)>]) -> Option<f32> {
    blends
        .iter()
        .flatten()
        .map(|(length, run)| (length - run).max(0.0))
        .reduce(f32::max)
}

/// The correction as multipliers on the records of the bones it turns.
fn record_turns(
    pose: &[Mat4],
    skeleton: &Skeleton,
    parent_overrides: &[(usize, usize)],
    look: &Look,
    correction: Correction,
    record_bones: &[RecordBone],
) -> Vec<(usize, Quat)> {
    let turns: Vec<(usize, Quat)> = match correction {
        Correction::Free => {
            if look.weight <= 0.0 {
                return Vec::new();
            }
            let weight = look.weight.min(1.0);
            look_bend::look_bends(
                pose,
                skeleton,
                look.point,
                POSE_FORWARD,
                look.records,
                look.anchor_drop,
            )
            .into_iter()
            .map(|(joint, turn)| (joint, Quat::IDENTITY.slerp(turn, weight)))
            .collect()
        }
        Correction::Held => held_turns(pose, skeleton, parent_overrides, look, record_bones),
    };
    into_records(pose, skeleton, parent_overrides, &turns)
}

/// [`Correction::Held`]: each record turns from where the bone it carries faces toward the look point, inside its
/// own ellipse, shallow bone first and measured in the pose the turns before it left.
fn held_turns(
    pose: &[Mat4],
    skeleton: &Skeleton,
    parent_overrides: &[(usize, usize)],
    look: &Look,
    record_bones: &[RecordBone],
) -> Vec<(usize, Quat)> {
    let mut bones = look_bend::authored_bones(skeleton, look.records);
    bones.sort_by_key(|bone| look_bend::joint_depth(skeleton, bone.joint));
    let mut turned = pose.to_vec();
    let mut turns = Vec::with_capacity(bones.len());
    for bone in bones {
        let Some(record) = record_bones
            .iter()
            .find(|record| record.joint == bone.joint)
        else {
            continue;
        };
        let Some(facing) = turned.get(record.facing).and_then(|matrix| {
            let (_, rotation, _) = matrix.to_scale_rotation_translation();
            ground(rotation * record.axis)
        }) else {
            continue;
        };
        let Some(frame) = BendFrame::new(&turned, skeleton, look.point, facing, look.anchor_drop)
        else {
            continue;
        };
        let Some((aim, _)) = look_bend::clamp_bend_axis(frame.forward, frame.look, &bone.limit)
        else {
            continue;
        };
        let Some(aim) = aim.try_normalize() else {
            continue;
        };
        let turn = Quat::from_rotation_arc(frame.forward, aim);
        look_bend::rotate_about_pivot(
            &mut turned,
            skeleton,
            parent_overrides,
            bone.joint,
            turn,
            1.0,
        );
        turns.push((bone.joint, turn));
    }
    turns
}

/// Turns in `pose`'s space, shallow-first and each about its own joint, as multipliers on those joints' records: a
/// record composes under its parent, so each turn is carried into the frame its parent stands in once the turns
/// above it have landed.
fn into_records(
    pose: &[Mat4],
    skeleton: &Skeleton,
    parent_overrides: &[(usize, usize)],
    turns: &[(usize, Quat)],
) -> Vec<(usize, Quat)> {
    let parent_of = |joint: usize| {
        parent_overrides
            .iter()
            .find(|(child, _)| *child == joint)
            .map(|&(_, parent)| parent)
            .or_else(|| skeleton.joints.get(joint).and_then(|j| j.parent))
    };
    let is_above = |ancestor: usize, joint: usize| {
        let mut current = parent_of(joint);
        let mut hops = 0;
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            hops += 1;
            if hops > skeleton.joints.len() {
                break;
            }
            current = parent_of(parent);
        }
        false
    };
    turns
        .iter()
        .enumerate()
        .map(|(index, &(joint, turn))| {
            let landed = turns[..index]
                .iter()
                .filter(|(earlier, _)| is_above(*earlier, joint))
                .fold(Quat::IDENTITY, |landed, (_, earlier)| *earlier * landed);
            let parent = parent_of(joint)
                .and_then(|parent| pose.get(parent))
                .map_or(Quat::IDENTITY, |matrix| {
                    matrix.to_scale_rotation_translation().1
                });
            let frame = landed * parent;
            (joint, frame.inverse() * turn * frame)
        })
        .collect()
}

/// A record's rotation as a unit quaternion, normalised the way composition consumes it; a degenerate sum carries no
/// axis and leaves the bone on its bind rotation.
fn unit(rotation: [f32; 4]) -> Quat {
    let q = Quat::from_array(rotation);
    let length = q.length();
    if length.is_finite() && length > f32::EPSILON {
        q / length
    } else {
        Quat::IDENTITY
    }
}

/// Down from `joint` while it has exactly one child: where its subtree fans out.
fn chain_end(skeleton: &Skeleton, joint: usize) -> usize {
    let mut end = joint;
    for _ in 0..skeleton.joints.len() {
        let mut children = skeleton
            .joints
            .iter()
            .enumerate()
            .filter(|(_, child)| child.parent == Some(end))
            .map(|(index, _)| index);
        match (children.next(), children.next()) {
            (Some(only), None) => end = only,
            _ => break,
        }
    }
    end
}

/// How far a rotation turns, in degrees.
fn degrees(rotation: Quat) -> f32 {
    rotation.angle_between(Quat::IDENTITY).to_degrees()
}

/// The ground-plane direction of `v` (pose space is `-Y` up).
fn ground(v: Vec3) -> Option<Vec3> {
    Vec3::new(v.x, 0.0, v.z).try_normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffxi_actor::animation::{LoopParams, TransitionParams};
    use ffxi_dat::datid::DatId;
    use ffxi_dat::skel::{standard_position, Joint, JointReference, LookAtLimit};
    use ffxi_dat::skel_anim::SkeletonAnimation;

    const CHEST: usize = 3;
    const NECK: usize = 4;
    const HEAD: usize = 5;
    const TOLERANCE_DEG: f32 = 0.5;

    /// Root, hips, spine, chest, neck, head and two shoulders on the chest, standing up `-Y` and facing `+X`, with
    /// Hume M's two authored look-at records on references 3 (neck) and 7 (chest).
    fn rig() -> Skeleton {
        let joint = |parent: Option<usize>, rise: f32| Joint {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [0.0, -rise, 0.0],
            parent,
        };
        let reference = |index: usize| JointReference {
            index,
            rotation: [0.0; 3],
            position_offset: [0.0; 3],
        };
        let mut references: Vec<JointReference> = (0..=standard_position::CHEST)
            .map(|_| reference(0))
            .collect();
        references[standard_position::NECK] = reference(NECK);
        references[standard_position::CHEST] = reference(CHEST);
        Skeleton {
            id: DatId::from_str("look"),
            joints: vec![
                joint(None, 0.0),
                joint(Some(0), 1.0),
                joint(Some(1), 0.1),
                joint(Some(2), 0.2),
                joint(Some(3), 0.25),
                joint(Some(4), 0.12),
                joint(Some(3), 0.24),
                joint(Some(3), 0.24),
            ],
            references,
            bounding_boxes: Vec::new(),
            look_at_limits: vec![
                LookAtLimit {
                    x_limit: 0.24,
                    y_limit: 0.16,
                    scale: 0.5,
                },
                LookAtLimit {
                    x_limit: 0.16,
                    y_limit: 0.06,
                    scale: 0.5,
                },
            ],
        }
    }

    /// A looping clip holding each named joint at a yaw about the up axis.
    fn clip(id: &[u8; 4], yaws: &[(usize, f32)]) -> SkeletonAnimation {
        let frames = 9;
        SkeletonAnimation {
            id: DatId::from_name(id),
            num_joints: 0,
            num_frames: frames,
            key_frame_duration: 1.0,
            key_frame_sets: yaws
                .iter()
                .map(|&(joint, yaw)| {
                    (
                        joint as u32,
                        vec![
                            KeyFrameTransform {
                                rotation: Quat::from_rotation_y(yaw).to_array(),
                                ..Default::default()
                            };
                            frames
                        ],
                    )
                })
                .collect(),
        }
    }

    struct Bench {
        skeleton: Skeleton,
        motions: SkeletonAnimationCoordinator,
        clips: BonePoseScratch,
        authority: SkeletonAuthority,
        pose: Vec<Mat4>,
        work: PoseScratch,
    }

    impl Bench {
        fn new() -> Self {
            let skeleton = rig();
            let authority = SkeletonAuthority::of(&skeleton);
            Self {
                skeleton,
                motions: SkeletonAnimationCoordinator::new(),
                clips: BonePoseScratch::new(),
                authority,
                pose: Vec::new(),
                work: PoseScratch::default(),
            }
        }

        fn play(&mut self, clip: SkeletonAnimation, blend: f32) {
            self.motions.register_animation(
                clip,
                LoopParams {
                    loop_duration: None,
                    num_loops: None,
                    low_priority: false,
                },
                Some(TransitionParams {
                    transition_in_time: blend,
                    ..Default::default()
                }),
                |_| true,
            );
        }

        /// The clips' pose this pass, before the authority.
        fn clips_pose(&mut self) -> Vec<Mat4> {
            self.motions.update(1.0);
            self.motions
                .sample_pose(self.skeleton.joints.len(), &mut self.clips);
            pose_world_mounted_into(
                &mut self.pose,
                &mut self.work,
                &self.skeleton,
                |bone| self.clips.get(bone),
                RootTransform::identity(),
                &[],
                None,
            );
            self.pose.clone()
        }

        fn pass(&mut self, look: Option<Look>, correction: Correction) -> Vec<Mat4> {
            self.clips_pose();
            self.settle(look, correction)
        }

        /// The authority over the clips' pose already composed this pass.
        fn settle(&mut self, look: Option<Look>, correction: Correction) -> Vec<Mat4> {
            self.authority.settle(
                &mut self.pose,
                &mut self.work,
                &self.skeleton,
                &self.clips,
                &self.motions,
                Composition {
                    root: RootTransform::identity(),
                    parent_overrides: &[],
                    mount: None,
                },
                look,
                correction,
                1.0,
            );
            self.pose.clone()
        }
    }

    /// The point straight ahead of the neck, at its height.
    fn ahead() -> Look {
        Look {
            point: Vec3::new(20.0, -1.55, 0.0),
            weight: 1.0,
            pace: 0.04,
            records: look_bend::BEND_RECORDS_MAX,
            anchor_drop: 0.0,
        }
    }

    fn yaw_deg(pose: &[Mat4], joint: usize) -> f32 {
        let (_, rotation, _) = pose[joint].to_scale_rotation_translation();
        let facing = rotation * POSE_FORWARD;
        facing.z.atan2(facing.x).to_degrees()
    }

    #[test]
    fn a_bone_let_go_rests_over_the_blend_that_let_it_go_and_an_instant_one_snaps() {
        let held_deg = 30.0_f32;
        for blend in [6.0_f32, 0.0] {
            let mut bench = Bench::new();
            bench.play(clip(b"aaa0", &[(HEAD, held_deg.to_radians())]), 0.0);
            for _ in 0..3 {
                bench.pass(None, Correction::Free);
            }
            bench.play(clip(b"bbb0", &[(1, 0.0)]), blend);
            let mut shown = Vec::new();
            let mut blending = Vec::new();
            for _ in 0..(blend as usize + 10) {
                let pose = bench.pass(None, Correction::Free);
                shown.push(yaw_deg(&pose, HEAD).abs());
                blending.push(bench.motions.is_transitioning());
            }
            // The outgoing clip holds the bone outright while the blend runs; the pass it lands is still the held
            // pose, and from there it walks home in equal steps over the blend's own length.
            let landed = blending.iter().position(|live| !live).unwrap();
            for step in 0..=blend as usize {
                let expected = held_deg * (1.0 - step as f32 / blend.max(1.0));
                let expected = if blend == 0.0 { 0.0 } else { expected };
                assert!(
                    (shown[landed + step] - expected).abs() < TOLERANCE_DEG,
                    "blend {blend}: {step} passes after landing the head read {:.2}, expected {expected:.2}",
                    shown[landed + step]
                );
            }
            assert!(shown[landed + blend as usize + 1] < TOLERANCE_DEG);
        }
    }

    #[test]
    fn locked_on_a_side_step_each_record_turns_from_its_own_bone_and_the_head_lands_on_the_point() {
        let chest_deg = 40.0_f32;
        let neck_deg = -28.6_f32;
        let clip_head_deg = -(chest_deg + neck_deg);

        let mut held = Bench::new();
        held.play(
            clip(
                b"mvl0",
                &[
                    (CHEST, chest_deg.to_radians()),
                    (NECK, neck_deg.to_radians()),
                ],
            ),
            0.0,
        );
        let pose = held.pass(Some(ahead()), Correction::Held);
        assert!(
            yaw_deg(&pose, HEAD).abs() < TOLERANCE_DEG,
            "held, the head should land on the point; read {:.2}",
            yaw_deg(&pose, HEAD)
        );
        let chest_limit_deg = 0.16_f32.atan2(0.5).to_degrees();
        assert!(
            (yaw_deg(&pose, CHEST).abs() - (chest_deg - chest_limit_deg)).abs() < TOLERANCE_DEG,
            "held, the chest takes out only its own record's reach; read {:.2}",
            yaw_deg(&pose, CHEST)
        );

        // Retail's look-at measures from the actor's forward, so a point dead ahead turns nothing.
        let mut free = Bench::new();
        free.play(
            clip(
                b"mvl0",
                &[
                    (CHEST, chest_deg.to_radians()),
                    (NECK, neck_deg.to_radians()),
                ],
            ),
            0.0,
        );
        let pose = free.pass(Some(ahead()), Correction::Free);
        assert!((yaw_deg(&pose, HEAD) - clip_head_deg).abs() < TOLERANCE_DEG);
    }

    #[test]
    fn a_change_of_correction_never_jumps_and_lands_with_the_blend_that_made_it() {
        let blend = 8.0_f32;
        let mut bench = Bench::new();
        bench.play(clip(b"mvl0", &[(CHEST, 40_f32.to_radians())]), 0.0);
        let mut last = Vec::new();
        for _ in 0..4 {
            last = bench.pass(Some(ahead()), Correction::Held);
        }
        bench.play(clip(b"idl0", &[(CHEST, 0.0)]), blend);
        let first = bench.pass(Some(ahead()), Correction::Free);
        for joint in [CHEST, NECK, HEAD] {
            let moved = yaw_deg(&first, joint) - yaw_deg(&last, joint);
            assert!(
                moved.abs() < 40.0 / blend + TOLERANCE_DEG,
                "joint {joint} jumped {moved:.2} on the pass the correction changed"
            );
        }
        let mut owed_until = None;
        let mut blend_until = None;
        for pass in 2..=(blend as usize + 4) {
            bench.pass(Some(ahead()), Correction::Free);
            if owed_until.is_none() && !(bench.authority.owes(CHEST) || bench.authority.owes(NECK))
            {
                owed_until = Some(pass);
            }
            if blend_until.is_none() && !bench.motions.is_transitioning() {
                blend_until = Some(pass);
            }
        }
        assert_eq!(owed_until, blend_until);
    }

    #[test]
    fn free_is_retails_look_at() {
        let off_side = Look {
            point: Vec3::new(10.0, -1.2, 6.0),
            weight: 0.7,
            ..ahead()
        };
        let mut bench = Bench::new();
        bench.play(
            clip(b"idl0", &[(CHEST, 0.2), (NECK, -0.1), (HEAD, 0.05)]),
            0.0,
        );
        let mut retail = bench.clips_pose();
        look_bend::apply_look_bends(
            &mut retail,
            &bench.skeleton,
            &[],
            off_side.point,
            POSE_FORWARD,
            off_side.weight,
            off_side.records,
            off_side.anchor_drop,
        );
        let shown = bench.settle(Some(off_side), Correction::Free);
        for (joint, (a, b)) in shown.iter().zip(&retail).enumerate() {
            assert!(
                a.abs_diff_eq(*b, 1e-4),
                "joint {joint}: authority {a:?} against retail's bend {b:?}"
            );
        }
    }
}
