use crate::{DatError, Result};

use crate::mmb::keys::KEY_TABLE;

#[derive(Debug, thiserror::Error)]
pub enum MzbError {
    #[error("MZB body too small: need at least {needed} bytes, got {actual}")]
    TooSmall { needed: usize, actual: usize },
    #[error("MZB collision-data offset {offset} out of range (body is {len} bytes)")]
    CollisionDataOutOfRange { offset: usize, len: usize },
    #[error("MZB placement table ends at {end}, past the {len}-byte body")]
    PlacementTableOutOfRange { end: usize, len: usize },
    #[error("MZB mesh-data offset {offset} out of range (body is {len} bytes)")]
    MeshDataOutOfRange { offset: usize, len: usize },
    #[error("MZB mesh record at {pos} has crossed offsets (verts={verts}, normals={normals}, tris={tris})")]
    CrossedOffsets {
        pos: usize,
        verts: usize,
        normals: usize,
        tris: usize,
    },
}

impl From<MzbError> for DatError {
    fn from(e: MzbError) -> Self {
        DatError::Mzb(format!("{e}"))
    }
}

/// research/XIClient/src/XIClient/include/Resource/Derived/ZoneBlockFormat.h:20-67
/// — `ZoneBlockHeader`, a packed 0x20-byte struct.
const HDR_SIZE_AND_VERSION: usize = 0x00;
const HDR_CHUNK_COUNT_AND_DECRYPT_INDEX: usize = 0x04;
const HDR_COLLISION_DATA_OFFSET: usize = 0x08;
const HDR_TERRAIN_SCALE_X: usize = 0x0C;
const HDR_TERRAIN_SCALE_Z: usize = 0x0D;
const HDR_TERRAIN_UNITS_X: usize = 0x0E;
const HDR_TERRAIN_UNITS_Z: usize = 0x0F;
const HDR_QUADTREE_OR_GROUP_COUNT: usize = 0x10;
const HDR_GROUP_LIST_OFFSET: usize = 0x14;
const HDR_LIGHTING_OFFSET: usize = 0x18;
const HDR_SUBSTRUCTURE_TYPE: usize = 0x1C;
const HDR_COLLISION_FLAGS: usize = 0x1D;
pub const MZB_HEADER_LEN: usize = 0x20;

/// Low 24 bits of the two packed header dwords; the top byte is the format
/// version / decrypt-table index respectively (ZoneBlockFormat.h:47-65).
const HDR_COUNT_MASK: u32 = 0x00FF_FFFF;

/// research/XIClient/src/XIClient/source/Resource/Derived/ZoneBlockResource.cpp:12
/// — `if (GetFormatVersion() < 27) return;`, i.e. only version 27 files carry
/// the pass-1 XOR at all.
const ENCRYPTED_MIN_VERSION: u8 = 27;

/// ZoneBlockResource.cpp:24 — "the first 8 bytes are never encrypted", so the
/// pass-1 region is `[8, 8 + encryptedByteCount)`.
const ENCRYPTED_REGION_START: usize = 8;

/// Pass 2 XORs the name of every placement record.
/// research/cexi-docs/zone/format.md:103 — 0x64-byte records start at 0x20.
pub const PLACEMENT_RECORD_LEN: usize = 0x64;
const PLACEMENT_NAME_LEN: usize = 16;
const PLACEMENT_NAME_XOR: u8 = 0x55;

pub fn decrypt_in_place(data: &mut [u8]) -> Result<()> {
    if data.len() < ENCRYPTED_REGION_START {
        return Err(MzbError::TooSmall {
            needed: ENCRYPTED_REGION_START,
            actual: data.len(),
        }
        .into());
    }

    let encrypted_byte_count = (u32::from_le_bytes([
        data[HDR_SIZE_AND_VERSION],
        data[HDR_SIZE_AND_VERSION + 1],
        data[HDR_SIZE_AND_VERSION + 2],
        data[HDR_SIZE_AND_VERSION + 3],
    ]) & HDR_COUNT_MASK) as usize;
    let node_count = (u32::from_le_bytes([
        data[HDR_CHUNK_COUNT_AND_DECRYPT_INDEX],
        data[HDR_CHUNK_COUNT_AND_DECRYPT_INDEX + 1],
        data[HDR_CHUNK_COUNT_AND_DECRYPT_INDEX + 2],
        data[HDR_CHUNK_COUNT_AND_DECRYPT_INDEX + 3],
    ]) & HDR_COUNT_MASK) as usize;
    let version = data[HDR_SIZE_AND_VERSION + 3];
    let decrypt_index = data[HDR_CHUNK_COUNT_AND_DECRYPT_INDEX + 3];

    if version >= ENCRYPTED_MIN_VERSION {
        let mut key: i32 = KEY_TABLE[(decrypt_index ^ 0xFF) as usize] as i32;
        let mut key_count: i32 = 0;
        // ZoneBlockResource.cpp:25-40 — `position` counts from 0 while the run
        // it inverts starts at byte 8, so the encrypted region is
        // [8, 8 + encryptedByteCount) and both loop bound and skip test compare
        // against the count, not against the region's end offset.
        let mut position = 0usize;

        while position < encrypted_byte_count {
            let piece_len = (((key >> 4) & 7) as usize) + 16;
            if (key & 1) == 1 && position + piece_len < encrypted_byte_count {
                let start = ENCRYPTED_REGION_START.saturating_add(position);
                let end = start.saturating_add(piece_len).min(data.len());
                if start < end {
                    for b in &mut data[start..end] {
                        *b ^= 0xFF;
                    }
                }
            }
            key_count = key_count.wrapping_add(1);
            key = key.wrapping_add(key_count);
            position = position.saturating_add(piece_len);
        }
    }

    for i in 0..node_count {
        let base = MZB_HEADER_LEN.saturating_add(i.saturating_mul(PLACEMENT_RECORD_LEN));
        let end = base.saturating_add(PLACEMENT_NAME_LEN);
        if end > data.len() {
            break;
        }
        for b in &mut data[base..end] {
            *b ^= PLACEMENT_NAME_XOR;
        }
    }

    Ok(())
}

pub fn decrypt(data: &[u8]) -> Result<Vec<u8>> {
    let mut buf = data.to_vec();
    decrypt_in_place(&mut buf)?;
    Ok(buf)
}

/// World units spanned by one placement-grid sub-block. A zone block is divided
/// into `block_width / MZB_SUB_BLOCK_SIZE` sub-blocks per axis
/// (research/xim ZoneDefParser.kt: `subBlocksX = blockWidth / 4`). Most zones
/// ship 40-unit blocks (10 sub-blocks); Port Jeuno ships 80 (20).
pub const MZB_SUB_BLOCK_SIZE: u32 = 4;

/// ZoneRenderer.cpp:553-563 — below this the 0x10 dword is `GroupListCount` and
/// there is no quadtree; from 21 on it is `QuadTreeOffset`. Corroborated by the
/// shipped data: every version 17/18/20 MZB has 0 there, every version 23+ one a
/// live offset.
const QUADTREE_MIN_VERSION: u8 = 21;

/// ZoneRenderer.cpp:383 and :518-523 — the header's lighting section and the
/// per-placement `LightReferences` only exist from version 18 on; older files
/// get zeroed references.
const LIGHT_BINDING_MIN_VERSION: u8 = 18;

