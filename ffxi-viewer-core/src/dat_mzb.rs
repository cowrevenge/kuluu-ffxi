#![cfg(not(target_arch = "wasm32"))]

use std::collections::VecDeque;
use std::fs;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::tasks::futures_lite::future;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use ffxi_dat::mmb::MmbHeader;
use ffxi_dat::mzb::AreaResourceId;
use ffxi_dat::{mmb, mzb, sub_area, walk, ChunkKind, DatRoot};

use crate::components::{IsSelf, WorldEntity};
use crate::snapshot::SceneState;
use ffxi_viewer_wire::EntityKind;

pub const DEFAULT_WORLD_DRAW_DISTANCE: f32 = 80.0;
pub const DEFAULT_MOB_DRAW_DISTANCE: f32 = 50.0;

pub const MMB_LOAD_DISTANCE_MARGIN: f32 = 1.25;

const MZB_MATERIAL_PALETTE: [[f32; 3]; 16] = [
    [0.85, 0.55, 0.40],
    [0.75, 0.65, 0.45],
    [0.50, 0.65, 0.55],
    [0.55, 0.70, 0.75],
    [0.65, 0.55, 0.75],
    [0.80, 0.65, 0.55],
    [0.65, 0.60, 0.50],
    [0.55, 0.55, 0.60],
    [0.70, 0.50, 0.45],
    [0.45, 0.55, 0.50],
    [0.60, 0.70, 0.40],
    [0.50, 0.45, 0.40],
    [0.75, 0.70, 0.50],
    [0.55, 0.60, 0.65],
    [0.45, 0.50, 0.55],
    [0.65, 0.65, 0.55],
];

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneGeomMode {
    #[default]
    Off,

    Collision,

    All,

    Camera,
}

impl ZoneGeomMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Collision => Self::All,
            Self::All => Self::Camera,
            Self::Camera => Self::Off,
            Self::Off => Self::Collision,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Collision => "collision",
            Self::All => "all",
            Self::Camera => "camera",
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraCollisionSource {
    #[default]
    Mzb,

    Mmb,

    Both,
}

impl CameraCollisionSource {
    pub fn cycle(self) -> Self {
        match self {
            Self::Mzb => Self::Mmb,
            Self::Mmb => Self::Both,
            Self::Both => Self::Mzb,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mzb => "mzb",
            Self::Mmb => "mmb",
            Self::Both => "both",
        }
    }

    pub fn uses_mzb(self) -> bool {
        matches!(self, Self::Mzb | Self::Both)
    }

    pub fn uses_mmb(self) -> bool {
        matches!(self, Self::Mmb | Self::Both)
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct DrawDistance {
    pub world: f32,
    pub mob: f32,

    pub zone_geom_mode: ZoneGeomMode,

    pub camera_collision_source: CameraCollisionSource,
}

impl Default for DrawDistance {
    fn default() -> Self {
        Self {
            world: DEFAULT_WORLD_DRAW_DISTANCE,
            mob: DEFAULT_MOB_DRAW_DISTANCE,
            zone_geom_mode: ZoneGeomMode::default(),
            camera_collision_source: CameraCollisionSource::default(),
        }
    }
}

#[derive(Component)]
pub struct MzbCollisionMesh;

#[derive(Resource, Default)]
pub struct MzbCollisionGeometry {
    pub positions: Vec<Vec3>,

    pub indices: Vec<u32>,

    /// World-space authored face normal per triangle, parallel to
    /// `indices.chunks(3)`. Empty means "fall back to the winding-derived
    /// normal", which cannot distinguish a floor from a ceiling — see
    /// [`MzbSubMesh::tri_normal`].
    pub tri_normals: Vec<Vec3>,

    /// Per triangle, parallel to `indices.chunks(3)`: retail's
    /// `DoubleSidedSkipPolicy` verdict. Only [`Self::camera_triangles`] reads it
    /// — grounding deliberately does not, because retail's movement query uses
    /// `BacksideCullingPolicy`, which skips nothing. Empty means "skip nothing",
    /// matching the `tri_normals` fallback convention.
    pub camera_skip: Vec<bool>,

    pub cell_index: std::collections::HashMap<(i32, i32), Vec<u32>>,

    /// DAT file the triangles came from. Grounding against a zone the player
    /// is no longer in sticks entities to the wrong surface (the nearest-floor
    /// snap is a fixed point), so the auto-loader clears this resource the
    /// moment the effective zone DAT changes instead of waiting for the new
    /// load to land.
    pub source_file_id: Option<u32>,
}

#[derive(Clone)]
pub struct LoadedZoneGeom {
    pub submeshes: Arc<Vec<MzbSubMesh>>,
    pub instances: Arc<Vec<MzbInstance>>,

    pub mmb_spawns: Result<ZoneMmbBuild, String>,
}

#[derive(Resource, Default)]
pub struct LoadMzbInFlight {
    pub tasks: std::collections::HashMap<u32, (Vec<LoadMzbRequest>, Task<LoadedZoneGeom>)>,
}

#[derive(Resource, Default)]
pub struct ZoneGeomCache {
    pub entries: VecDeque<(u32, LoadedZoneGeom)>,
}

pub const ZONE_GEOM_CACHE_CAP: usize = 4;

impl ZoneGeomCache {
    fn get_and_promote(&mut self, file_id: u32) -> Option<LoadedZoneGeom> {
        let pos = self.entries.iter().position(|(id, _)| *id == file_id)?;
        let entry = self.entries.remove(pos)?;
        let geom = entry.1.clone();
        self.entries.push_front(entry);
        Some(geom)
    }

    fn insert(&mut self, file_id: u32, geom: LoadedZoneGeom) {
        if let Some(pos) = self.entries.iter().position(|(id, _)| *id == file_id) {
            self.entries.remove(pos);
        }
        self.entries.push_front((file_id, geom));
        while self.entries.len() > ZONE_GEOM_CACHE_CAP {
            self.entries.pop_back();
        }
    }
}

pub const MZB_GRID_CELL: f32 = 8.0;

pub const FLOOR_NORMAL_MIN: f32 = 0.5;

/// Below this the placement matrix is singular and its inverse-transpose is all
/// NaN, which would silently make every triangle in the placement non-grounding.
const NORMAL_MATRIX_MIN_DET: f32 = 1e-6;

/// Tallest rise [`MzbCollisionGeometry::ground_step`] will climb in one tick.
///
/// Sized from the rise distribution over 120 lattice walks across Lower Jeuno
/// (`zz-ground-walk` with `KULUU_RISE_HIST`): stairs and ramps are 77% of rises
/// and all fall under 0.5, structural jumps between separate surfaces cluster at
/// 1.75 and above, and 0.5..1.5 is a sparse trough. This sits in that trough —
/// double the tallest stair riser, well under the shortest storey.
pub const MAX_GROUND_STEP_UP: f32 = 1.0;

impl MzbCollisionGeometry {
    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn ground_raycast(&self, xz: Vec2, ceiling_y: f32) -> Option<f32> {
        let mut best_y: Option<f32> = None;
        self.for_each_hit_in_column(xz, |hit_y, normal| {
            if normal.y < FLOOR_NORMAL_MIN || hit_y > ceiling_y {
                return;
            }
            best_y = Some(match best_y {
                Some(prev) if prev > hit_y => prev,
                _ => hit_y,
            });
        });
        best_y
    }

    /// Up-facing floor in this column whose height is closest to `ref_y`.
    /// Unlike [`Self::ground_raycast`]'s one-sided `ceiling` cutoff, "nearest"
    /// is a fixed point under grounding (a grounded entity's nearest floor is
    /// the floor it stands on), so it doesn't oscillate when the reference Y
    /// wobbles near a cutoff, and it picks the entity's own level in a
    /// multi-floor building instead of the floor above.
    ///
    /// Down-facing surfaces are rejected: accepting them (`normal.y.abs()`)
    /// makes the underside of every roof a landing surface, which is how a short
    /// walk in Lower Jeuno used to strand the player on a ceiling (kuluu-0nnl).
    pub fn ground_nearest(&self, xz: Vec2, ref_y: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        self.for_each_hit_in_column(xz, |hit_y, normal| {
            if normal.y < FLOOR_NORMAL_MIN {
                return;
            }
            best = Some(match best {
                Some(prev) if (prev - ref_y).abs() <= (hit_y - ref_y).abs() => prev,
                _ => hit_y,
            });
        });
        best
    }

    /// [`Self::ground_nearest`] restricted to floors the walker could actually
    /// step up onto. Unbounded downwards — descending a ledge is a fall, and
    /// with no gravity model a downward snap is how a fall is expressed.
    ///
    /// This is the player-movement entry point. `ground_nearest` is for placing
    /// an entity whose height is already known-good (other PCs, mobs, markers),
    /// where a reference Y far below the floor must still snap up.
    pub fn ground_step(&self, xz: Vec2, feet_y: f32, max_rise: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        self.for_each_hit_in_column(xz, |hit_y, normal| {
            if normal.y < FLOOR_NORMAL_MIN || hit_y > feet_y + max_rise {
                return;
            }
            best = Some(match best {
                Some(prev) if (prev - feet_y).abs() <= (hit_y - feet_y).abs() => prev,
                _ => hit_y,
            });
        });
        best
    }

    /// World triangles retail's chase camera sees — everything except the
    /// `DoubleSidedSkipPolicy` skip set. The camera BVH and every camera probe
    /// must come through here or they disagree with the game.
    ///
    /// Filtering happens here, before the BVH is built, because `CollisionBvh`
    /// reorders its triangle copy by leaf order: no per-triangle array can be
    /// mapped back once it has, so the skip cannot be applied afterwards.
    pub fn camera_triangles(&self) -> Vec<[Vec3; 3]> {
        let desynced = !self.camera_skip.is_empty() && self.camera_skip.len() != self.tri_count();
        if desynced {
            warn!(
                camera_skip = self.camera_skip.len(),
                tri_count = self.tri_count(),
                "camera_skip desynced from the triangle list; skipping by a stale index would \
                 drop the wrong surfaces, so nothing is skipped"
            );
        }
        self.indices
            .chunks_exact(3)
            .enumerate()
            .filter(|(i, _)| desynced || !self.camera_skip.get(*i).copied().unwrap_or(false))
            .map(|(_, t)| {
                [
                    self.positions[t[0] as usize],
                    self.positions[t[1] as usize],
                    self.positions[t[2] as usize],
                ]
            })
            .collect()
    }

    pub fn ground_raycast_all(&self, xz: Vec2) -> Vec<(f32, Vec3)> {
        let mut hits: Vec<(f32, Vec3)> = Vec::new();
        self.for_each_hit_in_column(xz, |hit_y, normal| hits.push((hit_y, normal)));
        hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        hits
    }

    fn for_each_hit_in_column(&self, xz: Vec2, mut visit: impl FnMut(f32, Vec3)) {
        const RAY_ORIGIN_Y: f32 = 1000.0;
        let orig = Vec3::new(xz.x, RAY_ORIGIN_Y, xz.y);
        let dir = Vec3::new(0.0, -1.0, 0.0);

        if !self.cell_index.is_empty() {
            let cell = (
                (xz.x / MZB_GRID_CELL).floor() as i32,
                (xz.y / MZB_GRID_CELL).floor() as i32,
            );
            if let Some(tri_ids) = self.cell_index.get(&cell) {
                for &tri_id in tri_ids {
                    self.visit_tri(orig, dir, tri_id as usize, &mut visit);
                }
            }
            return;
        }

        for tri_id in 0..(self.indices.len() / 3) {
            self.visit_tri(orig, dir, tri_id, &mut visit);
        }
    }

    fn visit_tri(&self, orig: Vec3, dir: Vec3, tri_id: usize, visit: &mut impl FnMut(f32, Vec3)) {
        let base = tri_id * 3;
        let v0 = self.positions[self.indices[base] as usize];
        let v1 = self.positions[self.indices[base + 1] as usize];
        let v2 = self.positions[self.indices[base + 2] as usize];
        if let Some(t) = ray_tri_intersect(orig, dir, v0, v1, v2) {
            let hit_y = orig.y + t * dir.y;
            let normal = match self.tri_normals.get(tri_id) {
                Some(n) => *n,
                None => (v1 - v0).cross(v2 - v0).normalize_or_zero(),
            };
            visit(hit_y, normal);
        }
    }
}

/// Bakes placed submeshes into the geometry the player grounds on. The client
/// and the `zz-*` collision probes both go through here, so a probe can't
/// disagree with what the game actually walks on.
///
/// Every submesh contributes, regardless of mesh flag bit 0: that bit is
/// `doesnt_block_los` (`ffxi-dat/src/mzb.rs`), a line-of-sight property that
/// ordinary walkable street tiles set. Gating collision on it left 13% of the
/// Lower Jeuno street columns with no floor to ground against (kuluu-0nnl).
pub fn build_collision_geometry(
    submeshes: &[MzbSubMesh],
    instances: &[MzbInstance],
    file_id: Option<u32>,
) -> MzbCollisionGeometry {
    let mut positions: Vec<Vec3> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut tri_normals: Vec<Vec3> = Vec::new();
    let mut camera_skip: Vec<bool> = Vec::new();
    let mut missing = 0usize;

    for inst in instances {
        let Some(sub) = submeshes.get(inst.submesh_idx) else {
            continue;
        };
        let n_mat = Mat3::from_mat4(inst.bevy_transform.to_matrix());
        let n_mat = if n_mat.determinant().abs() > NORMAL_MATRIX_MIN_DET {
            n_mat.inverse().transpose()
        } else {
            Mat3::IDENTITY
        };

        let base = positions.len() as u32;
        positions.extend(
            sub.positions
                .iter()
                .map(|v| inst.bevy_transform.transform_point(Vec3::from_array(*v))),
        );
        indices.extend(sub.indices.iter().map(|i| i + base));
        for t in 0..sub.indices.len() / 3 {
            match sub.tri_normal.get(t) {
                Some(n) => tri_normals.push((n_mat * Vec3::from_array(*n)).normalize_or_zero()),
                None => {
                    missing += 1;
                    tri_normals.push(Vec3::ZERO);
                }
            }
            camera_skip.push(mzb::double_sided_skip(
                sub.flags,
                sub.tri_camera_transparent.get(t).copied().unwrap_or(false),
            ));
        }
    }

    if missing > 0 {
        warn!(
            ?file_id,
            missing,
            total_tris = indices.len() / 3,
            "MZB triangles without an authored normal cannot be told floor-from-ceiling, \
             so they will not ground"
        );
    }

    MzbCollisionGeometry {
        cell_index: build_cell_index(&positions, &indices),
        positions,
        indices,
        tri_normals,
        camera_skip,
        source_file_id: file_id,
    }
}

fn build_cell_index(
    positions: &[Vec3],
    indices: &[u32],
) -> std::collections::HashMap<(i32, i32), Vec<u32>> {
    let mut idx: std::collections::HashMap<(i32, i32), Vec<u32>> = std::collections::HashMap::new();
    for (tri_id, tri) in indices.chunks_exact(3).enumerate() {
        let v0 = positions[tri[0] as usize];
        let v1 = positions[tri[1] as usize];
        let v2 = positions[tri[2] as usize];
        let min_x = v0.x.min(v1.x).min(v2.x);
        let max_x = v0.x.max(v1.x).max(v2.x);
        let min_z = v0.z.min(v1.z).min(v2.z);
        let max_z = v0.z.max(v1.z).max(v2.z);
        let cx0 = (min_x / MZB_GRID_CELL).floor() as i32;
        let cx1 = (max_x / MZB_GRID_CELL).floor() as i32;
        let cz0 = (min_z / MZB_GRID_CELL).floor() as i32;
        let cz1 = (max_z / MZB_GRID_CELL).floor() as i32;
        for cz in cz0..=cz1 {
            for cx in cx0..=cx1 {
                idx.entry((cx, cz)).or_default().push(tri_id as u32);
            }
        }
    }
    idx
}

fn ray_tri_intersect(orig: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    const EPS: f32 = 1e-7;
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let h = dir.cross(e2);
    let a = e1.dot(h);
    if a.abs() < EPS {
        return None;
    }
    let f = 1.0 / a;
    let s = orig - v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * e2.dot(q);
    if t > EPS {
        Some(t)
    } else {
        None
    }
}

#[derive(Component)]
pub struct MzbNonCollisionMesh;

pub fn apply_zone_geom_visibility(
    draw: Res<DrawDistance>,
    mut q_collision: Query<&mut Visibility, (With<MzbCollisionMesh>, Without<MzbNonCollisionMesh>)>,
    // WaterPlane is excluded: water surfaces are real visual geometry (the
    // retail client always draws them), not part of the MZB debug overlay.
    // The client leaves `zone_geom_mode` at the default `Off` (the visible
    // world comes from MMB placements), so gating water on it hid every pond
    // in the normal game.
    mut q_noncollision: Query<
        &mut Visibility,
        (
            With<MzbNonCollisionMesh>,
            Without<MzbCollisionMesh>,
            Without<WaterPlane>,
        ),
    >,
) {
    if !draw.is_changed() {
        return;
    }
    let (want_collision, want_noncollision) = match draw.zone_geom_mode {
        ZoneGeomMode::Off => (Visibility::Hidden, Visibility::Hidden),
        ZoneGeomMode::Collision => (Visibility::Inherited, Visibility::Hidden),
        ZoneGeomMode::All => (Visibility::Inherited, Visibility::Inherited),

        ZoneGeomMode::Camera => (Visibility::Inherited, Visibility::Hidden),
    };
    for mut v in q_collision.iter_mut() {
        if *v != want_collision {
            *v = want_collision;
        }
    }
    for mut v in q_noncollision.iter_mut() {
        if *v != want_noncollision {
            *v = want_noncollision;
        }
    }
}

#[derive(Component)]
pub struct MzbOverlay;

#[derive(Component)]
pub struct AutoMzbOverlay;

#[derive(Component)]
pub struct WaterPlane;

// Actual water footprint: the water-material submesh triangles flattened to the
// surface height (world XZ preserved), NOT a bounding box — a box unions
// disconnected ponds and floods the dry paths between them. The depth test clips
// the flattened surface to wherever terrain sits below it.
pub struct WaterSpec {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub min: Vec3,
    pub max: Vec3,
    pub parent: Entity,
    pub auto_loaded: bool,
}

// MZB load computes per-placement water footprints (CPU-side) and queues them
// here; spawn_zone_water streams them in distance-gated and nearest-first, like
// the MMB visual models (process_load_mmb_requests), instead of spawning the whole
// zone's water at once. Cleared on zone change in auto_load_zone_geometry_system.
#[derive(Resource, Default)]
pub struct PendingWaterSpawns {
    pub specs: std::collections::VecDeque<WaterSpec>,
}

// World units covered by one repeat of the water ripple texture. Vanilla water
// mesh UVs are baked as world XZ / WATER_TEX_TILE, so ripple size and scroll
// speed are world-sized and pond-independent, and a single material is shared
// by every pond (see ZoneWaterMaterial).
const WATER_TEX_TILE: f32 = 16.0;

// Scroll velocity in world units/sec (XZ). Gentle drift; sub-tile per second.
const WATER_SCROLL_WORLD: Vec2 = Vec2::new(0.55, 0.35);

#[derive(Message, Debug, Clone, Copy)]
pub struct LoadMzbRequest {
    pub file_id: u32,

    pub chunk_idx: Option<usize>,
    pub world_pos: Vec3,

    pub auto_loaded: bool,
}

pub struct MzbSubMesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,

    pub tri_material: Vec<u8>,

    /// Authored face normal per triangle, mesh-local. MZB winding does not
    /// imply facing — measured over Lower Jeuno / Port Jeuno / Windurst Woods /
    /// West Ronfaure, the winding-derived normal is exactly antiparallel to the
    /// authored one for 70-98% of triangles and parallel for the rest, never
    /// anything between. So `(v1-v0).cross(v2-v0)` recovers the plane but not
    /// which side is up, and only this can tell a floor from a ceiling.
    pub tri_normal: Vec<[f32; 3]>,

    /// Raw per-triangle camera-transparent bit, kept uncombined with [`Self::flags`]
    /// so probes can inspect both inputs to `double_sided_skip` independently.
    pub tri_camera_transparent: Vec<bool>,

    pub flags: u16,
}

pub struct MzbInstance {
    pub submesh_idx: usize,
    pub bevy_transform: Transform,

