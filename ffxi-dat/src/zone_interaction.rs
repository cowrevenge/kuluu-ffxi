//! Zone-interaction ("RID") chunk parser: the oriented trigger boxes a zone DAT
//! declares for zone lines, doors, sub-areas, fishing areas and elevators. Layout
//! mirrors research/xim/src/jsMain/kotlin/xim/resource/ZoneInteractionSection.kt:51-113,
//! verified byte-for-byte on retail DATs (zones 230/235) against LSB
//! vendor/server/sql/zonelines.sql.

use crate::datid::DatId;
use crate::kind::ChunkKind;
use crate::{chunk, DatError, Result};

const RID_MAGIC: &[u8; 3] = b"RID";
const DATA_OFFSET_FIELD: usize = 0x10;
const ENTRY_TABLE_HEADER_LEN: usize = 16;
const ENTRY_LEN: usize = 64;

const POSITION_OFFSET: usize = 0x00;
/// Where XIM reads the x component of a rotation vec3
/// (ZoneInteractionSection.kt:51-113) the retail DATs hold an integer: `0` in 350
/// of the 370 shipped `m`-rects and 500..558 in the other 20. See
/// [`ZoneInteraction::rect_class`].
const RECT_CLASS_OFFSET: usize = 0x0C;
const ROTATION_Y_OFFSET: usize = 0x10;
const ROTATION_Z_OFFSET: usize = 0x14;
const SIZE_OFFSET: usize = 0x18;
const SOURCE_ID_OFFSET: usize = 0x24;
const DEST_ID_OFFSET: usize = 0x28;
const PARAM_OFFSET: usize = 0x2C;
const TERRAIN_FLAGS_OFFSET: usize = 0x30;
const MAP_ID_OFFSET: usize = 0x32;
const ELEVATOR_BOTTOM_OFFSET: usize = 0x34;
const ELEVATOR_TOP_OFFSET: usize = 0x36;

/// Elevator offsets are fixed-point 1/256 y deltas from `position[1]`
/// (ZoneInteractionSection.kt:89-90).
const ELEVATOR_Y_SCALE: f32 = 256.0;

/// Retail scales a rect's local space by `1/size` and accepts the result on
/// `[-0.5, 0.5]` — see [`ZoneInteraction::contains`].
const UNIT_BOX_HALF_EXTENT: f32 = 0.5;

/// Mog House residence zone-line tag prefixes, the emitter side of the contract LSB
/// matches at vendor/server/src/map/packets/c2s/0x05e_maprect.cpp:74-75
/// ("zmr* classic cities; zms* WoTG [S] + Adoulin").
pub const MOG_HOUSE_PREFIX_CLASSIC: &str = "zmr";
pub const MOG_HOUSE_PREFIX_WOTG: &str = "zms";

/// The [`ZoneInteraction::rect_class`] `RidManager::Add` (RidManager.cpp:100-122)
/// puts in the hit-check array; every other class it drops.
pub const RECT_CLASS_HIT_CHECKED: u32 = 0;

/// One 64-byte RID entry: an oriented trigger box in FFXI-native zone space
/// (= LSB server coords; render via `mzb_to_bevy`, not `ffxi_to_bevy`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneInteraction {
    /// OBB center.
    pub position: [f32; 3],
    /// Which record class the entry belongs to; `0` is the hit-checked one.
    /// `RidManager::Add`
    /// (research/XIClient/src/XIClient/source/World/Zone/Triggers/RidManager.cpp:100-122)
    /// puts only the class-0 rects in the array the per-frame checks walk, and in
    /// the shipped DATs the non-zero classes are coarse sub-map regions (boxes of
    /// 200-1400 units whose ids resolve to Img chunks), not trigger volumes.
    pub rect_class: u32,
    /// Euler radians, applied ZYX. Component 0 is always `0.0`: those bytes are
    /// [`ZoneInteraction::rect_class`], and retail rotates the box by
    /// `-orientation.y` alone (RidManager.cpp:109).
    pub orientation: [f32; 3],
    /// FULL extents: x,z horizontal, y vertical; box vertically centered on `position`.
    pub size: [f32; 3],
    pub source_id: DatId,
    /// `None` iff all four bytes are zero — doors carry non-zero junk
    /// (e.g. `0x20,0,0,0`) which stays `Some`.
    pub dest_id: Option<DatId>,
    pub param: u32,
    pub terrain_flags: u16,
    pub map_id: u16,
    /// World y, already `position[1] + raw/256`.
    pub elevator_bottom_y: f32,
    pub elevator_top_y: f32,
}