/// CollisionManager.cpp:231-234 sets the collision-object record stride to 128
/// at version <= 0x19 and 192 above it. 192 is `sizeof(CollisionObjectData)`
/// and 128 is its two leading `Matrix4`s, so everything past them — including
/// the water-height word in `flags` — exists only above the split.
const LEGACY_COLLISION_OBJECT_MAX_VERSION: u8 = 0x19;

#[derive(Debug, Clone, Copy)]
pub struct MzbHeader {
    pub decode_length: u32,
    pub node_count: u32,
    pub version: u8,
    pub key_index: u8,
    pub zone_blocks_x: u8,
    pub zone_blocks_z: u8,

    pub block_width: u8,
    pub block_length: u8,

    /// Header 0x08. Zero is legal and means the zone ships no collision section
    /// at all — retail guards the whole collision path on it
    /// (ZoneRenderer.cpp:574). The 15 moving-vehicle zones are all zero.
    pub collision_data_offset: u32,

    /// Header 0x10, a version-dependent union: see [`Self::quadtree_offset`] and
    /// [`Self::group_list_count`].
    pub quadtree_or_group_count: u32,

    pub group_list_offset: u32,
    pub lighting_offset: u32,

    /// Header 0x1C. `ZoneType = SubstructureType + 1` (ZoneRenderer.cpp:373).
    pub substructure_type: u8,
    pub collision_flags: u8,
}

impl MzbHeader {
    /// Placement-grid cell counts: zone blocks × sub-blocks per block.
    pub fn grid_cells_x(&self) -> usize {
        (self.zone_blocks_x as usize)
            .saturating_mul((self.block_width as u32 / MZB_SUB_BLOCK_SIZE) as usize)
    }

    pub fn grid_cells_z(&self) -> usize {
        (self.zone_blocks_z as usize)
            .saturating_mul((self.block_length as u32 / MZB_SUB_BLOCK_SIZE) as usize)
    }

    pub fn has_collision_data(&self) -> bool {
        self.collision_data_offset != 0
    }

    pub fn quadtree_offset(&self) -> Option<u32> {
        (self.version >= QUADTREE_MIN_VERSION).then_some(self.quadtree_or_group_count)
    }

    pub fn group_list_count(&self) -> Option<u32> {
        (self.version < QUADTREE_MIN_VERSION).then_some(self.quadtree_or_group_count)
    }

    pub fn has_light_bindings(&self) -> bool {
        self.lighting_offset != 0 && self.version >= LIGHT_BINDING_MIN_VERSION
    }

    fn has_collision_object_tail(&self) -> bool {
        self.version > LEGACY_COLLISION_OBJECT_MAX_VERSION
    }
}

impl MzbHeader {
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < MZB_HEADER_LEN {
            return Err(MzbError::TooSmall {
                needed: MZB_HEADER_LEN,
                actual: body.len(),
            }
            .into());
        }

