use super::*;

#[derive(Debug, Clone, Copy)]
pub struct SystemMessage {
    pub para: u32,
    pub para2: u32,
    pub message_id: u16,
}

impl SystemMessage {
    pub const SIZE: usize = 12;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            para: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            para2: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            message_id: u16::from_le_bytes(body[8..10].try_into().unwrap()),
        })
    }
}

// vendor/server/src/map/packets/s2c/0x057_weather.h:32-37 (StartTime u32, WeatherNumber, WeatherOffsetTime u16)
#[derive(Debug, Clone, Copy)]
pub struct WeatherPacket {
    pub start_time: u32,
    pub weather_number: u16,
    pub offset_time: u16,
}

impl WeatherPacket {
    pub const SIZE: usize = 8;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            start_time: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            weather_number: u16::from_le_bytes(body[4..6].try_into().unwrap()),
            offset_time: u16::from_le_bytes(body[6..8].try_into().unwrap()),
        })
    }
}

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

/// s2c 0x02A GP_SERV_COMMAND_TALKNUMWORK — a zone-dialog message (LSB lua
/// `messageSpecial`) with up to 4 numeric parameters substituted into the
/// zone's dialog DAT entry `MesNum`.
/// vendor/server/src/map/packets/s2c/0x02a_talknumwork.h
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TalkNumWork {
    pub unique_no: u32,
    pub num: [i32; Self::NUM_COUNT],
    pub act_index: u16,
    pub mes_num: u16,
    pub kind: u8,
    pub flag: u8,
    pub name: [u8; Self::NAME_LEN],
}

impl TalkNumWork {
    pub const NUM_COUNT: usize = 4;
    pub const NAME_LEN: usize = 32;
    pub const SIZE: usize = 4 + Self::NUM_COUNT * 4 + 2 + 2 + 1 + 1 + Self::NAME_LEN;
    /// Added to MesNum when the sender is a PC and ShowName is false — the
    /// dialog index is the low 15 bits.
    /// vendor/server/src/map/packets/s2c/0x02a_talknumwork.cpp
    pub const MESNUM_HIDE_NAME_FLAG: u16 = 0x8000;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let mut num = [0i32; Self::NUM_COUNT];
        for (i, n) in num.iter_mut().enumerate() {
            let o = 4 + i * 4;
            *n = i32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        }
        Ok(Self {
            unique_no: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            num,
            act_index: u16::from_le_bytes([body[20], body[21]]),
            mes_num: u16::from_le_bytes([body[22], body[23]]),
            kind: body[24],
            flag: body[25],
            name: body[26..26 + Self::NAME_LEN].try_into().unwrap(),
        })
    }

    pub fn message_index(&self) -> u16 {
        self.mes_num & !Self::MESNUM_HIDE_NAME_FLAG
    }

    pub fn hide_name(&self) -> bool {
        self.mes_num & Self::MESNUM_HIDE_NAME_FLAG != 0
    }

    pub fn speaker_name(&self) -> Option<String> {
        let end = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(Self::NAME_LEN);
        (end > 0).then(|| String::from_utf8_lossy(&self.name[..end]).into_owned())
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

#[cfg(test)]
mod talk_num_work_tests {
    use super::*;

    // Field offsets pinned against vendor/server/src/map/packets/s2c/
    // 0x02a_talknumwork.h: UniqueNo u32 @0, num i32[4] @4, ActIndex u16 @20,
    // MesNum u16 @22, Type u8 @24, Flag u8 @25, String u8[32] @26.
    fn body() -> Vec<u8> {
        let mut b = vec![0u8; TalkNumWork::SIZE];
        b[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        b[4..8].copy_from_slice(&512i32.to_le_bytes());
        b[8..12].copy_from_slice(&(-7i32).to_le_bytes());
        b[20..22].copy_from_slice(&0x0042u16.to_le_bytes());
        b[22..24].copy_from_slice(&(6438u16 | TalkNumWork::MESNUM_HIDE_NAME_FLAG).to_le_bytes());
        b[24] = 3;
        b[25] = 1;
        b[26..26 + 5].copy_from_slice(b"Trion");
        b
    }

    #[test]
    fn decodes_all_fields_at_lsb_offsets() {
        let t = TalkNumWork::decode(&body()).expect("decode");
        assert_eq!(t.unique_no, 0xDEAD_BEEF);
        assert_eq!(t.num, [512, -7, 0, 0]);
        assert_eq!(t.act_index, 0x0042);
        assert_eq!(t.mes_num, 6438 | TalkNumWork::MESNUM_HIDE_NAME_FLAG);
        assert_eq!(t.kind, 3);
        assert_eq!(t.flag, 1);
        assert_eq!(t.speaker_name().as_deref(), Some("Trion"));
    }

    #[test]
    fn message_index_masks_the_hide_name_flag() {
        let t = TalkNumWork::decode(&body()).expect("decode");
        assert_eq!(t.message_index(), 6438);
        assert!(t.hide_name());

        let mut plain = body();
        plain[22..24].copy_from_slice(&6438u16.to_le_bytes());
        let t = TalkNumWork::decode(&plain).expect("decode");
        assert_eq!(t.message_index(), 6438);
        assert!(!t.hide_name());
    }

    #[test]
    fn empty_name_is_none() {
        let mut b = body();
        b[26..26 + TalkNumWork::NAME_LEN].fill(0);
        assert_eq!(TalkNumWork::decode(&b).unwrap().speaker_name(), None);
    }

    #[test]
    fn truncated_body_is_error() {
        let buf = vec![0u8; TalkNumWork::SIZE - 1];
        assert!(matches!(
            TalkNumWork::decode(&buf),
            Err(DecodeError::Truncated(n, have)) if n == TalkNumWork::SIZE && have == TalkNumWork::SIZE - 1
        ));
    }
}