impl ZoneInteraction {
    /// Classifiers mirror research/xim ZoneInteractionSection.kt:29-47.
    pub fn is_zone_line(&self) -> bool {
        self.source_id.starts_with("z") && self.dest_id.is_some()
    }

    pub fn is_zone_entrance(&self) -> bool {
        self.source_id.starts_with("z") && self.dest_id.is_none()
    }

    pub fn is_door(&self) -> bool {
        self.source_id.starts_with("_")
    }

    pub fn is_sub_area(&self) -> bool {
        self.source_id.starts_with("m")
    }

    pub fn is_fishing_area(&self) -> bool {
        self.source_id.starts_with("f")
    }

    pub fn is_mog_house_line(&self) -> bool {
        self.is_zone_line()
            && (self.source_id.starts_with(MOG_HOUSE_PREFIX_CLASSIC)
                || self.source_id.starts_with(MOG_HOUSE_PREFIX_WOTG))
    }

    /// A trigger volume that latches a sub-area. `RidManager::InitSubModels`
    /// (research/XIClient/src/XIClient/source/World/Zone/Triggers/RidManager.cpp:611-647)
    /// keeps the `m`-prefixed rects whose dest fourcc is non-zero, and
    /// `RidManager::Add` hit-checks only [`RECT_CLASS_HIT_CHECKED`].
    pub fn is_sub_area_trigger(&self) -> bool {
        self.is_sub_area() && self.dest_id.is_some() && self.rect_class == RECT_CLASS_HIT_CHECKED
    }

    /// The `m`-rects of the other classes: coarse sub-map regions rather than
    /// sub-area triggers, whose ids name Img chunks and not interior MZBs.
    pub fn is_sub_map_region(&self) -> bool {
        self.is_sub_area() && self.dest_id.is_some() && self.rect_class != RECT_CLASS_HIT_CHECKED
    }

    /// What a sub-area trigger latches, `None` when the rect is not one. `0` is a
    /// value in its own right — the signal to leave the active sub-area, not a
    /// record to discard.
    pub fn sub_area_param(&self) -> Option<u32> {
        self.is_sub_area_trigger().then_some(self.param)
    }

    /// The interior a sub-area trigger declares, `None` for the leave rects and for
    /// every non-trigger. research/cexi-docs/zone/subareas.md:65 names `param` as
    /// the id, which the retail install confirms — see [`crate::sub_area`].
    pub fn sub_area_id(&self) -> Option<u32> {
        self.sub_area_param().filter(|p| *p != 0)
    }

    /// Point-in-box in FFXI zone space. `RidManager::Add`
    /// (RidManager.cpp:100-122) builds the rect's inverse as
    /// `T(-position) · RotateY(-orientation.y) · S(1/size)` and the hit checks
    /// accept the transformed point on `[-0.5, 0.5]` — so only the yaw of
    /// [`ZoneInteraction::orientation`] shapes the box, and [`ZoneInteraction::size`]
    /// is its full extent.
    pub fn contains(&self, p: [f32; 3]) -> bool {
        let (dx, dy, dz) = (
            p[0] - self.position[0],
            p[1] - self.position[1],
            p[2] - self.position[2],
        );
        let (sin, cos) = self.orientation[1].sin_cos();
        let local = [dx * cos - dz * sin, dy, dx * sin + dz * cos];
        (0..3).all(|i| (local[i] / self.size[i]).abs() <= UNIT_BOX_HALF_EXTENT)
    }