        let dword = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);

        Ok(Self {
            decode_length: dword(HDR_SIZE_AND_VERSION) & HDR_COUNT_MASK,
            node_count: dword(HDR_CHUNK_COUNT_AND_DECRYPT_INDEX) & HDR_COUNT_MASK,
            version: body[HDR_SIZE_AND_VERSION + 3],
            key_index: body[HDR_CHUNK_COUNT_AND_DECRYPT_INDEX + 3],
            zone_blocks_x: body[HDR_TERRAIN_SCALE_X],
            zone_blocks_z: body[HDR_TERRAIN_SCALE_Z],
            block_width: body[HDR_TERRAIN_UNITS_X],
            block_length: body[HDR_TERRAIN_UNITS_Z],
            collision_data_offset: dword(HDR_COLLISION_DATA_OFFSET),
            quadtree_or_group_count: dword(HDR_QUADTREE_OR_GROUP_COUNT),
            group_list_offset: dword(HDR_GROUP_LIST_OFFSET),
            lighting_offset: dword(HDR_LIGHTING_OFFSET),
            substructure_type: body[HDR_SUBSTRUCTURE_TYPE],
            collision_flags: body[HDR_COLLISION_FLAGS],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MzbVertex {
    pub pos: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MzbNormal {
    pub n: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MzbTriangleInfo {
    pub material: u8,

    pub is_invalid: bool,

    /// Third index word's `0x4000`. Feeds [`double_sided_skip`] — the chase
    /// camera and line-of-sight pass through, movement does not.
    pub camera_transparent: bool,
}

/// Third index word: `triangle->VertexIndex3 & 0x4000` in
/// research/XIClient/src/XIClient/include/World/Zone/Terrain/CollisionQuery.hpp
/// `DoubleSidedSkipPolicy::SkipTriangle`.
const TRI_CAMERA_TRANSPARENT: u16 = 0x4000;

/// Second index word, same bit position. research/cexi-docs/zone/collision.md:204
/// claims player movement keys off it, but it measures 0 across Lower Jeuno,
/// Port Jeuno, Southern San d'Oria and West Ronfaure — if it gated blocking,
/// nothing in those zones would block. Parsed, unused, semantics unresolved.
const TRI_SECOND_WORD_FLAG: u16 = 0x4000;

/// research/XIClient/src/XIClient/include/World/Zone/Terrain/CollisionQuery.hpp
/// `DoubleSidedSkipPolicy::SkipTriangle`:
/// `header->Flags != 0 && (triangle->VertexIndex3 & 0x4000) != 0`.
///
/// The whole `u16` is tested, not bit 0. Measured over Lower Jeuno, Port Jeuno,
/// Southern San d'Oria, West Ronfaure and two Mog House DATs, `flags` only ever
/// holds 0x0000 or 0x0001, so `!= 0` and `& 1` are indistinguishable on retail
/// data — this keeps the authoritative form. (In those same zones every
/// camera-transparent triangle already sits in a `flags != 0` mesh, so the mesh
/// gate never excludes anything; it is kept because retail tests it.)
///
/// Movement uses `BacksideCullingPolicy`, whose `SkipTriangle` is
/// unconditionally false — grounding must never consult this.
/// Corroborated: research/cexi-docs/zone/collision.md:201-209.
pub fn double_sided_skip(mesh_flags: u16, camera_transparent: bool) -> bool {
    mesh_flags != 0 && camera_transparent
}

#[derive(Debug, Clone)]
pub struct MzbMesh {
    pub vertices: Vec<MzbVertex>,
    pub normals: Vec<MzbNormal>,

    pub triangles: Vec<[u32; 3]>,

    pub triangle_normals: Vec<u32>,

    pub tri_info: Vec<MzbTriangleInfo>,

    pub flags: u16,
}

/// research/XIClient/src/XIClient/include/Resource/Derived/ZoneBlockFormat.h:145-153
/// — `CollisionDataHeader`, the seven dwords the header's 0x08 offset points at.
const COLL_MESH_COUNT: usize = 0x00;
const COLL_MESH_DATA_OFFSET: usize = 0x04;
const COLL_GRID_DATA_OFFSET: usize = 0x10;
/// Everything this parser reads from `CollisionDataHeader`, i.e. through
/// `GridDataOffset`.
const COLL_HEADER_READ_LEN: usize = COLL_GRID_DATA_OFFSET + 4;

/// ZoneBlockFormat.h:166-176 — `CollisionMeshHeader`, one per collision mesh.
const MESH_VERTEX_ARRAY: usize = 0x00;
const MESH_NORMAL_ARRAY: usize = 0x04;
const MESH_INDEX_ARRAY: usize = 0x08;
const MESH_TRIANGLE_COUNT: usize = 0x0C;
const MESH_FLAGS: usize = 0x0E;
/// Through `Flags`; the struct also carries two reserved dwords this parser
/// never reads.
const MESH_RECORD_LEN: usize = MESH_FLAGS + 2;
/// ZoneBlockFormat.h:155-164 — `CollisionMeshTriangle`, four `unsigned short`.
const MESH_TRIANGLE_LEN: usize = 8;
/// `Common::Math::Vector3`, three `f32`.
const VEC3_LEN: usize = 12;

/// ZoneBlockFormat.h:178-192 — `CollisionObjectData` opens with the object's
/// world matrix (`Common::Math::Matrix4 one`), followed by a second `Matrix4`
/// and a `Matrix3`.
const COLL_OBJECT_MATRIX_LEN: usize = 4 * 4 * 4;
const COLL_OBJECT_NORMAL_MATRIX_LEN: usize = 3 * 3 * 4;
/// `CollisionObjectData::flags`, past all three matrices. Only present above
/// [`LEGACY_COLLISION_OBJECT_MAX_VERSION`].
const COLL_OBJECT_FLAGS: usize = 2 * COLL_OBJECT_MATRIX_LEN + COLL_OBJECT_NORMAL_MATRIX_LEN;

/// Resolve the collision-data header, or `None` when the zone ships no collision
/// section (ZoneRenderer.cpp:574 guards the whole path on a non-zero offset).
fn collision_header(body: &[u8], header: &MzbHeader) -> Result<Option<usize>> {
    if !header.has_collision_data() {
        return Ok(None);
    }
    let off = header.collision_data_offset as usize;
    if off.saturating_add(COLL_HEADER_READ_LEN) > body.len() {
        return Err(MzbError::CollisionDataOutOfRange {
            offset: off,
            len: body.len(),
        }
        .into());
    }
    Ok(Some(off))
}

pub fn parse_meshes(body: &[u8], header: &MzbHeader) -> Result<Vec<MzbMesh>> {
    let Some(mt) = collision_header(body, header)? else {
        return Ok(Vec::new());
    };

    let mesh_count = u32::from_le_bytes([
        body[mt + COLL_MESH_COUNT],
        body[mt + COLL_MESH_COUNT + 1],
        body[mt + COLL_MESH_COUNT + 2],
        body[mt + COLL_MESH_COUNT + 3],
    ]) as usize;
    let mesh_data_offset = u32::from_le_bytes([
        body[mt + COLL_MESH_DATA_OFFSET],
        body[mt + COLL_MESH_DATA_OFFSET + 1],
        body[mt + COLL_MESH_DATA_OFFSET + 2],
        body[mt + COLL_MESH_DATA_OFFSET + 3],
    ]) as usize;

    if mesh_data_offset >= body.len() {
        return Err(MzbError::MeshDataOutOfRange {
            offset: mesh_data_offset,
            len: body.len(),
        }
        .into());
    }

    let mut out = Vec::with_capacity(mesh_count);
    let mut pos = mesh_data_offset;
    for _ in 0..mesh_count {
        if pos + MESH_RECORD_LEN > body.len() {
            break;
        }
        let mesh = parse_one_mesh(body, pos)?;

        let tri_off = u32::from_le_bytes([
            body[pos + MESH_INDEX_ARRAY],
            body[pos + MESH_INDEX_ARRAY + 1],
            body[pos + MESH_INDEX_ARRAY + 2],
            body[pos + MESH_INDEX_ARRAY + 3],
        ]) as usize;

        let tri_count = u16::from_le_bytes([
            body[pos + MESH_TRIANGLE_COUNT],
            body[pos + MESH_TRIANGLE_COUNT + 1],
        ]) as usize;
        out.push(mesh);
        let next = tri_off.saturating_add(tri_count.saturating_mul(MESH_TRIANGLE_LEN));
        if next <= pos || next >= body.len() {
            break;
        }
        pos = next;
    }
    Ok(out)
}

fn parse_one_mesh(body: &[u8], pos: usize) -> Result<MzbMesh> {
    if pos + MESH_RECORD_LEN > body.len() {
        return Err(MzbError::MeshDataOutOfRange {
            offset: pos,
            len: body.len(),
        }
        .into());
    }

    let dword = |o: usize| {
        u32::from_le_bytes([
            body[pos + o],
            body[pos + o + 1],
            body[pos + o + 2],
            body[pos + o + 3],
        ]) as usize
    };
    let word = |o: usize| u16::from_le_bytes([body[pos + o], body[pos + o + 1]]);

    let verts_off = dword(MESH_VERTEX_ARRAY);
    let norms_off = dword(MESH_NORMAL_ARRAY);
    let tris_off = dword(MESH_INDEX_ARRAY);

    let tri_count = word(MESH_TRIANGLE_COUNT) as usize;
    let flags = word(MESH_FLAGS);

    if verts_off >= body.len() || norms_off >= body.len() || tris_off >= body.len() {
        return Err(MzbError::MeshDataOutOfRange {
            offset: verts_off.max(norms_off).max(tris_off),
            len: body.len(),
        }
        .into());
    }
    if !(verts_off <= norms_off && norms_off <= tris_off) {
        return Err(MzbError::CrossedOffsets {
            pos,
            verts: verts_off,
            normals: norms_off,
            tris: tris_off,
        }
        .into());
    }

    let vert_count = (norms_off - verts_off) / VEC3_LEN;
    let norm_count = (tris_off - norms_off) / VEC3_LEN;

    let mut vertices = Vec::with_capacity(vert_count);
    for i in 0..vert_count {
        let o = verts_off + i * VEC3_LEN;
        if o + VEC3_LEN > body.len() {
            break;
        }
        vertices.push(MzbVertex {
            pos: [
                f32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]),
                f32::from_le_bytes([body[o + 4], body[o + 5], body[o + 6], body[o + 7]]),
                f32::from_le_bytes([body[o + 8], body[o + 9], body[o + 10], body[o + 11]]),
            ],
        });
    }

    let mut normals = Vec::with_capacity(norm_count);
    for i in 0..norm_count {
        let o = norms_off + i * VEC3_LEN;
        if o + VEC3_LEN > body.len() {
            break;
        }
        normals.push(MzbNormal {
            n: [
                f32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]),
                f32::from_le_bytes([body[o + 4], body[o + 5], body[o + 6], body[o + 7]]),
                f32::from_le_bytes([body[o + 8], body[o + 9], body[o + 10], body[o + 11]]),
            ],
        });
    }

    let mut triangles = Vec::with_capacity(tri_count);
    let mut triangle_normals = Vec::with_capacity(tri_count);
    let mut tri_info = Vec::with_capacity(tri_count);
    for i in 0..tri_count {
        let o = tris_off + i * MESH_TRIANGLE_LEN;
        if o + MESH_TRIANGLE_LEN > body.len() {
            break;
        }

        let v0_raw = u16::from_le_bytes([body[o], body[o + 1]]);
        let v1_raw = u16::from_le_bytes([body[o + 2], body[o + 3]]);
        let v2_raw = u16::from_le_bytes([body[o + 4], body[o + 5]]);
        let n0_raw = u16::from_le_bytes([body[o + 6], body[o + 7]]);
        let v0 = (v0_raw & 0x7FFF) as u32;
        let v1 = (v1_raw & 0x3FFF) as u32;
        let v2 = (v2_raw & 0x3FFF) as u32;
        let n0 = (n0_raw & 0x7FFF) as u32;
        let m0 = ((v0_raw >> 15) & 1) as u8;
        let m1 = ((v1_raw >> 15) & 1) as u8;
        let m2 = ((v2_raw >> 15) & 1) as u8;
        let m3 = ((n0_raw >> 15) & 1) as u8;
        let material = m0 | (m1 << 1) | (m2 << 2) | (m3 << 3);
        let is_invalid = (v1_raw & TRI_SECOND_WORD_FLAG) != 0;
        let camera_transparent = (v2_raw & TRI_CAMERA_TRANSPARENT) != 0;
        triangles.push([v0, v1, v2]);
        triangle_normals.push(n0);
        tri_info.push(MzbTriangleInfo {
            material,
            is_invalid,
            camera_transparent,
        });
    }

    Ok(MzbMesh {
        vertices,
        normals,
        triangles,
        triangle_normals,
        tri_info,
        flags,
    })
}