    pub water_height_bevy: Option<f32>,
}

pub fn load_mzb_placed(
    file_id: u32,
    chunk_idx: Option<usize>,
) -> Result<(Vec<MzbSubMesh>, Vec<MzbInstance>), String> {
    let (header, plain, _chunks) = load_decrypted(file_id, chunk_idx)?;

    // A zero CollisionDataOffset is a legal state, not a degraded parse: the
    // moving-vehicle zones ship no static collision at all and retail skips the
    // whole path (ffxi-dat mzb::MzbHeader::has_collision_data).
    if !header.has_collision_data() {
        info!(
            "MZB {file_id}: no collision section (substructure type {}); zone has no static collision",
            header.substructure_type
        );
        return Ok((Vec::new(), Vec::new()));
    }

    let placements =
        mzb::parse_placements(&plain, &header).map_err(|e| format!("MZB parse_placements: {e}"))?;

    if placements.is_empty() {
        warn!(
            "MZB {file_id}: collision section at 0x{:X} yielded no placements for a {}x{} grid; \
             falling back to unplaced collision meshes",
            header.collision_data_offset,
            header.grid_cells_x(),
            header.grid_cells_z()
        );
        let meshes =
            mzb::parse_meshes(&plain, &header).map_err(|e| format!("MZB parse_meshes: {e}"))?;

        let pool = AsyncComputeTaskPool::get();
        let baked: Vec<Option<MzbSubMesh>> = pool.scope(|s| {
            for m in &meshes {
                s.spawn(async move {
                    if m.vertices.is_empty() || m.triangles.is_empty() {
                        None
                    } else {
                        Some(bake_submesh(m))
                    }
                });
            }
        });
        let mut submeshes = Vec::with_capacity(baked.len());
        let mut instances = Vec::with_capacity(baked.len());
        for sub in baked.into_iter().flatten() {
            let idx = submeshes.len();
            submeshes.push(sub);
            instances.push(MzbInstance {
                submesh_idx: idx,
                bevy_transform: Transform::IDENTITY,
                water_height_bevy: None,
            });
        }
        return Ok((submeshes, instances));
    }

    let mut unique_offsets: Vec<u32> = Vec::new();
    let mut offset_to_idx: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for p in &placements {
        if let std::collections::hash_map::Entry::Vacant(e) = offset_to_idx.entry(p.geometry_offset)
        {
            e.insert(unique_offsets.len());
            unique_offsets.push(p.geometry_offset);
        }
    }

    let pool = AsyncComputeTaskPool::get();
    let baked: Vec<Option<MzbSubMesh>> = pool.scope(|s| {
        let plain_ref = &plain;
        for &offset in &unique_offsets {
            s.spawn(async move {
                let m = mzb::parse_mesh_at(plain_ref, offset as usize).ok()?;
                if m.vertices.is_empty() || m.triangles.is_empty() {
                    return None;
                }
                Some(bake_submesh(&m))
            });
        }
    });

    let mut submeshes: Vec<MzbSubMesh> = Vec::with_capacity(baked.len());
    let mut unique_to_dense: Vec<Option<usize>> = Vec::with_capacity(baked.len());
    for sub in baked {
        match sub {
            Some(s) => {
                unique_to_dense.push(Some(submeshes.len()));
                submeshes.push(s);
            }
            None => unique_to_dense.push(None),
        }
    }
    let mut instances: Vec<MzbInstance> = Vec::with_capacity(placements.len());

    for p in placements {
        let Some(&unique_idx) = offset_to_idx.get(&p.geometry_offset) else {
            continue;
        };
        let Some(idx) = unique_to_dense[unique_idx] else {
            continue;
        };

        let m_native = Mat4::from_cols_array(&p.transform);

        let to_bevy = Mat4::from_cols(
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, -1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        );
        let m_bevy = to_bevy * m_native;

        let water_height_bevy = p.water_height.map(|h| -h);
        instances.push(MzbInstance {
            submesh_idx: idx,
            bevy_transform: Transform::from_matrix(m_bevy),
            water_height_bevy,
        });
    }

    Ok((submeshes, instances))
}

fn bake_submesh(m: &mzb::MzbMesh) -> MzbSubMesh {
    let positions: Vec<[f32; 3]> = m.vertices.iter().map(|v| v.pos).collect();
    let indices: Vec<u32> = m
        .triangles
        .iter()
        .flat_map(|t| [t[0], t[1], t[2]])
        .collect();
    let tri_material: Vec<u8> = m.tri_info.iter().map(|t| t.material).collect();
    let tri_normal: Vec<[f32; 3]> = m
        .triangle_normals
        .iter()
        .map(|&ni| m.normals.get(ni as usize).map_or([0.0; 3], |n| n.n))
        .collect();
    let tri_camera_transparent: Vec<bool> =
        m.tri_info.iter().map(|t| t.camera_transparent).collect();
    MzbSubMesh {
        positions,
        indices,
        tri_material,
        tri_normal,
        tri_camera_transparent,
        flags: m.flags,
    }
}

fn load_decrypted(
    file_id: u32,
    chunk_idx: Option<usize>,
) -> Result<(mzb::MzbHeader, Vec<u8>, ()), String> {
    let root =
        DatRoot::from_env_or_default().map_err(|e| format!("DatRoot::from_env_or_default: {e}"))?;
    let location = root
        .resolve(file_id)
        .map_err(|e| format!("resolve({file_id}): {e}"))?;
    let path = location.path_under(root.root());
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();

    let (idx, chunk) = match chunk_idx {
        Some(i) => (
            i,
            chunks
                .get(i)
                .ok_or_else(|| format!("chunk_idx {i} out of range ({} chunks)", chunks.len()))?,
        ),
        None => chunks
            .iter()
            .enumerate()
            .find(|(_, c)| c.kind == ChunkKind::Mzb as u8)
            .ok_or_else(|| {
                format!(
                    "no MZB (kind 0x1C) chunk in file_id {file_id} ({} chunks)",
                    chunks.len()
                )
            })?,
    };
    if chunk.kind != ChunkKind::Mzb as u8 {
        return Err(format!(
            "chunk[{idx}] kind=0x{:02X} ({:?}), not an MZB",
            chunk.kind,
            ChunkKind::label(chunk.kind),
        ));
    }

    let plain = mzb::decrypt(chunk.data).map_err(|e| format!("MZB decrypt: {e}"))?;
    let header = mzb::MzbHeader::parse(&plain).map_err(|e| format!("MZB header: {e}"))?;
    Ok((header, plain, ()))
}

#[derive(Debug, Clone, Copy)]
pub struct ZoneMmbSpawn {
    pub chunk_idx: usize,
    pub bevy_transform: Mat4,
    // Set for generator-driven water sheets (sea1/sea2): translucent tint +
    // per-layer UV-scroll. None for ordinary object-placed models.
    pub water: Option<crate::dat_mmb::GenWater>,

