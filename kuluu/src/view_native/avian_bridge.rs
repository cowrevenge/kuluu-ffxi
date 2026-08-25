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
use kuluu_render::dat_mzb::{MzbCollisionGeometry, WallClipResult};

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
/// Max height the horizontal slide may gain in one tick before it's reverted.
/// move_and_slide projects velocity onto contact planes, so a steep face lets
/// the slide itself ride up walls at any angle (goat climbing). Gains past
/// this epsilon revert the slide; the swept step is the ONLY climb path.
pub const SLIDE_UP_EPS: f32 = 0.08;
/// Max gained/horizontal ratio for a swept step to count as a stair not a wall.
/// A real riser (~0.286 over ~0.5 tread) is ~0.57; a wall-hump is >2. tan(60)
/// ~= 1.73 admits everything up to the 60deg we allow and rejects steeper.
pub const MAX_CLIMB_SLOPE: f32 = 1.73;
/// Furthest the ground search looks down for a walkable surface before the
/// character is considered stranded (mid-air, no reachable floor).
pub const MAX_FALL: f32 = 30.0;
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
    for (entity, mesh3d, global) in query.iter() {
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
        let xform = global.to_matrix();
        let mut verts: Vec<Vec3> = Vec::with_capacity(positions.len());
        for p in positions {
            verts.push(xform.transform_point3(Vec3::from_array(*p)));
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
}

fn capsule() -> Collider {
    Collider::capsule(RADIUS, SEG_LEN)
}

/// Vertical probe: distance the capsule travels along `dir` before contact,
/// capped at `max`, restricted to the given layer mask. Raw shape cast.
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
fn thin_probe() -> Collider {
    Collider::sphere(THIN_R)
}

/// True when a surface normal is close enough to straight up to be walkable.
fn is_walkable(normal: Vec3) -> bool {
    normal.y >= SLOPE_MAX_ANGLE.cos()
}

/// Ground normal under `center`, sampled with a thin sphere against WALL+DOOR
/// (doors are floors too). None if nothing within `max`. The wide walker
/// capsule grazes riser faces and misreads them as walls; the thin sphere sees
/// only what's underfoot.
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
) -> WallClipResult {
    let col = capsule();
    let feet0 = -z;
    let start = Vec3::new(x, feet0 + HALF, -y);
    let want = Vec3::new(dx, 0.0, -dy);
    let want_len = want.length();
    if want_len < 1e-6 {
        push.release();
        return WallClipResult::none(dx, dy);
    }
    let dt = dt.max(1e-4);

    // ---- PASS A: horizontal obstacle resolution ----
    // Decide whether a mob is blocking, and whether we've earned push-through.
    let mut excluded_mob: Option<Entity> = None;
    if let Some((_d, ent, is_mob)) = horizontal_obstacle(&av.sq, &col, start, want, None) {
        if is_mob {
            // Soft block: accrue press time; once past threshold, exclude it.
            if push.press(ent, dt) {
                excluded_mob = Some(ent);
            }
            // else: mob stays in the sweep, so the slide below stops at it.
        } else {
            // Wall or door in front: not a mob press, clear any mob accrual.
            push.release();
        }
    } else {
        push.release();
    }

    // Horizontal slide against walls+doors (+ any non-excluded mob), no height
    // gain allowed (revert goat-climbing).
    let mut hfilter = SpatialQueryFilter::from_mask(LayerMask::from([
        GameLayer::Wall,
        GameLayer::Door,
        GameLayer::Mob,
    ]));
    if let Some(e) = excluded_mob {
        hfilter = hfilter.with_excluded_entities([e]);
    }
    let mut p1 = slide_filtered(&av.mas, &col, start, want / dt, dt, &hfilter);
    if p1.y > start.y + SLIDE_UP_EPS {
        p1 = start;
    }
    let moved1 = Vec2::new(p1.x - start.x, p1.z - start.z).length();

    // ---- PASS B: vertical (swept step + ground snap + fall) ----
    // Everything below queries WALL+DOOR only (mobs are never floors).
    let gmask = ground_mask();

    // Swept stair step (up -> forward -> down) when the slide came up short.
    let mut p = p1;
    let short = want_len - moved1;
    if short > 1e-3 {
        let up = probe(&av.sq, &col, p1, Dir3::Y, MAX_STEP, gmask);
        if up > 1e-3 {
            let lifted = p1 + Vec3::Y * up;
            let dir2 = Vec3::new(want.x, 0.0, want.z).normalize_or_zero();
            let p2 = slide_filtered(
                &av.mas,
                &col,
                lifted,
                dir2 * (short / dt),
                dt,
                &SpatialQueryFilter::from_mask(gmask),
            );
            let fwd = Vec2::new(p2.x - lifted.x, p2.z - lifted.z).length();
            if fwd > 1e-5 {
                if let Some((down, _n)) = probe_hit(&av.sq, &col, p2, Dir3::NEG_Y, up + MAX_STEP, gmask) {
                    let p3 = p2 - Vec3::Y * down;
                    let gained = p3.y - p1.y;
                    let horiz = Vec2::new(p3.x - start.x, p3.z - start.z).length();
                    let climb_slope = if horiz > 1e-4 { gained / horiz } else { f32::INFINITY };
                    if gained > 1e-3
                        && gained <= MAX_STEP
                        && climb_slope <= MAX_CLIMB_SLOPE
                        && landing_walkable(&av.sq, p3, dir2)
                    {
                        p = p3;
                    }
                }
            }
        }
    }

    // Ground snap: capsule support within MAX_STEP settles unconditionally
    // (you can't be rejected off geometry you're standing on).
    let short_hit = probe_hit(&av.sq, &col, p, Dir3::NEG_Y, MAX_STEP, gmask);
    let mut settled = false;
    if let Some((down, _n)) = short_hit {
        if down < MAX_STEP {
            p -= Vec3::Y * down;
            settled = true;
        }
    }

    // Long fall: thin-probe down, skipping unwalkable faces, to real ground.
    if !settled {
        let thin = thin_probe();
        let mut search_from = p;
        let mut total_down = 0.0f32;
        let mut steps = 0u8;
        while steps < 8 && total_down < MAX_FALL {
            let remaining = MAX_FALL - total_down;
            let hit = av.sq.cast_shape(
                &thin,
                search_from,
                Quat::IDENTITY,
                Dir3::NEG_Y,
                &ShapeCastConfig::from_max_distance(remaining),
                &SpatialQueryFilter::from_mask(gmask),
            );
            match hit {
                None => break,
                Some(h) => {
                    let normal: Vec3 = h.normal1.into();
                    if is_walkable(normal) {
                        let surface_y = search_from.y - h.distance - THIN_R;
                        p.y = surface_y + HALF;
                        settled = true;
                        break;
                    } else {
                        let skip = h.distance + THIN_R;
                        search_from.y -= skip;
                        total_down += skip;
                        steps += 1;
                    }
                }
            }
        }
    }

    if !settled {
        // Stranded: nothing walkable within MAX_FALL. Revert to start.
        p = start;
    }

    WallClipResult {
        dx: p.x - start.x,
        dy: -(p.z - start.z),
        landed_floor: Some(p.y - HALF),
    }
}

/// Like `probe` but returns (distance, normal) for a masked down/any cast.
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

/// move_and_slide with an explicit filter (layer-restricted slide).
fn slide_filtered(
    mas: &MoveAndSlide,
    col: &Collider,
    from: Vec3,
    vel: Vec3,
    dt: f32,
    filter: &SpatialQueryFilter,
) -> Vec3 {
    if vel.length_squared() < 1e-12 || dt <= 0.0 {
        return from;
    }
    mas.move_and_slide(
        col,
        from,
        Quat::IDENTITY,
        vel,
        Duration::from_secs_f32(dt),
        &MoveAndSlideConfig::default(),
        filter,
        |_hit| MoveAndSlideHitResponse::Accept,
    )
    .position
}