pub fn parse_all(encrypted_body: &[u8]) -> Result<(MzbHeader, Vec<MzbMesh>)> {
    let plain = decrypt(encrypted_body)?;
    let header = MzbHeader::parse(&plain)?;
    let meshes = parse_meshes(&plain, &header)?;
    Ok((header, meshes))
}

#[derive(Debug, Clone, Copy)]
pub struct MzbPlacement {
    pub geometry_offset: u32,

    pub transform: [f32; 16],

    pub doesnt_block_los: bool,

    pub flip_winding: bool,

    pub grid_x: u16,
    pub grid_y: u16,

    pub water_height: Option<f32>,
}

pub fn parse_mesh_at(body: &[u8], offset: usize) -> Result<MzbMesh> {
    parse_one_mesh(body, offset)
}

pub fn parse_placements(body: &[u8], header: &MzbHeader) -> Result<Vec<MzbPlacement>> {
    let Some(mt) = collision_header(body, header)? else {
        return Ok(Vec::new());
    };
    let grid_offset = u32::from_le_bytes([
        body[mt + COLL_GRID_DATA_OFFSET],
        body[mt + COLL_GRID_DATA_OFFSET + 1],
        body[mt + COLL_GRID_DATA_OFFSET + 2],
        body[mt + COLL_GRID_DATA_OFFSET + 3],
    ]) as usize;
    if grid_offset == 0 || grid_offset >= body.len() {
        return Ok(Vec::new());
    }

    let gw = header.grid_cells_x();
    let gh = header.grid_cells_z();
    if gw == 0 || gh == 0 {
        return Ok(Vec::new());
    }

    let mut out: Vec<MzbPlacement> = Vec::new();

    for y in 0..gh {
        for x in 0..gw {
            let cell_ptr_off = grid_offset.saturating_add((y * gw + x) * 4);
            if cell_ptr_off + 4 > body.len() {
                continue;
            }
            let entry_off = u32::from_le_bytes([
                body[cell_ptr_off],
                body[cell_ptr_off + 1],
                body[cell_ptr_off + 2],
                body[cell_ptr_off + 3],
            ]) as usize;
            if entry_off == 0 || entry_off >= body.len() {
                continue;
            }

            let mut entries: Vec<u32> = Vec::new();
            let mut cur = entry_off;

            for _ in 0..4096 {
                if cur + 4 > body.len() {
                    break;
                }
                let v =
                    u32::from_le_bytes([body[cur], body[cur + 1], body[cur + 2], body[cur + 3]]);
                if v == 0 {
                    break;
                }
                entries.push(v);
                cur += 4;
            }
            if entries.is_empty() {
                continue;
            }

            let mut i = 1usize;
            while i + 1 < entries.len() {
                let mat_off = entries[i] as usize;
                let geo_off = entries[i + 1] as usize;
                i += 2;

                if mat_off == 0 || geo_off == 0 {
                    continue;
                }
                if mat_off + COLL_OBJECT_MATRIX_LEN > body.len()
                    || geo_off + MESH_RECORD_LEN > body.len()
                {
                    continue;
                }

                let mut m = [0.0f32; 16];
                for (k, slot) in m.iter_mut().enumerate() {
                    let o = mat_off + k * 4;
                    *slot = f32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
                }

                let det = m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2])
                    + m[8] * (m[1] * m[6] - m[5] * m[2]);

                let flags = u16::from_le_bytes([
                    body[geo_off + MESH_FLAGS],
                    body[geo_off + MESH_FLAGS + 1],
                ]);

                let water_off = mat_off + COLL_OBJECT_FLAGS;
                let water_height =
                    if header.has_collision_object_tail() && water_off + 4 <= body.len() {
                        let raw = i32::from_le_bytes([
                            body[water_off],
                            body[water_off + 1],
                            body[water_off + 2],
                            body[water_off + 3],
                        ]);
                        let signed_26 = (raw.wrapping_shl(6)) >> 10;
                        if signed_26 == 0 {
                            None
                        } else {
                            Some(signed_26 as f32 / 1024.0)
                        }
                    } else {
                        None
                    };

                out.push(MzbPlacement {
                    geometry_offset: geo_off as u32,
                    transform: m,
                    doesnt_block_los: (flags & 1) != 0,
                    flip_winding: det < 0.0,
                    grid_x: x as u16,
                    grid_y: y as u16,
                    water_height,
                });
            }
        }
    }

    Ok(out)
}

#[inline]
pub fn apply_placement(m: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    let (x, y, z) = (v[0], v[1], v[2]);
    [
        m[0] * x + m[4] * y + m[8] * z + m[12],
        m[1] * x + m[5] * y + m[9] * z + m[13],
        m[2] * x + m[6] * y + m[10] * z + m[14],
    ]
}

/// research/XIClient/src/XIClient/include/Resource/Derived/ZoneBlockFormat.h:76-104
/// — `PositionedMeshBlockData`, one 0x64-byte record per placed MMB.
/// Corroborated field-by-field by research/cexi-docs/zone/format.md:103-121.
const PL_MESH_BLOCK_NAME: usize = 0x00;
const PL_TRANSLATION: usize = 0x10;
const PL_ROTATION: usize = 0x1C;
const PL_SCALING: usize = 0x28;
const PL_BLOCK_ID: usize = 0x34;
const PL_LOD_NEAR: usize = 0x38;
const PL_LOD_MID: usize = 0x3C;
const PL_LOD_FAR: usize = 0x40;
const PL_AREA_RESOURCE_ID: usize = 0x4C;
const PL_SUB_AREA_LINK: usize = 0x50;
const PL_LIGHT_REFERENCES: usize = 0x54;
/// ZoneBlockFormat.h:11 — `LIGHT_REFERENCE_COUNT`.
const LIGHT_REFERENCE_COUNT: usize = 4;

#[derive(Debug, Clone, Copy)]
pub struct MmbPlacement {
    pub id: [u8; 16],
    pub trans: [f32; 3],

    pub rot: [f32; 3],
    pub scale: [f32; 3],