    // None when the placement resolved to a single mesh for all three LOD bands
    // (the majority), which needs no per-frame variant pick.
    pub lod: Option<ZoneMeshLod>,
}

/// World-space (Bevy frame) footprint of one area-bound placement.
#[derive(Debug, Clone, Copy)]
pub struct ZoneAreaBox {
    pub area_id: AreaResourceId,
    pub min: Vec3,
    pub max: Vec3,
}

impl ZoneAreaBox {
    /// The block is "under the actor" when its footprint covers them and their
    /// feet sit inside its height, slackened by [`AREA_FOOT_SLACK`].
    ///
    /// Two block shapes carry an area and both have to answer yes: a room shell,
    /// which encloses the actor outright, and a floor slab, whose top *is* the
    /// surface the actor stands on and so coincides with their feet.
    fn holds(&self, p: Vec3) -> bool {
        block_holds(self.min, self.max, p)
    }

    fn footprint(&self) -> f32 {
        block_footprint(self.min, self.max)
    }
}

fn block_holds(min: Vec3, max: Vec3, p: Vec3) -> bool {
    p.x >= min.x
        && p.x <= max.x
        && p.z >= min.z
        && p.z <= max.z
        && p.y >= min.y - AREA_FOOT_SLACK
        && p.y <= max.y + AREA_FOOT_SLACK
}

/// Ground area the block covers. The tie-break between overlapping boxes is
/// horizontal, not volumetric: zones ship area-bound sheets with no thickness at
/// all (Al'Taieu's `ev02` planes), and a zero-volume box would otherwise outrank
/// every real interior it crosses.
fn block_footprint(min: Vec3, max: Vec3) -> f32 {
    (max.x - min.x).max(0.0) * (max.z - min.z).max(0.0)
}

/// Vertical slack on [`ZoneAreaBox::holds`]. An area box is the *render* mesh's
/// bounds while the player's Y comes from the MZB *collision* surface, so a floor
/// slab's top lands near — not exactly at — the feet standing on it. Sized by
/// [`MAX_GROUND_STEP_UP`]: a gap the walker would step over is still one floor.
const AREA_FOOT_SLACK: f32 = MAX_GROUND_STEP_UP;

/// One chunk's authored light binding with the footprint it lights.
///
/// Retail resolves `LightReferences[4]` once at load and holds the four D3D
/// slots for the whole chunk (ZoneRenderer.cpp:284-313), so the set is a
/// property of the geometry, never of the camera.
#[derive(Debug, Clone, Copy)]
pub struct ZoneChunkLightBox {
    pub min: Vec3,
    pub max: Vec3,
    pub lights: [Option<mzb::LightId>; mzb::LIGHT_REFERENCE_COUNT],
}

#[derive(Clone)]
pub struct ZoneMmbBuild {
    pub spawns: Vec<ZoneMmbSpawn>,
    pub area_boxes: Vec<ZoneAreaBox>,
    pub light_boxes: Vec<ZoneChunkLightBox>,
}

/// Which [`AreaResourceId`] each point of the zone belongs to.
///
/// Retail asks the collision map for the FourCC of the block under the actor and
/// draws that actor's fog and ambient from the matching `XiArea`
/// (CollidableActor.cpp:218-228, SkeletalMeshActor.cpp:2743-2749). Our MZB
/// collision section carries no area id — only the render placements do
/// (ZoneBlockFormat.h:99) — so the block is found by its own bounds instead.
/// The areas that differ from the zone environment are building interiors and
/// the blocks that floor them, so [`ZoneAreaBox::holds`] answers the same
/// question the ground query does.
#[derive(Resource, Default)]
pub struct ZoneAreaMap {
    pub boxes: Vec<ZoneAreaBox>,

    /// DAT file the boxes came from, so a stale zone's interiors can't keep
    /// tinting the new one (same reason [`MzbCollisionGeometry::source_file_id`]
    /// exists).
    pub source_file_id: Option<u32>,
}

impl ZoneAreaMap {
    /// The area at `p`, or `None` for the zone-wide environment.
    ///
    /// Innermost wins: areas nest (a room inside a building shell), and retail's
    /// per-block answer is the most specific block holding the actor.
    pub fn area_at(&self, p: Vec3) -> Option<AreaResourceId> {
        self.boxes
            .iter()
            .filter(|b| b.holds(p))
            .min_by(|a, b| {
                a.footprint()
                    .partial_cmp(&b.footprint())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|b| b.area_id)
    }
}

/// Which lights each point of the zone is authored to receive.
///
/// Retail binds a chunk's `LightReferences[4]` into D3D light slots 2-5 once at
/// load (research/XIClient/src/XIClient/source/Rendering/ZoneRenderer.cpp:284-313,
/// :339-353) and every model drawn over that chunk keeps those slots, so the
/// point lights on an actor are the ones its chunk authors — not the ones that
/// happen to be nearest. `boxes` is empty for a zone that ships no light-binding
/// table, which is the only case the nearest-N pick still answers.
#[derive(Resource, Default)]
pub struct ZoneChunkLightMap {
    pub boxes: Vec<ZoneChunkLightBox>,

    /// DAT file the boxes came from, so a stale zone cannot keep lighting the
    /// new one (same reason [`ZoneAreaMap::source_file_id`] exists).
    pub source_file_id: Option<u32>,
}

impl ZoneChunkLightMap {
    pub fn is_authored(&self) -> bool {
        !self.boxes.is_empty()
    }

    /// The authored lights of the chunk at `p`, innermost chunk first — the same
    /// most-specific-block answer [`ZoneAreaMap::area_at`] gives.
    pub fn lights_at(&self, p: Vec3) -> Option<[Option<mzb::LightId>; mzb::LIGHT_REFERENCE_COUNT]> {
        self.boxes
            .iter()
            .filter(|b| block_holds(b.min, b.max, p))
            .min_by(|a, b| {
                block_footprint(a.min, a.max)
                    .partial_cmp(&block_footprint(b.min, b.max))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|b| b.lights)
    }
}

/// The per-frame mesh-variant pick retail makes in `RenderChunk2`
/// (research/XIClient/src/XIClient/source/Rendering/ZoneRenderer.cpp:1085-1094).
/// One placement spawns one entity per *distinct* mesh in its
/// [`mzb::MmbLodSet`]; `level_mask` says which distance bands that entity serves.
#[derive(Component, Debug, Clone, Copy)]
pub struct ZoneMeshLod {
    pub thresholds: mzb::MmbLodThresholds,
    pub level_mask: u8,
    pub uses_lod_rendering: bool,
}

impl ZoneMeshLod {
    pub fn is_drawn_at(&self, camera_dist_sq: f32) -> bool {
        if self.uses_lod_rendering && mzb::beyond_lod_far_cull(camera_dist_sq, self.thresholds) {
            return false;
        }
        self.thresholds.select(camera_dist_sq).mask() & self.level_mask != 0
    }
}

pub fn build_zone_mmb_spawns(
    file_id: u32,
    chunk_idx: Option<usize>,
) -> Result<ZoneMmbBuild, String> {
    let root =
        DatRoot::from_env_or_default().map_err(|e| format!("DatRoot::from_env_or_default: {e}"))?;
    let location = root
        .resolve(file_id)
        .map_err(|e| format!("resolve({file_id}): {e}"))?;
    let path = location.path_under(root.root());
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();

    let pool = AsyncComputeTaskPool::get();
    let mmb_chunk_refs: Vec<(usize, &[u8])> = chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.kind == ChunkKind::Mmb as u8)
        .map(|(idx, c)| (idx, c.data))
        .collect();
    type ParsedMmb = (usize, String, Option<([f32; 3], [f32; 3])>);
    let parsed: Vec<Option<ParsedMmb>> = pool.scope(|s| {
        for (idx, data) in &mmb_chunk_refs {
            let idx = *idx;
            let data = *data;
            s.spawn(async move {
                let dec = mmb::decrypt(data).ok()?;
                let hdr = MmbHeader::parse(&dec).ok()?;

                Some((idx, hdr.zone_mesh_name(), hdr.local_bounds()))
            });
        }
    });
    let mut mmb_names: Vec<String> = Vec::with_capacity(parsed.len());
    let mut mmb_indices: Vec<usize> = Vec::with_capacity(parsed.len());
    let mut mmb_bounds: Vec<Option<([f32; 3], [f32; 3])>> = Vec::with_capacity(parsed.len());
    for entry in parsed.into_iter().flatten() {
        mmb_indices.push(entry.0);
        mmb_names.push(entry.1);
        mmb_bounds.push(entry.2);
    }

    use std::collections::HashMap;
    let mut name_to_locals: HashMap<&str, Vec<usize>> = HashMap::new();
    for (local, name) in mmb_names.iter().enumerate() {
        if !name.is_empty() {
            name_to_locals.entry(name.as_str()).or_default().push(local);
        }
    }

    // A generator's linked model resolves by the MMB chunk's 4-byte DatId (chunk
    // name), which is NOT the same as the MMB header's zone_mesh_name — e.g. Port
    // Windurst "taki" (DatId) carries zone_mesh_name "takin". Keyed on chunks index
    // so it maps straight to ZoneMmbSpawn.chunk_idx.
    let mut datid_to_chunk_idx: HashMap<String, usize> = HashMap::new();
    for (idx, c) in chunks.iter().enumerate() {
        if c.kind != ChunkKind::Mmb as u8 {
            continue;
        }
        let id = String::from_utf8_lossy(&c.name)
            .trim_end_matches('\0')
            .trim_end()
            .to_string();
        if !id.is_empty() {
            datid_to_chunk_idx.entry(id).or_insert(idx);
        }
    }

    let (_, mzb_chunk) = match chunk_idx {
        Some(i) => (
            i,
            chunks
                .get(i)
                .ok_or_else(|| format!("chunk_idx {i} out of range ({} chunks)", chunks.len()))?,
        ),
        None => chunks
            .iter()
            .enumerate()
            .find(|(_, c)| c.kind == ChunkKind::Mzb as u8)
            .ok_or_else(|| {
                format!(
                    "no MZB chunk in file_id {file_id} ({} chunks)",
                    chunks.len()
                )
            })?,
    };
    let plain = mzb::decrypt(mzb_chunk.data).map_err(|e| format!("MZB decrypt: {e}"))?;
    let header = mzb::MzbHeader::parse(&plain).map_err(|e| format!("MZB header: {e}"))?;
    let placements = mzb::parse_mmb_placements(&plain, &header)
        .map_err(|e| format!("MZB parse_mmb_placements: {e}"))?;

    // The auto-load path never swaps an interior in, so every shell stays up —
    // retail's "not inside a building" state. `/subarea` is the manual way in.
    const ACTIVE_SUB_AREA: Option<u32> = None;
    let drawn_flags = mzb::drawn_placements(&placements, ACTIVE_SUB_AREA);

    let sub_areas = match sub_area::from_dat(&bytes) {
        Ok(s) => s,
        Err(e) => {
            warn!("MZB {file_id}: sub-area section unreadable ({e}); no interiors offered");
            Vec::new()
        }
    };
    if !sub_areas.is_empty() {
        info!(
            "MZB {file_id}: {} sub-area interior(s) declared but not loaded — ids {:?} (`/subarea`)",
            sub_areas.len(),
            sub_areas.iter().map(|s| s.id).collect::<Vec<_>>()
        );
    }
    let undeclared = sub_area::undeclared_placeholder_links(&sub_areas, &placements);
    if !undeclared.is_empty() {
        warn!(
            "MZB {file_id}: placement sub-area links {undeclared:?} have no trigger rect; \
             retail could never swap those shells either"
        );
    }

