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

use kuluu_render::components::{IsSelf, WorldEntity};
use kuluu_render::dat_mzb::{MzbCollisionGeometry, WallClipResult, MAX_GROUND_STEP_UP};

/// Collision classes in the unified avian world. Every collider carries
/// exactly one membership so the position resolver can ask "what did I hit"
/// and branch: walls and doors both block-and-slide (a door is a wall you
/// can't pass, not a hard freeze), mobs soft-block with push-through. The
/// resolver casts per-layer or reads the hit entity's layer to decide.
#[derive(PhysicsLayer, Default)]
pub enum GameLayer {
    /// Unclassified / default. Nothing meaningful should land here.
    #[default]
    Default,
    /// Static zone geometry (MZB walls, floors, ramps, stairs). Block + slide.
    Wall,
    /// MMB placements that block movement (doors, gates, solid furniture).
    /// Block + slide exactly like Wall, but a distinct class so it can never
    /// be treated as a mob (no push-through) and so doors can also count as
    /// FLOORS in the vertical pass (stand on a closed drawbridge).
    Door,
    /// Per-entity obstacle capsules (mobs/NPCs/players). Soft block: sustained
    /// forward pressure past a threshold excludes that one entity and you pass.
    Mob,
}

/// Membership helpers so collider spawns read clearly and stay consistent.
fn wall_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Wall, LayerMask::ALL)
}
fn mob_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Mob, LayerMask::ALL)
}

/// Capsule dimensions (bevy units = yalms). Radius matches the hand-rolled
/// walker's PLAYER_WALL_RADIUS; total height = 2*RADIUS + SEG_LEN.
pub const RADIUS: f32 = 0.4;
pub const SEG_LEN: f32 = 1.0;
/// Feet -> capsule center.
pub const HALF: f32 = RADIUS + SEG_LEN * 0.5;
/// Max riser a swept step may clear (MAX_GROUND_STEP_UP + slack).
pub const MAX_STEP: f32 = 0.45;
/// Steepest surface treated as walkable ground. 60deg: normal.y >= cos(60)=0.5.
pub const SLOPE_MAX_ANGLE: f32 = std::f32::consts::PI / 3.0;
/// Radius of the thin walkability/ground probe. The 0.8-wide walker capsule
/// grazes riser faces and misreads them as walls; a small sphere sees only
/// what's actually underfoot.
const THIN_R: f32 = 0.05;
/// Seconds of sustained forward pressure into the SAME mob before it stops
/// blocking (excluded from the sweep) and the player passes through. Retail
/// FFXI soft body-block.
pub const PUSH_THROUGH_SECS: f32 = 0.8;

pub struct AvianBridgePlugin;

impl Plugin for AvianBridgePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            .insert_resource(Gravity(Vec3::ZERO))
            .init_resource::<ZoneAvianCollider>()
            .add_observer(despawn_door_leaf_collider);
        // The three collider-sync systems (sync_zone_collider,
        // sync_mob_collider_radius, sync_mob_colliders) are scheduled in mod.rs
        // into FixedUpdate .before(dispatch_movement_system), NOT here in Update.
        // Reason: avian runs its physics + spatial-query pipeline in
        // FixedPostUpdate (which is BEFORE Update in the frame). The walker
        // (dispatch_movement_system) sweeps in FixedUpdate. If the colliders
        // synced in Update they'd land a full frame after the walker already
        // cast, so a just-spawned mob/door would be walk-through-able its first
        // tick. Ordering them before the walker in the same FixedUpdate makes
        // each collider present and positioned before the sweep.
    }
}

/// Schedules the three collider-sync systems into FixedUpdate before the walker.
/// Called from mod.rs where `dispatch_movement_system` is in scope.
pub fn add_collider_sync_systems(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        (
            sync_zone_collider,
            sync_door_leaf_colliders,
            sync_mob_collider_radius,
            sync_mob_colliders,
        )
            .before(super::input::dispatch_movement_system),
    );
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
        commands.entity(e).try_despawn();
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
                wall_layers(),
                Transform::default(),
            ))
            .id(),
    );
}

// ---------------------------------------------------------------------------
// REMOVED: sync_door_colliders (the MMB-visual -> Door-layer collider mirror).
//
// Why it's gone (Bastok Mines ROM/1/34.DAT, probed 2026-08-26): the MZB
// collision section is retail's authored truth of what blocks. 855 of 1525
// visual placements have an index-parallel collision object at the identical
// origin (walls, kabe-atariyou invisible collision helpers, door1/door2/
// doorstop); the other 670 are visual-only decor that retail NEVER collides
// (cho_door among them -- the chocobo door has NO collision object by
// authorial intent). dat_mzb's build_collision_geometry already instances all
// collision objects into the Wall-layer trimesh, so mirroring visual MMB
// meshes into physics (a) duplicated every real wall -- avian reported the
// MZB copy one tick ("wall-REAL") and the MMB copy the next ("door-REAL"),
// the coin-toss flip-flop -- and (b) invented blockers for the 670
// visual-only placements, which is where every "door in the wall" and the
// original chocobo-doorway phantom came from. Single collision authority =
// the MZB collision section, like retail.
//
// Openable-door state (door1/door2 etc., which ARE in the collision section)
// is future work: per-object suppression in MzbCollisionGeometry (the
// sub_area suppression machinery is the template), keyed by the
// collision-object index that is parallel to the named visual placement.
// ---------------------------------------------------------------------------

/// Links a door-leaf collider back to a named visual mesh (a submesh child
/// carrying MmbDebugInfo), so input.rs's debug lookup can print which door
/// blocked. Inserted by `sync_door_leaf_colliders`.
#[derive(Component)]
pub struct DoorColliderSource(pub Entity);

// ---------------------------------------------------------------------------
// Door-leaf colliders: the one place door SOLIDITY lives.
//
// A door in this engine is three things with three owners: the MESH (a `_`/`@`
// FourCC placement group; each drawn leaf carries ZoneDoorLeaf and is re-posed
// by apply_zone_door_stages as it swings), the STATE (the server door entity:
// kind Other + EntityLook::Door, whose door_id FourCC == the group's BlockID
// and whose animation byte drives the open/clos routines), and the COLLISION,
// which before this system was owned by NOBODY: the MZB gives door groups no
// collision by authorial intent (retail door solidity is dynamic), the old
// blanket MMB collider mirror is deleted, and the door entity itself is
// kind-Other so the mob path ignores it. Result: doors rendered, animated,
// and walk-through in every state.
//
// This system closes the gap: one standalone Door-layer trimesh per
// door-routine leaf, verts baked through the AUTHORED (closed) pose --
// mirror-correct via the full matrix, independent of the current swing --
// then toggled by the leaf's live pose: authored pose = closed = solid;
// any displacement (open or mid-swing) = ColliderDisabled = passable.
// Only groups with door open/clos routines qualify (doors.dir); other
// underscore families stay MZB-only.
// ---------------------------------------------------------------------------

/// On the leaf placement: its standalone collider entity, for streaming
/// teardown (see `despawn_door_leaf_collider`).
#[derive(Component)]
struct DoorLeafCollider(Entity);

fn sync_door_leaf_colliders(
    mut commands: Commands,
    doors: Res<kuluu_render::zone_doors::ZoneDoors>,
    meshes: Res<Assets<Mesh>>,
    to_build: Query<
        (Entity, &kuluu_render::zone_doors::ZoneDoorLeaf, &Children),
        Without<DoorLeafCollider>,
    >,
    mesh_children: Query<&Mesh3d>,
    built: Query<(&DoorLeafCollider, &kuluu_render::zone_doors::ZoneDoorLeaf)>,
    disabled_q: Query<&ColliderDisabled>,
) {
    // Build pass.
    for (leaf_ent, leaf, kids) in to_build.iter() {
        if doors.dir(leaf.four_cc).is_none() {
            continue; // not a door-routine group
        }
        let xform = leaf.posed_transform(kuluu_render::zone_doors::DoorPose::default());
        let mut verts: Vec<Vec3> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        let mut name_src: Option<Entity> = None;
        let mut ready = true;
        for child in kids.iter() {
            let Ok(m3) = mesh_children.get(child) else {
                continue;
            };
            let Some(mesh) = meshes.get(m3.0.id()) else {
                ready = false; // asset still loading: retry next tick
                break;
            };
            let Some(positions) = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
            else {
                continue;
            };
            let Some(indices) = mesh.indices() else {
                continue;
            };
            if name_src.is_none() {
                name_src = Some(child);
            }
            let base = verts.len() as u32;
            verts.extend(
                positions
                    .iter()
                    .map(|v| xform.transform_point3(Vec3::from_array(*v))),
            );
            let mut it = indices.iter();
            while let (Some(a), Some(b), Some(c)) = (it.next(), it.next(), it.next()) {
                tris.push([base + a as u32, base + b as u32, base + c as u32]);
            }
        }
        if !ready || tris.is_empty() {
            continue;
        }
        let collider = commands
            .spawn((
                RigidBody::Static,
                Collider::trimesh(verts, tris),
                CollisionLayers::new(GameLayer::Door, LayerMask::ALL),
                Transform::default(),
                DoorColliderSource(name_src.unwrap_or(leaf_ent)),
            ))
            .id();
        commands.entity(leaf_ent).insert(DoorLeafCollider(collider));
    }

    // Toggle pass: authored pose = closed = solid; any displacement = open.
    for (dc, leaf) in built.iter() {
        let pose = doors.pose(leaf.key());
        let closed = pose.rotation == Vec3::ZERO && pose.translation == Vec3::ZERO;
        let is_disabled = disabled_q.get(dc.0).is_ok();
        if closed == is_disabled {
            if let Ok(mut e) = commands.get_entity(dc.0) {
                if closed {
                    e.remove::<ColliderDisabled>();
                } else {
                    e.insert(ColliderDisabled);
                }
            }
        }
    }
}

/// Frees the standalone leaf collider when its leaf placement streams out
/// (a root entity is not caught by the placement's recursive despawn).
fn despawn_door_leaf_collider(
    trigger: On<Remove, DoorLeafCollider>,
    q: Query<&DoorLeafCollider>,
    mut commands: Commands,
) {
    if let Ok(dc) = q.get(trigger.event().event_target()) {
        // try_despawn: the zone sweep may have taken the collider already;
        // already-gone is a legitimate state here, not an error to log.
        commands.entity(dc.0).try_despawn();
    }
}

