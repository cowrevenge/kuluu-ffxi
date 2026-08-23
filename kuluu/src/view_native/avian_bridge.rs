//! Avian-backed character sweep (kuluu <-> avian3d 0.7 bridge).
//!
//! The zone MZB triangle soup is mirrored into one static avian trimesh
//! collider; dispatch_movement_system routes the per-tick wall clamp through
//! avian move-and-slide (horizontal) plus raw shape casts (vertical: the
//! classic up-forward-down swept stair step, then a ground snap). The height
//! returned is the SWEPT capsule height — it emerges from the motion; there is
//! no post-hoc floor snap left to warp.

use avian3d::prelude::*;
use bevy::prelude::*;
use std::time::Duration;

use kuluu_render::dat_mzb::{MzbCollisionGeometry, WallClipResult};

/// Capsule dimensions (bevy units = yalms). Radius matches the hand-rolled
/// walker's PLAYER_WALL_RADIUS; total height = 2*RADIUS + SEG_LEN.
pub const RADIUS: f32 = 0.4;
pub const SEG_LEN: f32 = 1.0;
/// Feet -> capsule center.
pub const HALF: f32 = RADIUS + SEG_LEN * 0.5;
/// Max riser a swept step may clear (MAX_GROUND_STEP_UP + slack).
pub const MAX_STEP: f32 = 0.45;

pub struct AvianBridgePlugin;

impl Plugin for AvianBridgePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            .insert_resource(Gravity(Vec3::ZERO))
            .init_resource::<ZoneAvianCollider>()
            .add_systems(Update, sync_zone_collider);
    }
}

/// The one static trimesh entity mirroring the currently loaded zone blocks.
#[derive(Resource, Default)]
pub struct ZoneAvianCollider {
    pub entity: Option<Entity>,
    pub tris: usize,
}

fn sync_zone_collider(
    geom: Res<MzbCollisionGeometry>,
    mut zc: ResMut<ZoneAvianCollider>,
    mut commands: Commands,
) {
    if !geom.is_changed() {
        return;
    }
    if let Some(e) = zc.entity.take() {
        commands.entity(e).despawn();
    }
    let (positions, tris) = geom.trimesh_data();
    zc.tris = tris.len();
    if tris.is_empty() {
        return;
    }
    zc.entity = Some(
        commands
            .spawn((
                RigidBody::Static,
                Collider::trimesh(positions, tris),
                Transform::default(),
            ))
            .id(),
    );
}

/// MoveAndSlide + SpatialQuery bundled so dispatch grows by one param only.
#[derive(bevy::ecs::system::SystemParam)]
pub struct AvianMoveParams<'w, 's> {
    pub mas: MoveAndSlide<'w, 's>,
    pub sq: SpatialQuery<'w, 's>,
}

fn capsule() -> Collider {
    Collider::capsule(RADIUS, SEG_LEN)
}

fn slide(mas: &MoveAndSlide, col: &Collider, from: Vec3, vel: Vec3, dt: f32) -> Vec3 {
    if vel.length_squared() < 1e-12 || dt <= 0.0 {
        return from;
    }
    let out = mas.move_and_slide(
        col,
        from,
        Quat::IDENTITY,
        vel,
        Duration::from_secs_f32(dt),
        &MoveAndSlideConfig::default(),
        &SpatialQueryFilter::default(),
        |_hit| MoveAndSlideHitResponse::Accept,
    );
    out.position
}

/// Vertical probe: distance the capsule travels along `dir` before contact,
/// capped at `max`. Raw shape cast — never slides sideways.
fn probe(sq: &SpatialQuery, col: &Collider, from: Vec3, dir: Dir3, max: f32) -> f32 {
    match sq.cast_shape(
        col,
        from,
        Quat::IDENTITY,
        dir,
        &ShapeCastConfig::from_max_distance(max),
        &SpatialQueryFilter::default(),
    ) {
        Some(hit) => hit.distance,
        None => max,
    }
}

/// Drop-in replacement for `MzbCollisionGeometry::wall_clip_wire`, same wire
/// contract: ffxi x/y horizontal, z vertical (grows DOWN); bevy x=x, z=-y,
/// y=-z (up).
pub fn wall_clip_avian(
    av: &AvianMoveParams,
    x: f32,
    y: f32,
    z: f32,
    dx: f32,
    dy: f32,
    dt: f32,
) -> WallClipResult {
    let col = capsule();
    let feet0 = -z;
    let start = Vec3::new(x, feet0 + HALF, -y);
    let want = Vec3::new(dx, 0.0, -dy);
    let want_len = want.length();
    if want_len < 1e-6 {
        return WallClipResult::none(dx, dy);
    }
    let dt = dt.max(1e-4);

    // 1) Horizontal move-and-slide.
    let p1 = slide(&av.mas, &col, start, want / dt, dt);
    let moved1 = Vec2::new(p1.x - start.x, p1.z - start.z).length();

    // 2) Swept stair step (up -> forward -> down) when the slide came up short.
    let mut p = p1;
    let short = want_len - moved1;
    if short > 1e-3 {
        let up = probe(&av.sq, &col, p1, Dir3::Y, MAX_STEP);
        if up > 1e-3 {
            let lifted = p1 + Vec3::Y * up;
            let dir2 = Vec3::new(want.x, 0.0, want.z).normalize_or_zero();
            let p2 = slide(&av.mas, &col, lifted, dir2 * (short / dt), dt);
            let fwd = Vec2::new(p2.x - lifted.x, p2.z - lifted.z).length();
            if fwd > 1e-3 {
                let down = probe(&av.sq, &col, p2, Dir3::NEG_Y, up + MAX_STEP);
                let p3 = p2 - Vec3::Y * down;
                // Only keep the step if it actually gained ground height;
                // otherwise it was a wall — keep the plain slide result.
                if p3.y > p1.y + 1e-3 {
                    p = p3;
                }
            }
        }
    }

    // 3) Ground snap: stick to the floor beneath (down-steps and slopes).
    let down = probe(&av.sq, &col, p, Dir3::NEG_Y, MAX_STEP);
    if down > 1e-4 && down < MAX_STEP {
        p -= Vec3::Y * down;
    }

    WallClipResult {
        dx: p.x - start.x,
        dy: -(p.z - start.z),
        landed_floor: Some(p.y - HALF),
    }
}
