use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::archive::DatRoot;
use crate::{DatError, Result};

pub const NPC_LIST_FILE_ID_BASE: u32 = 6720;

pub const RECORD_SIZE: usize = 0x20;

pub const NAME_LEN: usize = 0x1C;

const ID_OFFSET: usize = NAME_LEN;

const ID_MARKER: u32 = 0x0100_0000;

const MAX_ZONE_ID: u16 = 0x0FFF;

#[derive(Debug)]
pub struct NpcNameTable {
    zone_id: u16,
    source: PathBuf,
    bytes: Box<[u8]>,
    by_id: HashMap<u32, usize>,
}

impl NpcNameTable {
    pub fn open(root: &DatRoot, zone_id: u16) -> Result<Self> {
        let file_id = NPC_LIST_FILE_ID_BASE + u32::from(zone_id);
        let location = root.resolve(file_id)?;
        let path = location.path_under(root);
        let bytes = fs::read(&path).map_err(|source| DatError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self::new(zone_id, path, bytes.into_boxed_slice()))
    }

    pub fn from_bytes(zone_id: u16, bytes: impl Into<Box<[u8]>>) -> Self {
        Self::new(zone_id, PathBuf::from("<in-memory>"), bytes.into())
    }

    fn new(zone_id: u16, source: PathBuf, bytes: Box<[u8]>) -> Self {
        let by_id = index_by_embedded_id(&bytes);
        Self {
            zone_id,
            source,
            bytes,
            by_id,
        }
    }