/// The texture-matched horizontal radius for this entity's mob obstacle
/// capsule, captured once from the live model AABB. The AABB lives on a mesh
/// DESCENDANT of the WorldEntity (WorldEntity -> actor_root -> mesh child),
/// updated every frame by `update_actor_mesh_aabbs`; we snapshot it once the
/// child exists rather than re-reading each frame (a walk cycle barely moves
/// the horizontal extent, and re-sizing a collider every tick thrashes the
/// broadphase).
#[derive(Component, Clone, Copy)]
struct MobColliderRadius {
    /// Horizontal capsule radius (wider of the two ground-plane half-extents).
    radius: f32,
    /// Vertical half-extent, for capsule segment length.
    half_height: f32,
}

/// Snapshot each non-self entity's model AABB into a `MobColliderRadius` once
/// its mesh descendant carrying the live `Aabb` exists. Separated from collider
/// spawn so the timing hazard (the Aabb child may not exist for the first
/// frames after the entity spawns) is handled by simply retrying until it does,
/// without repeatedly rebuilding a collider.
fn sync_mob_collider_radius(
    mut commands: Commands,
    entities: Query<
        (Entity, &WorldEntity, Option<&Children>),
        (Without<IsSelf>, Without<MobColliderRadius>),
    >,
    children_q: Query<&Children>,
    aabb_q: Query<&bevy::camera::primitives::Aabb>,
) {
    for (entity, _we, kids) in entities.iter() {
        let Some(kids) = kids else { continue };
        // Search descendants (actor_root -> mesh child) for the live Aabb.
        if let Some(aabb) = find_descendant_aabb(kids, &children_q, &aabb_q) {
            let he = aabb.half_extents;
            let radius = he.x.max(he.z);
            // Ignore degenerate/not-yet-posed bounds; retry next frame.
            if radius > 1e-3 && he.y > 1e-3 {
                commands.entity(entity).insert(MobColliderRadius {
                    radius,
                    half_height: he.y,
                });
            }
        }
    }
}

/// Depth-first search of an entity's descendants for the first `Aabb`.
fn find_descendant_aabb(
    kids: &Children,
    children_q: &Query<&Children>,
    aabb_q: &Query<&bevy::camera::primitives::Aabb>,
) -> Option<bevy::camera::primitives::Aabb> {
    for child in kids.iter() {
        if let Ok(aabb) = aabb_q.get(child) {
            return Some(*aabb);
        }
        if let Ok(grandkids) = children_q.get(child) {
            if let Some(found) = find_descendant_aabb(grandkids, children_q, aabb_q) {
                return Some(found);
            }
        }
    }
    None
}

/// Links a visual entity to its separate Mob-collider entity. The collider is
/// NOT a component on the visual: the visual's Transform is written every frame
/// by the scene sync from server data, and avian's PhysicsTransformPlugin also
/// wants to own the Transform of any body — two writers, one Transform, the
/// exact conflict this whole unification exists to kill. So the obstacle
/// collider lives on its own entity that only avian + this system touch, parked
/// each frame at the visual's position.
#[derive(Component)]
struct MobColliderLink(Entity);

/// Marks a spawned collider entity's owner so it can be despawned when the
/// visual goes away.
#[derive(Component)]
pub struct MobColliderOwner(Entity);

/// Spawn a KINEMATIC Mob-layer capsule on a SEPARATE entity for each non-self
/// visual with a known model radius, and keep it parked at the visual's current
/// position each frame by writing the collider's Transform (avian's
/// PhysicsTransformPlugin syncs Position/Rotation from it). Kinematic because
/// the server owns mob position; the mob is a pure obstacle for the player's
/// sweep, never simulated. IsSelf is excluded. When the visual despawns, its
/// collider entity is despawned too (owner link).
fn sync_mob_colliders(
    mut commands: Commands,
    to_build: Query<
        (Entity, &Transform, &MobColliderRadius),
        (
            Without<IsSelf>,
            Without<MobColliderLink>,
            Without<MobColliderOwner>,
        ),
    >,
    visuals: Query<
        (&Transform, &MobColliderRadius),
        (
            With<MobColliderLink>,
            Without<IsSelf>,
            Without<MobColliderOwner>,
        ),
    >,
    links: Query<(Entity, &MobColliderLink)>,
    mut collider_tf: Query<
        &mut Transform,
        (
            With<MobColliderOwner>,
            Without<MobColliderLink>,
            Without<IsSelf>,
        ),
    >,
    owners: Query<(Entity, &MobColliderOwner)>,
) {
    // Spawn a separate collider entity for each newly-sized visual.
    // The visual transform sits at the FEET and the model AABB extends up from
    // there, but a capsule is centered on its Transform — parking it raw puts
    // every mob half-sunk with an effective top of half_height instead of
    // 2 x half_height. Center it on the AABB (feet + half_height), exactly like
    // the player capsule's feet0 + HALF.
    for (visual, t, r) in to_build.iter() {
        let seg = (r.half_height * 2.0 - r.radius * 2.0).max(0.05);
        let center = t.translation + Vec3::Y * r.half_height;
        let collider = commands
            .spawn((
                RigidBody::Kinematic,
                Collider::capsule(r.radius, seg),
                mob_layers(),
                Transform::from_translation(center),
                MobColliderOwner(visual),
            ))
            .id();
        commands.entity(visual).insert(MobColliderLink(collider));
    }
    // Park each collider on its visual's current position (AABB-centered, as above).
    for (visual, link) in links.iter() {
        let Ok((vt, r)) = visuals.get(visual) else {
            continue;
        };
        if let Ok(mut ct) = collider_tf.get_mut(link.0) {
            // A half-sunk short mob only touches the player's bottom cap: a steep
            // normal that slide_walls_only classifies as walkable floor — which is
            // why collision "only worked on some mobs / some approach angles".
            ct.translation = vt.translation + Vec3::Y * r.half_height;
        }
    }
    // Despawn colliders whose visual is gone.
    for (collider, owner) in owners.iter() {
        if visuals.get(owner.0).is_err() && links.get(owner.0).is_err() {
            // try_despawn: at zone boundaries the teardown can race this
            // sweep for the same collider; second-in-line must be silent.
            commands.entity(collider).try_despawn();
        }
    }
}

/// MoveAndSlide + SpatialQuery bundled so dispatch grows by one param only.
#[derive(bevy::ecs::system::SystemParam)]
pub struct AvianMoveParams<'w, 's> {
    pub mas: MoveAndSlide<'w, 's>,
    pub sq: SpatialQuery<'w, 's>,
    pub geom: Res<'w, MzbCollisionGeometry>,
    /// Mob collider -> actor link, for the body-block test.
    pub mob_owner: Query<'w, 's, &'static MobColliderOwner>,
    /// Actor kind ([obj] never body-blocks), for the body-block test.
    pub world_ents: Query<'w, 's, &'static WorldEntity>,
    /// Descendant walk for the drawn test (same walk the radius snapshot does).
    pub children: Query<'w, 's, &'static Children>,
    /// "Is the texture drawn": a rendered mesh in the actor's subtree.
    pub mesh_vis: Query<'w, 's, &'static InheritedVisibility, With<Mesh3d>>,
}

/// Should this mob collider body-block the walker at all? Rule, in order:
///   1. EntityKind::Other -- the HUD's "[obj]": door objects, "???" points,
///      event triggers. These NEVER body-block, whatever mesh they carry;
///      retail does not collide with object entities. (The invisible
///      "? [obj]" plaza blocker is this class: real entity, real placeholder
///      mesh, draws nothing.)
///   2. Character kinds block only when their texture is actually drawn: a
///      rendered mesh in the actor's subtree (same descendant walk the radius
///      snapshot does; InheritedVisibility so a real mob doesn't turn
///      walk-through when the camera looks away). Undrawn actor = invisible
///      entity = walk through.
fn mob_body_blocks(av: &AvianMoveParams, collider_ent: Entity) -> bool {
    let Ok(owner) = av.mob_owner.get(collider_ent) else {
        return false;
    };
    if let Ok(we) = av.world_ents.get(owner.0) {
        if matches!(we.kind, kuluu_snapshot::EntityKind::Other) {
            return false;
        }
    }
    let Ok(kids) = av.children.get(owner.0) else {
        return false;
    };
    drawn_mesh_in(kids, av)
}

fn drawn_mesh_in(kids: &Children, av: &AvianMoveParams) -> bool {
    for child in kids.iter() {
        if let Ok(vis) = av.mesh_vis.get(child) {
            if vis.get() {
                return true;
            }
        }
        if let Ok(k) = av.children.get(child) {
            if drawn_mesh_in(k, av) {
                return true;
            }
        }
    }
    false
}

fn capsule() -> Collider {
    Collider::capsule(RADIUS, SEG_LEN)
}

/// Vertical probe: distance the capsule travels along `dir` before contact,
/// capped at `max`, restricted to the given layer mask. Raw shape cast.
#[allow(dead_code)]
fn probe(
    sq: &SpatialQuery,
    col: &Collider,
    from: Vec3,
    dir: Dir3,
    max: f32,
    mask: LayerMask,
) -> f32 {
    match sq.cast_shape(
        col,
        from,
        Quat::IDENTITY,
        dir,
        &ShapeCastConfig::from_max_distance(max),
        &SpatialQueryFilter::from_mask(mask),
    ) {
        Some(hit) => hit.distance,
        None => max,
    }
}

/// A thin sphere collider for walkability sampling (see THIN_R).
#[allow(dead_code)]
fn thin_probe() -> Collider {
    Collider::sphere(THIN_R)
}

/// True when a surface normal is close enough to straight up to be walkable.
#[allow(dead_code)]
fn is_walkable(normal: Vec3) -> bool {
    normal.y >= SLOPE_MAX_ANGLE.cos()
}