    let mut rr_cursor: HashMap<String, usize> = HashMap::new();
    let mut gated = 0usize;
    let mut unresolved = 0usize;
    let mut lod_families = 0usize;
    let mut out = Vec::with_capacity(placements.len());
    let mut area_boxes: Vec<ZoneAreaBox> = Vec::new();
    let light_bindings = mzb::parse_light_bindings(&plain, &header);
    let mut light_boxes: Vec<ZoneChunkLightBox> = Vec::new();
    for (p, &drawn) in placements.iter().zip(drawn_flags.iter()) {
        let id = p.id_str().trim_end_matches('\0').trim_end();

        // Retail's BlockManager.GetByName answers one mesh per name; duplicate
        // zone_mesh_names inside one zone file are an ambiguity only this client
        // has, and successive placements take successive duplicates. Only the
        // placement's own name advances that cursor, so the LOD sibling lookups
        // cannot shuffle which duplicate a later placement gets. Advance it even
        // for gated placements: retail resolves every chunk's mesh LOD before
        // SetRenderTypes classifies it (ZoneRenderer.cpp:406 ResolveMeshReference ->
        // InitializeMeshLOD :100-143, then :572).
        let mut base_pick: Option<Option<usize>> = None;
        let lod_set = mzb::resolve_mmb_lod_set_with(id, |name| {
            if name == id {
                if base_pick.is_none() {
                    base_pick = Some(name_to_locals.get(name).map(|locals| {
                        let cursor = rr_cursor.entry(name.to_string()).or_insert(0);
                        let local = locals[*cursor % locals.len()];
                        *cursor += 1;
                        local
                    }));
                }
                return base_pick.flatten();
            }
            name_to_locals.get(name).and_then(|l| l.first().copied())
        });

        // Retail composes Identity -> scale -> RotateX -> RotateY -> RotateZ ->
        // translate in D3D row-vector convention (research/XIClient/src/XIClient/
        // source/Rendering/ZoneRenderer.cpp:430-443, with Matrix4.cpp:200-204
        // `MultiplyRight` = this*r and :508-511 `out = v*M`), which transposes to
        // column-vector T*Rz*Ry*Rx*S — glam's *extrinsic* XYZ. The intrinsic
        // `EulerRot::XYZ` is Rx*Ry*Rz, the reverse. Self-proven by the
        // order-reversed inverse chain at ZoneRenderer.cpp:459-466.
        let m_ffxi = Mat4::from_scale_rotation_translation(
            Vec3::new(p.scale[0], p.scale[1], p.scale[2]),
            Quat::from_euler(EulerRot::XYZEx, p.rot[0], p.rot[1], p.rot[2]),
            Vec3::new(p.trans[0], p.trans[1], p.trans[2]),
        );

        let to_bevy = Mat4::from_cols(
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, -1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        );
        let bevy_transform = to_bevy * m_ffxi;

        // Built before the render gate: retail binds every positioned block to
        // its area (ZoneRenderer.cpp:710-718) and the interiors that carry one
        // are `_`-keyed blocks the *second* draw pass owns, so a gate-filtered
        // list would drop exactly the areas we need.
        let area_id = p.effective_area_resource_id();
        if area_id != 0 {
            if let Some(bounds) = lod_set
                .distinct_indices()
                .first()
                .and_then(|&local| mmb_bounds[local])
            {
                let (min, max) = world_bounds_from_local(bevy_transform, bounds.0, bounds.1);
                area_boxes.push(ZoneAreaBox { area_id, min, max });
            }
        }

        if !drawn {
            gated += 1;
            continue;
        }
        let variants = lod_set.distinct_indices();
        if variants.is_empty() {
            unresolved += 1;
            continue;
        }

        let chunk_lights = mzb::resolve_chunk_lights(&p.light_references, &light_bindings);
        if chunk_lights.iter().any(Option::is_some) {
            if let Some(bounds) = variants.first().and_then(|&local| mmb_bounds[local]) {
                let (min, max) = world_bounds_from_local(bevy_transform, bounds.0, bounds.1);
                light_boxes.push(ZoneChunkLightBox {
                    min,
                    max,
                    lights: chunk_lights,
                });
            }
        }

        let uses_lod_rendering = p.uses_lod_rendering();
        if variants.len() > 1 {
            lod_families += 1;
        }
        // A placement whose three bands all land on one mesh and that carries no
        // far cull has nothing to decide per frame, so it stays a plain always-on
        // spawn rather than paying for a distance query.
        let needs_lod_component = variants.len() > 1 || uses_lod_rendering;
        for local in variants {
            out.push(ZoneMmbSpawn {
                chunk_idx: mmb_indices[local],
                bevy_transform,
                water: None,
                lod: needs_lod_component.then(|| ZoneMeshLod {
                    thresholds: p.lod_thresholds(),
                    level_mask: lod_set.level_mask(local),
                    uses_lod_rendering,
                }),
            });
        }
    }

    if gated > 0 || unresolved > 0 {
        info!(
            "MZB {file_id}: RenderType gate skipped {gated}/{} placements (zone-line stand-ins, \
             event geometry, sub-area placeholders), {unresolved} resolved to no mesh block; \
             {} spawned across {lod_families} multi-LOD families",
            placements.len(),
            out.len()
        );
    }

    // Generator-driven water sheets: FFXI instances the broad canal/harbor water
    // (e.g. Port Windurst tshimonosea1/2) via zone Generator chunks whose linked
    // model name resolves to an MMB zone-mesh — NOT via the object list above.
    // Each carries a translucent tint (alpha < 1) and per-layer UV-scroll. See
    // ffxi-dat Generator::parse_model_spawn.
    let zone_prefix = mzb::infer_zone_prefix(&mmb_names);
    for c in &chunks {
        if c.kind != ChunkKind::Generator as u8 {
            continue;
        }
        // research/xim EnvironmentManager.updateWeatherEffects + Particle.kt:232-258:
        // the weat/<type>/ sky generators (cloud canopies cld1/cld2 and per-weather
        // variants like ~4cl) set the follow_camera config bit (0x0004) — they are
        // camera-relative sky registered through EffectManager, NOT world geometry.
        // They share the water signature below (singleton + uv_scroll), so a
        // name-based skip missed variants and spawned the cloud dome as a static
        // sheet draped over the zone (kuluu-nfrp). Reject any camera-follow
        // generator; real water (sea1/sea2, izu*) is world-anchored (follow=false).
        let follows_camera = ffxi_dat::generator::Generator::parse_cloud_generator(c.name, c.data)
            .ok()
            .flatten()
            .is_some_and(|d| d.follow_camera);
        if follows_camera {
            continue;
        }
        let Ok(Some(ms)) = ffxi_dat::generator::Generator::parse_model_spawn(c.data) else {
            continue;
        };
        // Scrolling sheets are the water surfaces (sea1/sea2 scroll their UVs);
        // static model-spawns (ships, floors, collision hulls) have zero scroll
        // and are left to the normal geometry path — spawning them here would
        // give them the translucent water material.
        if ms.uv_scroll == [0.0, 0.0] {
            continue;
        }
        // Singletons (max_life_frames == 0) are static sheets. life > 0 generators
        // (e.g. Port Windurst "rivs", Bastok "tki*") emit particles over time and
        // belong to the particle path; taking them here would double-render them.
        let is_singleton = ffxi_dat::particle_gen::ParticleGeneratorDef::parse(c.data)
            .ok()
            .flatten()
            .is_none_or(|d| d.max_life_frames == 0.0);
        if !is_singleton {
            continue;
        }
        let name = ms.model_name_str().trim_end();
        // Resolve by the MMB DatId first (the generator's own linkage), then fall
        // back to the header zone_mesh_name path. Only names that resolve to an MMB
        // zone-mesh are water sheets; other 0x0B generators link D3M billboards.
        let Some(chunk_idx) = datid_to_chunk_idx.get(name).copied().or_else(|| {
            mzb::resolve_mmb_index(name, &zone_prefix, &mmb_names).map(|local| mmb_indices[local])
        }) else {
            continue;
        };
        let b = ms.base_position;
        // Same basis as object placements: to_bevy * (FFXI model-local -> world),
        // with the generator's 0x0F scale / 0x09 rotation applied — the sea
        // sheets are tiny tiles (lowsea AABB ±1.4) the generator blows up 500×,
        // so a translation-only transform renders the sea as a speck. A
        // pre-flipped translation would move the origin correctly but leave the
        // model geometry unflipped (mirrored/sunk out of view).
        let to_bevy = Mat4::from_cols(
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, -1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        );
        let m_ffxi = Mat4::from_scale_rotation_translation(
            Vec3::from_array(ms.scale),
            Quat::from_euler(
                EulerRot::XYZ,
                ms.rotation[0],
                ms.rotation[1],
                ms.rotation[2],
            ),
            Vec3::new(b[0], b[1], b[2]),
        );
        let bevy_transform = to_bevy * m_ffxi;
        let (local_min, local_max) = chunks
            .get(chunk_idx)
            .and_then(|c| mmb::decrypt(c.data).ok())
            .and_then(|d| MmbHeader::parse(&d).ok().and_then(|h| h.local_bounds()))
            .unwrap_or(([0.0; 3], [0.0; 3]));
        let (world_min, world_max) = world_bounds_from_local(bevy_transform, local_min, local_max);
        out.push(ZoneMmbSpawn {
            chunk_idx,
            bevy_transform,
            water: Some(crate::dat_mmb::GenWater {
                tint: Vec4::from_array(ms.tint),
                uv_scroll: Vec2::new(ms.uv_scroll[0], ms.uv_scroll[1]),
                world_min,
                world_max,
            }),
            // Generator sheets carry no placement record, so no LOD triple.
            lod: None,
        });
    }

    let diag_enabled = match std::env::var("FFXI_DIAG_ZONE_GEOM") {
        Ok(s) if s == "*" || s == "all" || s.eq_ignore_ascii_case("any") => true,
        Ok(s) => s.parse::<u32>().ok() == Some(file_id),
        _ => false,
    };
    if diag_enabled {
        use std::collections::HashMap;

        let mut name_counts: HashMap<&str, u32> = HashMap::new();
        for n in &mmb_names {
            *name_counts.entry(n.trim_end()).or_insert(0) += 1;
        }
        let mut dup_names: Vec<(&str, u32)> = name_counts
            .iter()
            .filter(|(_, &c)| c > 1)
            .map(|(&n, &c)| (n, c))
            .collect();
        dup_names.sort_by_key(|x| std::cmp::Reverse(x.1));

        let mut placement_id_counts: HashMap<String, u32> = HashMap::new();
        let mut bucket0: Vec<String> = Vec::new();
        let mut bucket1: u32 = 0;
        let mut bucket_many: Vec<(String, usize)> = Vec::new();
        for p in &placements {
            let id = p.id_str().trim_end_matches('\0').trim_end().to_string();
            *placement_id_counts.entry(id.clone()).or_insert(0) += 1;
            let matches_len = name_to_locals.get(id.as_str()).map_or(0, |v| v.len());
            match matches_len {
                0 => bucket0.push(id),
                1 => bucket1 += 1,
                n => bucket_many.push((id, n)),
            }
        }

        let mut roundrobin_smoke: Vec<(String, u32, usize)> = Vec::new();
        for (id, count) in &placement_id_counts {
            if *count < 2 {
                continue;
            }
            let m = name_to_locals.get(id.as_str()).map_or(0, |v| v.len());
            if m > 1 {
                roundrobin_smoke.push((id.clone(), *count, m));
            }
        }
        roundrobin_smoke.sort_by_key(|x| std::cmp::Reverse(x.1));

        let mut unmatched_unique: HashMap<String, u32> = HashMap::new();
        for id in &bucket0 {
            *unmatched_unique.entry(id.clone()).or_insert(0) += 1;
        }
        let mut um_list: Vec<(String, u32)> = unmatched_unique.into_iter().collect();
        um_list.sort_by_key(|x| std::cmp::Reverse(x.1));

        info!(
            target: "ffxi_viewer_core::dat_mzb::diag",
            file_id,
            placements = placements.len(),
            spawned = out.len(),
            mmb_names = mmb_names.len(),
            distinct_names = name_to_locals.len(),
            dup_asset_names = dup_names.len(),
            match0 = bucket0.len(),
            match1 = bucket1,
            match_many = bucket_many.len(),
            roundrobin_smoke = roundrobin_smoke.len(),
            "DIAG-zonegeom summary",
        );
        if !dup_names.is_empty() {
            let head: Vec<&(&str, u32)> = dup_names.iter().take(20).collect();
            info!(
                target: "ffxi_viewer_core::dat_mzb::diag",
                "DIAG-zonegeom duplicate mmb asset_names (top 20): {head:?}",
            );
        }
        if !um_list.is_empty() {
            let head: Vec<&(String, u32)> = um_list.iter().take(20).collect();
            info!(
                target: "ffxi_viewer_core::dat_mzb::diag",
                "DIAG-zonegeom unmatched placement ids (id × count, top 20): {head:?}",
            );
        }
        if !roundrobin_smoke.is_empty() {
            let head: Vec<&(String, u32, usize)> = roundrobin_smoke.iter().take(20).collect();
            info!(
                target: "ffxi_viewer_core::dat_mzb::diag",
                "DIAG-zonegeom round-robin smoke (id, placement_count, matches, top 20): {head:?}",
            );
        }

        if !out.is_empty() {
            let mut tx_min = Vec3::splat(f32::INFINITY);
            let mut tx_max = Vec3::splat(f32::NEG_INFINITY);
            let mut sc_min = Vec3::splat(f32::INFINITY);
            let mut sc_max = Vec3::splat(f32::NEG_INFINITY);
            let mut tiny_scale: Vec<(usize, [f32; 3])> = Vec::new();
            let mut sample: Vec<(usize, [f32; 3], [f32; 3])> = Vec::new();
            for sp in out.iter() {
                let (scale, _rot, trans) = sp.bevy_transform.to_scale_rotation_translation();
                tx_min = tx_min.min(trans);
                tx_max = tx_max.max(trans);
                sc_min = sc_min.min(scale);
                sc_max = sc_max.max(scale);
                if scale.length() < 1e-3 {
                    tiny_scale.push((sp.chunk_idx, [scale.x, scale.y, scale.z]));
                }
                if sample.len() < 5 {
                    sample.push((
                        sp.chunk_idx,
                        [trans.x, trans.y, trans.z],
                        [scale.x, scale.y, scale.z],
                    ));
                }
            }
            info!(
                target: "ffxi_viewer_core::dat_mzb::diag",
                tx_min = ?[tx_min.x, tx_min.y, tx_min.z],
                tx_max = ?[tx_max.x, tx_max.y, tx_max.z],
                sc_min = ?[sc_min.x, sc_min.y, sc_min.z],
                sc_max = ?[sc_max.x, sc_max.y, sc_max.z],
                tiny_scale_n = tiny_scale.len(),
                "DIAG-zonegeom transform extents (Bevy frame)",
            );
            if !tiny_scale.is_empty() {
                let head: Vec<&(usize, [f32; 3])> = tiny_scale.iter().take(10).collect();
                info!(
                    target: "ffxi_viewer_core::dat_mzb::diag",
                    "DIAG-zonegeom tiny-scale spawns (chunk_idx, scale.xyz, top 10): {head:?}",
                );
            }
            info!(
                target: "ffxi_viewer_core::dat_mzb::diag",
                "DIAG-zonegeom sample spawns (chunk_idx, trans.xyz, scale.xyz, first 5): {sample:?}",
            );
        }
    }

