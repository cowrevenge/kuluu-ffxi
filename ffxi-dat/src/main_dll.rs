use std::path::Path;

use crate::{DatError, Result};

// research/xim MainDll.kt — table offsets are located by scanning FFXiMain.dll for a known
// big-endian marker word, starting at 0x30000. The marker bytes ARE the first entries of the
// table, so the matched position is used directly as the table base; per-race entries are
// little-endian u16 at base + race_index * 2.
const SCAN_START: usize = 0x30000;
const SCAN_WORDS: usize = 0xC000;

const WEAPON_SKILL_HINT: u32 = 0xCB81_CB81;
const DANCE_SKILL_HINT: u32 = 0xB9E2_B9E2;
// research/xim MainDll.kt:47 emoteAnimationOffsetHint.
const EMOTE_HINT: u32 = 0x4827_4827;

// research/xim ZoneMapTable.kt
const ZONE_MAP_HINT: u64 = 0x6400_0001_0001_0100;
const ZONE_MAP_STRIDE: usize = 0x0E;
const ZONE_MAP_NEXT_DIVISOR: usize = 0x13;
const ZONE_MAP_SIZE_NUMERATOR: u16 = 2560;

/// The record's low nibble at byte 4 picks which file-table base its
/// `file_table_offset` counts from. research/xim `ZoneMapTable.getFileTableOffset`.
const ZONE_MAP_FILE_TABLE_BASES: [u32; 4] = [0x14C0, 0xD02F, 0xD147, 0x1592];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneMapRecord {
    pub zone_id: u16,
    pub sub_zone_id: u8,
    /// The map image's own DAT file id. Carrying it here is what lets a caller
    /// take the image and the calibration below from one row, instead of
    /// cross-referencing a table keyed on a different index (kuluu-bqm5).
    pub file_id: u32,
    pub size: u16,
    pub x_offset: i16,
    pub y_offset: i16,
}

pub struct MainDll {
    bytes: Vec<u8>,
    weapon_skill_base: usize,
    dance_skill_base: usize,
    emote_base: Option<usize>,
    zone_map_base: Option<usize>,
}