/// Ground normal under `center`, sampled with a thin sphere against WALL+DOOR
/// (doors are floors too). None if nothing within `max`. The wide walker
/// capsule grazes riser faces and misreads them as walls; the thin sphere sees
/// only what's underfoot.
#[allow(dead_code)]
fn ground_normal(sq: &SpatialQuery, center: Vec3, max: f32) -> Option<Vec3> {
    let hit = sq.cast_shape(
        &thin_probe(),
        center,
        Quat::IDENTITY,
        Dir3::NEG_Y,
        &ShapeCastConfig::from_max_distance(max),
        &SpatialQueryFilter::from_mask(ground_mask()),
    )?;
    Some(hit.normal1.into())
}

/// Multi-sampled walkability for a swept-step landing: center and +/- along
/// travel. Walkable if ANY sample passes (a tread edge always has one probe
/// mid-tread); a uniform steep wall fails all three. Permissive on all-miss.
#[allow(dead_code)]
fn landing_walkable(sq: &SpatialQuery, at: Vec3, dir_xz: Vec3) -> bool {
    const SPREAD: f32 = 0.15;
    let max = HALF + MAX_STEP;
    let mut any_hit = false;
    for off in [Vec3::ZERO, dir_xz * SPREAD, dir_xz * -SPREAD] {
        if let Some(n) = ground_normal(sq, at + off, max) {
            any_hit = true;
            if is_walkable(n) {
                return true;
            }
        }
    }
    !any_hit
}

/// Layer mask for ground/floor: walls and doors (a closed drawbridge is floor).
fn ground_mask() -> LayerMask {
    LayerMask::from([GameLayer::Wall, GameLayer::Door])
}

/// Layer mask for the camera boom: walls and doors block the camera; mobs
/// never do (you always see through/past creatures). Same solid world the
/// walker collides against — one collision authority for movement AND camera.
pub fn camera_mask() -> LayerMask {
    LayerMask::from([GameLayer::Wall, GameLayer::Door])
}
/// Layer mask for obstacle bodies: mobs only.
fn mob_mask() -> LayerMask {
    LayerMask::from([GameLayer::Mob])
}

/// Layer mask for doors only.
fn door_mask() -> LayerMask {
    LayerMask::from([GameLayer::Door])
}

/// True if a capsule sweep from `start` along `want` hits anything in `mask`.
/// (Superseded by entity_in_layer for stop classification; kept for reference.)
#[allow(dead_code)]
fn layer_ahead(
    sq: &SpatialQuery,
    col: &Collider,
    start: Vec3,
    want: Vec3,
    mask: LayerMask,
) -> bool {
    let len = want.length();
    if len < 1e-6 {
        return false;
    }
    let Ok(dir) = Dir3::new(want / len) else {
        return false;
    };
    sq.cast_shape(
        col,
        start,
        Quat::IDENTITY,
        dir,
        &ShapeCastConfig::from_max_distance(len),
        &SpatialQueryFilter::from_mask(mask),
    )
    .is_some()
}

/// Does `ent` belong to `mask`? Casts mask-only along the move and checks the
/// hit entity IS `ent`. This classifies the SPECIFIC entity avian stopped us on
/// -- not any door/mob somewhere in the path -- killing false positives.
fn entity_in_layer(
    sq: &SpatialQuery,
    col: &Collider,
    start: Vec3,
    want: Vec3,
    mask: LayerMask,
    ent: Entity,
) -> bool {
    let len = want.length();
    if len < 1e-6 {
        return false;
    }
    let Ok(dir) = Dir3::new(want / len) else {
        return false;
    };
    sq.cast_shape(
        col,
        start,
        Quat::IDENTITY,
        dir,
        &ShapeCastConfig::from_max_distance(len),
        &SpatialQueryFilter::from_mask(mask),
    )
    .map(|h| h.entity == ent)
    .unwrap_or(false)
}

/// The single horizontal-obstacle question: cast the capsule along `want` from
/// `start` and report the FIRST thing hit and how far. Doors and walls are one
/// slide class; mobs are their own. Returns (hit distance, hit entity, is_mob).
/// None = clear path.
fn horizontal_obstacle(
    sq: &SpatialQuery,
    col: &Collider,
    start: Vec3,
    want: Vec3,
    excluded: Option<Entity>,
) -> Option<(f32, Entity, bool)> {
    let len = want.length();
    if len < 1e-6 {
        return None;
    }
    let dir = Dir3::new(want / len).ok()?;
    // Cast against walls+doors+mobs together; nearest hit wins.
    let mut filter = SpatialQueryFilter::from_mask(LayerMask::from([
        GameLayer::Wall,
        GameLayer::Door,
        GameLayer::Mob,
    ]));
    if let Some(e) = excluded {
        filter = filter.with_excluded_entities([e]);
    }
    let hit = sq.cast_shape(
        col,
        start,
        Quat::IDENTITY,
        dir,
        &ShapeCastConfig::from_max_distance(len),
        &filter,
    )?;
    // Is the hit entity a mob? Test its membership by re-casting mob-only to the
    // same distance and seeing if the same entity is the mob-layer hit. Simpler:
    // record the entity and let the caller classify via a mob-only probe.
    // Here we approximate: a hit within a mob-only cast at <= this distance and
    // same entity means mob.
    let mob_here = {
        let mut mf = SpatialQueryFilter::from_mask(mob_mask());
        if let Some(e) = excluded {
            mf = mf.with_excluded_entities([e]);
        }
        sq.cast_shape(
            col,
            start,
            Quat::IDENTITY,
            dir,
            &ShapeCastConfig::from_max_distance(len),
            &mf,
        )
        .map(|mh| mh.entity == hit.entity)
        .unwrap_or(false)
    };
    Some((hit.distance, hit.entity, mob_here))
}

/// Per-entity push-through accrual: which mob the player is currently shoving,
/// and for how long. Lives in `dispatch_movement_system` as a Local and is
/// passed into the resolver.
#[derive(Default)]
pub struct PushThrough {
    pub target: Option<Entity>,
    pub secs: f32,
}

impl PushThrough {
    /// Register a block against `mob` this tick; returns true if the mob should
    /// now be pushed through (excluded from the sweep).
    fn press(&mut self, mob: Entity, dt: f32) -> bool {
        if self.target == Some(mob) {
            self.secs += dt;
        } else {
            self.target = Some(mob);
            self.secs = dt;
        }
        self.secs >= PUSH_THROUGH_SECS
    }
    fn release(&mut self) {
        self.target = None;
        self.secs = 0.0;
    }
}