    if !area_boxes.is_empty() {
        let ids = mzb::area_resource_ids(&placements);
        info!(
            file_id,
            "MZB: {} placements bound to {} area(s) {:?} — per-area fog/diffuse lighting",
            area_boxes.len(),
            ids.len(),
            ids.iter().map(area_id_label).collect::<Vec<_>>(),
        );
    }

    log_light_binding_coverage(file_id, &chunks, &light_bindings, &light_boxes);

    Ok(ZoneMmbBuild {
        spawns: out,
        area_boxes,
        light_boxes,
    })
}

/// The authored bindings only light a chunk if the zone also ships the Generator
/// chunk that defines the light, and a zone can ship neither. Both gaps are
/// silent at the pixel level, so they are reported at load.
fn log_light_binding_coverage(
    file_id: u32,
    chunks: &[ffxi_dat::chunk::Chunk<'_>],
    light_bindings: &[mzb::LightId],
    light_boxes: &[ZoneChunkLightBox],
) {
    if light_bindings.is_empty() {
        info!(
            file_id,
            "MZB: no authored light-binding table — chunk point lighting falls back to the \
             nearest-N pick"
        );
        return;
    }
    let defined: std::collections::HashSet<mzb::LightId> = chunks
        .iter()
        .filter(|c| ChunkKind::from_u8(c.kind) == Some(ChunkKind::Generator))
        .filter(|c| {
            matches!(
                ffxi_dat::generator::Generator::parse_point_light(c.data),
                Ok(Some(_))
            )
        })
        .map(|c| u32::from_le_bytes(c.name))
        .collect();
    let missing: Vec<String> = light_bindings
        .iter()
        .filter(|id| !defined.contains(id))
        .map(light_id_label)
        .collect();
    info!(
        file_id,
        "MZB: {} authored light(s) bound to {} chunk(s) — static per-chunk light slots",
        light_bindings.len(),
        light_boxes.len(),
    );
    if !missing.is_empty() {
        warn!(
            file_id,
            "MZB: {} authored light(s) have no point-light Generator chunk {:?} — the chunks \
             binding them light with fewer slots than retail",
            missing.len(),
            missing,
        );
    }
}

/// FourCC of a [`mzb::LightId`] as authored, for logs.
pub fn light_id_label(id: &mzb::LightId) -> String {
    id.to_le_bytes()
        .iter()
        .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
        .collect()
}

/// FourCC of an [`AreaResourceId`] as authored, for logs.
pub fn area_id_label(id: &AreaResourceId) -> String {
    id.to_le_bytes()
        .iter()
        .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
        .collect()
}

fn world_bounds_from_local(
    transform: Mat4,
    local_min: [f32; 3],
    local_max: [f32; 3],
) -> (Vec3, Vec3) {
    let mut world_min = Vec3::splat(f32::INFINITY);
    let mut world_max = Vec3::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let p = Vec3::new(
            if corner & 1 == 0 {
                local_min[0]
            } else {
                local_max[0]
            },
            if corner & 2 == 0 {
                local_min[1]
            } else {
                local_max[1]
            },
            if corner & 4 == 0 {
                local_min[2]
            } else {
                local_max[2]
            },
        );
        let w = transform.transform_point3(p);
        world_min = world_min.min(w);
        world_max = world_max.max(w);
    }
    (world_min, world_max)
}

/// One entry of [`zone_sub_areas`]: a building interior the zone can swap in,
/// paired with whether its DAT is actually present in this install.
#[derive(Debug, Clone)]
pub struct ZoneSubArea {
    pub sub_area: sub_area::SubArea,
    pub resolves: bool,
}

/// The sub-areas declared by the zone DAT `file_id`, ascending by id.
///
/// A declared sub-area whose interior DAT is missing stays in the list with
/// `resolves == false` rather than being dropped, so a gap reads as a gap.
pub fn zone_sub_areas(file_id: u32) -> Result<Vec<ZoneSubArea>, String> {
    let root =
        DatRoot::from_env_or_default().map_err(|e| format!("DatRoot::from_env_or_default: {e}"))?;
    let location = root
        .resolve(file_id)
        .map_err(|e| format!("resolve({file_id}): {e}"))?;
    let path = location.path_under(root.root());
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(sub_area::from_dat(&bytes)
        .map_err(|e| format!("sub-area parse of file {file_id}: {e}"))?
        .into_iter()
        .map(|s| ZoneSubArea {
            resolves: root.resolve(s.file_id).is_ok(),
            sub_area: s,
        })
        .collect())
}

pub fn load_mzb(file_id: u32, chunk_idx: Option<usize>) -> Result<Vec<MzbSubMesh>, String> {
    let root =
        DatRoot::from_env_or_default().map_err(|e| format!("DatRoot::from_env_or_default: {e}"))?;
    let location = root
        .resolve(file_id)
        .map_err(|e| format!("resolve({file_id}): {e}"))?;
    let path = location.path_under(root.root());
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let chunks: Vec<_> = walk(&bytes).filter_map(Result::ok).collect();

    let (idx, chunk) = match chunk_idx {
        Some(i) => (
            i,
            chunks
                .get(i)
                .ok_or_else(|| format!("chunk_idx {i} out of range ({} chunks)", chunks.len()))?,
        ),
        None => chunks
            .iter()
            .enumerate()
            .find(|(_, c)| c.kind == ChunkKind::Mzb as u8)
            .ok_or_else(|| {
                format!(
                    "no MZB (kind 0x1C) chunk in file_id {file_id} ({} chunks)",
                    chunks.len()
                )
            })?,
    };
    if chunk.kind != ChunkKind::Mzb as u8 {
        return Err(format!(
            "chunk[{idx}] kind=0x{:02X} ({:?}), not an MZB",
            chunk.kind,
            ChunkKind::label(chunk.kind),
        ));
    }

    let (_header, meshes) =
        mzb::parse_all(chunk.data).map_err(|e| format!("MZB parse_all: {e}"))?;

    let mut out = Vec::with_capacity(meshes.len());
    for m in meshes {
        if m.vertices.is_empty() || m.triangles.is_empty() {
            continue;
        }
        out.push(bake_submesh(&m));
    }
    Ok(out)
}

pub fn kick_load_mzb_tasks(
    mut events: MessageReader<LoadMzbRequest>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut toasts: MessageWriter<crate::snapshot::ToastEvent>,
    draw: Res<DrawDistance>,
    mut collision_geometry: ResMut<MzbCollisionGeometry>,
    mut area_map: ResMut<ZoneAreaMap>,
    mut chunk_light_map: ResMut<ZoneChunkLightMap>,
    mut load_mmb_tx: MessageWriter<crate::dat_mmb::LoadMmbRequest>,
    mut pending_water: ResMut<PendingWaterSpawns>,
    mut in_flight: ResMut<LoadMzbInFlight>,
    mut cache: ResMut<ZoneGeomCache>,
) {
    let init_vis = compute_init_visibility(draw.zone_geom_mode);
    for req in events.read() {
        if let Some(geom) = cache.get_and_promote(req.file_id) {
            spawn_mzb_overlay(
                *req,
                &geom,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut toasts,
                &mut collision_geometry,
                &mut area_map,
                &mut chunk_light_map,
                &mut load_mmb_tx,
                &mut pending_water,
                init_vis,
                true,
            );
            continue;
        }

        if let Some((reqs, _)) = in_flight.tasks.get_mut(&req.file_id) {
            reqs.push(*req);
            continue;
        }

        let file_id = req.file_id;
        let chunk_idx = req.chunk_idx;
        let pool = AsyncComputeTaskPool::get();
        let task = pool.spawn(async move {
            let (submeshes, instances) = match load_mzb_placed(file_id, chunk_idx) {
                Ok(s) => s,
                Err(msg) => {
                    return LoadedZoneGeom {
                        submeshes: Arc::new(Vec::new()),
                        instances: Arc::new(Vec::new()),
                        mmb_spawns: Err(msg),
                    };
                }
            };
            let mmb_spawns = build_zone_mmb_spawns(file_id, chunk_idx);
            LoadedZoneGeom {
                submeshes: Arc::new(submeshes),
                instances: Arc::new(instances),
                mmb_spawns,
            }
        });
        in_flight.tasks.insert(file_id, (vec![*req], task));
    }
}

pub fn poll_load_mzb_tasks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut toasts: MessageWriter<crate::snapshot::ToastEvent>,
    draw: Res<DrawDistance>,
    mut collision_geometry: ResMut<MzbCollisionGeometry>,
    mut area_map: ResMut<ZoneAreaMap>,
    mut chunk_light_map: ResMut<ZoneChunkLightMap>,
    mut load_mmb_tx: MessageWriter<crate::dat_mmb::LoadMmbRequest>,
    mut pending_water: ResMut<PendingWaterSpawns>,
    mut in_flight: ResMut<LoadMzbInFlight>,
    mut cache: ResMut<ZoneGeomCache>,
) {
    let init_vis = compute_init_visibility(draw.zone_geom_mode);

    let mut completed: Vec<(u32, Vec<LoadMzbRequest>, LoadedZoneGeom)> = Vec::new();
    in_flight.tasks.retain(|file_id, (reqs, task)| {
        match future::block_on(future::poll_once(task)) {
            Some(geom) => {
                completed.push((*file_id, std::mem::take(reqs), geom));
                false
            }
            None => true,
        }
    });
    for (file_id, reqs, geom) in completed {
        let cache_eligible = !geom.submeshes.is_empty() && !geom.instances.is_empty();
        if cache_eligible {
            cache.insert(file_id, geom.clone());
        }
        for req in reqs {
            spawn_mzb_overlay(
                req,
                &geom,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut toasts,
                &mut collision_geometry,
                &mut area_map,
                &mut chunk_light_map,
                &mut load_mmb_tx,
                &mut pending_water,
                init_vis,
                false,
            );
        }
    }
}

fn compute_init_visibility(mode: ZoneGeomMode) -> (Visibility, Visibility) {
    match mode {
        ZoneGeomMode::Off => (Visibility::Hidden, Visibility::Hidden),
        ZoneGeomMode::Collision | ZoneGeomMode::Camera => {
            (Visibility::Inherited, Visibility::Hidden)
        }
        ZoneGeomMode::All => (Visibility::Inherited, Visibility::Inherited),
    }
}

// Flat water tint — linear-space conversion of the old StandardMaterial
// placeholder srgba(0.20, 0.30, 0.31, 0.40). The procedural ripple texture
// modulates it (a stand-in until the retail scrolling water texture set
// (MMB 0x8000 section) is parsed); unlike the old PBR material, this runs the
// FFXI zone lighting model, so ponds track zone time-of-day/weather light
// like the terrain.
const WATER_TINT: Vec4 = Vec4::new(0.033, 0.073, 0.078, 0.40);

/// Shared handle for the vanilla water-surface material, so `scroll_water_uv`
/// can integrate `uv_offset` on one asset instead of per-spawn clones.
#[derive(Resource, Default)]
pub struct ZoneWaterMaterial(pub Option<Handle<crate::ffxi_zone_material::FfxiZoneMaterial>>);

fn simple_water_material(texture: Handle<Image>) -> crate::ffxi_zone_material::FfxiZoneMaterial {
    crate::ffxi_zone_material::FfxiZoneMaterial::new(
        Some(texture),
        crate::skinned_ffxi_material::FfxiMaterialFlags {
            // (has_texture, blend-emit [0x8000 translucent path], unused,
            // discard threshold — 0 so the cutout test never fires).
            flags: Vec4::new(1.0, 1.0, 0.0, 0.0),
        },
        WATER_TINT,
        Vec4::ZERO,
        AlphaMode::Blend,
        crate::ffxi_zone_material::FfxiZoneMaterialKey {
            // The old StandardMaterial was double-sided (cull_mode: None).
            back_face_culling: false,
            mirrored: false,
            // Bounding-box plane is near-coplanar with the sloped pond bed at
            // the shoreline; the decal polygon-offset pulls the surface toward
            // the camera so it wins the depth test there (replaces the old
            // constant `depth_bias: 1000.0`).
            z_bias_level: 1,
            depth_write: false,
        },
    )
}

/// Scrolls the shared water material's UVs. Runs on the single cached asset in
/// [`ZoneWaterMaterial`]; every water plane in the zone shares it.
pub fn scroll_water_uv(
    time: Res<Time>,
    water_mat: Res<ZoneWaterMaterial>,
    mut materials: ResMut<Assets<crate::ffxi_zone_material::FfxiZoneMaterial>>,
) {
    let Some(handle) = water_mat.0.as_ref() else {
        return;
    };
    // get_mut_untracked: uv_offset flows to the GPU through the persistent
    // buffers in upload_zone_material_buffers; marking the asset Modified here
    // would needlessly rebuild its bind group every frame (same pattern as
    // zone_clouds).
    if let Some(material) = materials.get_mut_untracked(handle) {
        // fract() keeps the offset small so f32 precision holds over long
        // sessions; the texture repeats, so a whole-tile jump is invisible.
        let t = time.elapsed_secs();
        let uv = Vec2::new(
            (t * WATER_SCROLL_WORLD.x / WATER_TEX_TILE).fract(),
            (t * WATER_SCROLL_WORLD.y / WATER_TEX_TILE).fract(),
        );
        material.uv_offset = Vec4::new(uv.x, uv.y, 0.0, 0.0);
    }
}

// Tileable procedural ripple: low-contrast sum of integer-frequency sine bands
// so opposite edges match exactly. Stands in until the DAT-sourced water
// texture set is parsed; the scroll mechanism (uv_transform animation) is the
// part that carries over.
fn water_ripple_image() -> Image {
    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    const N: usize = 64;
    let mut data = Vec::with_capacity(N * N * 4);
    let tau = std::f32::consts::TAU;
    for y in 0..N {
        for x in 0..N {
            let u = x as f32 / N as f32;
            let v = y as f32 / N as f32;
            // Integer wave vectors -> exact tiling at the texture border.
            let w = (tau * (3.0 * u + v)).sin()
                + (tau * (u - 2.0 * v)).sin()
                + 0.5 * (tau * (5.0 * u + 4.0 * v)).sin();
            // w in [-2.5, 2.5] -> luma around 1.0 with subtle modulation, so
            // the material's base_color tint still sets the overall look.
            let l = 1.0 + 0.10 * (w / 2.5);
            let b = (l.clamp(0.0, 1.0) * 255.0) as u8;
            data.extend_from_slice(&[b, b, b, 255]);
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    img
}

// `world_tile_uvs`: the vanilla FFXI water material wants world XZ /
// WATER_TEX_TILE baked into the mesh, so the shared material's ripples are
// world-sized and continuous across ponds. bevy_water (enhanced) instead wants
// UVs normalised over the footprint bounds, so its `coord_offset`/
// `coord_scale` recover world coords for a continuous, world-scaled wave
// field.
fn build_water_surface_mesh(spec: &WaterSpec, world_tile_uvs: bool) -> Mesh {
    let dx = (spec.max.x - spec.min.x).max(0.01);
    let dz = (spec.max.z - spec.min.z).max(0.01);
    let uvs: Vec<[f32; 2]> = if world_tile_uvs {
        spec.positions
            .iter()
            .map(|p| [p[0] / WATER_TEX_TILE, p[2] / WATER_TEX_TILE])
            .collect()
    } else {
        spec.positions
            .iter()
            .map(|p| [(p[0] - spec.min.x) / dx, (p[2] - spec.min.z) / dz])
            .collect()
    };
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, spec.positions.clone());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0, 1.0, 0.0]; spec.positions.len()],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    // Neutral 0.5 vertex colour: the zone shader's XIM `2 · vertexColor`
    // overbright convention makes 0.5 the identity, leaving the water colour
    // entirely to WATER_TINT.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[0.5, 0.5, 0.5, 1.0]; spec.positions.len()],
    );
    mesh.insert_indices(Indices::U32(spec.indices.clone()));
    mesh
}

