//! Occlusion-aware point lights: a lamp only reaches surfaces the zone geometry lets it.
//!
//! The DAT's per-chunk light bindings decide WHICH slots a surface may receive, but nothing in
//! them says whether the lamp is actually visible from that surface — a wall behind a pillar
//! within range still got painted by the full attenuation term. This system closes that gap:
//! for every authored (surface, lamp) pair it raycasts lamp -> surface through the zone
//! collision BVH and zeroes the slot when geometry stands between them. The zeroed binding is
//! written back into the material asset, so `update_zone_point_lighting` repacks the chunk's
//! point-light arrays from the reduced set on its own (no second write path). Zone geometry is
//! static within a load, so a zeroed slot stays zeroed for the zone's lifetime.
//!
//! Lamps gated off by time of day carry zero colour (`zone_point_lights::at_time`) and already
//! contribute nothing; their bindings are left intact so they return when the track turns them
//! back on.
//!
//! Actors get the same test against their current position: a person standing in a pillar's
//! shadow stops receiving that lamp. The filter runs before `update_ffxi_actor_point_lights`
//! so the registry write of the frame uses the filtered indices; when that system recomputes
//! a selection (the actor moved past its reselect epsilon) the unfiltered set is visible for at
//! most one frame, which reads as nothing.

use bevy::prelude::*;
use kuluu_render::{
    ffxi_actor_render::FfxiRenderActor, ffxi_zone_material::FfxiZoneMaterial,
    zone_point_lights::ActiveSceneLights,
};

use super::collision_bvh::ZoneCollisionBvh;

// The lamp fixture mesh sits at the light position (the DAT's basePosition is inside the
// lantern), so a ray started there hits its own housing and occludes everything. Start past it.
const LIGHT_OCCLUSION_NEAR_CLIP: f32 = 1.0;

/// True when any zone collision geometry stands between `from` and `to`.
fn bvh_blocks(bvh: &ZoneCollisionBvh, from: Vec3, to: Vec3) -> bool {
    let dir = to - from;
    let dist = dir.length();
    if dist <= LIGHT_OCCLUSION_NEAR_CLIP {
        return false;
    }
    let d = dir / dist;
    let origin = from + d * LIGHT_OCCLUSION_NEAR_CLIP;
    bvh.0.as_ref().is_some_and(|b| {
        b.ray_cast(origin, d, dist - LIGHT_OCCLUSION_NEAR_CLIP)
            .is_some()
    })
}

/// Zero the authored light slots of every zone material whose lamp is occluded from that
/// surface, and drop the same lamps from actors' cached point-light selections. Runs after the
/// BVH build (so a freshly loaded zone has its geometry) and before the actor registry write;
/// the zone-material repack lands on `update_zone_point_lighting`'s next pass, one frame later —
/// occlusion only changes when a zone loads or an actor walks between lamp and wall, so the lag
/// is not visible.
pub fn apply_light_occlusion_system(
    bvh: Res<ZoneCollisionBvh>,
    active: Option<Res<ActiveSceneLights>>,
    q_zone: Query<(Entity, &GlobalTransform, &MeshMaterial3d<FfxiZoneMaterial>)>,
    mut materials: ResMut<Assets<FfxiZoneMaterial>>,
    mut q_actors: Query<&mut FfxiRenderActor, With<GlobalTransform>>,
) {
    let Some(active) = active else { return };
    if active.lights.is_empty() || bvh.0.is_none() {
        return;
    }

    for (_entity, gt, mat_handle) in &q_zone {
        let Some(mut mat) = materials.get_mut(&mat_handle.0) else {
            continue;
        };
        // No authored binding: nothing to occlude (the nearest-N fallback path is the
        // no-binding zone's only lighting and stays as-is).
        if !mat.light_bindings.iter().any(Option::is_some) {
            continue;
        }
        let target = gt.translation();
        for slot in 0..mat.light_bindings.len() {
            let Some(id) = mat.light_bindings[slot] else {
                continue;
            };
            // ToD-gated-off lamps carry zero colour and contribute nothing; leave the binding
            // intact so the lamp returns when its track turns it back on.
            let Some(light) = active.lights.iter().find(|l| l.light_id == id) else {
                continue;
            };
            if light.color != Vec3::ZERO && bvh_blocks(&bvh, light.world_pos, target) {
                mat.light_bindings[slot] = None;
            }
        }
    }

    for mut actor in &mut q_actors {
        let Some(eval_pos) = actor.point_light_eval_pos() else {
            continue;
        };
        actor.filter_point_light_selection(|i| {
            active
                .lights
                .get(i as usize)
                .is_some_and(|l| l.color != Vec3::ZERO && !bvh_blocks(&bvh, l.world_pos, eval_pos))
        });
    }
}