/// THE resolver. One authority, two passes, fixed priority. Replaces the old
/// `wall_clip_avian`. Wire contract unchanged: ffxi x/y horizontal, z vertical
/// (grows DOWN); bevy x=x, z=-y, y=-z (up).
///
/// Pass A (horizontal, never touches Y): slide on walls+doors (no height gain
/// -> no goat), soft-block on mobs (push-through after PUSH_THROUGH_SECS).
/// Pass B (vertical, never touches XZ): settle on walls+doors within a step,
/// swept-step climb gated by climb-slope, else fall to real ground/door-floor.
pub fn resolve_position(
    av: &AvianMoveParams,
    push: &mut PushThrough,
    x: f32,
    y: f32,
    z: f32,
    dx: f32,
    dy: f32,
    dt: f32,
    // OUT: the one detect_stairs result this tick, for asp + HUD to read (dedup).
    det_out: &mut Option<super::input::StairDetection>,
    // OUT: when the block was classified as a door, the door entity, so the
    // caller can resolve its mesh/texture name for debug.
    door_ent_out: &mut Option<Entity>,
) -> WallClipResult {
    let col = capsule();
    let feet0 = -z;
    let start = Vec3::new(x, feet0 + HALF, -y);
    let want = Vec3::new(dx, 0.0, -dy);
    let want_len = want.length();
    let dt = dt.max(1e-4);

    // Ceiling to cast ground rays down from: a bit above the head.
    // The floor PLANE: cast down from body-center (feet + 1.0) to the floor at
    // the player's XZ. Body-relative origin, so it is stable whether the player
    // is grounded OR in the air -- the plane does not move with the (possibly
    // airborne) live Y. Everything below references THIS, not the live position,
    // so the height output can't feed back into its own input.
    let body_center_y = feet0 + 1.0;
    let plane_y = av
        .geom
        .ground_raycast(Vec2::new(x, -y), body_center_y)
        .unwrap_or(feet0);

    // THE ONE detect_stairs call this tick (word of god). Runs from the stable
    // plane at the CURRENT xz. apply_self_prediction_system and the HUD read
    // this same result via LastStairDetection -- no duplicate raycasting.
    let det = super::input::detect_stairs(Vec3::new(x, plane_y, -y), &av.geom);
    *det_out = Some(det);
    *door_ent_out = None;

    // ---- STOPPED (no input): settle onto the actual tread ----
    // While moving we ride the smooth footprint ramp (below); the instant input
    // stops we drop onto the real stepped surface underneath. The 0.2 up/down
    // guard: ground_step accepts a floor up to MAX_GROUND_STEP_UP above the feet
    // (rise onto a tread you are wedged just below) and any distance below
    // (fall to the tread). This is what "fall to the tread when you stop" means,
    // and the up-accept keeps you from sinking through the stairs.
    if want_len < 1e-6 {
        push.release();
        let floor = av
            .geom
            .ground_step(Vec2::new(x, -y), feet0, MAX_GROUND_STEP_UP)
            .or_else(|| av.geom.ground_nearest(Vec2::new(x, -y), feet0));
        return WallClipResult {
            dx: 0.0,
            dy: 0.0,
            landed_floor: floor,
            dbg_is_a_stop: false,
            dbg_stop_slope: false,
            dbg_slope_angle: 0.0,
            dbg_stop_steps: false,
            dbg_tall_wall: false,
            dbg_step_slope: 0.0,
            dbg_step_height: 0.0,
            dbg_stop_wall: false,
            dbg_wall_height: 0.0,
            dbg_stop_door: false,
            dbg_stop_mob: false,
            dbg_soft_timer: 0.0,
            dbg_block_nx: 0.0,
            dbg_block_ny: 0.0,
            dbg_block_nz: 0.0,
            dbg_reason: "stopped-input",
            dbg_hit_x: 0.0,
            dbg_hit_y: 0.0,
            dbg_hit_z: 0.0,
        };
    }

    // =====================================================================
    // ORCHESTRATION (word of god). One ordered sequence, one authority. The
    // slide is STEP ONE; its result flows into ONE priority-ordered
    // classification that makes ONE decision; step three assembles the result.
    // Nothing overrides anything after the fact. The debug flags are set by the
    // SAME classification, so the HUD can never disagree with what moved us.
    // =====================================================================
    const STEP_HEIGHT: f32 = 0.4; // max auto-climb
    const SNAP_DOWN: f32 = 0.4; // ground-snap reach below feet
    let move_dir = Vec2::new(want.x, want.z).normalize_or_zero();
    let here = Vec2::new(x, -y);

    // debug accumulators (set by the single classification below)
    let mut dbg_is_a_stop = false;
    let mut dbg_stop_slope = false;
    let mut dbg_slope_angle = 0.0f32;
    let mut dbg_stop_steps = false;
    let mut dbg_step_slope = 0.0f32;
    let mut dbg_step_height = 0.0f32;
    let mut dbg_stop_wall = false;
    let mut dbg_wall_height = 0.0f32;
    let mut dbg_stop_door = false;
    let mut dbg_stop_mob = false;
    let dbg_soft_timer = (PUSH_THROUGH_SECS - push.secs).max(0.0);
    let mut dbg_reason: &'static str = "moving-free";

    // ---- STEP 1: SLIDE (the orchestration runs avian, once) ----------------
    // Mob push-through accrual decides whether one mob entity is excluded, then
    // we build the filter and slide. slide_walls_only returns the moved position
    // AND the first blocking (non-walkable) contact normal (None = not stopped).
    let mut excluded_mob: Option<Entity> = None;
    if let Some((_d, ent, is_mob)) = horizontal_obstacle(&av.sq, &col, start, want, None) {
        if is_mob {
            if !mob_body_blocks(av, ent) {
                // No drawn texture on the actor = invisible server entity
                // (event trigger, door object). Those never body-block:
                // exclude instantly, no push-through timer. Walls behind
                // still apply -- the slide runs with only this one entity
                // excluded from the filter.
                excluded_mob = Some(ent);
                push.release();
            } else if push.press(ent, dt) {
                excluded_mob = Some(ent);
            }
        } else {
            push.release();
        }
    } else {
        push.release();
    }
    let mut hfilter = SpatialQueryFilter::from_mask(LayerMask::from([
        GameLayer::Wall,
        GameLayer::Door,
        GameLayer::Mob,
    ]));
    if let Some(e) = excluded_mob {
        hfilter = hfilter.with_excluded_entities([e]);
    }
    let mut block_normal: Option<Vec3> = None;
    let mut block_entity: Option<Entity> = None;
    let mut block_point: Option<Vec3> = None;
    let p1 = slide_walls_only(
        &av.mas,
        &col,
        start,
        want / dt,
        dt,
        &hfilter,
        &mut block_normal,
        &mut block_entity,
        &mut block_point,
    );
    let slide_xz = Vec2::new(p1.x, p1.z);

    // ---- STEP 2: CLASSIFY (ONE decision, priority order) -------------------
    // Inputs: the slide result (slide_xz, block_normal) + the detector (det).
    // We pick exactly ONE outcome and set (move_xz, final_feet, debug) from it.
    // Priority: slope-ride(pre-triggered) > stairs-ahead > blocked(door>mob>wall)
    // > free-walk. The ride's result is collected separately (ride_result) and
    // applied AFTER the chain, so no branch ever reassigns a decided tick —
    // one decision per tick, and definite-assignment stays provable.
    let mut move_xz;
    let mut final_feet;

    // Avian absorbed most of the requested move without necessarily capturing
    // a block: a rounded-bottom contact on a low sill reads walkable -> Ignore,
    // the capsule embeds, and the slide comes back truncated with no evidence.
    // This flag is what lets the lip ride below rescue that silent stop.
    let want_len = Vec2::new(want.x, want.z).length();
    let slide_len = (slide_xz - Vec2::new(start.x, start.z)).length();
    let slide_truncated = want_len > 1e-4 && slide_len < want_len * 0.6;

    // (a) Walkable stairs OR a small lip ahead. Two detector signals ride:
    //     - banded risers (band != 0): real staircases. Avian sees each riser
    //       as a vertical wall every tick; letting the block win = stop/go.
    //     - sub-band lips (band == 0, small positive rise): door sills and
    //       thresholds below the detector's riser quantum -- the HUD's GRAY
    //       orbs. Avian has NO step height, so a 0.1-yalm sill hard-stops the
    //       capsule silently ("moving-free" with a zero delta). Retail walks
    //       straight over these. Lips ride ONLY when the slide was actually
    //       truncated, so this rescues a stuck capsule and never bypasses
    //       normal wall sliding on gently uneven ground.
    //     The detector is the authority on "is this walkable" for both.
    let mut lip_h: f32 = 0.0;
    let stairs_ahead = {
        let mut found = false;
        for &(oxz, oy, _g, band) in det.sample_data.iter() {
            if oy.is_nan() {
                continue; // invalid sample
            }
            let along = (oxz - here).dot(move_dir);
            if band == 0 {
                // Sub-band lip: UP only (drops are ground-snap's job), CLOSE
                // ahead only (step when we reach it, not from a yalm out).
                let rise = oy - plane_y;
                if along >= -0.2 && along <= 0.9 && rise > 0.02 && rise <= STEP_HEIGHT {
                    lip_h = lip_h.max(rise);
                }
                continue;
            }
            let rise = (oy - plane_y).abs(); // up OR down both walkable
            if along >= -0.2 && rise > 0.02 && rise <= STEP_HEIGHT * 3.0 {
                found = true;
                break;
            }
        }
        found
    };
    let lip_ride = !stairs_ahead && lip_h > 0.0 && slide_truncated;

    // TALL-WALL VETO: a stair riser tops out at STEP_HEIGHT, so a ray fired
    // JUST above that height, a hand's width ahead, can only hit something
    // taller than a step -- a wall. Without this, the detector seeing treads
    // on the far side of a thin side wall rode the walker straight through
    // it. The reach is tight (RADIUS + 0.15) so the SECOND riser of a real
    // staircase -- one tread deeper, the first thing tall enough to cross
    // this ray -- stays out of range and never vetoes legitimate climbing.
    // `det.ramp_locked` included too: the pre-triggered slope-ride engages BEFORE a
    // riser is close enough for ring samples, so the veto must fire then as well —
    // otherwise we'd ride the walker straight through a wall standing between us and
    // the staircase.
    let tall_wall_before_step = (stairs_ahead || det.ramp_locked || lip_h > 0.0)
        && Dir3::new(Vec3::new(move_dir.x, 0.0, move_dir.y))
            .ok()
            .is_some_and(|d| {
                av.sq
                    .cast_ray(
                        Vec3::new(start.x, plane_y + STEP_HEIGHT + 0.01, start.z),
                        d,
                        RADIUS + 0.15,
                        true,
                        &SpatialQueryFilter::from_mask(LayerMask::from([
                            GameLayer::Wall,
                            GameLayer::Door,
                        ])),
                    )
                    // A hit on a WALKABLE face (normal within SLOPE_MAX_ANGLE of
                    // up) is a slope we can ride, not a wall: no veto. Only steep
                    // faces (>60deg from vertical) block the step/ride.
                    .is_some_and(|hit| hit.normal.y < SLOPE_MAX_ANGLE.cos())
            });

    // (a0) SLOPE-RIDE — continuous stair follow, pre-triggered. When the detector
    // holds a locked ramp line (march-measured or fit), feet ride THAT LINE instead
    // of snapping to det.ramp_near.1 per tick: wire Y advances at slope × progress,
    // so treads are one continuous incline on the WIRE (collision + c2s 0x015), not
    // just in render — ground_step never gets a turn mid-stair, which is what used to
    // force us down onto the slab under a buried staircase. The rise is anchored at
    // the FIRST RISER (march_first_riser_rel): the flat approach before it stays at
    // current foot level exactly, so the follow engages while we are still walking UP
    // TO the steps (no float over the last flat strip).
    // HOLE DETECTOR: a down-ray at the destination xz must find floor within
    // STAIR_HOLE_DROP of the ride line; a missing tread / open hole drops us through
    // via ground-snap — exactly what plain walking did before slope-ride existed.
    const STAIR_HOLE_DROP: f32 = 0.5;
    // Some(destination, feet) when the ride decides this tick; resolved after
    // the legacy chain below (it outranks every arm).
    let mut ride_result: Option<(Vec2, f32)> = None;
    let mut ride_hole_fall = false;
    if det.ramp_locked && !tall_wall_before_step {
        let target_xz = Vec2::new(start.x + want.x, start.z + want.z);
        // Measured ground a step behind us along the ramp direction: tells the
        // ride whether we're already ON a rising surface (smooth-ramp anchor).
        let behind_y = av
            .geom
            .ground_raycast(
                Vec2::new(start.x - move_dir.x * 0.15, start.z - move_dir.y * 0.15),
                plane_y + 2.0,
            )
            .unwrap_or(f32::NAN);
        // Measured ground at the destination: "the step is soon" engagement.
        let dest_y = av.geom.ground_raycast(target_xz, plane_y + 2.0).unwrap_or(f32::NAN);
        if let Some(pred) = slope_ride_feet(&det, plane_y, here, target_xz, behind_y, dest_y) {
            if av
                .geom
                .ground_raycast(target_xz, pred.max(plane_y) + 0.5)
                .is_some_and(|actual| actual >= pred - STAIR_HOLE_DROP)
            {
                dbg_reason = "slope-ride";
                dbg_is_a_stop = true;
                dbg_stop_steps = true;
                // Continuous-ride tick: no discrete step happened, so dispatch must not
                // re-arm the stair-settle dip clamp — it would swallow our small per-tick
                // descent deltas (pre-ride code produced per-riser jumps that cleared the
                // 0.08 gate).
                dbg_stop_slope = true;
                dbg_step_height = pred - plane_y;
                dbg_step_slope = det.best_slope;
                dbg_slope_angle = det.best_slope.atan().to_degrees();
                ride_result = Some((target_xz, pred)); // resolved after the chain below
            } else {
                // Hole in the stair: no real floor under the ride line at the
                // destination. Disengage; ground-snap (below) drops us through.
                dbg_reason = "stair-hole-fall";
                ride_hole_fall = true;
            }
        }
    }

    // Hole-fall ticks skip the hold arm A as well: its ramp_near flat-hold would
    // float us across a missing tread instead of letting ground-snap drop us.
    if ride_result.is_none()
        && !ride_hole_fall
        && (stairs_ahead || lip_ride)
        && !tall_wall_before_step
    {
        if stairs_ahead {
            dbg_reason = "stairs-ahead";
            dbg_step_height = det.ramp_near.1 - plane_y;
            dbg_step_slope = det.best_slope;
            dbg_slope_angle = det.best_slope.atan().to_degrees();
            final_feet = det.ramp_near.1;
        } else {
            // Lip-step: full forward move, feet lifted onto the sill this
            // tick so the capsule clears the edge (no embed, no truncation).
            dbg_reason = "lip-step";
            dbg_step_height = lip_h;
            final_feet = plane_y + lip_h;
        }
        dbg_is_a_stop = true;
        dbg_stop_steps = true;
        move_xz = Vec2::new(start.x + want.x, start.z + want.z);
    } else if let Some(n) = block_normal {
        // (b) BLOCKED by something that is not a walkable staircase. Classify the
        //     SPECIFIC entity avian stopped us on (block_entity).
        dbg_is_a_stop = true;

        // GROUND TRUTH: cast a clean forward ray a short distance against
        // walls+doors. If NOTHING is really in front of us, avian's move_and_slide
        // fabricated the contact (depenetration artifact) -- we should NOT block.
        let probe_from = Vec3::new(start.x, start.y, start.z);
        let clean_hit = Dir3::new(want / want.length().max(1e-6))
            .ok()
            .and_then(|d| {
                av.sq.cast_ray(
                    probe_from,
                    d,
                    RADIUS + 0.6, // just in front (capsule radius + a little)
                    true,
                    &SpatialQueryFilter::from_mask(LayerMask::from([
                        GameLayer::Wall,
                        GameLayer::Door,
                    ])),
                )
            });
        // Record for debug: did the clean forward ray actually find a face?
        let real = clean_hit.is_some();

        let ent = block_entity;
        let door_hit =
            ent.is_some_and(|e| entity_in_layer(&av.sq, &col, start, want, door_mask(), e));
        let mob_hit = excluded_mob.is_none()
            && ent.is_some_and(|e| entity_in_layer(&av.sq, &col, start, want, mob_mask(), e));

        if door_hit {
            dbg_reason = if real { "door-REAL" } else { "door-PHANTOM" };
            dbg_stop_door = true;
            *door_ent_out = ent;
            move_xz = slide_xz;
            final_feet = det.center_y;
        } else if mob_hit {
            // REAL/PHANTOM by the drawn test, NOT the forward ray: the ray only
            // sees Wall+Door, so a mob in open space always read "PHANTOM",
            // visible or not. Drawn actor = real body = soft block. Undrawn =
            // invisible entity = walk through. (This arm normally only fires
            // for a second undrawn mob behind one already excluded pre-slide.)
            if ent.is_some_and(|e| mob_body_blocks(av, e)) {
                dbg_reason = "mob-REAL";
                dbg_stop_mob = true;
                move_xz = slide_xz;
                final_feet = det.center_y;
            } else {
                dbg_reason = "mob-PHANTOM-pass";
                dbg_is_a_stop = false;
                let dest = Vec2::new(start.x + want.x, start.z + want.z);
                move_xz = dest;
                final_feet = av
                    .geom
                    .ground_step(dest, plane_y, SNAP_DOWN)
                    .unwrap_or(det.center_y);
            }
        } else {
            // STEP-UP RESCUE: the slide was blocked, but there is walkable
            // ground within MAX_GROUND_STEP_UP above our feet just ahead — that's
            // a riser or lip, not a wall. Shallow risers (rise < capsule radius)
            // wedge the rounded bottom in their corner and avian reports blended
            // edge normals (n.y ~0.4) that read as "wall"; the tall-wall veto can
            // also hold the step arms off when the NEXT riser is within reach on
            // narrow treads. The legacy walker stepped these every time: climb.
            // Gated hard — only a floor actually WITHIN step reach of our feet,
            // so a plain wall stop never gets its height forced by ground_step.
            let dest = Vec2::new(start.x + want.x, start.z + want.z);
            if let Some(floor) = av.geom.ground_step(dest, plane_y, MAX_GROUND_STEP_UP)
                .filter(|&floor| floor > plane_y + 0.02)
            {
                dbg_reason = "step-up-rescue";
                dbg_stop_steps = true;
                move_xz = dest; // full forward move, feet lifted — no embed
                final_feet = floor;
            } else {
                dbg_reason = if real { "wall-REAL" } else { "wall-PHANTOM" };
                let angle = n.y.clamp(-1.0, 1.0).acos();
                dbg_stop_wall = true;
                dbg_slope_angle = angle.to_degrees();
                dbg_wall_height = 1.0;
                move_xz = slide_xz;
                final_feet = det.center_y;
            }
        }
    } else {
        // (c) FREE WALK: not stopped, no stairs. Take avian's slide result and
        //     snap to the ground under the new position.
        move_xz = slide_xz;
        // A hole-fall disengage skips the locked-ramp flat hold: the ground-snap
        // below is what drops us through the gap.
        if det.ramp_locked && !ride_hole_fall {
            dbg_stop_slope = true; // informational: walking a locked ramp
            dbg_slope_angle = det.best_slope.atan().to_degrees();
            final_feet = det.ramp_near.1;
        } else if let Some(g) = av.geom.ground_step(slide_xz, plane_y, SNAP_DOWN) {
            final_feet = g;
        } else {
            final_feet = det.center_y;
        }
    }

    // SLOPE-RIDE outranks every legacy arm above (one decision per tick): when
    // it fired, its result replaces whatever the chain assigned. Without this a
    // successful ascent tick fell into C's flat hold, which froze wire Y at
    // current foot level — the climb would only ever happen in render.
    if let Some((r_xz, r_feet)) = ride_result {
        move_xz = r_xz;
        final_feet = r_feet;
    }

    // TEMP (stair diagnosis): console trace of decision flips — a flickering
    // ramp lock shows up as slope-ride/stairs-ahead alternating line by line.
    // Remove once the stair work is verified in-game.
    {
        static LAST_REASON: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(usize::MAX);
        let cur = dbg_reason.as_bytes().as_ptr() as usize;
        if LAST_REASON.swap(cur, std::sync::atomic::Ordering::Relaxed) != cur {
            tracing::info!(
                reason = dbg_reason,
                slope_deg = dbg_slope_angle,
                step_h = dbg_step_height,
                "stair decision change",
            );
        }
    }

    // ---- STEP 3: ASSEMBLE --------------------------------------------------
    WallClipResult {
        dx: move_xz.x - start.x,
        dy: -(move_xz.y - start.z),
        landed_floor: Some(final_feet),
        dbg_is_a_stop,
        dbg_stop_slope,
        dbg_slope_angle,
        dbg_stop_steps,
        dbg_tall_wall: tall_wall_before_step,
        dbg_step_slope,
        dbg_step_height,
        dbg_stop_wall,
        dbg_wall_height,
        dbg_stop_door,
        dbg_stop_mob,
        dbg_soft_timer,
        dbg_block_nx: block_normal.map(|n| n.x).unwrap_or(0.0),
        dbg_block_ny: block_normal.map(|n| n.y).unwrap_or(0.0),
        dbg_block_nz: block_normal.map(|n| n.z).unwrap_or(0.0),
        dbg_reason,
        dbg_hit_x: block_point.map(|p| p.x).unwrap_or(0.0),
        dbg_hit_y: block_point.map(|p| p.y).unwrap_or(0.0),
        dbg_hit_z: block_point.map(|p| p.z).unwrap_or(0.0),
    }
}