    /// The RectID c2s 0x05E carries and the primary key of LSB zonelines.sql:
    /// the source fourcc reinterpreted as a LE u32.
    pub fn rect_id(&self) -> u32 {
        u32::from_le_bytes(self.source_id.0)
    }
}

fn rd_u32(body: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
}

fn rd_u16(body: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([body[off], body[off + 1]])
}

fn rd_i16(body: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([body[off], body[off + 1]])
}

fn rd_f32(body: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
}

fn rd_vec3(body: &[u8], off: usize) -> [f32; 3] {
    [
        rd_f32(body, off),
        rd_f32(body, off + 4),
        rd_f32(body, off + 8),
    ]
}

/// Parse one RID chunk body. `data_offset` (@0x10) is relative to the body start:
/// [`chunk::Chunk::data`] already excludes the 16-byte section header, matching XIM's
/// `dataStartPosition` — unlike generator.rs, whose offsets are section-absolute and
/// need the -16 adjustment.
pub fn parse(body: &[u8]) -> Result<Vec<ZoneInteraction>> {
    if body.len() < DATA_OFFSET_FIELD + 4 {
        return Err(DatError::Rid(format!(
            "body too short for header: {}",
            body.len()
        )));
    }
    if &body[..RID_MAGIC.len()] != RID_MAGIC {
        return Err(DatError::Rid(format!(
            "bad magic {:02X?}",
            &body[..RID_MAGIC.len()]
        )));
    }

    let data_offset = rd_u32(body, DATA_OFFSET_FIELD) as usize;
    if body.len() < data_offset + ENTRY_TABLE_HEADER_LEN {
        return Err(DatError::Rid(format!(
            "data_offset {data_offset:#x} beyond body ({})",
            body.len()
        )));
    }

    let entry_count = rd_u32(body, data_offset) as usize;
    for i in 1..4 {
        let zero = rd_u32(body, data_offset + i * 4);
        if zero != 0 {
            return Err(DatError::Rid(format!(
                "expected zero u32 #{i} after entry count, got {zero:#x}"
            )));
        }
    }

    let entries_start = data_offset + ENTRY_TABLE_HEADER_LEN;
    let entries_end = entries_start + entry_count * ENTRY_LEN;
    if body.len() < entries_end {
        return Err(DatError::Rid(format!(
            "{entry_count} entries need {entries_end} bytes, body has {}",
            body.len()
        )));
    }

    let mut out = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let e = &body[entries_start + i * ENTRY_LEN..entries_start + (i + 1) * ENTRY_LEN];
        let position = rd_vec3(e, POSITION_OFFSET);
        let dest_raw: [u8; 4] = e[DEST_ID_OFFSET..DEST_ID_OFFSET + 4].try_into().unwrap();
        out.push(ZoneInteraction {
            position,
            rect_class: rd_u32(e, RECT_CLASS_OFFSET),
            orientation: [
                0.0,
                rd_f32(e, ROTATION_Y_OFFSET),
                rd_f32(e, ROTATION_Z_OFFSET),
            ],
            size: rd_vec3(e, SIZE_OFFSET),
            source_id: DatId(
                e[SOURCE_ID_OFFSET..SOURCE_ID_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            ),
            dest_id: (dest_raw != [0u8; 4]).then_some(DatId(dest_raw)),
            param: rd_u32(e, PARAM_OFFSET),
            terrain_flags: rd_u16(e, TERRAIN_FLAGS_OFFSET),
            map_id: rd_u16(e, MAP_ID_OFFSET),
            elevator_bottom_y: position[1]
                + rd_i16(e, ELEVATOR_BOTTOM_OFFSET) as f32 / ELEVATOR_Y_SCALE,
            elevator_top_y: position[1] + rd_i16(e, ELEVATOR_TOP_OFFSET) as f32 / ELEVATOR_Y_SCALE,
        });
    }
    Ok(out)
}