    pub fn zone_id(&self) -> u16 {
        self.zone_id
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn len(&self) -> usize {
        self.bytes.len() / RECORD_SIZE
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn record_id(&self, index: usize) -> Option<u32> {
        record_id(&self.bytes, index)
    }

    pub fn lookup_by_slot(&self, slot: u16) -> Option<&str> {
        if slot == 0 {
            return None;
        }
        record_name(&self.bytes, usize::from(slot))
    }

    pub fn lookup_by_id(&self, npc_id: u32) -> Option<&str> {
        if (npc_id & 0xFF00_0000) != ID_MARKER {
            return None;
        }
        let zone_bits = ((npc_id >> 12) & 0xFFF) as u16;
        if zone_bits != self.zone_id {
            return None;
        }
        if let Some(&index) = self.by_id.get(&npc_id) {
            return record_name(&self.bytes, index);
        }
        if self.by_id.is_empty() {
            return self.lookup_by_slot((npc_id & 0xFFF) as u16);
        }
        None
    }
}

fn record_id(bytes: &[u8], index: usize) -> Option<u32> {
    let offset = index.checked_mul(RECORD_SIZE)?;
    let end = offset.checked_add(RECORD_SIZE)?;
    let raw = bytes.get(offset + ID_OFFSET..end)?;
    Some(u32::from_le_bytes(raw.try_into().ok()?))
}

fn record_name(bytes: &[u8], index: usize) -> Option<&str> {
    let offset = index.checked_mul(RECORD_SIZE)?;
    let name_bytes = bytes.get(offset..offset.checked_add(NAME_LEN)?)?;
    let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    let trimmed = &name_bytes[..end];
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        return None;
    }
    std::str::from_utf8(trimmed).ok()
}

fn index_by_embedded_id(bytes: &[u8]) -> HashMap<u32, usize> {
    let mut map: HashMap<u32, usize> = HashMap::new();
    for index in 0..bytes.len() / RECORD_SIZE {
        let Some(id) = record_id(bytes, index).filter(|&id| id != 0) else {
            continue;
        };
        match map.entry(id) {
            Entry::Vacant(slot) => {
                slot.insert(index);
            }
            Entry::Occupied(mut slot) => {
                if record_name(bytes, *slot.get()).is_none() && record_name(bytes, index).is_some()
                {
                    slot.insert(index);
                }
            }
        }
    }
    map
}

/// Inverse of [`split_id`]: the full entity unique-no for a zone-static
/// entity's 12-bit slot (targid), e.g. to name a wide-scan ActIndex.
pub fn compose_id(zone_id: u16, slot: u16) -> u32 {
    ID_MARKER | (u32::from(zone_id) << 12) | u32::from(slot & 0xFFF)
}

pub fn split_id(npc_id: u32) -> Option<(u16, u16)> {
    if (npc_id & 0xFF00_0000) != ID_MARKER {
        return None;
    }
    let zone = ((npc_id >> 12) & 0xFFF) as u16;
    if zone > MAX_ZONE_ID {
        return None;
    }
    let slot = (npc_id & 0xFFF) as u16;
    Some((zone, slot))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ZONE_ID: u16 = 230;

    fn entity_id(zone_id: u16, slot: u16) -> u32 {
        compose_id(zone_id, slot)
    }

    #[test]
    fn compose_id_roundtrips_through_split_id() {
        assert_eq!(split_id(compose_id(230, 10)), Some((230, 10)));
        assert_eq!(compose_id(230, 10), 17_719_306);
    }

    fn synth_table(zone_id: u16) -> NpcNameTable {
        let mut buf = vec![0u8; 11 * RECORD_SIZE];

        write_record(&mut buf, 0, b"none", 0);

        write_record(&mut buf, 1, b"Ceraule", entity_id(zone_id, 1));

        write_record(&mut buf, 10, b"Apairemant", entity_id(zone_id, 10));
        NpcNameTable::from_bytes(zone_id, buf)
    }

    fn synth_shifted_table(zone_id: u16) -> NpcNameTable {
        let mut buf = vec![0u8; 8 * RECORD_SIZE];
        write_record(&mut buf, 3, b"Voidwatch Purveyor", entity_id(zone_id, 5));
        write_record(&mut buf, 5, b"Rolandienne", entity_id(zone_id, 7));
        NpcNameTable::from_bytes(zone_id, buf)
    }

    fn write_record(buf: &mut [u8], index: usize, name: &[u8], id: u32) {
        let off = index * RECORD_SIZE;
        buf[off..off + name.len()].copy_from_slice(name);

        buf[off + ID_OFFSET..off + RECORD_SIZE].copy_from_slice(&id.to_le_bytes());
    }

    #[test]
    fn split_id_extracts_zone_and_slot() {
        assert_eq!(split_id(17_719_306), Some((230, 10)));

        assert_eq!(split_id(0x0000_0000), None);
        assert_eq!(split_id(0x0200_0000), None);
    }

    #[test]
    fn lookup_by_slot_returns_name_at_slot() {
        let t = synth_table(230);
        assert_eq!(t.lookup_by_slot(1), Some("Ceraule"));
        assert_eq!(t.lookup_by_slot(10), Some("Apairemant"));
    }

    #[test]
    fn lookup_by_slot_zero_is_always_none() {
        let t = synth_table(230);
        assert_eq!(t.lookup_by_slot(0), None);
    }

    #[test]
    fn lookup_by_slot_returns_none_for_empty_record() {
        let t = synth_table(230);

        assert_eq!(t.lookup_by_slot(2), None);
    }

    #[test]
    fn lookup_by_slot_returns_none_for_out_of_range_slot() {
        let t = synth_table(230);
        assert_eq!(t.lookup_by_slot(999), None);
    }

    #[test]
    fn lookup_by_slot_rejects_non_ascii_name() {
        let mut buf = vec![0u8; 2 * RECORD_SIZE];
        write_record(&mut buf, 1, b"\x80valid?", 1);
        let t = NpcNameTable::from_bytes(230, buf);
        assert_eq!(t.lookup_by_slot(1), None);
    }

    #[test]
    fn lookup_by_slot_accepts_space_in_name() {
        let mut buf = vec![0u8; 2 * RECORD_SIZE];
        write_record(&mut buf, 1, b"Synergy Engineer", 1);
        let t = NpcNameTable::from_bytes(230, buf);
        assert_eq!(t.lookup_by_slot(1), Some("Synergy Engineer"));
    }

    #[test]
    fn lookup_by_id_returns_name_for_matching_zone() {
        let t = synth_table(230);
        assert_eq!(t.lookup_by_id(17_719_306), Some("Apairemant"));
    }

    #[test]
    fn lookup_by_id_rejects_wrong_zone() {
        let t = synth_table(230);

        let wrong_zone_id = 0x0100_0000 | (100u32 << 12) | 10;
        assert_eq!(t.lookup_by_id(wrong_zone_id), None);
    }

    #[test]
    fn lookup_by_id_rejects_ids_without_entity_marker() {
        let t = synth_table(230);

        assert_eq!(t.lookup_by_id(0x000E_600A), None);
    }

    #[test]
    fn record_id_reads_the_embedded_entity_id() {
        let t = synth_shifted_table(TEST_ZONE_ID);
        assert_eq!(t.record_id(3), Some(entity_id(TEST_ZONE_ID, 5)));
        assert_eq!(t.record_id(4), Some(0));
        assert_eq!(t.record_id(t.len()), None);
    }

    #[test]
    fn lookup_by_id_addresses_records_by_embedded_id_not_record_index() {
        let t = synth_shifted_table(TEST_ZONE_ID);

        assert_eq!(
            t.lookup_by_id(entity_id(TEST_ZONE_ID, 5)),
            Some("Voidwatch Purveyor")
        );
        assert_eq!(
            t.lookup_by_id(entity_id(TEST_ZONE_ID, 7)),
            Some("Rolandienne")
        );

        assert_eq!(t.lookup_by_slot(3), Some("Voidwatch Purveyor"));
        assert_eq!(t.lookup_by_slot(5), Some("Rolandienne"));
    }

    #[test]
    fn lookup_by_id_returns_none_rather_than_the_name_at_that_record_index() {
        let t = synth_shifted_table(TEST_ZONE_ID);
        assert_eq!(t.lookup_by_id(entity_id(TEST_ZONE_ID, 3)), None);
    }

    #[test]
    fn lookup_by_id_prefers_a_named_record_over_a_blank_duplicate() {
        let mut buf = vec![0u8; 4 * RECORD_SIZE];
        write_record(&mut buf, 1, b"", entity_id(TEST_ZONE_ID, 9));
        write_record(&mut buf, 2, b"Ceraule", entity_id(TEST_ZONE_ID, 9));
        let t = NpcNameTable::from_bytes(TEST_ZONE_ID, buf);

        assert_eq!(t.lookup_by_id(entity_id(TEST_ZONE_ID, 9)), Some("Ceraule"));
    }

    #[test]
    fn lookup_by_id_falls_back_to_slot_addressing_when_no_record_carries_an_id() {
        let mut buf = vec![0u8; 4 * RECORD_SIZE];
        write_record(&mut buf, 2, b"Ceraule", 0);
        let t = NpcNameTable::from_bytes(TEST_ZONE_ID, buf);

        assert_eq!(t.lookup_by_id(entity_id(TEST_ZONE_ID, 2)), Some("Ceraule"));
    }

    fn retail_table(zone_id: u16) -> Option<NpcNameTable> {
        let root = crate::archive::open_test_install()?;
        match NpcNameTable::open(&root, zone_id) {
            Ok(table) => Some(table),
            Err(err) => {
                eprintln!("skipping: no NPC-name table for zone {zone_id} ({err})");
                None
            }
        }
    }

    #[test]
    fn retail_table_resolves_every_named_record_through_its_own_embedded_id() {
        let Some(table) = retail_table(TEST_ZONE_ID) else {
            return;
        };

        let mut checked = 0usize;
        let mut shifted = 0usize;
        for index in 0..table.len() {
            let Some(id) = table.record_id(index).filter(|&id| id != 0) else {
                continue;
            };
            let Some((zone, slot)) = split_id(id) else {
                panic!("record {index} carries a non-entity id {id:#010x}");
            };
            assert_eq!(zone, TEST_ZONE_ID, "record {index} id {id:#010x}");
            if usize::from(slot) != index {
                shifted += 1;
            }
            let Some(name) = table.lookup_by_slot(index as u16) else {
                continue;
            };
            assert_eq!(
                table.lookup_by_id(id),
                Some(name),
                "record {index} id {id:#010x}"
            );
            checked += 1;
        }

        assert!(
            checked > 0,
            "zone {TEST_ZONE_ID} table had no named records"
        );
        assert!(
            shifted > 0,
            "zone {TEST_ZONE_ID} table has no index/slot drift, so this test is vacuous"
        );
    }

    const NPC_LIST_INSERT_PREFIX: &str = "INSERT INTO `npc_list` VALUES (";

    fn next_sql_string(chars: &mut std::str::Chars<'_>) -> Option<String> {
        chars.by_ref().find(|&c| c == '\'')?;
        let mut out = String::new();
        loop {
            match chars.next()? {
                '\\' => out.push(chars.next()?),
                '\'' => return Some(out),
                c => out.push(c),
            }
        }
    }

    fn parse_npc_list_row(line: &str) -> Option<(u32, String)> {
        let rest = line.strip_prefix(NPC_LIST_INSERT_PREFIX)?;
        let (id, rest) = rest.split_once(',')?;
        let id: u32 = id.trim().parse().ok()?;
        let mut chars = rest.chars();
        next_sql_string(&mut chars)?;
        let display_name = next_sql_string(&mut chars)?;
        Some((id, display_name))
    }

    fn lsb_npc_list() -> Option<HashMap<u32, String>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .join("vendor/server/sql/npc_list.sql");
        let sql = match fs::read_to_string(&path) {
            Ok(sql) => sql,
            Err(err) => {
                eprintln!("skipping: {} unreadable ({err})", path.display());
                return None;
            }
        };
        Some(sql.lines().filter_map(parse_npc_list_row).collect())
    }

    #[test]
    fn retail_names_match_the_lsb_npc_list_for_the_reported_ids() {
        const REPORTED_IDS: [u32; 3] = [17_719_636, 17_719_638, 17_719_640];

        let Some(table) = retail_table(TEST_ZONE_ID) else {
            return;
        };
        let Some(npc_list) = lsb_npc_list() else {
            return;
        };

        for id in REPORTED_IDS {
            let expected = npc_list
                .get(&id)
                .unwrap_or_else(|| panic!("npc_list.sql has no row for {id}"));
            assert_eq!(table.lookup_by_id(id), Some(expected.as_str()), "npc {id}");
        }
    }
}