/// Continuous stair-ride height at this tick's destination xz: the detector's measured
/// ramp line, anchored so the flat approach before the first riser stays at current foot
/// level and rise accumulates only past it (see SLOPE-RIDE in [`resolve_position`]).
/// Returns `None` when the ride doesn't apply this tick. All xz are bevy space; `plane_y`
/// is current foot level (bevy up); `behind_y` is the measured ground a step BEHIND us
/// along the ramp direction; `dest_y` is the measured ground at this tick's destination
/// xz. NaN when there's no floor at that point.
fn slope_ride_feet(
    det: &super::input::StairDetection,
    plane_y: f32,
    here: Vec2,
    target: Vec2,
    behind_y: f32,
    dest_y: f32,
) -> Option<f32> {
    // One riser + margin — a larger single-tick delta means the fit is lying.
    const MAX_TICK_DELTA: f32 = 0.45;

    let slope = det.best_slope;
    if !slope.is_finite() || slope.abs() < 0.02 {
        return None; // not a stair-grade surface
    }
    // Ramp direction from the detector's gizmo endpoints (near = player-side anchor).
    let line = det.ramp_far.0 - det.ramp_near.0;
    let len = line.length();
    if len < 1e-4 {
        return None; // degenerate line — not actually on/beside a locked ramp
    }
    let dir = line / len;
    let here_t = ((here - det.ramp_near.0).dot(dir)).max(0.0);
    let target_t = (target - det.ramp_near.0).dot(dir);
    if target_t <= here_t {
        return None; // no progress along the ramp this tick — stay flat, ground-snap handles it
    }
    // The march measured where the first riser sits relative to the player; before it,
    // the surface is flat at foot level. (Pink-fit lock with no march data: near-zero approach.)
    let mut d0 = det.march_first_riser_rel.unwrap_or(0.3);
    // SMOOTH-RAMP ANCHOR: discrete stairs have a flat approach — the ground stays
    // at foot level until the first riser face (distance d0), so anchoring there
    // keeps wire Y exactly flat over that strip. A smooth ramp has NO such strip:
    // the surface starts rising within this tick's travel, and anchoring at the
    // march's "first riser" (a probe-step counting artifact on a continuous slope)
    // kept wire Y flat while the destination was already above it — the capsule
    // bottom embedded in the face, avian depenetrated us backward every tick
    // (oscillation), and the tall-wall veto locked the ride out permanently.
    // When the measured ground at our own position or just behind us is already
    // rising relative to foot level, there is no flat approach: anchor at our
    // position so the rise engages from this tick.
    let on_rising_surface = {
        let here_y = det.center_y;
        (here_y.is_finite() && plane_y - here_y > 0.05)
            || (behind_y.is_finite() && plane_y - behind_y > 0.05)
    };
    if slope > 0.0 && on_rising_surface {
        d0 = 0.0;
    }
    // "The step is soon": even while we still stand on the flat strip, if the
    // measured ground at this tick's DESTINATION is already above foot level,
    // the rise starts within our travel — anchoring past it would leave wire Y
    // flat into an embedded tick. Engage from now; MAX_TICK_DELTA below caps
    // how fast we can merge up (a walker starting under/inside the slope climbs
    // toward it over a few ticks, clipping through as the animation hides).
    let dest_above = dest_y.is_finite() && dest_y - plane_y > 0.05;
    if slope > 0.0 && dest_above {
        d0 = 0.0;
    }
    let feet = plane_y + slope * ((target_t - (here_t + d0)).max(0.0));
    if (feet - plane_y).abs() > MAX_TICK_DELTA {
        return None;
    }
    Some(feet)
}