/// All zone interactions in a zone resource DAT: every [`ChunkKind::Rid`] chunk,
/// matched on kind only — the fourcc name is zone-specific (e.g. `t_sa`, `m_sa`).
pub fn from_dat(bytes: &[u8]) -> Result<Vec<ZoneInteraction>> {
    let mut out = Vec::new();
    for c in chunk::walk(bytes).flatten() {
        if ChunkKind::from_u8(c.kind) == Some(ChunkKind::Rid) {
            out.extend(parse(c.data)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DATA_OFFSET: usize = 0x30;

    fn put_f32x3(buf: &mut [u8], off: usize, v: [f32; 3]) {
        for (i, f) in v.iter().enumerate() {
            buf[off + i * 4..off + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn synth_entry(
        position: [f32; 3],
        rect_class: u32,
        orientation: [f32; 3],
        size: [f32; 3],
        source: &[u8; 4],
        dest: &[u8; 4],
        param: u32,
        terrain_flags: u16,
        map_id: u16,
        ev: [i16; 2],
    ) -> [u8; ENTRY_LEN] {
        let mut e = [0u8; ENTRY_LEN];
        put_f32x3(&mut e, POSITION_OFFSET, position);
        e[RECT_CLASS_OFFSET..RECT_CLASS_OFFSET + 4].copy_from_slice(&rect_class.to_le_bytes());
        e[ROTATION_Y_OFFSET..ROTATION_Y_OFFSET + 4].copy_from_slice(&orientation[1].to_le_bytes());
        e[ROTATION_Z_OFFSET..ROTATION_Z_OFFSET + 4].copy_from_slice(&orientation[2].to_le_bytes());
        put_f32x3(&mut e, SIZE_OFFSET, size);
        e[SOURCE_ID_OFFSET..SOURCE_ID_OFFSET + 4].copy_from_slice(source);
        e[DEST_ID_OFFSET..DEST_ID_OFFSET + 4].copy_from_slice(dest);
        e[PARAM_OFFSET..PARAM_OFFSET + 4].copy_from_slice(&param.to_le_bytes());
        e[TERRAIN_FLAGS_OFFSET..TERRAIN_FLAGS_OFFSET + 2]
            .copy_from_slice(&terrain_flags.to_le_bytes());
        e[MAP_ID_OFFSET..MAP_ID_OFFSET + 2].copy_from_slice(&map_id.to_le_bytes());
        e[ELEVATOR_BOTTOM_OFFSET..ELEVATOR_BOTTOM_OFFSET + 2].copy_from_slice(&ev[0].to_le_bytes());
        e[ELEVATOR_TOP_OFFSET..ELEVATOR_TOP_OFFSET + 2].copy_from_slice(&ev[1].to_le_bytes());
        e
    }

    fn synth_body(entries: &[[u8; ENTRY_LEN]]) -> Vec<u8> {
        let mut body = vec![0u8; TEST_DATA_OFFSET + ENTRY_TABLE_HEADER_LEN];
        body[..4].copy_from_slice(b"RID\0");
        body[4..8].copy_from_slice(&6u32.to_le_bytes());
        body[DATA_OFFSET_FIELD..DATA_OFFSET_FIELD + 4]
            .copy_from_slice(&(TEST_DATA_OFFSET as u32).to_le_bytes());
        body[TEST_DATA_OFFSET..TEST_DATA_OFFSET + 4]
            .copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for e in entries {
            body.extend_from_slice(e);
        }
        body
    }

    #[test]
    fn synthetic_body_roundtrips() {
        let trigger = synth_entry(
            [164.933, -5.547, 164.792],
            RECT_CLASS_HIT_CHECKED,
            [0.0, 3.93, 0.0],
            [12.0, 8.0, 2.0],
            b"zmr0",
            b"zmr1",
            253,
            0,
            1,
            [0, 0],
        );
        let marker = synth_entry(
            [162.591, -4.103, 162.423],
            RECT_CLASS_HIT_CHECKED,
            [0.0, 2.36, 0.0],
            [1.0, 4.0, 4.0],
            b"zmr1",
            &[0u8; 4],
            253,
            0x1,
            1,
            [0, 0],
        );
        let door = synth_entry(
            [0.0, -1.0, -8.0],
            RECT_CLASS_HIT_CHECKED,
            [0.0; 3],
            [2.0, 3.0, 1.0],
            b"_720",
            &[0x20, 0, 0, 0],
            0,
            0,
            0,
            [-128, 256],
        );

        let all = parse(&synth_body(&[trigger, marker, door])).unwrap();
        assert_eq!(all.len(), 3);

        let t = &all[0];
        assert_eq!(t.position, [164.933, -5.547, 164.792]);
        assert_eq!(t.orientation, [0.0, 3.93, 0.0]);
        assert_eq!(t.size, [12.0, 8.0, 2.0]);
        assert_eq!(t.source_id, DatId(*b"zmr0"));
        assert_eq!(t.dest_id, Some(DatId(*b"zmr1")));
        assert_eq!(t.param, 253);
        assert_eq!(t.map_id, 1);
        assert!(t.is_zone_line());
        assert!(t.is_mog_house_line());
        assert!(!t.is_zone_entrance());

        let m = &all[1];
        assert_eq!(m.dest_id, None, "all-zero dest is None");
        assert!(m.is_zone_entrance());
        assert!(!m.is_zone_line());
        assert_eq!(m.terrain_flags, 0x1);

        let d = &all[2];
        assert_eq!(
            d.dest_id,
            Some(DatId([0x20, 0, 0, 0])),
            "non-zero junk dest stays Some"
        );
        assert!(d.is_door());
        assert!(!d.is_zone_line(), "non-'z' source is not a zone line");
        assert!((d.elevator_bottom_y - (-1.0 + -128.0 / 256.0)).abs() < 1e-6);
        assert!((d.elevator_top_y - (-1.0 + 1.0)).abs() < 1e-6);
    }

    #[test]
    fn rect_class_is_an_integer_and_keeps_out_of_the_orientation() {
        const SUB_MAP_REGION_CLASS: u32 = 555;
        let e = synth_entry(
            [0.0; 3],
            SUB_MAP_REGION_CLASS,
            [0.0, 1.5, 0.0],
            [1.0; 3],
            b"m6t1",
            &[0x20, 0, 0, 0],
            0x1C6,
            0,
            0,
            [0, 0],
        );
        let parsed = parse(&synth_body(&[e])).unwrap()[0];
        assert_eq!(parsed.rect_class, SUB_MAP_REGION_CLASS);
        assert_eq!(parsed.orientation, [0.0, 1.5, 0.0]);
        assert!(parsed.is_sub_map_region());
        assert!(!parsed.is_sub_area_trigger());
        assert_eq!(parsed.sub_area_param(), None);
        assert_eq!(parsed.sub_area_id(), None);
    }

    /// Gated on a retail install (self-skips without one). Pins the measured split
    /// across every shipped zone: 350 of the 370 `m`-rects are class 0 and the
    /// other 20 fall in 500..=558, so the classifier is separating two real record
    /// classes rather than reading noise.
    #[test]
    fn shipped_m_rects_split_into_two_classes() {
        const EXPECTED_TRIGGERS: usize = 350;
        const EXPECTED_REGIONS: usize = 20;
        const REGION_CLASS_RANGE: std::ops::RangeInclusive<u32> = 500..=558;

        let Some(root) = crate::archive::open_test_install() else {
            eprintln!("skipping: no FFXI install");
            return;
        };

        let (mut triggers, mut regions) = (0usize, 0usize);
        for zone_id in 0u16..=400 {
            let Some(file_id) = crate::zone_dat::zone_id_to_mzb_file_id(zone_id) else {
                continue;
            };
            let Ok(loc) = root.resolve(file_id) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
                continue;
            };
            for i in from_dat(&bytes).unwrap() {
                if !i.is_sub_area() {
                    continue;
                }
                if i.rect_class == RECT_CLASS_HIT_CHECKED {
                    triggers += 1;
                } else {
                    assert!(
                        REGION_CLASS_RANGE.contains(&i.rect_class),
                        "zone {zone_id} rect class {}",
                        i.rect_class
                    );
                    regions += 1;
                }
            }
        }
        assert_eq!((triggers, regions), (EXPECTED_TRIGGERS, EXPECTED_REGIONS));
    }

    #[test]
    fn bad_magic_and_truncation_error() {
        let mut body = synth_body(&[]);
        body[0] = b'X';
        assert!(parse(&body).is_err());

        let body = synth_body(&[]);
        assert!(parse(&body[..DATA_OFFSET_FIELD]).is_err());

        let mut body = synth_body(&[]);
        body[TEST_DATA_OFFSET..TEST_DATA_OFFSET + 4].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse(&body).is_err(), "entry count beyond body errors");
    }

    /// Pins the coupling with the kuluu-nav zonelines scrape: LSB stores the trigger's
    /// source fourcc as the zonelines.sql primary key (vendor/server/sql/zonelines.sql:748).
    #[test]
    fn rect_id_matches_lsb_zonelines_primary_key() {
        assert_eq!(u32::from_le_bytes(*b"zmr0"), 812805498);
        let e = synth_entry(
            [0.0; 3],
            RECT_CLASS_HIT_CHECKED,
            [0.0; 3],
            [1.0; 3],
            b"zmr0",
            b"zmr1",
            0,
            0,
            0,
            [0, 0],
        );
        let all = parse(&synth_body(&[e])).unwrap();
        assert_eq!(all[0].rect_id(), 812805498);
    }

    /// Gated on a retail install (self-skips without one). Pins the parser against the
    /// real zone 230 DAT and the LSB invariant that zonelines.sql from_pos was dumped
    /// from these rects; zone 256 (Western Adoulin) proves the high-file-id branch.
    #[test]
    fn real_zone_dats_carry_mog_house_rects_when_install_present() {
        let Some(root) = crate::archive::open_test_install() else {
            eprintln!("skipping: no FFXI install");
            return;
        };

        let file_id = crate::zone_dat::zone_id_to_mzb_file_id(230).unwrap();
        let loc = root.resolve(file_id).unwrap();
        let bytes = std::fs::read(loc.path_under(&root)).unwrap();
        let all = from_dat(&bytes).unwrap();

        let trigger = all
            .iter()
            .find(|i| i.source_id == DatId(*b"zmr0"))
            .expect("zone 230 has the zmr0 MH trigger");
        assert!(trigger.is_mog_house_line());
        assert_eq!(trigger.dest_id, Some(DatId(*b"zmr1")));
        assert!(
            (trigger.position[0] - 164.933).abs() < 0.01,
            "x = {}",
            trigger.position[0]
        );
        assert!(
            (trigger.position[1] - -5.547).abs() < 0.01,
            "y = {}",
            trigger.position[1]
        );
        assert!(
            (trigger.position[2] - 164.792).abs() < 0.01,
            "z = {}",
            trigger.position[2]
        );
        assert!(
            (trigger.size[0] - 12.0).abs() < 0.01,
            "sx = {}",
            trigger.size[0]
        );
        assert!(
            (trigger.size[1] - 8.0).abs() < 0.01,
            "sy = {}",
            trigger.size[1]
        );
        assert!(
            (trigger.size[2] - 2.0).abs() < 0.01,
            "sz = {}",
            trigger.size[2]
        );

        let marker = all
            .iter()
            .find(|i| i.source_id == DatId(*b"zmr1"))
            .expect("zone 230 has the zmr1 arrival marker");
        assert!(marker.is_zone_entrance());

        let high_file_id = crate::zone_dat::zone_id_to_mzb_file_id(256).unwrap();
        assert!(
            high_file_id > 83635,
            "zone 256 uses the high-file-id branch"
        );
        let loc = root.resolve(high_file_id).unwrap();
        let bytes = std::fs::read(loc.path_under(&root)).unwrap();
        let all = from_dat(&bytes).unwrap();
        assert!(
            all.iter()
                .any(|i| i.is_mog_house_line() && i.source_id.starts_with(MOG_HOUSE_PREFIX_WOTG)),
            "Western Adoulin carries a zms* MH trigger"
        );
    }
}