impl MainDll {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("FFXiMain.dll");
        let bytes = std::fs::read(&path).map_err(|source| DatError::Io {
            path: path.clone(),
            source,
        })?;
        let weapon_skill_base =
            find_offset(&bytes, WEAPON_SKILL_HINT).ok_or(DatError::DllMarkerNotFound {
                hint: WEAPON_SKILL_HINT,
            })?;
        let dance_skill_base =
            find_offset(&bytes, DANCE_SKILL_HINT).ok_or(DatError::DllMarkerNotFound {
                hint: DANCE_SKILL_HINT,
            })?;
        let emote_base = find_offset(&bytes, EMOTE_HINT);
        let zone_map_base = find_offset_u64(&bytes, ZONE_MAP_HINT);
        Ok(Self {
            bytes,
            weapon_skill_base,
            dance_skill_base,
            emote_base,
            zone_map_base,
        })
    }

    pub fn zone_map(&self, zone_id: u16, sub_zone_id: u8) -> Option<ZoneMapRecord> {
        self.zone_maps(zone_id)
            .into_iter()
            .find(|rec| rec.sub_zone_id == sub_zone_id)
    }

    /// Every map the zone ships, in table order. A quarter of the zones number
    /// their maps from 1, so callers must enumerate rather than assume a
    /// sub-zone 0 exists (kuluu-bqm5).
    pub fn zone_maps(&self, zone_id: u16) -> Vec<ZoneMapRecord> {
        let mut out = Vec::new();
        let Some(mut base) = self.zone_map_base else {
            return out;
        };
        loop {
            let Some(rec) = self.bytes.get(base..base + ZONE_MAP_STRIDE) else {
                return out;
            };
            if u16::from_le_bytes([rec[0], rec[1]]) == zone_id {
                if let Some(parsed) = parse_zone_map(rec) {
                    out.push(parsed);
                }
            }
            match self.bytes.get(base + ZONE_MAP_NEXT_DIVISOR) {
                Some(0) | None => return out,
                Some(_) => base += ZONE_MAP_STRIDE,
            }
        }
    }

    pub fn base_weapon_skill_index(&self, race_index: u8) -> Option<u16> {
        self.read16(self.weapon_skill_base + race_index as usize * 2)
    }

    pub fn base_dance_skill_index(&self, race_index: u8) -> Option<u16> {
        self.read16(self.dance_skill_base + race_index as usize * 2)
    }

    /// First emote-animation file id for a race (the look race byte, HumeM=1);
    /// research/xim MainDll.kt:120-121.
    pub fn base_emote_index(&self, race_index: u8) -> Option<u16> {
        self.read16(self.emote_base? + race_index as usize * 2)
    }

    fn read16(&self, off: usize) -> Option<u16> {
        let b = self.bytes.get(off..off + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
}

fn find_offset(bytes: &[u8], hint: u32) -> Option<usize> {
    let mut pos = SCAN_START;
    for _ in 0..SCAN_WORDS {
        let b = bytes.get(pos..pos + 4)?;
        if u32::from_be_bytes([b[0], b[1], b[2], b[3]]) == hint {
            return Some(pos);
        }
        pos += 4;
    }
    None
}

/// One `ZONE_MAP_STRIDE`-byte row. `None` when the divisor is 0, which is how
/// the table marks a zone that ships no drawable map.
fn parse_zone_map(rec: &[u8]) -> Option<ZoneMapRecord> {
    let divisor = rec[5];
    if divisor == 0 {
        return None;
    }
    let base = *ZONE_MAP_FILE_TABLE_BASES.get(usize::from(rec[4] & 0x0F))?;
    let file_table_offset = i16::from_le_bytes([rec[8], rec[9]]);
    Some(ZoneMapRecord {
        zone_id: u16::from_le_bytes([rec[0], rec[1]]),
        sub_zone_id: rec[2],
        file_id: base.wrapping_add_signed(i32::from(file_table_offset)),
        size: ZONE_MAP_SIZE_NUMERATOR / u16::from(divisor),
        x_offset: i16::from_le_bytes([rec[10], rec[11]]),
        y_offset: i16::from_le_bytes([rec[12], rec[13]]),
    })
}

fn find_offset_u64(bytes: &[u8], hint: u64) -> Option<usize> {
    let mut pos = SCAN_START;
    for _ in 0..SCAN_WORDS {
        let b = bytes.get(pos..pos + 8)?;
        let word = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        if word == hint {
            return Some(pos);
        }
        pos += 4;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_offset_matches_big_endian_marker_word_aligned() {
        let mut bytes = vec![0u8; SCAN_START + 0x40];
        let at = SCAN_START + 0x20;
        bytes[at..at + 4].copy_from_slice(&0xCB81_CB81u32.to_be_bytes());
        assert_eq!(find_offset(&bytes, WEAPON_SKILL_HINT), Some(at));
    }

    #[test]
    fn find_offset_none_when_absent() {
        let bytes = vec![0u8; SCAN_START + 0x40];
        assert_eq!(find_offset(&bytes, WEAPON_SKILL_HINT), None);
    }

    #[test]
    fn read16_is_little_endian_per_race() {
        let mut bytes = vec![0u8; SCAN_START + 0x40];
        let base = SCAN_START + 0x20;
        bytes[base..base + 4].copy_from_slice(&WEAPON_SKILL_HINT.to_be_bytes());
        // race_index 1 -> base + 2
        bytes[base + 2] = 0x34;
        bytes[base + 3] = 0x12;
        let dll = MainDll {
            bytes,
            weapon_skill_base: base,
            dance_skill_base: base,
            emote_base: Some(base),
            zone_map_base: None,
        };
        assert_eq!(dll.base_weapon_skill_index(1), Some(0x1234));
        assert_eq!(dll.base_emote_index(1), Some(0x1234));
    }

    #[test]
    fn missing_emote_marker_yields_none() {
        let dll = MainDll {
            bytes: vec![0u8; 4],
            weapon_skill_base: 0,
            dance_skill_base: 0,
            emote_base: None,
            zone_map_base: None,
        };
        assert_eq!(dll.base_emote_index(1), None);
    }

    #[test]
    fn zone_map_parses_record_and_stops_at_zero_divisor() {
        let base = 0usize;
        let mut bytes = vec![0u8; 64];
        bytes[0..2].copy_from_slice(&100u16.to_le_bytes());
        bytes[2] = 0;
        bytes[5] = 5;
        bytes[10..12].copy_from_slice(&10i16.to_le_bytes());
        bytes[12..14].copy_from_slice(&(-20i16).to_le_bytes());
        bytes[base + ZONE_MAP_NEXT_DIVISOR] = 1;
        let r1 = ZONE_MAP_STRIDE;
        bytes[r1..r1 + 2].copy_from_slice(&230u16.to_le_bytes());
        bytes[r1 + 5] = 8;

        let dll = MainDll {
            bytes,
            weapon_skill_base: 0,
            dance_skill_base: 0,
            emote_base: None,
            zone_map_base: Some(base),
        };
        let rec = dll.zone_map(100, 0).expect("zone 100 record");
        assert_eq!(rec.size, 512);
        assert_eq!((rec.x_offset, rec.y_offset), (10, -20));
        assert_eq!(dll.zone_map(230, 0).map(|r| r.size), Some(320));
        assert_eq!(dll.zone_map(999, 0), None);
    }

    #[test]
    fn zone_maps_enumerates_a_zone_that_numbers_its_maps_from_one() {
        let mut bytes = vec![0u8; ZONE_MAP_STRIDE * 3];
        for (slot, sub) in [(0usize, 1u8), (1, 2)] {
            let at = slot * ZONE_MAP_STRIDE;
            bytes[at..at + 2].copy_from_slice(&238u16.to_le_bytes());
            bytes[at + 2] = sub;
            bytes[at + 5] = 4;
            bytes[at + 8..at + 10].copy_from_slice(&i16::from(sub).to_le_bytes());
            bytes[at + ZONE_MAP_NEXT_DIVISOR] = 1;
        }
        let dll = MainDll {
            bytes,
            weapon_skill_base: 0,
            dance_skill_base: 0,
            emote_base: None,
            zone_map_base: Some(0),
        };

        assert_eq!(dll.zone_map(238, 0), None, "this zone has no sub-zone 0");
        let maps = dll.zone_maps(238);
        assert_eq!(
            maps.iter().map(|r| r.sub_zone_id).collect::<Vec<_>>(),
            vec![1, 2],
            "enumerating still finds both maps"
        );
        assert_eq!(
            maps.iter().map(|r| r.file_id).collect::<Vec<_>>(),
            vec![
                ZONE_MAP_FILE_TABLE_BASES[0] + 1,
                ZONE_MAP_FILE_TABLE_BASES[0] + 2
            ],
            "each record names its own map DAT"
        );
    }

    /// Gated on a retail install (self-skips). The defect this guards is a
    /// zone whose maps are numbered from 1 being looked up at sub-zone 0 and
    /// silently coming back empty (kuluu-bqm5).
    #[test]
    fn real_dll_zone_maps_cover_every_zone_that_ships_one() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(crate::archive::DEFAULT_INSTALL_DIR);
        let Ok(dll) = MainDll::load(&root) else {
            return;
        };

        // Windurst Waters numbers its two maps 1 and 2.
        let waters = dll.zone_maps(238);
        assert_eq!(waters.len(), 2, "zone 238 ships two maps");
        assert!(
            dll.zone_map(238, 0).is_none(),
            "and none of them is sub-zone 0"
        );
        assert!(
            waters.iter().all(|r| r.file_id != 0 && r.size > 0),
            "each carries a usable file id and span"
        );

        // Across the whole table, enumerating never loses a zone that a
        // sub-zone-0 lookup would have found.
        let mut from_zero = 0usize;
        let mut enumerated = 0usize;
        for zone in 0..=u16::MAX {
            let maps = dll.zone_maps(zone);
            if !maps.is_empty() {
                enumerated += 1;
            }
            if dll.zone_map(zone, 0).is_some() {
                from_zero += 1;
                assert!(!maps.is_empty(), "zone {zone} regressed");
            }
        }
        assert!(
            enumerated > from_zero,
            "enumerating reaches more zones than a sub-zone-0 lookup ({enumerated} vs {from_zero})"
        );
    }
}