/// Like `probe` but returns (distance, normal) for a masked down/any cast.
#[allow(dead_code)]
fn probe_hit(
    sq: &SpatialQuery,
    col: &Collider,
    from: Vec3,
    dir: Dir3,
    max: f32,
    mask: LayerMask,
) -> Option<(f32, Vec3)> {
    let hit = sq.cast_shape(
        col,
        from,
        Quat::IDENTITY,
        dir,
        &ShapeCastConfig::from_max_distance(max),
        &SpatialQueryFilter::from_mask(mask),
    )?;
    Some((hit.distance, hit.normal1.into()))
}

#[cfg(test)]
mod live_path_tests {
    //! Live-path coverage for [`resolve_position`] + [`crate::view_native::input::detect_stairs`].
    //!
    //! The legacy hand-rolled walker (`MzbCollisionGeometry::wall_clip_wire`) is no
    //! longer on the production path — dispatch_movement_system routes every tick
    //! through this resolver against a real avian world. These tests mirror the
    //! scenario suite that used to pin wall_clip_wire (kuluu-render's
    //! `wall_collision_tests`), re-pointed at the live implementation: same geometry
    //! builders, same walk-loop contract (input.rs' consumption of WallClipResult),
    //! real avian move-and-slide + spatial queries.

    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use ffxi_dat::mzb::NO_SUB_AREA_LINK;
    use kuluu_render::dat_mzb::{MzbCollisionBlock, MzbCollisionGeometry};

    /// Same tick contract as the legacy suite and input.rs' fixed step: 30 Hz,
    /// run speed.
    const DT: f32 = 1.0 / 30.0;
    const RUN: f32 = 6.0;

    // ------------------------------------------------------------------
    // Geometry builders (ported from kuluu-render's wall_collision_tests)
    // ------------------------------------------------------------------

