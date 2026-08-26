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

use kuluu_render::components::{CameraOccluder, IsSelf, WorldEntity};
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
fn door_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Door, LayerMask::ALL)
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
            .init_resource::<ZoneAvianCollider>();
        // The four collider-sync systems (sync_zone_collider, sync_door_colliders,
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

/// Schedules the four collider-sync systems into FixedUpdate before the walker.
/// Called from mod.rs where `dispatch_movement_system` is in scope.
pub fn add_collider_sync_systems(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        (
            sync_zone_collider,
            sync_door_colliders,
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
                wall_layers(),
                Transform::default(),
            ))
            .id(),
    );
}

/// Marks a `CameraOccluder` (MMB placement: door, gate, furniture) that has
/// been given a Door-layer avian collider, so we build each one exactly once.
/// Lifecycle rides the occluder entity: when the placement despawns, this
/// component and its collider go with it — no separate bookkeeping.
#[derive(Component)]
struct DoorColliderBuilt;

/// Mirror MMB placement geometry into the avian world as static Door colliders
/// so the walker HARD-BLOCKS on doors (slides along them, never through). At
/// baseline only MZB walls were in the walker's collision world; doors are
/// MMB `CameraOccluder` entities that only the camera BVH saw, which is why
/// you could walk straight through a closed door. Same triangle source the
/// camera BVH uses (world-transformed occluder mesh), now also a walker
/// obstacle. Vertices are baked to world space and the collider entity sits at
/// identity, matching the zone-collider pattern (no double transform).
fn sync_door_colliders(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    query: Query<
        (Entity, &Mesh3d, &GlobalTransform),
        (With<CameraOccluder>, Without<DoorColliderBuilt>),
    >,
) {
    for (entity, mesh3d, _global) in query.iter() {
        let Some(mesh) = meshes.get(mesh3d.0.id()) else {
            continue;
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
        // Build the trimesh from LOCAL mesh verts. avian positions the collider
        // by the entity's transform, so pre-transforming to world space applied
        // the placement transform TWICE -- the collider landed at
        // origin + world_verts, a phantom wall far from the drawn mesh (found
        // via tshimono26_h blocking open floor at (-35,61.6) with its real wall
        // drawn near (+65,+121.6), origin (-100,0,-60)). Local verts = collider
        // exactly where the mesh renders.
        let mut verts: Vec<Vec3> = Vec::with_capacity(positions.len());
        for p in positions {
            verts.push(Vec3::from_array(*p));
        }
        let mut tris: Vec<[u32; 3]> = Vec::with_capacity(indices.len() / 3);
        let mut it = indices.iter();
        while let (Some(a), Some(b), Some(c)) = (it.next(), it.next(), it.next()) {
            tris.push([a as u32, b as u32, c as u32]);
        }
        if tris.is_empty() {
            // Mesh not ready or empty: leave unmarked so we retry next frame.
            continue;
        }
        commands.entity(entity).insert((
            RigidBody::Static,
            Collider::trimesh(verts, tris),
            door_layers(),
            DoorColliderBuilt,
        ));
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
struct MobColliderOwner(Entity);

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
        &Transform,
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
    for (visual, t, r) in to_build.iter() {
        let seg = (r.half_height * 2.0 - r.radius * 2.0).max(0.05);
        let collider = commands
            .spawn((
                RigidBody::Kinematic,
                Collider::capsule(r.radius, seg),
                mob_layers(),
                Transform::from_translation(t.translation),
                MobColliderOwner(visual),
            ))
            .id();
        commands.entity(visual).insert(MobColliderLink(collider));
    }
    // Park each collider on its visual's current position.
    for (visual, link) in links.iter() {
        let Ok(vt) = visuals.get(visual) else { continue };
        if let Ok(mut ct) = collider_tf.get_mut(link.0) {
            ct.translation = vt.translation;
        }
    }
    // Despawn colliders whose visual is gone.
    for (collider, owner) in owners.iter() {
        if visuals.get(owner.0).is_err() && links.get(owner.0).is_err() {
            commands.entity(collider).despawn();
        }
    }
}

/// MoveAndSlide + SpatialQuery bundled so dispatch grows by one param only.
#[derive(bevy::ecs::system::SystemParam)]
pub struct AvianMoveParams<'w, 's> {
    pub mas: MoveAndSlide<'w, 's>,
    pub sq: SpatialQuery<'w, 's>,
    pub geom: Res<'w, MzbCollisionGeometry>,
}

fn capsule() -> Collider {
    Collider::capsule(RADIUS, SEG_LEN)
}

/// Vertical probe: distance the capsule travels along `dir` before contact,
/// capped at `max`, restricted to the given layer mask. Raw shape cast.
#[allow(dead_code)]
fn probe(sq: &SpatialQuery, col: &Collider, from: Vec3, dir: Dir3, max: f32, mask: LayerMask) -> f32 {
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
fn layer_ahead(sq: &SpatialQuery, col: &Collider, start: Vec3, want: Vec3, mask: LayerMask) -> bool {
    let len = want.length();
    if len < 1e-6 {
        return false;
    }
    let Ok(dir) = Dir3::new(want / len) else { return false; };
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
    let Ok(dir) = Dir3::new(want / len) else { return false; };
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
            landed_floor: floor.map(|f| -f),
            dbg_is_a_stop: false,
            dbg_stop_slope: false,
            dbg_slope_angle: 0.0,
            dbg_stop_steps: false,
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
    const SNAP_DOWN: f32 = 0.4;   // ground-snap reach below feet
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
            if push.press(ent, dt) {
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
        &av.mas, &col, start, want / dt, dt, &hfilter, &mut block_normal, &mut block_entity,
        &mut block_point,
    );
    let slide_xz = Vec2::new(p1.x, p1.z);

    // ---- STEP 2: CLASSIFY (ONE decision, priority order) -------------------
    // Inputs: the slide result (slide_xz, block_normal) + the detector (det).
    // We pick exactly ONE outcome and set (move_xz, final_feet, debug) from it.
    // Priority: stairs-ahead > blocked(door>mob>wall) > free-walk.
    let move_xz;
    let final_feet;

    // (a) Walkable stairs ahead (detector sees an up/down band in our path).
    //     This takes priority over avian's per-riser block: a staircase reads to
    //     avian as a vertical wall every tick, so if we let the block win we get
    //     the stop/go cycle. The detector is the authority on "is this walkable".
    let stairs_ahead = {
        let mut found = false;
        for &(oxz, oy, _g, band) in det.sample_data.iter() {
            if band == 0 || oy.is_nan() {
                continue; // green (same tread) or invalid
            }
            let along = (oxz - here).dot(move_dir);
            let rise = (oy - plane_y).abs(); // up OR down both walkable
            if along >= -0.2 && rise > 0.02 && rise <= STEP_HEIGHT * 3.0 {
                found = true;
                break;
            }
        }
        found
    };

    if stairs_ahead {
        dbg_reason = "stairs-ahead";
        dbg_is_a_stop = true;
        dbg_stop_steps = true;
        dbg_step_height = det.ramp_near.1 - plane_y;
        dbg_step_slope = det.best_slope;
        dbg_slope_angle = det.best_slope.atan().to_degrees();
        move_xz = Vec2::new(start.x + want.x, start.z + want.z);
        final_feet = det.ramp_near.1;
    } else if let Some(n) = block_normal {
        // (b) BLOCKED by something that is not a walkable staircase. Classify the
        //     SPECIFIC entity avian stopped us on (block_entity).
        dbg_is_a_stop = true;

        // GROUND TRUTH: cast a clean forward ray a short distance against
        // walls+doors. If NOTHING is really in front of us, avian's move_and_slide
        // fabricated the contact (depenetration artifact) -- we should NOT block.
        let probe_from = Vec3::new(start.x, start.y, start.z);
        let clean_hit = Dir3::new(want / want.length().max(1e-6)).ok().and_then(|d| {
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
        let door_hit = ent.is_some_and(|e| {
            entity_in_layer(&av.sq, &col, start, want, door_mask(), e)
        });
        let mob_hit = excluded_mob.is_none()
            && ent.is_some_and(|e| {
                entity_in_layer(&av.sq, &col, start, want, mob_mask(), e)
            });

        if door_hit {
            dbg_reason = if real { "door-REAL" } else { "door-PHANTOM" };
            dbg_stop_door = true;
            *door_ent_out = ent;
            move_xz = slide_xz;
            final_feet = det.center_y;
        } else if mob_hit {
            dbg_reason = if real { "mob-REAL" } else { "mob-PHANTOM" };
            dbg_stop_mob = true;
            move_xz = slide_xz;
            final_feet = det.center_y;
        } else {
            dbg_reason = if real { "wall-REAL" } else { "wall-PHANTOM" };
            let angle = n.y.clamp(-1.0, 1.0).acos();
            dbg_stop_wall = true;
            dbg_slope_angle = angle.to_degrees();
            dbg_wall_height = 1.0;
            move_xz = slide_xz;
            final_feet = det.center_y;
        }
    } else {
        // (c) FREE WALK: not stopped, no stairs. Take avian's slide result and
        //     snap to the ground under the new position.
        move_xz = slide_xz;
        if det.ramp_locked {
            dbg_stop_slope = true; // informational: walking a locked ramp
            dbg_slope_angle = det.best_slope.atan().to_degrees();
            final_feet = det.ramp_near.1;
        } else if let Some(g) = av.geom.ground_step(slide_xz, plane_y, SNAP_DOWN) {
            final_feet = g;
        } else {
            final_feet = det.center_y;
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
                    let pt: Vec3 = Vec3::new(
                        hit.point.x as f32,
                        hit.point.y as f32,
                        hit.point.z as f32,
                    );
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