    /// FourCC. A non-zero `BlockID` makes retail classify the chunk RenderType 0,
    /// which is never drawn by the normal pass (ZoneRenderer.cpp:619-641).
    pub block_id: u32,

    /// Squared against the camera distance to pick the high/mid/low mesh variant
    /// (ZoneRenderer.cpp:492-504, :1087-1094). `lod_far` is also the draw
    /// distance past which the chunk stops being drawn at all.
    pub lod_near: f32,
    pub lod_mid: f32,
    pub lod_far: f32,

    /// FourCC of the area this chunk belongs to; drives per-area fog and the
    /// weather diffuse lights (ZoneRenderer.cpp:515).
    pub area_resource_id: u32,

    /// The sub-area (building interior) whose geometry replaces this placeholder,
    /// 0 when there is none. Retail hides the chunk while that sub-area is the
    /// active collision map — RenderType 1 (ZoneRenderer.cpp:637-638,
    /// research/cexi-docs/zone/subareas.md:76-84).
    pub sub_area_link: u32,

    /// 1-based indices into the header's light-binding table; 0 = unused. Zeroed
    /// below [`LIGHT_BINDING_MIN_VERSION`], as retail does
    /// (ZoneRenderer.cpp:518-523).
    pub light_references: [u32; LIGHT_REFERENCE_COUNT],
}

impl MmbPlacement {
    pub fn id_str(&self) -> &str {
        let end = self
            .id
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.id.len());
        std::str::from_utf8(&self.id[..end]).unwrap_or("")
    }
}

pub fn parse_mmb_placements(body: &[u8], header: &MzbHeader) -> Result<Vec<MmbPlacement>> {
    let count = header.node_count as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    let table_end = MZB_HEADER_LEN.saturating_add(count.saturating_mul(PLACEMENT_RECORD_LEN));
    if table_end > body.len() {
        return Err(MzbError::PlacementTableOutOfRange {
            end: table_end,
            len: body.len(),
        }
        .into());
    }
    let has_lights = header.version >= LIGHT_BINDING_MIN_VERSION;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = MZB_HEADER_LEN + i * PLACEMENT_RECORD_LEN;
        let rec = &body[off..off + PLACEMENT_RECORD_LEN];
        let mut id = [0u8; PLACEMENT_NAME_LEN];
        id.copy_from_slice(&rec[PL_MESH_BLOCK_NAME..PL_MESH_BLOCK_NAME + PLACEMENT_NAME_LEN]);
        let f = |o: usize| f32::from_le_bytes([rec[o], rec[o + 1], rec[o + 2], rec[o + 3]]);
        let d = |o: usize| u32::from_le_bytes([rec[o], rec[o + 1], rec[o + 2], rec[o + 3]]);
        let mut light_references = [0u32; LIGHT_REFERENCE_COUNT];
        if has_lights {
            for (k, slot) in light_references.iter_mut().enumerate() {
                *slot = d(PL_LIGHT_REFERENCES + k * 4);
            }
        }
        out.push(MmbPlacement {
            id,
            trans: [
                f(PL_TRANSLATION),
                f(PL_TRANSLATION + 4),
                f(PL_TRANSLATION + 8),
            ],
            rot: [f(PL_ROTATION), f(PL_ROTATION + 4), f(PL_ROTATION + 8)],
            scale: [f(PL_SCALING), f(PL_SCALING + 4), f(PL_SCALING + 8)],
            block_id: d(PL_BLOCK_ID),
            lod_near: f(PL_LOD_NEAR),
            lod_mid: f(PL_LOD_MID),
            lod_far: f(PL_LOD_FAR),
            area_resource_id: d(PL_AREA_RESOURCE_ID),
            sub_area_link: d(PL_SUB_AREA_LINK),
            light_references,
        });
    }
    Ok(out)
}

pub fn resolve_mmb_index(
    placement_id: &str,
    zone_prefix: &str,
    mmb_asset_names: &[String],
) -> Option<usize> {
    resolve_mmb_indices(placement_id, zone_prefix, mmb_asset_names)
        .into_iter()
        .next()
}

pub fn resolve_mmb_indices(
    placement_id: &str,
    zone_prefix: &str,
    mmb_asset_names: &[String],
) -> Vec<usize> {
    let exact: Vec<usize> = mmb_asset_names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (n.trim_end() == placement_id).then_some(i))
        .collect();
    if !exact.is_empty() {
        return exact;
    }

    let mut prefixed = String::with_capacity(zone_prefix.len() + placement_id.len());
    prefixed.push_str(zone_prefix);
    prefixed.push_str(placement_id);
    let pre: Vec<usize> = mmb_asset_names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (n.trim_end() == prefixed).then_some(i))
        .collect();
    if !pre.is_empty() {
        return pre;
    }

    let id_bytes = placement_id.as_bytes();
    if id_bytes.len() >= 8 {
        let needle = &id_bytes[..8];
        let v: Vec<usize> = mmb_asset_names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let t = n.trim_end().as_bytes();
                (t.len() >= 8 && &t[t.len() - 8..] == needle).then_some(i)
            })
            .collect();
        if !v.is_empty() {
            return v;
        }
    }

    if placement_id.len() < 3 {
        return Vec::new();
    }
    mmb_asset_names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.trim_end().ends_with(placement_id).then_some(i))
        .collect()
}