// Drains water footprints queued by the MZB load and spawns one surface each:
// the vanilla translucent plane, or — when built with `enhanced-water` and the
// GraphicsSettings toggle is on — bevy_water's animated material on the same
// footprint mesh. Reads the setting at drain time, so a toggle change takes
// effect on the next zone (re)load.
fn water_dist_sq_xz(spec: &WaterSpec, self_pos: Vec3) -> f32 {
    let cx = 0.5 * (spec.min.x + spec.max.x);
    let cz = 0.5 * (spec.min.z + spec.max.z);
    let dx = cx - self_pos.x;
    let dz = cz - self_pos.z;
    dx * dx + dz * dz
}

pub fn spawn_zone_water(
    mut commands: Commands,
    mut pending: ResMut<PendingWaterSpawns>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::ffxi_zone_material::FfxiZoneMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut water_mat: ResMut<ZoneWaterMaterial>,
    settings: Res<crate::graphics::GraphicsSettings>,
    self_q: Query<&GlobalTransform, With<IsSelf>>,
    #[cfg(feature = "enhanced-water")] mut water_materials: ResMut<
        Assets<crate::water_enhanced::StandardWaterMaterial>,
    >,
) {
    if pending.specs.is_empty() {
        return;
    }

    let self_pos = self_q.single().ok().map(|t| t.translation());
    if let Some(self_pos) = self_pos {
        pending.specs.make_contiguous().sort_by(|a, b| {
            water_dist_sq_xz(a, self_pos).total_cmp(&water_dist_sq_xz(b, self_pos))
        });
    }
    let load_radius = settings.view_distance * MMB_LOAD_DISTANCE_MARGIN;
    let load_radius_sq = load_radius * load_radius;
    let enhanced = cfg!(feature = "enhanced-water") && settings.enhanced_water;
    let simple_mat = water_mat
        .0
        .get_or_insert_with(|| {
            materials.add(simple_water_material(images.add(water_ripple_image())))
        })
        .clone();

    // Water surfaces are real visual geometry, not part of the MZB debug
    // overlay: the retail client always draws them regardless of the debug
    // zone-geom mode, and the client leaves `zone_geom_mode` at the default
    // `Off` (the visible world comes from MMB placements). Gating on the mode
    // here spawned every pond Hidden in the normal game. WaterPlane is also
    // excluded from apply_zone_geom_visibility's non-collision query so the
    // debug toggle never hides water after the fact.
    let water_vis = Visibility::Inherited;

    const WATER_SPAWN_BUDGET: usize = 32;
    let mut spawned = 0usize;
    let mut retained: std::collections::VecDeque<WaterSpec> =
        std::collections::VecDeque::with_capacity(pending.specs.len());

    while let Some(spec) = pending.specs.pop_front() {
        if let Some(self_pos) = self_pos {
            // Sorted nearest-first, so the first out-of-range spec means the rest
            // are too — retain them all for when the player moves closer.
            if water_dist_sq_xz(&spec, self_pos) > load_radius_sq {
                retained.push_back(spec);
                retained.append(&mut pending.specs);
                break;
            }
        }
        if spawned >= WATER_SPAWN_BUDGET {
            retained.push_back(spec);
            continue;
        }
        spawned += 1;

        let mesh = Mesh3d(meshes.add(build_water_surface_mesh(&spec, !enhanced)));
        let mut e;
        #[cfg(feature = "enhanced-water")]
        if enhanced {
            let mat = crate::water_enhanced::pond_water_material(
                &mut water_materials,
                spec.min,
                spec.max,
            );
            e = commands.spawn((
                MzbOverlay,
                WaterPlane,
                MzbNonCollisionMesh,
                mesh,
                mat,
                Transform::IDENTITY,
                water_vis,
                bevy::light::NotShadowReceiver,
                ChildOf(spec.parent),
            ));
            if spec.auto_loaded {
                e.insert(AutoMzbOverlay);
            }
            continue;
        }

        e = commands.spawn((
            MzbOverlay,
            WaterPlane,
            MzbNonCollisionMesh,
            mesh,
            MeshMaterial3d(simple_mat.clone()),
            Transform::IDENTITY,
            water_vis,
            bevy::light::NotShadowReceiver,
            ChildOf(spec.parent),
        ));
        if spec.auto_loaded {
            e.insert(AutoMzbOverlay);
        }
    }
    pending.specs = retained;
    let _ = enhanced;
}