    fn quad(b: &mut MzbCollisionBlock, v: [Vec3; 4], n: Vec3, link: u32) {
        let i0 = b.positions.len() as u32;
        b.positions.extend_from_slice(&v);
        b.indices
            .extend_from_slice(&[i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
        b.tri_normals.extend_from_slice(&[n, n]);
        b.tri_sub_area.extend_from_slice(&[link, link]);
    }

    fn staircase(steps: usize, d: f32, r: f32, balustrades: bool) -> MzbCollisionGeometry {
        let mut b = MzbCollisionBlock::default();
        quad(
            &mut b,
            [
                Vec3::new(-10.0, 0.0, -3.0),
                Vec3::new(0.0, 0.0, -3.0),
                Vec3::new(0.0, 0.0, 3.0),
                Vec3::new(-10.0, 0.0, 3.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        for i in 0..steps {
            let x0 = i as f32 * d;
            let y0 = i as f32 * r;
            let y1 = y0 + r;
            quad(
                &mut b,
                [
                    Vec3::new(x0, y0, -3.0),
                    Vec3::new(x0, y0, 3.0),
                    Vec3::new(x0, y1, 3.0),
                    Vec3::new(x0, y1, -3.0),
                ],
                Vec3::new(-1.0, 0.0, 0.0),
                NO_SUB_AREA_LINK,
            );
            quad(
                &mut b,
                [
                    Vec3::new(x0, y1, -3.0),
                    Vec3::new(x0 + d, y1, -3.0),
                    Vec3::new(x0 + d, y1, 3.0),
                    Vec3::new(x0, y1, 3.0),
                ],
                Vec3::Y,
                NO_SUB_AREA_LINK,
            );
        }
        let xt = steps as f32 * d;
        let yt = steps as f32 * r;
        quad(
            &mut b,
            [
                Vec3::new(xt, yt, -3.0),
                Vec3::new(xt + 10.0, yt, -3.0),
                Vec3::new(xt + 10.0, yt, 3.0),
                Vec3::new(xt, yt, 3.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        if balustrades {
            let sl = (d * d + r * r).sqrt();
            for zed in [3.0f32, -3.0] {
                let nn = Vec3::new(0.0, 0.0, -zed.signum());
                quad(
                    &mut b,
                    [
                        Vec3::new(-2.0, 0.0, zed),
                        Vec3::new(xt + 2.0, yt, zed),
                        Vec3::new(xt + 2.0, yt + 1.2, zed),
                        Vec3::new(-2.0, 1.2, zed),
                    ],
                    nn,
                    NO_SUB_AREA_LINK,
                );
                let sn = Vec3::new(-r / sl, d / sl, 0.0);
                quad(
                    &mut b,
                    [
                        Vec3::new(0.0, 0.0, zed - 0.2 * zed.signum()),
                        Vec3::new(xt, yt, zed - 0.2 * zed.signum()),
                        Vec3::new(xt + 0.3, yt, zed - 0.2 * zed.signum()),
                        Vec3::new(0.3, 0.0, zed - 0.2 * zed.signum()),
                    ],
                    sn,
                    NO_SUB_AREA_LINK,
                );
            }
        }
        MzbCollisionGeometry::from_block(b)
    }

    fn flat_with_wall(wall_x: f32, height: f32, link: u32) -> MzbCollisionGeometry {
        let mut b = MzbCollisionBlock::default();
        quad(
            &mut b,
            [
                Vec3::new(-10.0, 0.0, -10.0),
                Vec3::new(10.0, 0.0, -10.0),
                Vec3::new(10.0, 0.0, 10.0),
                Vec3::new(-10.0, 0.0, 10.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        quad(
            &mut b,
            [
                Vec3::new(wall_x, 0.0, -10.0),
                Vec3::new(wall_x, 0.0, 10.0),
                Vec3::new(wall_x, height, 10.0),
                Vec3::new(wall_x, height, -10.0),
            ],
            Vec3::new(-1.0, 0.0, 0.0),
            link,
        );
        MzbCollisionGeometry::from_block(b)
    }

    fn corridor(gap: f32) -> MzbCollisionGeometry {
        let mut b = MzbCollisionBlock::default();
        quad(
            &mut b,
            [
                Vec3::new(-10.0, 0.0, -10.0),
                Vec3::new(10.0, 0.0, -10.0),
                Vec3::new(10.0, 0.0, 10.0),
                Vec3::new(-10.0, 0.0, 10.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        let h = gap / 2.0;
        quad(
            &mut b,
            [
                Vec3::new(2.0, 0.0, h),
                Vec3::new(3.0, 0.0, h),
                Vec3::new(3.0, 2.5, h),
                Vec3::new(2.0, 2.5, h),
            ],
            Vec3::new(0.0, 0.0, -1.0),
            NO_SUB_AREA_LINK,
        );
        quad(
            &mut b,
            [
                Vec3::new(2.0, 0.0, -h),
                Vec3::new(3.0, 0.0, -h),
                Vec3::new(3.0, 2.5, -h),
                Vec3::new(2.0, 2.5, -h),
            ],
            Vec3::new(0.0, 0.0, 1.0),
            NO_SUB_AREA_LINK,
        );
        MzbCollisionGeometry::from_block(b)
    }

    fn parapet_platform(wall_x: f32, wall_h: f32, plat_y: f32) -> MzbCollisionGeometry {
        let mut b = MzbCollisionBlock::default();
        quad(
            &mut b,
            [
                Vec3::new(-10.0, 0.0, -10.0),
                Vec3::new(wall_x, 0.0, -10.0),
                Vec3::new(wall_x, 0.0, 10.0),
                Vec3::new(-10.0, 0.0, 10.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        quad(
            &mut b,
            [
                Vec3::new(wall_x, 0.0, -10.0),
                Vec3::new(wall_x, 0.0, 10.0),
                Vec3::new(wall_x, wall_h, 10.0),
                Vec3::new(wall_x, wall_h, -10.0),
            ],
            Vec3::new(-1.0, 0.0, 0.0),
            NO_SUB_AREA_LINK,
        );
        quad(
            &mut b,
            [
                Vec3::new(wall_x, plat_y, -10.0),
                Vec3::new(10.0, plat_y, -10.0),
                Vec3::new(10.0, plat_y, 10.0),
                Vec3::new(wall_x, plat_y, 10.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        MzbCollisionGeometry::from_block(b)
    }

    /// The Bastok Mines stair as measured 2026-08-23: riser/tread flight with a
    /// level-0 terrace slab continuing UNDER the treads.
    fn stair_with_ground_under(steps: usize, d: f32, r: f32) -> MzbCollisionGeometry {
        let mut b = MzbCollisionBlock::default();
        quad(
            &mut b,
            [
                Vec3::new(-10.0, 0.0, -3.0),
                Vec3::new(steps as f32 * d + 5.0, 0.0, -3.0),
                Vec3::new(steps as f32 * d + 5.0, 0.0, 3.0),
                Vec3::new(-10.0, 0.0, 3.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        for i in 0..steps {
            let x0 = i as f32 * d;
            let y0 = i as f32 * r;
            let y1 = y0 + r;
            quad(
                &mut b,
                [
                    Vec3::new(x0, y0, -3.0),
                    Vec3::new(x0, y0, 3.0),
                    Vec3::new(x0, y1, 3.0),
                    Vec3::new(x0, y1, -3.0),
                ],
                Vec3::new(-1.0, 0.0, 0.0),
                NO_SUB_AREA_LINK,
            );
            quad(
                &mut b,
                [
                    Vec3::new(x0, y1, -3.0),
                    Vec3::new(x0 + d, y1, -3.0),
                    Vec3::new(x0 + d, y1, 3.0),
                    Vec3::new(x0, y1, 3.0),
                ],
                Vec3::Y,
                NO_SUB_AREA_LINK,
            );
        }
        let xt = steps as f32 * d;
        let yt = steps as f32 * r;
        quad(
            &mut b,
            [
                Vec3::new(xt, yt, -3.0),
                Vec3::new(xt + 10.0, yt, -3.0),
                Vec3::new(xt + 10.0, yt, 3.0),
                Vec3::new(xt, yt, 3.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        MzbCollisionGeometry::from_block(b)
    }

    // ------------------------------------------------------------------
    // Live world + walk driver
    // ------------------------------------------------------------------

    /// One static trimesh collider mirroring the zone blocks — exactly what
    /// `sync_zone_collider` spawns in production.
    fn spawn_zone_collider(world: &mut World, geom: &MzbCollisionGeometry) -> Option<Entity> {
        let (positions, tris) = geom.trimesh_data();
        if tris.is_empty() {
            return None;
        }
        Some(
            world
                .spawn((
                    RigidBody::Static,
                    Collider::trimesh(positions, tris),
                    wall_layers(),
                    Transform::default(),
                ))
                .id(),
        )
    }

    /// Headless app with the avian world built around `geom`: PhysicsPlugins +
    /// the zone trimesh collider + the geometry resource, then forced physics
    /// steps so avian builds its collider trees / spatial queries (a headless
    /// test advances Time by ~0 real seconds, so no fixed step would run on its
    /// own).
    fn live_app(geom: MzbCollisionGeometry) -> App {
        let mut app = App::new();
        // TimePlugin inserts the Time resources + fixed-step runner that
        // PhysicsPlugins' schedule is driven by (App::new() alone has no Time).
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(PhysicsPlugins::default());
        // Avian's collider backend + cache systems expect the mesh asset store
        // and its event stream; a minimal test app has no asset plugin, so
        // initialize both.
        let world = app.world_mut();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<bevy::ecs::message::Messages<AssetEvent<Mesh>>>();
        spawn_zone_collider(&mut app.world_mut(), &geom);
        app.insert_resource(geom);
        app.init_resource::<LiveOut>();
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(1.0 / 60.0));
            app.update();
        }
        app
    }

    #[derive(Resource)]
    struct WalkReq {
        x: f32,
        y: f32,
        z: f32,
        dx: f32,
        dy: f32,
    }

    #[derive(Resource, Default)]
    struct LiveOut(Vec<WallClipResult>);

    /// One tick of the LIVE resolver — dispatch_movement_system's exact call.
    fn walk_tick(
        avian: AvianMoveParams,
        mut push: Local<PushThrough>,
        req: Res<WalkReq>,
        mut out: ResMut<LiveOut>,
    ) {
        let clip = resolve_position(
            &avian, &mut push, req.x, req.y, req.z, req.dx, req.dy, DT, &mut None, &mut None,
        );
        out.0.push(clip);
    }

    /// Walk from wire (x,y,z) in wire dir for `ticks` through the live resolver —
    /// input.rs' exact consumption: x += dx, y += dy, landed_floor owns z.
    fn live_walk(
        app: &mut App,
        start: (f32, f32, f32),
        dir: (f32, f32),
        ticks: usize,
    ) -> (f32, f32, f32) {
        let mut pos = start;
        for _ in 0..ticks {
            app.world_mut().insert_resource(WalkReq {
                x: pos.0,
                y: pos.1,
                z: pos.2,
                dx: dir.0 * RUN * DT,
                dy: dir.1 * RUN * DT,
            });
            app.world_mut()
                .run_system_once(walk_tick)
                .expect("walk_tick runs");
            let clip = app.world_mut().resource_mut::<LiveOut>().0.pop().unwrap();
            pos.0 += clip.dx;
            pos.1 += clip.dy;
            if let Some(f) = clip.landed_floor {
                pos.2 = -f;
            }
        }
        pos
    }

    // ------------------------------------------------------------------
    // The suite — live-path versions of kuluu-render's wall_collision_tests
    // ------------------------------------------------------------------

    #[test]
    fn stairs_ascend_full_matrix_live() {
        // Riser heights stay at or below MAX_GROUND_STEP_UP (0.4): nothing over
        // half a yalm is climbable, so 0.5-riser flights are NOT in the matrix —
        // they are pinned by riser_above_step_height_blocks_ascent_live instead.
        for (r, d) in [
            (0.2, 0.25),
            (0.25, 0.3),
            (0.3, 0.4),
            (0.35, 0.5),
            (0.4, 0.4),
            (0.3, 0.9),
        ] {
            for bal in [false, true] {
                let mut app = live_app(staircase(12, d, r, bal));
                let (x, _y, z) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 600);
                assert!(
                    (-z - 12.0 * r).abs() < 0.05 && x > 12.0 * d,
                    "stuck ascending r={r} d={d} bal={bal}: x={x:.2} h={:.2}",
                    -z
                );
            }
        }
    }

    /// A flight whose risers exceed MAX_GROUND_STEP_UP (0.4) cannot be climbed:
    /// the walker stops at the foot of the first riser and never gains height.
    #[test]
    fn riser_above_step_height_blocks_ascent_live() {
        let mut app = live_app(staircase(12, 0.5, 0.5, false));
        let (x, _y, z) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 600);
        assert!(
            x < -0.2 && z.abs() < 0.05,
            "blocked at the foot of the 0.5-riser flight: x={x:.3} h={z:.3}",
        );
    }

    #[test]
    fn stairs_descend_full_matrix_live() {
        for (r, d) in [(0.2, 0.25), (0.3, 0.4), (0.35, 0.5), (0.5, 0.5), (0.3, 0.9)] {
            for bal in [false, true] {
                let mut app = live_app(staircase(12, d, r, bal));
                let top = (12.0 * d + 3.0, 0.0, -(12.0 * r));
                let (x, _y, z) = live_walk(&mut app, top, (-1.0, 0.0), 600);
                assert!(
                    (-z).abs() < 0.05 && x < -1.0,
                    "stuck descending r={r} d={d} bal={bal}: x={x:.2} h={:.2}",
                    -z
                );
            }
        }
    }

    /// A flush riser at or below MAX_GROUND_STEP_UP (0.4) is a step: climb it.
    #[test]
    fn flush_step_onto_platform_works_live() {
        let mut app = live_app(parapet_platform(2.0, 0.35, 0.35));
        let (x, _y, z) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 300);
        assert!(
            x > 3.0 && (-z - 0.35).abs() < 0.05,
            "stepped up: x={x:.2} h={:.2}",
            -z
        );
    }

    /// A flush riser ABOVE MAX_GROUND_STEP_UP is a wall, not a step: the walker
    /// stops in standoff and never climbs (0.8 > 0.4 — nothing over half a yalm
    /// is walkable).
    #[test]
    fn flush_riser_above_step_height_blocks_live() {
        let mut app = live_app(parapet_platform(2.0, 0.8, 0.8));
        let (x, _y, z) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 300);
        assert!(
            x < 2.0 - 0.35 && x > 2.0 - 0.65 && z.abs() < 0.05,
            "standoff at the tall riser: x={x:.3} h={z:.3}",
        );
    }

    #[test]
    fn corridor_pass_and_block_live() {
        let mut app = live_app(corridor(1.2));
        let (x, _, _) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 300);
        assert!(x > 4.0, "1.2 gap passes: x={x:.2}");

        let mut app = live_app(corridor(0.7));
        let (x, _, _) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 300);
        assert!(x < 2.0, "0.7 gap blocks: x={x:.2}");
    }

    #[test]
    fn embedded_start_recovers_and_never_tunnels_live() {
        let mut app = live_app(flat_with_wall(2.0, 3.0, NO_SUB_AREA_LINK));
        // Start embedded in the wall (capsule radius 0.4, face at x=2).
        let (x, _, _) = live_walk(&mut app, (1.75, 0.0, 0.0), (1.0, 0.0), 120);
        assert!(x < 2.0, "never crossed: x={x:.3}");

        let mut app = live_app(flat_with_wall(2.0, 3.0, NO_SUB_AREA_LINK));
        let (xa, _, _) = live_walk(&mut app, (1.75, 0.0, 0.0), (-1.0, 0.0), 60);
        assert!(xa < 1.0, "walking away works: x={xa:.2}");
    }

    /// The #2 regression on the live path: a suppressed sub-area shell must be
    /// absent from the avian trimesh (trimesh_data mirrors MZB suppression), so
    /// the resolver walks straight through where the unsuppressed wall blocks.
    #[test]
    fn suppressed_shell_is_walk_through_live() {
        let mut app = live_app(flat_with_wall(2.0, 3.0, 7));
        let (x, _, _) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 120);
        assert!(x < 2.0, "blocks while unsuppressed: x={x:.2}");

        let mut g = flat_with_wall(2.0, 3.0, 7);
        g.set_suppressed(Some(7));
        let mut app = live_app(g);
        let (x2, _, _) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 120);
        assert!(x2 > 4.0, "suppressed shell passes: x={x2:.2}");
    }

    #[test]
    fn stair_with_ground_under_climbs_live() {
        for (r, d) in [(0.26, 0.48), (0.35, 0.5)] {
            let mut app = live_app(stair_with_ground_under(8, d, r));
            // Start flush against the first riser — his spawn was ~0.35 yalms off it.
            let start_x = -RADIUS + 0.05;
            let (x, _y, z) = live_walk(&mut app, (start_x, 0.0, 0.0), (1.0, 0.0), 600);
            assert!(
                (-z - 8.0 * r).abs() < 0.05 && x > 8.0 * d,
                "pinned at foot of buried stair r={r} d={d}: x={x:.2} h={:.3}",
                -z
            );
        }
    }

    #[test]
    fn stair_with_ground_under_descends_live() {
        let (r, d) = (0.26f32, 0.48);
        let mut app = live_app(stair_with_ground_under(8, d, r));
        let top = (8.0 * d + 3.0, 0.0, -(8.0 * r));
        let (x, _y, z) = live_walk(&mut app, top, (-1.0, 0.0), 600);
        assert!(
            (-z).abs() < 0.05 && x < -1.0,
            "stuck descending buried stair: x={x:.2} h={:.3}",
            -z
        );
    }

    #[test]
    fn ramp_40_degrees_walks_up_free_live() {
        let top = 4.0 * (40f32.to_radians().tan());
        let mut b = MzbCollisionBlock::default();
        quad(
            &mut b,
            [
                Vec3::new(-10.0, 0.0, -6.0),
                Vec3::new(2.0, 0.0, -6.0),
                Vec3::new(2.0, 0.0, 6.0),
                Vec3::new(-10.0, 0.0, 6.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        let run = 4.0;
        let l = (run * run + top * top).sqrt();
        let n = Vec3::new(-top / l, run / l, 0.0);
        quad(
            &mut b,
            [
                Vec3::new(2.0, 0.0, -6.0),
                Vec3::new(6.0, top, -6.0),
                Vec3::new(6.0, top, 6.0),
                Vec3::new(2.0, 0.0, 6.0),
            ],
            n,
            NO_SUB_AREA_LINK,
        );
        quad(
            &mut b,
            [
                Vec3::new(6.0, top, -6.0),
                Vec3::new(16.0, top, -6.0),
                Vec3::new(16.0, top, 6.0),
                Vec3::new(6.0, top, 6.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        let mut app = live_app(MzbCollisionGeometry::from_block(b));
        let (x, _y, z) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 400);
        assert!(
            x > 6.5 && (-z - top).abs() < 0.05,
            "ramp free: x={x:.2} h={:.2}",
            -z
        );
    }

    #[test]
    fn cliff_descent_is_never_a_wall_live() {
        let mut b = MzbCollisionBlock::default();
        quad(
            &mut b,
            [
                Vec3::new(-10.0, 2.0, -6.0),
                Vec3::new(2.0, 2.0, -6.0),
                Vec3::new(2.0, 2.0, 6.0),
                Vec3::new(-10.0, 2.0, 6.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        quad(
            &mut b,
            [
                Vec3::new(2.0, 0.0, -6.0),
                Vec3::new(2.0, 0.0, 6.0),
                Vec3::new(2.0, 2.0, 6.0),
                Vec3::new(2.0, 2.0, -6.0),
            ],
            Vec3::new(1.0, 0.0, 0.0),
            NO_SUB_AREA_LINK,
        );
        quad(
            &mut b,
            [
                Vec3::new(2.0, 0.0, -6.0),
                Vec3::new(10.0, 0.0, -6.0),
                Vec3::new(10.0, 0.0, 6.0),
                Vec3::new(2.0, 0.0, 6.0),
            ],
            Vec3::Y,
            NO_SUB_AREA_LINK,
        );
        let mut app = live_app(MzbCollisionGeometry::from_block(b));
        let (x, _y, z) = live_walk(&mut app, (0.0, 0.0, -2.0), (1.0, 0.0), 200);
        assert!(
            x > 4.0 && (-z).abs() < 0.05,
            "walked off and landed: x={x:.2} h={:.2}",
            -z
        );
    }

    #[test]
    fn tall_wall_blocks_with_standoff_live() {
        let mut app = live_app(flat_with_wall(2.0, 3.0, NO_SUB_AREA_LINK));
        let (x, _y, _z) = live_walk(&mut app, (-2.0, 0.0, 0.0), (1.0, 0.0), 300);
        assert!(x < 2.0 - 0.35 && x > 2.0 - 0.65, "standoff: x={x:.3}");
    }

    // ------------------------------------------------------------------
    // Resolver-specific coverage the legacy suite never had
    // ------------------------------------------------------------------

    /// The STOPPED arm (want_len < 1e-6): no input settles onto the actual tread
    /// instead of holding the smooth ramp height.
    #[test]
    fn stopped_input_settles_onto_tread() {
        let mut app = live_app(staircase(4, 0.5, 0.3, false));
        // Stand on the top terrace (feet at 4*0.3), no input: floor must be the
        // terrace, not a lower tread or NaN.
        let clip = {
            app.world_mut().insert_resource(WalkReq {
                x: 2.5,
                y: 0.0,
                z: -(4.0 * 0.3),
                dx: 0.0,
                dy: 0.0,
            });
            app.world_mut()
                .run_system_once(walk_tick)
                .expect("walk_tick runs");
            app.world_mut().resource_mut::<LiveOut>().0.pop().unwrap()
        };
        assert_eq!(clip.dbg_reason, "stopped-input", "reason={}", clip.dbg_reason);
        let floor = clip.landed_floor.expect("settled arm reports a floor");
        assert!((floor - 4.0 * 0.3).abs() < 1e-3, "tread: {floor:.4}");
        // And on flat ground the stop settles to the floor underfoot.
        let mut app = live_app(flat_with_wall(2.0, 3.0, NO_SUB_AREA_LINK));
        app.world_mut().insert_resource(WalkReq {
            x: -5.0,
            y: 0.0,
            z: 0.0,
            dx: 0.0,
            dy: 0.0,
        });
        app.world_mut()
            .run_system_once(walk_tick)
            .expect("walk_tick runs");
        let clip = app.world_mut().resource_mut::<LiveOut>().0.pop().unwrap();
        let flat_floor = clip.landed_floor.expect("settled arm reports a floor");
        assert!(flat_floor.abs() < 1e-3, "flat: {flat_floor:.4}");
    }

    /// detect_stairs is the word-of-god detector resolve_position rides on: flat
    /// ground must NOT lock a ramp.
    #[test]
    fn detect_stairs_flat_ground_not_locked() {
        let g = flat_with_wall(2.0, 3.0, NO_SUB_AREA_LINK);
        // Bevy space: target at feet level on the flat approach (wire y=0).
        let det = crate::view_native::input::detect_stairs(Vec3::new(-5.0, 0.0, 0.0), &g);
        assert!(!det.ramp_locked, "flat ground locked a ramp");
    }

    /// A staircase ahead locks the detector with a slope near r/d.
    #[test]
    fn detect_stairs_locks_ramp_on_staircase_ahead() {
        let (d, r) = (0.5f32, 0.3);
        let g = staircase(12, d, r, false);
        // Stand on the flat approach just before the first riser.
        let det = crate::view_native::input::detect_stairs(Vec3::new(-1.0, 0.0, 0.0), &g);
        assert!(det.ramp_locked, "staircase ahead did not lock");
        let want = r / d;
        assert!(
            (det.best_slope - want).abs() < 0.25,
            "slope {:.3} vs expected ~{:.3}",
            det.best_slope, want
        );
    }

    /// A tall wall ahead is not a ramp: the detector must stay unlocked.
    #[test]
    fn detect_stairs_wall_ahead_not_locked() {
        let g = flat_with_wall(2.0, 3.0, NO_SUB_AREA_LINK);
        let det = crate::view_native::input::detect_stairs(Vec3::new(-1.0, 0.0, 0.0), &g);
        assert!(!det.ramp_locked, "wall ahead locked a ramp");
    }
}

/// move_and_slide that only treats WALLS as blocking. Any contact whose surface
/// is walkable (normal within SLOPE_MAX_ANGLE of straight up: floor, ramps up to
/// 60deg, stair treads) returns `Ignore` -- the slide does not stop or deflect on
/// it, so walkable ground never blocks horizontal travel (fixes "stuck on flat
/// floor" and removes any surface the slide could ride up = no goat). Steeper
/// faces (>60deg = true walls) return `Accept` and block/slide as normal.
fn slide_walls_only(
    mas: &MoveAndSlide,
    col: &Collider,
    from: Vec3,
    vel: Vec3,
    dt: f32,
    filter: &SpatialQueryFilter,
    // OUT: the normal of the first blocking (non-walkable) contact, if any.
    // Some(normal) => the slide was stopped by a wall/steep face this tick.
    block_normal: &mut Option<Vec3>,
    // OUT: the entity of the first blocking contact, for layer classification.
    block_entity: &mut Option<Entity>,
    // OUT: the world contact POINT of the block (where collision happened).
    block_point: &mut Option<Vec3>,
) -> Vec3 {
    if vel.length_squared() < 1e-12 || dt <= 0.0 {
        return from;
    }
    let mut captured: Option<Vec3> = None;
    let mut captured_ent: Option<Entity> = None;
    let mut captured_pt: Option<Vec3> = None;
    let pos = mas
        .move_and_slide(
            col,
            from,
            Quat::IDENTITY,
            vel,
            Duration::from_secs_f32(dt),
            &MoveAndSlideConfig::default(),
            filter,
            |hit| {
                // hit.normal is a Dir3 pointing away from the character. Up-y
                // >= cos(60deg) => walkable => ignore (not a wall). Otherwise
                // it's a blocking face: capture its normal for classification.
                let n: Vec3 = (*hit.normal).into();
                if n.y >= SLOPE_MAX_ANGLE.cos() {
                    MoveAndSlideHitResponse::Ignore
                } else {
                    // SANITY GATE: a real block is within the capsule's reach. If
                    // avian reports a contact point far from the player (a
                    // depenetration artifact or degenerate trimesh contact
                    // returning garbage coords), it is NOT in front of us --
                    // ignore it instead of treating distant geometry as a wall.
                    let pt: Vec3 =
                        Vec3::new(hit.point.x as f32, hit.point.y as f32, hit.point.z as f32);
                    let reach = RADIUS + HALF + 0.5; // capsule reach + margin
                    let horiz = Vec2::new(pt.x - from.x, pt.z - from.z).length();
                    if horiz > reach {
                        // Contact is not actually in front of us -> phantom.
                        MoveAndSlideHitResponse::Ignore
                    } else {
                        if captured.is_none() {
                            captured = Some(n);
                            captured_ent = Some(hit.entity);
                            captured_pt = Some(pt);
                        }
                        MoveAndSlideHitResponse::Accept
                    }
                }
            },
        )
        .position;
    *block_normal = captured;
    *block_entity = captured_ent;
    *block_point = captured_pt;
    pos
}
