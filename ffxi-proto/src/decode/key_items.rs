use super::*;

/// s2c 0x055 GP_SERV_COMMAND_SCENARIOITEM (key items). One packet carries a
/// single 512-bit table: 16 u32 `GetItemFlag` (owned) followed by 16 u32
/// `LookItemFlag` (examined), then the `TableIndex`. A key-item's global id is
/// `table_index * 512 + bit`.
/// vendor/server/src/map/packets/s2c/0x055_scenarioitem.h
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioItem {
    pub table_index: u16,
    pub get_flags: [u32; Self::WORDS],
    pub look_flags: [u32; Self::WORDS],
}

impl ScenarioItem {
    pub const WORDS: usize = 16;
    pub const BITS_PER_TABLE: usize = Self::WORDS * 32;
    pub const SIZE: usize = Self::WORDS * 4 * 2 + 4;
    /// vendor/server/src/common/mmo.h:237-246 — keyitems_t holds 8 tables of
    /// 512 bits (global key-item id = table * 512 + bit).
    pub const TABLE_COUNT: usize = 8;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let rd = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        let mut get_flags = [0u32; Self::WORDS];
        let mut look_flags = [0u32; Self::WORDS];
        for i in 0..Self::WORDS {
            get_flags[i] = rd(i * 4);
            look_flags[i] = rd(Self::WORDS * 4 + i * 4);
        }
        let table_index = u16::from_le_bytes([body[Self::WORDS * 8], body[Self::WORDS * 8 + 1]]);
        Ok(Self {
            table_index,
            get_flags,
            look_flags,
        })
    }

    pub fn owned_key_item_ids(&self) -> Vec<u16> {
        Self::ids_from_flags(self.table_index, &self.get_flags)
    }

    pub fn seen_key_item_ids(&self) -> Vec<u16> {
        Self::ids_from_flags(self.table_index, &self.look_flags)
    }

    pub fn ids_from_flags(table_index: u16, flags: &[u32; Self::WORDS]) -> Vec<u16> {
        let base = table_index as usize * Self::BITS_PER_TABLE;
        let mut ids = Vec::new();
        for (word, &word_flags) in flags.iter().enumerate() {
            for bit in 0..32 {
                if word_flags & (1 << bit) != 0 {
                    ids.push((base + word * 32 + bit) as u16);
                }
            }
        }
        ids
    }
}

#[cfg(test)]
mod scenario_item_tests {
    use super::*;

    fn body_with(table_index: u16, get: &[(usize, u32)], look: &[(usize, u32)]) -> Vec<u8> {
        let mut body = vec![0u8; ScenarioItem::SIZE];
        for &(word, flags) in get {
            body[word * 4..word * 4 + 4].copy_from_slice(&flags.to_le_bytes());
        }
        for &(word, flags) in look {
            let o = ScenarioItem::WORDS * 4 + word * 4;
            body[o..o + 4].copy_from_slice(&flags.to_le_bytes());
        }
        let o = ScenarioItem::WORDS * 8;
        body[o..o + 2].copy_from_slice(&table_index.to_le_bytes());
        body
    }

    #[test]
    fn decodes_table_index_and_flags() {
        let body = body_with(2, &[(0, 0b101), (3, 1 << 7)], &[(0, 0b10)]);
        let si = ScenarioItem::decode(&body).expect("decode");
        assert_eq!(si.table_index, 2);
        assert_eq!(si.get_flags[0], 0b101);
        assert_eq!(si.get_flags[3], 1 << 7);
        assert_eq!(si.look_flags[0], 0b10);
    }

    #[test]
    fn owned_ids_account_for_table_offset() {
        let body = body_with(2, &[(0, 0b101), (3, 1 << 7)], &[]);
        let si = ScenarioItem::decode(&body).expect("decode");
        let base = 2 * ScenarioItem::BITS_PER_TABLE;
        assert_eq!(
            si.owned_key_item_ids(),
            vec![base as u16, (base + 2) as u16, (base + 3 * 32 + 7) as u16,]
        );
    }

    #[test]
    fn seen_ids_read_look_flags_with_table_offset() {
        let body = body_with(1, &[(0, 0b1)], &[(0, 0b10), (2, 1 << 3)]);
        let si = ScenarioItem::decode(&body).expect("decode");
        let base = ScenarioItem::BITS_PER_TABLE;
        assert_eq!(
            si.seen_key_item_ids(),
            vec![(base + 1) as u16, (base + 2 * 32 + 3) as u16]
        );
    }

    #[test]
    fn truncated_body_is_error() {
        let buf = vec![0u8; ScenarioItem::SIZE - 1];
        assert!(matches!(
            ScenarioItem::decode(&buf),
            Err(DecodeError::Truncated(n, have)) if n == ScenarioItem::SIZE && have == ScenarioItem::SIZE - 1
        ));
    }
}