#[allow(clippy::too_many_arguments)]
fn spawn_mzb_overlay(
    req: LoadMzbRequest,
    geom: &LoadedZoneGeom,
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    toasts: &mut MessageWriter<crate::snapshot::ToastEvent>,
    collision_geometry: &mut ResMut<MzbCollisionGeometry>,
    area_map: &mut ResMut<ZoneAreaMap>,
    chunk_light_map: &mut ResMut<ZoneChunkLightMap>,
    load_mmb_tx: &mut MessageWriter<crate::dat_mmb::LoadMmbRequest>,
    pending_water: &mut PendingWaterSpawns,
    init_vis: (Visibility, Visibility),
    _from_cache: bool,
) {
    let (init_collision_vis, init_noncollision_vis) = init_vis;

    // Ahead of the no-geometry bail below: the areas describe where the player
    // stands, which is a question the fog answers even for a zone whose
    // collision section is empty.
    if let Ok(build) = &geom.mmb_spawns {
        area_map.boxes = build
            .area_boxes
            .iter()
            .map(|b| ZoneAreaBox {
                area_id: b.area_id,
                min: b.min + req.world_pos,
                max: b.max + req.world_pos,
            })
            .collect();
        area_map.source_file_id = Some(req.file_id);

        chunk_light_map.boxes = build
            .light_boxes
            .iter()
            .map(|b| ZoneChunkLightBox {
                min: b.min + req.world_pos,
                max: b.max + req.world_pos,
                lights: b.lights,
            })
            .collect();
        chunk_light_map.source_file_id = Some(req.file_id);
    }

    let submeshes: &[MzbSubMesh] = geom.submeshes.as_slice();
    let instances: &[MzbInstance] = geom.instances.as_slice();
    if submeshes.is_empty() || instances.is_empty() {
        let err_detail = match &geom.mmb_spawns {
            Err(msg) => format!(" — load error: {msg}"),
            Ok(_) => String::new(),
        };
        push_system_msg(
            toasts,
            format!(
                "/load_mzb {}: 0 renderable meshes ({} submeshes, {} instances){}",
                req.file_id,
                submeshes.len(),
                instances.len(),
                err_detail,
            ),
        );
        return;
    }

    let n_submeshes = submeshes.len();
    let n_instances = instances.len();

    let collision_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        cull_mode: None,
        ..default()
    });
    let noncollision_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        cull_mode: None,
        ..default()
    });

    let mut parent_spawn = commands.spawn((
        crate::components::InGameEntity,
        MzbOverlay,
        Transform::from_translation(req.world_pos),
        Visibility::default(),
    ));
    if req.auto_loaded {
        parent_spawn.insert(AutoMzbOverlay);
    }
    let parent = parent_spawn.id();

    let mut collision_positions: Vec<[f32; 3]> = Vec::new();
    let mut collision_indices: Vec<u32> = Vec::new();
    let mut collision_tri_mat: Vec<u8> = Vec::new();
    let mut noncollision_positions: Vec<[f32; 3]> = Vec::new();
    let mut noncollision_indices: Vec<u32> = Vec::new();
    let mut noncollision_tri_mat: Vec<u8> = Vec::new();

    for inst in instances.iter() {
        let sub = &submeshes[inst.submesh_idx];
        let blocks_los = sub.flags & 1 == 0;
        let (positions, indices, tri_mat) = if blocks_los {
            (
                &mut collision_positions,
                &mut collision_indices,
                &mut collision_tri_mat,
            )
        } else {
            (
                &mut noncollision_positions,
                &mut noncollision_indices,
                &mut noncollision_tri_mat,
            )
        };
        let base = positions.len() as u32;
        for v in &sub.positions {
            let p = inst
                .bevy_transform
                .transform_point(Vec3::new(v[0], v[1], v[2]));
            positions.push([p.x, p.y, p.z]);
        }
        for &i in &sub.indices {
            indices.push(i + base);
        }
        tri_mat.extend_from_slice(&sub.tri_material);
    }

    let spawn_merged = |commands: &mut Commands,
                        positions: Vec<[f32; 3]>,
                        indices: Vec<u32>,
                        tri_mat: Vec<u8>,
                        material: Handle<StandardMaterial>,
                        parent: bevy::ecs::entity::Entity,
                        auto_loaded: bool,
                        is_collision: bool,
                        init_vis: Visibility,
                        meshes: &mut ResMut<Assets<Mesh>>| {
        if positions.is_empty() || indices.is_empty() {
            return;
        }

        let mut vert_mat: Vec<u8> = vec![0u8; positions.len()];
        for (tri_idx, tri) in indices.chunks_exact(3).enumerate() {
            let m = tri_mat.get(tri_idx).copied().unwrap_or(0);
            vert_mat[tri[0] as usize] = m;
            vert_mat[tri[1] as usize] = m;
            vert_mat[tri[2] as usize] = m;
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_indices(Indices::U32(indices));

        mesh.compute_smooth_normals();

        if let Some(normals) = mesh
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .and_then(|a| a.as_float3())
        {
            let colors: Vec<[f32; 4]> = normals
                .iter()
                .zip(vert_mat.iter())
                .map(|(n, &m)| {
                    let shade = 0.4 + 0.6 * (n[1] * 0.5 + 0.5);
                    let pal = MZB_MATERIAL_PALETTE[(m & 0x0F) as usize];
                    [pal[0] * shade, pal[1] * shade, pal[2] * shade, 1.0]
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
        let mut child = commands.spawn((
            MzbOverlay,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            init_vis,
            ChildOf(parent),
        ));
        if is_collision {
            child.insert(MzbCollisionMesh);
        } else {
            child.insert(MzbNonCollisionMesh);
        }
        if auto_loaded {
            child.insert(AutoMzbOverlay);
        }
    };

    let collision_verts = collision_positions.len();
    let collision_tris = collision_indices.len() / 3;
    let noncollision_verts = noncollision_positions.len();
    let noncollision_tris = noncollision_indices.len() / 3;

    **collision_geometry = build_collision_geometry(submeshes, instances, Some(req.file_id));

    spawn_merged(
        commands,
        collision_positions,
        collision_indices,
        collision_tri_mat,
        collision_mat,
        parent,
        req.auto_loaded,
        true,
        init_collision_vis,
        meshes,
    );
    spawn_merged(
        commands,
        noncollision_positions,
        noncollision_indices,
        noncollision_tri_mat,
        noncollision_mat,
        parent,
        req.auto_loaded,
        false,
        init_noncollision_vis,
        meshes,
    );

    let total_verts = collision_verts + noncollision_verts;
    let total_tris = collision_tris + noncollision_tris;
    push_system_msg(
            toasts,
            format!(
                "/load_mzb {}: {n_submeshes} submeshes, {n_instances} placements → merged {total_verts} verts / {total_tris} tris ({collision_verts}v {collision_tris}t collision, {noncollision_verts}v {noncollision_tris}t non-collision)",
                req.file_id,
            ),
        );

    // One localized footprint per water-material placement (NOT merged by height),
    // so spawn_zone_water can distance-gate each like an MMB placement. Merging the
    // whole zone's water into one mesh would make it un-streamable.
    //
    // Footprints already covered by a generator water sheet are dropped: the
    // sheet IS the retail water visual there, and the placeholder plane
    // double-drawing under it reads as a darker rectangle with hard seams.
    // The Y window absorbs the collision-vs-visual gap (Lower Jeuno: collision
    // water_height 24 vs sheet surface 17.5, a 6.5-unit offset) without eating
    // genuinely separate ponds high above a sheet.
    const GEN_SHEET_SUPPRESS_Y: f32 = 10.0;
    let sheets: Vec<(Vec3, Vec3)> = geom
        .mmb_spawns
        .as_ref()
        .map(|build| {
            build
                .spawns
                .iter()
                .filter_map(|s| s.water.map(|w| (w.world_min, w.world_max)))
                .collect()
        })
        .unwrap_or_default();
    let mut water_added = 0usize;
    let mut water_suppressed = 0usize;
    for inst in instances.iter() {
        let Some(h_bevy) = inst.water_height_bevy else {
            continue;
        };
        let sub = &submeshes[inst.submesh_idx];
        if sub.positions.is_empty() || sub.indices.is_empty() {
            continue;
        }

        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(sub.positions.len());
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in &sub.positions {
            let p = inst
                .bevy_transform
                .transform_point(Vec3::new(v[0], v[1], v[2]));
            // Flatten to the flat water surface; keep world XZ to follow the
            // actual shoreline rather than a bounding box.
            let flat = [p.x, h_bevy, p.z];
            min = min.min(Vec3::from_array(flat));
            max = max.max(Vec3::from_array(flat));
            positions.push(flat);
        }
        let center = 0.5 * (min + max);
        let covered = sheets.iter().any(|(smin, smax)| {
            center.x >= smin.x
                && center.x <= smax.x
                && center.z >= smin.z
                && center.z <= smax.z
                && h_bevy >= smin.y - GEN_SHEET_SUPPRESS_Y
                && h_bevy <= smax.y + GEN_SHEET_SUPPRESS_Y
        });
        if covered {
            water_suppressed += 1;
            continue;
        }
        pending_water.specs.push_back(WaterSpec {
            positions,
            indices: sub.indices.clone(),
            min,
            max,
            parent,
            auto_loaded: req.auto_loaded,
        });
        water_added += 1;
    }
    if water_added > 0 || water_suppressed > 0 {
        push_system_msg(
            toasts,
            format!(
                "/load_mzb {}: {} water surface{} queued ({} under generator sheets)",
                req.file_id,
                water_added,
                if water_added == 1 { "" } else { "s" },
                water_suppressed,
            ),
        );
    }

    match &geom.mmb_spawns {
        Ok(build) => {
            let n = build.spawns.len();
            let offset = Mat4::from_translation(req.world_pos);
            for s in &build.spawns {
                load_mmb_tx.write(crate::dat_mmb::LoadMmbRequest {
                    file_id: req.file_id,
                    chunk_idx: s.chunk_idx,
                    world_pos: Vec3::ZERO,
                    entity_id: None,
                    world_transform: Some(offset * s.bevy_transform),
                    water: s.water,
                    lod: s.lod,
                });
            }
            push_system_msg(
                toasts,
                format!(
                    "/load_mzb {}: queued {n} visual MMB placements",
                    req.file_id
                ),
            );
        }
        Err(msg) => {
            push_system_msg(
                toasts,
                format!("/load_mzb {}: zone-MMB spawn: {msg}", req.file_id),
            );
        }
    }
}

#[derive(Resource, Default)]
pub struct LastAutoLoadedZone {
    pub file_id: Option<u32>,
}

pub fn auto_load_zone_geometry_system(
    scene_state: Res<SceneState>,
    mut toasts: MessageWriter<crate::snapshot::ToastEvent>,
    mut last: ResMut<LastAutoLoadedZone>,
    mut commands: Commands,
    mut load_tx: MessageWriter<LoadMzbRequest>,
    auto_q: Query<Entity, With<AutoMzbOverlay>>,
    mut mzb_in_flight: ResMut<LoadMzbInFlight>,
    mut mmb_queue: ResMut<crate::dat_mmb::MmbLoadQueue>,
    mut mmb_in_flight: ResMut<crate::dat_mmb::MmbLoadInFlight>,
    mut pending_water: ResMut<PendingWaterSpawns>,
    mut collision_geometry: ResMut<MzbCollisionGeometry>,
    mut area_map: ResMut<ZoneAreaMap>,
) {
    let current = crate::snapshot::effective_zone_file_id(&scene_state.snapshot);
    if current == last.file_id {
        return;
    }

    for e in auto_q.iter() {
        if let Ok(mut ec) = commands.get_entity(e) {
            ec.despawn();
        }
    }

    // Keeping the old zone's triangles until the new load lands grounds
    // entities against geometry they're not in: entering a Mog House snapped
    // the player onto a city surface at the MH-origin column, and the
    // nearest-floor snap then resolved that stuck Y to the MH model's roof.
    if collision_geometry.source_file_id != current {
        *collision_geometry = MzbCollisionGeometry::default();
    }
    if area_map.source_file_id != current {
        *area_map = ZoneAreaMap::default();
    }

    if !mzb_in_flight.tasks.is_empty() {
        mzb_in_flight.tasks.clear();
    }
    mmb_queue
        .pending
        .retain(|r| !(r.entity_id.is_none() && r.world_transform.is_some()));
    mmb_in_flight.tasks.clear();
    // Drop any old-zone water footprints still queued for streaming; the spawned
    // ones go with the despawned AutoMzbOverlay parent above.
    pending_water.specs.clear();
    last.file_id = current;

    match current {
        Some(file_id) => {
            load_tx.write(LoadMzbRequest {
                file_id,
                chunk_idx: None,
                world_pos: Vec3::ZERO,
                auto_loaded: true,
            });

            let zone_label = scene_state
                .snapshot
                .zone_id
                .map_or_else(|| "?".to_string(), |z| z.to_string());
            let myroom_label = scene_state
                .snapshot
                .myroom
                .map(|m| format!(" (Mog House model {})", m.model))
                .unwrap_or_default();
            push_system_msg(
                &mut toasts,
                format!("auto-load: zone {zone_label}{myroom_label} -> DAT file {file_id}"),
            );
        }
        None => {
            let Some(zone_id) = scene_state.snapshot.zone_id else {
                return;
            };
            push_system_msg(
                &mut toasts,
                format!("auto-load: no DAT mapping for zone {zone_id} (Phase 11b table pending)"),
            );
        }
    }
}

/// research/XIClient/src/XIClient/source/Rendering/ZoneRenderer.cpp:1071-1094 —
/// `RenderChunk2` measures from the camera eye to the placement translation, culls
/// on the squared Lod far distance when the chunk is flagged `UsesLodRendering`,
/// and then hands the device whichever of the h/m/l variants the squared distance
/// falls into.
pub fn select_zone_mmb_lod(
    camera_q: Query<&GlobalTransform, With<crate::camera::OperatorCamera>>,
    mut lod_q: Query<(&GlobalTransform, &ZoneMeshLod, &mut Visibility)>,
) {
    let Ok(camera_t) = camera_q.single() else {
        return;
    };
    let eye = camera_t.translation();

    for (chunk_t, lod, mut vis) in lod_q.iter_mut() {
        let want = if lod.is_drawn_at(chunk_t.translation().distance_squared(eye)) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
}

pub fn cull_mzb_by_distance(
    draw: Res<DrawDistance>,
    self_q: Query<&GlobalTransform, With<IsSelf>>,

    mut mzb_q: Query<(&GlobalTransform, &mut Visibility), (With<MzbOverlay>, With<Mesh3d>)>,
) {
    let Ok(self_t) = self_q.single() else {
        return;
    };
    let self_pos = self_t.translation();
    let cull_sq = draw.world * draw.world;

    for (mzb_t, mut vis) in mzb_q.iter_mut() {
        let mzb_pos = mzb_t.translation();

        let dx = mzb_pos.x - self_pos.x;
        let dz = mzb_pos.z - self_pos.z;
        let d_sq = dx * dx + dz * dz;
        let want = if d_sq > cull_sq {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };

        if *vis != want {
            *vis = want;
        }
    }
}

pub fn cull_entities_by_distance(
    draw: Res<DrawDistance>,
    self_q: Query<&GlobalTransform, With<IsSelf>>,
    mut ent_q: Query<(&WorldEntity, &GlobalTransform, &mut Visibility), Without<IsSelf>>,
) {
    let Ok(self_t) = self_q.single() else {
        return;
    };
    let self_pos = self_t.translation();
    let cull_sq = draw.mob * draw.mob;

    for (ent, ent_t, mut vis) in ent_q.iter_mut() {
        if matches!(ent.kind, EntityKind::Pc) {
            if *vis != Visibility::Inherited {
                *vis = Visibility::Inherited;
            }
            continue;
        }
        let p = ent_t.translation();
        let dx = p.x - self_pos.x;
        let dz = p.z - self_pos.z;
        let want = if dx * dx + dz * dz > cull_sq {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *vis != want {
            *vis = want;
        }
    }
}

fn push_system_msg(toasts: &mut MessageWriter<crate::snapshot::ToastEvent>, text: String) {
    toasts.write(crate::snapshot::ToastEvent::debug(text));
}

#[cfg(test)]
mod placement_transform_tests {
    use super::*;

    #[test]
    fn placement_euler_is_extrinsic_rz_ry_rx() {
        let (rx, ry, rz) = (0.3_f32, -0.7, 1.1);
        let composed = Quat::from_euler(EulerRot::XYZEx, rx, ry, rz);
        let by_hand =
            Quat::from_rotation_z(rz) * Quat::from_rotation_y(ry) * Quat::from_rotation_x(rx);
        assert!(
            composed.abs_diff_eq(by_hand, 1e-6),
            "{composed:?} != {by_hand:?}"
        );
        // A placement rotated on one axis cannot tell the two orders apart, which
        // is why the intrinsic form went unnoticed; a multi-axis one must.
        assert!(!composed.abs_diff_eq(Quat::from_euler(EulerRot::XYZ, rx, ry, rz), 1e-3));
    }
}

#[cfg(test)]
mod ground_tests {
    use super::*;

    fn floor_at(h: f32) -> ([Vec3; 4], [u32; 6]) {
        (
            [
                Vec3::new(-4.0, h, -4.0),
                Vec3::new(4.0, h, -4.0),
                Vec3::new(4.0, h, 4.0),
                Vec3::new(-4.0, h, 4.0),
            ],
            [0, 1, 2, 0, 2, 3],
        )
    }

    /// `facings` is the authored normal per slab; `Vec3::Y` is a floor,
    /// `Vec3::NEG_Y` a ceiling. MZB winding does not imply facing, so these are
    /// independent of the vertex order in [`floor_at`].
    fn slabs(slabs: &[(f32, Vec3)]) -> MzbCollisionGeometry {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let mut tri_normals = Vec::new();
        for (h, facing) in slabs {
            let (verts, idx) = floor_at(*h);
            let base = positions.len() as u32;
            positions.extend_from_slice(&verts);
            indices.extend(idx.iter().map(|i| base + i));
            tri_normals.extend([*facing; 2]);
        }
        MzbCollisionGeometry {
            positions,
            indices,
            tri_normals,
            camera_skip: Vec::new(),
            cell_index: std::collections::HashMap::new(),
            source_file_id: None,
        }
    }

    fn two_floors(low: f32, high: f32) -> MzbCollisionGeometry {
        slabs(&[(low, Vec3::Y), (high, Vec3::Y)])
    }

    /// Two instances of one submesh: `camera_skip` must be per *placed* triangle,
    /// so the submesh's pattern repeats once per instance and stays aligned with
    /// `indices.chunks(3)`.
    #[test]
    fn camera_skip_is_per_placed_triangle() {
        let sub = |flags: u16, transparent: [bool; 2]| MzbSubMesh {
            positions: floor_at(0.0)
                .0
                .to_vec()
                .iter()
                .map(|v| v.to_array())
                .collect(),
            indices: floor_at(0.0).1.to_vec(),
            tri_material: vec![0; 2],
            tri_normal: vec![[0.0, 1.0, 0.0]; 2],
            tri_camera_transparent: transparent.to_vec(),
            flags,
        };
        // Submesh 0 is camera-transparent on its second triangle; submesh 1 has
        // the bit set but flags == 0, so the mesh gate must veto it.
        let submeshes = vec![sub(1, [false, true]), sub(0, [true, true])];
        let instances = vec![
            MzbInstance {
                submesh_idx: 0,
                bevy_transform: Transform::IDENTITY,
                water_height_bevy: None,
            },
            MzbInstance {
                submesh_idx: 0,
                bevy_transform: Transform::from_xyz(50.0, 0.0, 0.0),
                water_height_bevy: None,
            },
            MzbInstance {
                submesh_idx: 1,
                bevy_transform: Transform::from_xyz(100.0, 0.0, 0.0),
                water_height_bevy: None,
            },
        ];
        let geom = build_collision_geometry(&submeshes, &instances, None);

        assert_eq!(geom.camera_skip.len(), geom.tri_count());
        assert_eq!(
            geom.camera_skip,
            vec![false, true, false, true, false, false],
            "the submesh pattern repeats per instance, and flags == 0 vetoes"
        );
        assert_eq!(
            geom.camera_triangles().len(),
            geom.tri_count() - 2,
            "camera_triangles drops exactly the skipped ones"
        );
    }

    #[test]
    fn ground_nearest_is_fixed_point() {
        let geom = two_floors(0.0, 4.0);
        let g = geom.ground_nearest(Vec2::ZERO, 0.0).unwrap();
        assert_eq!(g, 0.0, "a grounded entity's nearest floor is its own floor");
        assert_eq!(
            geom.ground_nearest(Vec2::ZERO, g).unwrap(),
            g,
            "re-running on the result is stable (no per-frame oscillation)"
        );
    }

    #[test]
    fn ground_nearest_picks_own_level_not_floor_above() {
        let geom = two_floors(0.0, 4.0);
        assert_eq!(
            geom.ground_nearest(Vec2::ZERO, 4.3).unwrap(),
            4.0,
            "standing on the upper floor stays on it, not snapped down"
        );
        assert_eq!(
            geom.ground_nearest(Vec2::ZERO, 0.3).unwrap(),
            0.0,
            "near the lower floor stays low, not pulled up to the floor above"
        );
    }

    #[test]
    fn ground_nearest_ignores_ceilings() {
        // The Lower Jeuno shape that stranded the player on a roof (kuluu-0nnl):
        // street at 1.0, and the slab roofing the walkway below presenting its
        // underside at 6.44 and its walkable top at 7.01.
        let geom = slabs(&[
            (1.0, Vec3::Y),
            (6.44, Vec3::NEG_Y),
            (7.01, Vec3::Y),
            (9.2, Vec3::NEG_Y),
        ]);
        let at = |ref_y| geom.ground_nearest(Vec2::ZERO, ref_y).expect("a floor");
        assert!(
            (at(1.0) - 1.0).abs() < 1e-3,
            "standing on the street stays on the street, got {}",
            at(1.0)
        );
        assert!(
            (at(6.0) - 7.01).abs() < 1e-3,
            "a ceiling underside is not a landing surface; the slab's top is, got {}",
            at(6.0)
        );
    }

    #[test]
    fn ground_nearest_rejects_a_ceiling_only_column() {
        let geom = slabs(&[(6.44, Vec3::NEG_Y)]);
        assert_eq!(
            geom.ground_nearest(Vec2::ZERO, 1.0),
            None,
            "a column holding only a ceiling grounds nowhere, rather than snapping up to it"
        );
    }

    #[test]
    fn ground_step_refuses_an_out_of_reach_floor() {
        // Lower Jeuno step 42: the street tile is missing from this column, and
        // the only up-facing floor is the roof 6.1 above. Unbounded, that was a
        // single snap onto the roof (kuluu-0nnl).
        let geom = slabs(&[(7.13, Vec3::Y)]);
        assert_eq!(
            geom.ground_step(Vec2::ZERO, 1.0, MAX_GROUND_STEP_UP),
            None,
            "a floor beyond step-up range is not a landing surface"
        );
        assert!(
            geom.ground_nearest(Vec2::ZERO, 1.0).is_some(),
            "…while unbounded nearest-floor still finds it, which is the old bug"
        );
    }

    #[test]
    fn ground_step_climbs_a_stair_and_falls_freely() {
        // Walking onto the next tread: this column holds only the higher step.
        let tread = slabs(&[(0.4, Vec3::Y)]);
        let up = tread.ground_step(Vec2::ZERO, 0.0, MAX_GROUND_STEP_UP);
        assert!(
            up.is_some_and(|y| (y - 0.4).abs() < 1e-3),
            "a stair riser inside the bound still climbs, got {up:?}"
        );

        let geom = slabs(&[(0.4, Vec3::Y), (12.0, Vec3::Y)]);
        let down = geom.ground_step(Vec2::ZERO, 12.0, MAX_GROUND_STEP_UP);
        assert!(
            down.is_some_and(|y| (y - 12.0).abs() < 1e-3),
            "standing on the top floor is a fixed point, got {down:?}"
        );
        // Off the ledge: the upper floor is gone from this column, leaving a
        // drop of 11.6 — far past the step-up bound, which must not clamp it.
        let below = slabs(&[(0.4, Vec3::Y)]);
        let fall = below.ground_step(Vec2::ZERO, 12.0, MAX_GROUND_STEP_UP);
        assert!(
            fall.is_some_and(|y| (y - 0.4).abs() < 1e-3),
            "descent is unbounded — walking off a ledge falls, got {fall:?}"
        );
    }

    #[test]
    fn ground_nearest_grounds_entity_below_floor() {
        let geom = two_floors(0.0, 4.0);
        assert_eq!(
            geom.ground_nearest(Vec2::ZERO, -50.0).unwrap(),
            0.0,
            "a pathing entity sent a flat reference Y far below ground still snaps up"
        );
    }
}

#[cfg(test)]
mod lod_tests {
    use super::*;

    fn thresholds(near: f32, mid: f32, far: f32) -> mzb::MmbLodThresholds {
        mzb::MmbPlacement {
            id: [0u8; 16],
            trans: [0.0; 3],
            rot: [0.0; 3],
            scale: [1.0; 3],
            block_id: 0,
            lod_near: near,
            lod_mid: mid,
            lod_far: far,
            special_effects: 0,
            area_resource_id: 0,
            sub_area_link: 0,
            light_references: [0; 4],
        }
        .lod_thresholds()
    }

    const ALL_BANDS: u8 = mzb::MmbLodLevel::High.mask()
        | mzb::MmbLodLevel::Medium.mask()
        | mzb::MmbLodLevel::Low.mask();

    fn lod(level: mzb::MmbLodLevel, uses_lod_rendering: bool) -> ZoneMeshLod {
        ZoneMeshLod {
            thresholds: thresholds(10.0, 100.0, 1000.0),
            level_mask: level.mask(),
            uses_lod_rendering,
        }
    }

    // The three spawned variants of one placement partition the distance line, so
    // exactly one is ever drawn.
    #[test]
    fn exactly_one_variant_is_drawn_at_any_distance() {
        let variants = [
            lod(mzb::MmbLodLevel::High, false),
            lod(mzb::MmbLodLevel::Medium, false),
            lod(mzb::MmbLodLevel::Low, false),
        ];
        for dist in [0.0f32, 9.9, 10.0, 10.1, 99.9, 100.0, 100.1, 5_000.0] {
            let drawn = variants
                .iter()
                .filter(|v| v.is_drawn_at(dist * dist))
                .count();
            assert_eq!(drawn, 1, "distance {dist}");
        }

        assert!(variants[0].is_drawn_at(10.0 * 10.0));
        assert!(variants[1].is_drawn_at(100.0 * 100.0));
        assert!(variants[2].is_drawn_at(100.1 * 100.1));
    }

    // A placement whose bands collapse onto one mesh serves every band.
    #[test]
    fn a_single_mesh_serving_all_bands_is_always_drawn() {
        let all = ZoneMeshLod {
            thresholds: thresholds(10.0, 100.0, 1000.0),
            level_mask: ALL_BANDS,
            uses_lod_rendering: false,
        };
        for dist in [0.0f32, 50.0, 500.0, 100_000.0] {
            assert!(all.is_drawn_at(dist * dist));
        }
    }

    // ZoneRenderer.cpp:1057-1064 — the far cull only applies to chunks that opted
    // into LOD rendering, and it outranks the band pick.
    #[test]
    fn the_far_cull_applies_only_to_lod_flagged_chunks() {
        // (10, 100, 40): retail authors far *inside* mid, so the low mesh's own
        // band starts past a cull that has already removed the chunk.
        let mut prop = lod(mzb::MmbLodLevel::Low, true);
        prop.thresholds = thresholds(10.0, 100.0, 40.0);
        prop.level_mask = ALL_BANDS;
        assert!(prop.is_drawn_at(39.0 * 39.0));
        assert!(!prop.is_drawn_at(41.0 * 41.0));

        prop.uses_lod_rendering = false;
        assert!(prop.is_drawn_at(41.0 * 41.0));
    }
}

#[cfg(test)]
mod area_map_tests {
    use super::*;

    fn area_box(name: &[u8; 4], min: Vec3, max: Vec3) -> ZoneAreaBox {
        ZoneAreaBox {
            area_id: mzb::area_resource_id_from_dir_name(name),
            min,
            max,
        }
    }

    fn map(boxes: Vec<ZoneAreaBox>) -> ZoneAreaMap {
        ZoneAreaMap {
            boxes,
            source_file_id: None,
        }
    }

    #[test]
    fn a_point_outside_every_area_is_the_zone_wide_environment() {
        let m = map(vec![area_box(b"ev01", Vec3::ZERO, Vec3::splat(10.0))]);
        assert_eq!(m.area_at(Vec3::new(-1.0, 5.0, 5.0)), None);
        assert_eq!(m.area_at(Vec3::new(5.0, 5.0, 11.0)), None);
        assert_eq!(map(Vec::new()).area_at(Vec3::ZERO), None);
    }

    // The block an actor stands *on* is a slab whose top is their feet, so the
    // Y window has to reach past the bounds in both directions — the alternative
    // is an interior whose floor never claims the player standing on it.
    #[test]
    fn feet_resting_on_a_slab_resolve_to_its_area() {
        const SLAB_TOP: f32 = 4.0;
        let m = map(vec![area_box(
            b"ev01",
            Vec3::new(0.0, 3.0, 0.0),
            Vec3::new(10.0, SLAB_TOP, 10.0),
        )]);
        let ev01 = Some(mzb::area_resource_id_from_dir_name(b"ev01"));
        assert_eq!(m.area_at(Vec3::new(5.0, SLAB_TOP, 5.0)), ev01);
        assert_eq!(
            m.area_at(Vec3::new(5.0, SLAB_TOP + AREA_FOOT_SLACK * 0.5, 5.0)),
            ev01
        );
        // A storey up is a different floor, not this one.
        assert_eq!(
            m.area_at(Vec3::new(5.0, SLAB_TOP + AREA_FOOT_SLACK + 1.0, 5.0)),
            None
        );
    }

    #[test]
    fn a_point_inside_one_area_resolves_to_it() {
        let m = map(vec![area_box(b"ev01", Vec3::ZERO, Vec3::splat(10.0))]);
        assert_eq!(
            m.area_at(Vec3::splat(5.0)),
            Some(mzb::area_resource_id_from_dir_name(b"ev01"))
        );
    }

    // A thickness-free sheet is not "more specific" than the room it crosses:
    // the tie-break has to be horizontal or Al'Taieu's zero-volume `ev02` planes
    // outrank every interior they pass through.
    #[test]
    fn a_flat_sheet_does_not_outrank_the_interior_it_crosses() {
        let m = map(vec![
            area_box(b"ev01", Vec3::splat(40.0), Vec3::splat(50.0)),
            area_box(
                b"ev02",
                Vec3::new(-500.0, 45.0, -500.0),
                Vec3::new(500.0, 45.0, 500.0),
            ),
        ]);
        assert_eq!(
            m.area_at(Vec3::splat(45.0)),
            Some(mzb::area_resource_id_from_dir_name(b"ev01"))
        );
    }

    // Areas nest — a room inside a building shell — and retail's per-block answer
    // is the block the actor is actually standing in, so the tighter box wins.
    #[test]
    fn nested_areas_resolve_to_the_innermost() {
        let m = map(vec![
            area_box(b"ev01", Vec3::ZERO, Vec3::splat(100.0)),
            area_box(b"ev02", Vec3::splat(40.0), Vec3::splat(50.0)),
        ]);
        assert_eq!(
            m.area_at(Vec3::splat(45.0)),
            Some(mzb::area_resource_id_from_dir_name(b"ev02"))
        );
        assert_eq!(
            m.area_at(Vec3::splat(60.0)),
            Some(mzb::area_resource_id_from_dir_name(b"ev01"))
        );
    }

    fn light_box(ids: &[&[u8; 4]], min: Vec3, max: Vec3) -> ZoneChunkLightBox {
        let mut lights = [None; mzb::LIGHT_REFERENCE_COUNT];
        for (slot, id) in ids.iter().enumerate() {
            lights[slot] = Some(u32::from_le_bytes(**id));
        }
        ZoneChunkLightBox { min, max, lights }
    }

    fn light_map(boxes: Vec<ZoneChunkLightBox>) -> ZoneChunkLightMap {
        ZoneChunkLightMap {
            boxes,
            source_file_id: None,
        }
    }

    #[test]
    fn a_zone_with_no_binding_table_is_not_authored() {
        let m = light_map(Vec::new());
        assert!(!m.is_authored());
        assert_eq!(m.lights_at(Vec3::ZERO), None);
    }

    #[test]
    fn a_point_over_a_chunk_takes_that_chunk_s_authored_lights() {
        let m = light_map(vec![light_box(
            &[b"li12", b"l421"],
            Vec3::ZERO,
            Vec3::splat(10.0),
        )]);
        assert!(m.is_authored());
        assert_eq!(
            m.lights_at(Vec3::splat(5.0)),
            Some([
                Some(u32::from_le_bytes(*b"li12")),
                Some(u32::from_le_bytes(*b"l421")),
                None,
                None
            ])
        );
        assert_eq!(
            m.lights_at(Vec3::new(50.0, 5.0, 5.0)),
            None,
            "off every chunk that binds a light, nothing is bound"
        );
    }

    // Same most-specific-block rule as the area map: the chunk an actor stands on
    // is the tightest one holding them, not the terrain slab underneath it.
    #[test]
    fn overlapping_chunks_bind_the_innermost_lights() {
        let m = light_map(vec![
            light_box(&[b"li12"], Vec3::ZERO, Vec3::splat(100.0)),
            light_box(&[b"lt01"], Vec3::splat(40.0), Vec3::splat(50.0)),
        ]);
        assert_eq!(
            m.lights_at(Vec3::splat(45.0)).unwrap()[0],
            Some(u32::from_le_bytes(*b"lt01"))
        );
        assert_eq!(
            m.lights_at(Vec3::splat(60.0)).unwrap()[0],
            Some(u32::from_le_bytes(*b"li12"))
        );
    }
}
