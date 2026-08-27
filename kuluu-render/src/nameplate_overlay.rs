use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Camera3dDepthLoadOp, ClearColorConfig, Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;

use crate::camera::OperatorCamera;
use crate::components::InGameEntity;

/// Retail draws names to the backbuffer after the scene and its effects
/// (research/XIClient/src/XIClient/source/Rendering/Active/CXiActorNameDraw.cpp).
/// In-scene plates instead ride the main camera's post stack, where the bloom
/// composite lerps the local scene blur into every pixel — letters visibly
/// shift with whatever is behind them (kuluu-zxxb follow-up). This overlay
/// camera renders the plates in their own pass after the main camera: no
/// bloom, no fog, no tonemap, while `Camera3dDepthLoadOp::Load` on the shared
/// depth texture (bevy_core_pipeline `prepare_core_3d_depth_textures` caches
/// by (target, msaa)) keeps world geometry occluding them.
pub const NAMEPLATE_RENDER_LAYER: usize = 4;

/// After the operator camera (0), before the render-scale composite camera.
pub const NAMEPLATE_OVERLAY_CAMERA_ORDER: isize = 1;

#[derive(Component)]
pub struct NameplateOverlayCamera;

pub fn nameplate_render_layers() -> RenderLayers {
    RenderLayers::layer(NAMEPLATE_RENDER_LAYER)
}

/// Keeps the overlay camera mirroring the operator camera: same target (the
/// window, or the render-scale off-screen image), same Msaa (the depth-share
/// cache key), same view. Spawns it when missing — the operator camera is
/// respawned on AA changes, and both are `InGameEntity`, so the pair dies at
/// OnExit(InGame) together and this recreates the overlay next frame in-game.
#[allow(clippy::type_complexity)]
pub fn sync_nameplate_overlay_camera(
    mut commands: Commands,
    main_q: Query<
        (
            &Camera,
            Option<&RenderTarget>,
            &Transform,
            bevy::ecs::change_detection::Ref<'static, Projection>,
            &Msaa,
        ),
        (With<OperatorCamera>, Without<NameplateOverlayCamera>),
    >,
    mut overlay_q: Query<
        (
            Entity,
            &mut Camera,
            Option<&RenderTarget>,
            &mut Transform,
            &mut Projection,
            &mut Msaa,
        ),
        (With<NameplateOverlayCamera>, Without<OperatorCamera>),
    >,
) {
    let Ok((main_cam, main_target, main_t, main_proj, main_msaa)) = main_q.single() else {
        for (e, ..) in &overlay_q {
            commands.entity(e).try_despawn();
        }
        return;
    };
    let want_target = main_target.cloned().unwrap_or_default();

    let Ok((entity, mut cam, target, mut t, mut proj, mut msaa)) = overlay_q.single_mut() else {
        commands.spawn((
            InGameEntity,
            NameplateOverlayCamera,
            // Plate clicks resolve to their entity (picking.rs
            // resolve_hit_entity_id); the plates only exist in this view now.
            bevy::picking::mesh_picking::MeshPickingCamera,
            nameplate_render_layers(),
            Camera3d {
                depth_load_op: Camera3dDepthLoadOp::Load,
                ..default()
            },
            Camera {
                order: NAMEPLATE_OVERLAY_CAMERA_ORDER,
                clear_color: ClearColorConfig::None,
                is_active: main_cam.is_active,
                ..default()
            },
            want_target,
            // Shares the main camera's view target (cached per
            // (target, hdr, msaa)) so the pass composites over the scene
            // instead of a fresh texture.
            Hdr,
            Tonemapping::None,
            *main_msaa,
            *main_t,
            (*main_proj).clone(),
        ));
        return;
    };

    if cam.is_active != main_cam.is_active {
        cam.is_active = main_cam.is_active;
    }
    if target.and_then(|t| t.as_image()) != want_target.as_image() {
        commands.entity(entity).insert(want_target);
    }
    if *t != *main_t {
        *t = *main_t;
    }
    // Copy the projection ONLY when the source changed (FOV/AA edits). The
    // previous unconditional per-frame clone marked Projection changed every
    // frame on a camera sharing the main view target -- per-frame camera
    // recompute churn, and the last unguarded writer on the scaled path.
    if main_proj.is_changed() {
        *proj = (*main_proj).clone();
    }
    if *msaa != *main_msaa {
        *msaa = *main_msaa;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::WORLD_GIZMO_LAYER;
    use crate::minimap::topdown::MINIMAP_BAKE_LAYER;

    #[test]
    fn nameplate_layer_collides_with_no_other_view() {
        assert_ne!(NAMEPLATE_RENDER_LAYER, 0);
        assert_ne!(NAMEPLATE_RENDER_LAYER, WORLD_GIZMO_LAYER);
        assert_ne!(NAMEPLATE_RENDER_LAYER, MINIMAP_BAKE_LAYER);
    }
}