pub fn infer_zone_prefix(mmb_asset_names: &[String]) -> String {
    let mut iter = mmb_asset_names.iter();
    let first = match iter.next() {
        Some(s) => s.as_str(),
        None => return String::new(),
    };
    let mut prefix_len = first.len().min(8);
    for name in iter {
        let cap = name.len().min(prefix_len);
        let common = first
            .as_bytes()
            .iter()
            .zip(name.as_bytes())
            .take(cap)
            .take_while(|(a, b)| a == b)
            .count();
        prefix_len = prefix_len.min(common);
        if prefix_len == 0 {
            break;
        }
    }
    first[..prefix_len].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_mzb() -> Vec<u8> {
        let mut buf = vec![0u8; 0x8C];

        let decode_len = 0x8Cu32 | (0x10 << 24);
        buf[0..4].copy_from_slice(&decode_len.to_le_bytes());

        buf[4..8].copy_from_slice(&0u32.to_le_bytes());

        buf[8..12].copy_from_slice(&0x20u32.to_le_bytes());

        buf[0x20..0x24].copy_from_slice(&1u32.to_le_bytes());
        buf[0x24..0x28].copy_from_slice(&0x30u32.to_le_bytes());

        buf[0x30..0x34].copy_from_slice(&0x40u32.to_le_bytes());
        buf[0x34..0x38].copy_from_slice(&0x70u32.to_le_bytes());
        buf[0x38..0x3C].copy_from_slice(&0x7Cu32.to_le_bytes());

        buf[0x3C..0x40].copy_from_slice(&2u32.to_le_bytes());

        let verts: [[f32; 3]; 4] = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        for (i, v) in verts.iter().enumerate() {
            let o = 0x40 + i * 12;
            buf[o..o + 4].copy_from_slice(&v[0].to_le_bytes());
            buf[o + 4..o + 8].copy_from_slice(&v[1].to_le_bytes());
            buf[o + 8..o + 12].copy_from_slice(&v[2].to_le_bytes());
        }

        buf[0x70..0x74].copy_from_slice(&0.0f32.to_le_bytes());
        buf[0x74..0x78].copy_from_slice(&1.0f32.to_le_bytes());
        buf[0x78..0x7C].copy_from_slice(&0.0f32.to_le_bytes());

        let tris: [[u16; 4]; 2] = [
            [0x8000, 1 | 0x4000, 2, 0],
            [0, 2, 3 | 0x4000 | 0x8000, 0x8000],
        ];
        for (i, t) in tris.iter().enumerate() {
            let o = 0x7C + i * 8;
            for (j, &val) in t.iter().enumerate() {
                buf[o + j * 2..o + j * 2 + 2].copy_from_slice(&val.to_le_bytes());
            }
        }

        buf
    }

    #[test]
    fn decrypt_plaintext_is_noop() {
        let orig = synth_mzb();
        let mut buf = orig.clone();
        decrypt_in_place(&mut buf).unwrap();
        assert_eq!(buf, orig, "version < 0x1B should bypass pass 1 entirely");
    }

    #[test]
    fn header_parses_basic_fields() {
        let body = synth_mzb();
        let h = MzbHeader::parse(&body).unwrap();
        assert_eq!(h.version, 0x10);
        assert_eq!(h.key_index, 0x00);
        assert_eq!(h.node_count, 0);
        assert_eq!(h.collision_data_offset, 0x20);
    }

    // Each header dword is read from its own fixed offset. 0x18 in particular is
    // `LightingOffset`, not the placement count it used to be named after.
    #[test]
    fn header_fields_come_from_their_own_offsets() {
        let mut body = synth_mzb();
        body[0x08..0x0C].copy_from_slice(&0x1111_1111u32.to_le_bytes());
        body[0x10..0x14].copy_from_slice(&0x2222_2222u32.to_le_bytes());
        body[0x14..0x18].copy_from_slice(&0x3333_3333u32.to_le_bytes());
        body[0x18..0x1C].copy_from_slice(&0x4444_4444u32.to_le_bytes());
        body[0x1C] = 2;
        body[0x1D] = 0x0C;

        let h = MzbHeader::parse(&body).unwrap();
        assert_eq!(h.collision_data_offset, 0x1111_1111);
        assert_eq!(h.quadtree_or_group_count, 0x2222_2222);
        assert_eq!(h.group_list_offset, 0x3333_3333);
        assert_eq!(h.lighting_offset, 0x4444_4444);
        assert_eq!(h.substructure_type, 2);
        assert_eq!(h.collision_flags, 0x0C);
    }

    // The 15 moving-vehicle zones ship `CollisionDataOffset == 0`. Retail wraps
    // the whole collision path in `if (CollisionDataOffset != 0)`
    // (ZoneRenderer.cpp:574), so zero is a legal state, not a parse failure — and
    // the terrain bytes at 0x0C..0x10 must never be re-read as the offset. The
    // 0x50505050 below is exactly the garbage the old probe loop produced for
    // zone 46's 80x80-block terrain.
    #[test]
    fn zero_collision_offset_is_legal_and_empty() {
        let mut body = synth_mzb();
        body[0x08..0x0C].copy_from_slice(&0u32.to_le_bytes());
        body[0x0C] = 0x50;
        body[0x0D] = 0x50;
        body[0x0E] = 0x50;
        body[0x0F] = 0x50;

        let h = MzbHeader::parse(&body).unwrap();
        assert_eq!(h.collision_data_offset, 0);
        assert!(!h.has_collision_data());
        assert!(
            parse_meshes(&body, &h).unwrap().is_empty(),
            "no collision section means no meshes, not an error"
        );
        assert_eq!(
            parse_placements(&body, &h).unwrap().len(),
            0,
            "no collision section means no placements, not an error"
        );
    }

    #[test]
    fn out_of_range_collision_offset_still_errors() {
        let mut body = synth_mzb();
        let past_end = (body.len() as u32) + 0x1000;
        body[0x08..0x0C].copy_from_slice(&past_end.to_le_bytes());
        let h = MzbHeader::parse(&body).unwrap();
        assert!(parse_meshes(&body, &h).is_err());
        assert!(parse_placements(&body, &h).is_err());
    }

    // ZoneRenderer.cpp:553-563 (quadtree) and :383/:518-523 (light bindings).
    #[test]
    fn header_unions_follow_the_format_version() {
        let mut body = synth_mzb();
        body[0x10..0x14].copy_from_slice(&0x1234u32.to_le_bytes());
        body[0x18..0x1C].copy_from_slice(&0x5678u32.to_le_bytes());

        body[3] = QUADTREE_MIN_VERSION - 1;
        let old = MzbHeader::parse(&body).unwrap();
        assert_eq!(old.quadtree_offset(), None);
        assert_eq!(old.group_list_count(), Some(0x1234));

        body[3] = QUADTREE_MIN_VERSION;
        let new = MzbHeader::parse(&body).unwrap();
        assert_eq!(new.quadtree_offset(), Some(0x1234));
        assert_eq!(new.group_list_count(), None);

        body[3] = LIGHT_BINDING_MIN_VERSION - 1;
        assert!(!MzbHeader::parse(&body).unwrap().has_light_bindings());
        body[3] = LIGHT_BINDING_MIN_VERSION;
        assert!(MzbHeader::parse(&body).unwrap().has_light_bindings());
        body[0x18..0x1C].copy_from_slice(&0u32.to_le_bytes());
        assert!(!MzbHeader::parse(&body).unwrap().has_light_bindings());
    }

    #[test]
    fn mesh_table_parses_and_indices_are_masked() {
        let body = synth_mzb();
        let h = MzbHeader::parse(&body).unwrap();
        let meshes = parse_meshes(&body, &h).unwrap();
        assert_eq!(meshes.len(), 1);

        let m = &meshes[0];
        assert_eq!(m.vertices.len(), 4);
        assert_eq!(m.normals.len(), 1);
        assert_eq!(m.triangles.len(), 2);

        assert_eq!(m.vertices[0].pos, [0.0, 0.0, 0.0]);
        assert_eq!(m.vertices[1].pos, [1.0, 0.0, 0.0]);

        assert_eq!(
            m.triangles[0],
            [0, 1, 2],
            "indices: v0 masked with 0x7FFF, v1/v2 with 0x3FFF"
        );
        assert_eq!(m.triangle_normals[0], 0);
        assert_eq!(m.tri_info[0].material, 0b0001, "material from v0 top bit");
        assert!(m.tri_info[0].is_invalid, "is_invalid from v1 bit 14");
        assert!(!m.tri_info[0].camera_transparent);

        assert_eq!(m.triangles[1], [0, 2, 3]);
        assert_eq!(
            m.tri_info[1].material, 0b1100,
            "material composed from v2 + n0 top bits"
        );
        assert!(!m.tri_info[1].is_invalid);
        assert!(
            m.tri_info[1].camera_transparent,
            "camera_transparent from v2 bit 14"
        );
    }

    #[test]
    fn double_sided_skip_tests_the_whole_flags_word() {
        assert!(!double_sided_skip(0, true), "flags == 0 never skips");
        assert!(!double_sided_skip(1, false), "bit clear never skips");
        assert!(double_sided_skip(1, true));
        // The guard that matters: retail tests `Flags != 0`, not `Flags & 1`.
        // `doesnt_block_los` right next door uses `& 1`, so this pins the two
        // apart against a well-meaning "unification".
        assert!(
            double_sided_skip(2, true),
            "any nonzero flags value gates the skip, not just bit 0"
        );
    }

    #[test]
    fn pass2_node_xor_runs() {
        let mut buf = vec![0u8; 0x20 + 0x64];
        buf[0..4].copy_from_slice(&((0x10u32 << 24) | 0x20).to_le_bytes());
        buf[4..8].copy_from_slice(&1u32.to_le_bytes());

        buf[8..12].copy_from_slice(&0x20u32.to_le_bytes());
        for b in &mut buf[0x20..0x30] {
            *b = 0xAA;
        }
        decrypt_in_place(&mut buf).unwrap();
        for b in &buf[0x20..0x30] {
            assert_eq!(
                *b,
                0xAA ^ 0x55,
                "pass 2 should XOR first 16 bytes of each node with 0x55"
            );
        }

        for b in &buf[0x30..0x20 + 0x64] {
            assert_eq!(*b, 0);
        }
    }

    #[test]
    fn pass1_xor_runs_when_version_is_encrypted() {
        let mut encrypted = vec![0u8; 64];
        encrypted[0..4].copy_from_slice(&((0x1Bu32 << 24) | 64).to_le_bytes());
        encrypted[4..8].copy_from_slice(&0u32.to_le_bytes());

        let original = encrypted.clone();
        let mut any_change = false;
        for seed in 0u8..=0xFF {
            let mut tmp = original.clone();
            tmp[7] = seed;
            decrypt_in_place(&mut tmp).unwrap();
            if tmp[8..] != original[8..] {
                any_change = true;
                break;
            }
        }
        assert!(
            any_change,
            "at least one key_index should produce a pass-1 XOR change"
        );
    }

    #[allow(clippy::identity_op)]
    fn synth_mzb_with_placement() -> Vec<u8> {
        let mut buf = vec![0u8; 0x260];

        let decode_len = (buf.len() as u32) | (0x10u32 << 24);
        buf[0..4].copy_from_slice(&decode_len.to_le_bytes());
        buf[4..8].copy_from_slice(&0u32.to_le_bytes());

        buf[8..12].copy_from_slice(&0x20u32.to_le_bytes());
        buf[0x0C] = 1;
        buf[0x0D] = 1;
        buf[0x0E] = 40;
        buf[0x0F] = 40;

        buf[0x20..0x24].copy_from_slice(&1u32.to_le_bytes());
        buf[0x24..0x28].copy_from_slice(&0x40u32.to_le_bytes());
        buf[0x30..0x34].copy_from_slice(&0x80u32.to_le_bytes());

        buf[0x40..0x44].copy_from_slice(&0x50u32.to_le_bytes());
        buf[0x44..0x48].copy_from_slice(&0x70u32.to_le_bytes());
        buf[0x48..0x4C].copy_from_slice(&0x7Cu32.to_le_bytes());
        buf[0x4C..0x50].copy_from_slice(&2u32.to_le_bytes());

        buf[0x80..0x84].copy_from_slice(&0x210u32.to_le_bytes());

        buf[0x210..0x214].copy_from_slice(&0xDEADu32.to_le_bytes());
        buf[0x214..0x218].copy_from_slice(&0x220u32.to_le_bytes());
        buf[0x218..0x21C].copy_from_slice(&0x40u32.to_le_bytes());
        buf[0x21C..0x220].copy_from_slice(&0u32.to_le_bytes());

        let mut m = [0.0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        m[12] = 100.0;
        m[13] = 200.0;
        m[14] = 300.0;
        for (k, v) in m.iter().enumerate() {
            buf[0x220 + k * 4..0x220 + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }

        buf
    }

    #[test]
    fn placements_decode_one_cell() {
        let body = synth_mzb_with_placement();
        let h = MzbHeader::parse(&body).unwrap();
        assert_eq!(h.zone_blocks_x, 1);
        assert_eq!(h.zone_blocks_z, 1);
        assert_eq!(h.grid_cells_x(), 10);
        assert_eq!(h.grid_cells_z(), 10);

        let placements = parse_placements(&body, &h).unwrap();
        assert_eq!(
            placements.len(),
            1,
            "exactly one (mat,geo) pair in cell (0,0)"
        );
        let p = placements[0];
        assert_eq!(p.geometry_offset, 0x40);
        assert_eq!(p.grid_x, 0);
        assert_eq!(p.grid_y, 0);
        assert!(
            !p.flip_winding,
            "identity rotation has positive determinant"
        );

        assert_eq!(p.transform[12], 100.0);
        assert_eq!(p.transform[13], 200.0);
        assert_eq!(p.transform[14], 300.0);

        let world = apply_placement(&p.transform, [0.0, 0.0, 0.0]);
        assert_eq!(world, [100.0, 200.0, 300.0]);

        let world = apply_placement(&p.transform, [1.0, 0.0, 0.0]);
        assert_eq!(world, [101.0, 200.0, 300.0]);
    }

    #[test]
    fn placements_empty_when_no_grid() {
        let body = synth_mzb();
        let h = MzbHeader::parse(&body).unwrap();
        let placements = parse_placements(&body, &h).unwrap();
        assert!(placements.is_empty(), "grid_width=0 → no placements");
    }

    // Port Jeuno (DAT 346) ships 80-unit zone blocks, so its placement grid is
    // 20 sub-blocks per block, not the 10 every other zone uses. Hardcoding 10
    // made every cell lookup read the wrong stride, parse_placements returned
    // empty, and the whole zone fell back to unplaced (identity) geometry — no
    // collision under the player anywhere in the zone.
    #[test]
    fn placements_grid_stride_follows_block_width() {
        let mut body = synth_mzb_with_placement();
        body[0x0E] = 80;
        body[0x0F] = 80;

        let h = MzbHeader::parse(&body).unwrap();
        assert_eq!(h.grid_cells_x(), 20);
        assert_eq!(h.grid_cells_z(), 20);

        // Move the only cell pointer from (0,0) to row 1, which sits at index 20
        // under the correct stride and index 10 under the old hardcoded one.
        let grid = 0x80usize;
        body[grid..grid + 4].copy_from_slice(&0u32.to_le_bytes());
        let row = 1usize;
        let cell = grid + row * h.grid_cells_x() * 4;
        body[cell..cell + 4].copy_from_slice(&0x210u32.to_le_bytes());

        let placements = parse_placements(&body, &h).unwrap();
        assert!(
            placements
                .iter()
                .any(|p| p.grid_x == 0 && p.grid_y == 1 && p.geometry_offset == 0x40),
            "row-1 cell must be reached at stride 20, got {:?}",
            placements
                .iter()
                .map(|p| (p.grid_x, p.grid_y))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn placement_flip_winding_on_negative_det() {
        let mut body = synth_mzb_with_placement();

        body[0x220..0x224].copy_from_slice(&(-1.0f32).to_le_bytes());
        let h = MzbHeader::parse(&body).unwrap();
        let placements = parse_placements(&body, &h).unwrap();
        assert_eq!(placements.len(), 1);
        assert!(
            placements[0].flip_winding,
            "negative determinant should set flip_winding"
        );
    }

    #[test]
    fn too_small_errors() {
        let small = vec![0u8; 4];
        let err = decrypt(&small).unwrap_err();
        assert!(matches!(err, DatError::Mzb(_)));
    }

    // `CollisionObjectData::flags` — where the section water height is packed —
    // sits 164 bytes into a record that only exists at full length above the
    // version split (CollisionManager.cpp:231-234).
    #[test]
    fn water_height_is_gated_on_the_collision_object_version() {
        let mut body = synth_mzb_with_placement();
        let mat_off = 0x220usize;
        let water_off = mat_off + COLL_OBJECT_FLAGS;
        body.resize(water_off + 4, 0);
        // 0x1000 decodes as ((0x1000 << 6) >> 10) / 1024 = 0.25.
        body[water_off..water_off + 4].copy_from_slice(&0x1000u32.to_le_bytes());

        body[3] = LEGACY_COLLISION_OBJECT_MAX_VERSION;
        let legacy = MzbHeader::parse(&body).unwrap();
        assert_eq!(
            parse_placements(&body, &legacy).unwrap()[0].water_height,
            None,
            "the legacy record is shorter than the flags field"
        );

        body[3] = LEGACY_COLLISION_OBJECT_MAX_VERSION + 1;
        let modern = MzbHeader::parse(&body).unwrap();
        assert_eq!(
            parse_placements(&body, &modern).unwrap()[0].water_height,
            Some(0.25)
        );
    }

    fn synth_mmb_placement_record(version: u8) -> Vec<u8> {
        let mut body = vec![0u8; MZB_HEADER_LEN + PLACEMENT_RECORD_LEN];
        let size_and_version = (body.len() as u32) | ((version as u32) << 24);
        body[0..4].copy_from_slice(&size_and_version.to_le_bytes());
        body[4..8].copy_from_slice(&1u32.to_le_bytes());

        let rec = MZB_HEADER_LEN;
        body[rec..rec + 4].copy_from_slice(b"blk\0");
        let put_f32 = |body: &mut Vec<u8>, o: usize, v: f32| {
            body[rec + o..rec + o + 4].copy_from_slice(&v.to_le_bytes());
        };
        let put_u32 = |body: &mut Vec<u8>, o: usize, v: u32| {
            body[rec + o..rec + o + 4].copy_from_slice(&v.to_le_bytes());
        };
        put_f32(&mut body, PL_TRANSLATION, 1.0);
        put_f32(&mut body, PL_TRANSLATION + 4, 2.0);
        put_f32(&mut body, PL_TRANSLATION + 8, 3.0);
        put_f32(&mut body, PL_ROTATION, 4.0);
        put_f32(&mut body, PL_SCALING, 5.0);
        put_u32(&mut body, PL_BLOCK_ID, 0xAABB_CCDD);
        put_f32(&mut body, PL_LOD_NEAR, 10.0);
        put_f32(&mut body, PL_LOD_MID, 20.0);
        put_f32(&mut body, PL_LOD_FAR, 30.0);
        put_u32(&mut body, PL_AREA_RESOURCE_ID, 0x1234_5678);
        put_u32(&mut body, PL_SUB_AREA_LINK, 0x1CE);
        for k in 0..LIGHT_REFERENCE_COUNT {
            put_u32(&mut body, PL_LIGHT_REFERENCES + k * 4, (k as u32) + 1);
        }
        body
    }

    #[test]
    fn mmb_placement_reads_the_whole_record() {
        let body = synth_mmb_placement_record(27);
        let h = MzbHeader::parse(&body).unwrap();
        let p = parse_mmb_placements(&body, &h).unwrap();
        assert_eq!(p.len(), 1);
        let p = p[0];
        assert_eq!(p.id_str(), "blk");
        assert_eq!(p.trans, [1.0, 2.0, 3.0]);
        assert_eq!(p.rot[0], 4.0);
        assert_eq!(p.scale[0], 5.0);
        assert_eq!(p.block_id, 0xAABB_CCDD);
        assert_eq!((p.lod_near, p.lod_mid, p.lod_far), (10.0, 20.0, 30.0));
        assert_eq!(p.area_resource_id, 0x1234_5678);
        assert_eq!(p.sub_area_link, 0x1CE);
        assert_eq!(p.light_references, [1, 2, 3, 4]);
    }

    // ZoneRenderer.cpp:518-523 zeroes LightReferences below version 18 rather
    // than reading whatever those bytes hold.
    #[test]
    fn mmb_placement_light_references_need_version_18() {
        let body = synth_mmb_placement_record(LIGHT_BINDING_MIN_VERSION - 1);
        let h = MzbHeader::parse(&body).unwrap();
        let p = parse_mmb_placements(&body, &h).unwrap();
        assert_eq!(p[0].light_references, [0; LIGHT_REFERENCE_COUNT]);
        assert_eq!(
            p[0].block_id, 0xAABB_CCDD,
            "everything below 0x54 is version-independent"
        );
    }

    /// Zone ids measured to ship `CollisionDataOffset == 0` — moving-vehicle
    /// zones, `SubstructureType == 2`. The full set is
    /// {1, 3, 46, 47, 58, 59, 60, 220, 221, 223-228}; these three sample it.
    const NO_COLLISION_ZONE_IDS: [u16; 3] = [1, 46, 220];
    /// Lower Jeuno — an ordinary zone with a populated collision grid.
    const COLLISION_ZONE_ID: u16 = 230;

    fn zone_mzb_body(root: &crate::DatRoot, zone_id: u16) -> Vec<u8> {
        let file_id = crate::zone_dat::zone_id_to_mzb_file_id(zone_id).unwrap();
        let loc = root.resolve(file_id).unwrap();
        let bytes = std::fs::read(loc.path_under(root.root())).unwrap();
        let chunks: Vec<_> = crate::walk(&bytes).filter_map(Result::ok).collect();
        let chunk = chunks
            .iter()
            .find(|c| c.kind == crate::ChunkKind::Mzb as u8)
            .unwrap_or_else(|| panic!("zone {zone_id} has no MZB chunk"));
        decrypt(chunk.data).unwrap()
    }

    #[test]
    fn retail_zero_collision_zones_parse_empty() {
        let Some(root) = crate::archive::open_test_install() else {
            eprintln!("skipping: no FFXI install");
            return;
        };

        for zone_id in NO_COLLISION_ZONE_IDS {
            let body = zone_mzb_body(&root, zone_id);
            let h = MzbHeader::parse(&body).unwrap();
            assert_eq!(h.collision_data_offset, 0, "zone {zone_id}");
            assert_eq!(h.substructure_type, 2, "zone {zone_id}");
            assert!(parse_placements(&body, &h).unwrap().is_empty());
            assert!(parse_meshes(&body, &h).unwrap().is_empty());
        }

        let body = zone_mzb_body(&root, COLLISION_ZONE_ID);
        let h = MzbHeader::parse(&body).unwrap();
        assert!(h.has_collision_data());
        assert_eq!(h.substructure_type, 0);
        assert!(!parse_placements(&body, &h).unwrap().is_empty());
    }

    #[test]
    fn mmb_placement_table_past_the_body_errors() {
        let mut body = synth_mmb_placement_record(27);
        body.truncate(body.len() - 1);
        let h = MzbHeader::parse(&body).unwrap();
        assert!(parse_mmb_placements(&body, &h).is_err());
    }
}
