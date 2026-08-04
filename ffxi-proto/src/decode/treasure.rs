use super::*;

/// Slots in one treasure pool (vendor/server/src/map/treasure_pool.h:38
/// `TREASUREPOOL_SIZE`).
pub const TREASURE_POOL_SIZE: usize = 10;

/// `GC_ITEM_TROPHY_ENTRY_KIND` — whether the local client has acted on a pool
/// item yet (research/XiPackets/world/server/0x00D2, `Entry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophyEntryKind {
    None,
    Passed,
    Lotted,
}

impl TrophyEntryKind {
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Passed,
            2 => Self::Lotted,
            _ => Self::None,
        }
    }
}

/// s2c 0x0D2 GP_SERV_COMMAND_TROPHY_LIST — one item (and/or gil) found and
/// placed in the treasure pool. Also replayed per pool item when a party
/// member zones back in.
/// vendor/server/src/map/packets/s2c/0x0d2_trophy_list.h
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrophyList {
    pub item_count: u32,
    /// Entity (or object) that dropped or contained the treasure. 0 when the
    /// packet only carries gil.
    pub target_unique_no: u32,
    pub gold: u16,
    /// 0 when the packet only carries gil.
    pub item_no: u16,
    pub target_act_index: u16,
    pub slot: u8,
    pub entry: TrophyEntryKind,
    /// Found inside a container (chest/coffer) rather than on a defeated mob —
    /// selects retail's "in the" wording over "on".
    pub is_container: bool,
    pub start_time: u32,
    /// The local player's own lot, already gated on `IsLocallyLotted`.
    pub own_lot: Option<u16>,
    pub loot_unique_no: u32,
    pub loot_act_index: u16,
    pub loot_point: u16,
    pub loot_act_name: Option<String>,
    /// The dropper is a named entity, so retail drops the "the " prefix. Most
    /// notorious monsters set this.
    pub named: bool,
    /// The dropper is referred to plurally ("seem" vs "seems").
    pub single: bool,
}

impl TrophyList {
    /// setSize(0x3C) minus the 4-byte subpacket header.
    pub const SIZE: usize = 56;

    const FLAG_NAMED: u8 = 1 << 0;
    const FLAG_SINGLE: u8 = 1 << 1;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let locally_lotted = u16::from_le_bytes([body[24], body[25]]) != 0;
        let point = u16::from_le_bytes([body[26], body[27]]);
        let flags = body[52];
        Ok(Self {
            item_count: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            target_unique_no: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            gold: u16::from_le_bytes([body[8], body[9]]),
            item_no: u16::from_le_bytes([body[12], body[13]]),
            target_act_index: u16::from_le_bytes([body[14], body[15]]),
            slot: body[16],
            entry: TrophyEntryKind::from_raw(body[17]),
            is_container: body[18] != 0,
            start_time: u32::from_le_bytes(body[20..24].try_into().unwrap()),
            own_lot: locally_lotted.then_some(point),
            loot_unique_no: u32::from_le_bytes(body[28..32].try_into().unwrap()),
            loot_act_index: u16::from_le_bytes([body[32], body[33]]),
            loot_point: u16::from_le_bytes([body[34], body[35]]),
            loot_act_name: read_name_slot(&body[36..52]),
            named: flags & Self::FLAG_NAMED != 0,
            single: flags & Self::FLAG_SINGLE != 0,
        })
    }

    /// A packet with no item id carries only found gil.
    pub fn is_gil_only(&self) -> bool {
        self.item_no == 0
    }
}

/// `JudgeFlg` — the verdict a 0x0D3 reports for a pool item.
/// research/XiPackets/world/server/0x00D3, cross-read with
/// `GP_TROPHY_SOLUTION_STATE` in
/// vendor/server/src/map/packets/s2c/0x0d3_trophy_solution.h.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophyJudge {
    /// No verdict yet — someone lotted or passed and the item stays in the pool.
    Pending,
    /// Won or randomly distributed.
    Won,
    /// Won, but the winner could not hold it, so it is lost.
    WinnerIneligible,
    /// Cleared with no message. The client treats every value >= 3 this way.
    SilentClear,
}

impl TrophyJudge {
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Pending,
            1 => Self::Won,
            2 => Self::WinnerIneligible,
            _ => Self::SilentClear,
        }
    }
}

/// s2c 0x0D3 GP_SERV_COMMAND_TROPHY_SOLUTION — an action taken against a pool
/// item (lot, pass, distribution).
/// vendor/server/src/map/packets/s2c/0x0d3_trophy_solution.h
///
/// `loot_*` describe the current winning lot; `entry_*` the player whose action
/// produced this packet. Under `TrophyJudge::Won` the client reinterprets both
/// `loot_unique_no` and `entry_unique_no` as message-id offsets rather than
/// entity ids: 0 selects the first-person wording, anything else third-person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrophySolution {
    pub loot_unique_no: u32,
    pub entry_unique_no: u32,
    pub loot_act_index: u16,
    pub loot_point: i16,
    pub entry_act_index: u16,
    /// `EntryFlg`: the acting player lotted (true) rather than passed.
    pub entry_lotted: bool,
    pub entry_point: i16,
    pub slot: u8,
    pub judge: TrophyJudge,
    /// `sLootName` — the current winning lotter.
    pub loot_name: Option<String>,
    /// `sLootName2` — the player who lotted or passed.
    pub entry_name: Option<String>,
}

impl TrophySolution {
    /// setSize(0x3C) minus the 4-byte subpacket header.
    pub const SIZE: usize = 56;

    /// `EntryActIndex : 15` / `EntryFlg : 1` share one u16, MSVC packing the
    /// 15-bit index into the low bits.
    const ENTRY_ACT_INDEX_MASK: u16 = 0x7FFF;
    const ENTRY_FLG_BIT: u16 = 0x8000;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let entry_word = u16::from_le_bytes([body[12], body[13]]);
        Ok(Self {
            loot_unique_no: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            entry_unique_no: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            loot_act_index: u16::from_le_bytes([body[8], body[9]]),
            loot_point: i16::from_le_bytes([body[10], body[11]]),
            entry_act_index: entry_word & Self::ENTRY_ACT_INDEX_MASK,
            entry_lotted: entry_word & Self::ENTRY_FLG_BIT != 0,
            entry_point: i16::from_le_bytes([body[14], body[15]]),
            slot: body[16],
            judge: TrophyJudge::from_raw(body[17]),
            loot_name: read_name_slot(&body[18..34]),
            entry_name: read_name_slot(&body[34..50]),
        })
    }

    /// Whether this packet reports a lot worth announcing. Retail prints the
    /// roll only when a point value and an acting name are both present;
    /// passes are silent (research/XiPackets/world/server/0x00D3).
    pub fn announces_lot(&self) -> bool {
        self.entry_point > 0 && self.entry_name.is_some()
    }
}

#[cfg(test)]
mod trophy_list_tests {
    use super::*;

    // Field offsets pinned against vendor/server/src/map/packets/s2c/
    // 0x0d2_trophy_list.h: TrophyItemNum u32 @0, TargetUniqueNo u32 @4,
    // Gold u16 @8, padding00 u16 @10, TrophyItemNo u16 @12, TargetActIndex u16
    // @14, TrophyItemIndex u8 @16, Entry u8 @17, IsContainer u8 @18, padding01
    // u8 @19, StartTime u32 @20, IsLocallyLotted u16 @24, Point u16 @26,
    // LootUniqueNo u32 @28, LootActIndex u16 @32, LootPoint u16 @34,
    // LootActName u8[16] @36, flags u8 @52, padding02 u8[3] @53.
    fn body() -> Vec<u8> {
        let mut b = vec![0u8; TrophyList::SIZE];
        b[0..4].copy_from_slice(&1u32.to_le_bytes());
        b[4..8].copy_from_slice(&0x0100_0042u32.to_le_bytes());
        b[8..10].copy_from_slice(&25u16.to_le_bytes());
        b[12..14].copy_from_slice(&0x1234u16.to_le_bytes());
        b[14..16].copy_from_slice(&0x0042u16.to_le_bytes());
        b[16] = 3;
        b[17] = 2;
        b[18] = 1;
        b[20..24].copy_from_slice(&123_456u32.to_le_bytes());
        b[24..26].copy_from_slice(&1u16.to_le_bytes());
        b[26..28].copy_from_slice(&777u16.to_le_bytes());
        b[28..32].copy_from_slice(&0x0100_0099u32.to_le_bytes());
        b[32..34].copy_from_slice(&0x0099u16.to_le_bytes());
        b[34..36].copy_from_slice(&900u16.to_le_bytes());
        b[36..36 + 8].copy_from_slice(b"Macnugge");
        b[52] = TrophyList::FLAG_NAMED | TrophyList::FLAG_SINGLE;
        b
    }

    #[test]
    fn decodes_all_fields_at_lsb_offsets() {
        let t = TrophyList::decode(&body()).expect("decode");
        assert_eq!(t.item_count, 1);
        assert_eq!(t.target_unique_no, 0x0100_0042);
        assert_eq!(t.gold, 25);
        assert_eq!(t.item_no, 0x1234);
        assert_eq!(t.target_act_index, 0x0042);
        assert_eq!(t.slot, 3);
        assert_eq!(t.entry, TrophyEntryKind::Lotted);
        assert!(t.is_container);
        assert_eq!(t.start_time, 123_456);
        assert_eq!(t.own_lot, Some(777));
        assert_eq!(t.loot_unique_no, 0x0100_0099);
        assert_eq!(t.loot_act_index, 0x0099);
        assert_eq!(t.loot_point, 900);
        assert_eq!(t.loot_act_name.as_deref(), Some("Macnugge"));
        assert!(t.named);
        assert!(t.single);
    }

    #[test]
    fn point_is_ignored_unless_locally_lotted() {
        // IsLocallyLotted gates Point; retail treats Point as 0 when it is
        // clear (research/XiPackets/world/server/0x00D2, IsLocallyLotted).
        let mut b = body();
        b[24..26].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(TrophyList::decode(&b).unwrap().own_lot, None);
    }

    #[test]
    fn zero_item_id_is_a_gil_only_packet() {
        let mut b = body();
        b[12..14].copy_from_slice(&0u16.to_le_bytes());
        assert!(TrophyList::decode(&b).unwrap().is_gil_only());
        assert!(!TrophyList::decode(&body()).unwrap().is_gil_only());
    }

    #[test]
    fn truncated_body_is_error() {
        let buf = vec![0u8; TrophyList::SIZE - 1];
        assert!(matches!(
            TrophyList::decode(&buf),
            Err(DecodeError::Truncated(n, have)) if n == TrophyList::SIZE && have == TrophyList::SIZE - 1
        ));
    }
}

#[cfg(test)]
mod trophy_solution_tests {
    use super::*;

    // Field offsets pinned against vendor/server/src/map/packets/s2c/
    // 0x0d3_trophy_solution.h: LootUniqueNo u32 @0, EntryUniqueNo u32 @4,
    // LootActIndex u16 @8, LootPoint i16 @10, EntryActIndex:15/EntryFlg:1 @12,
    // EntryPoint i16 @14, TrophyItemIndex u8 @16, JudgeFlg u8 @17,
    // sLootName u8[16] @18, sLootName2 u8[16] @34, padding00 u8[6] @50.
    fn body() -> Vec<u8> {
        let mut b = vec![0u8; TrophySolution::SIZE];
        b[0..4].copy_from_slice(&0x0100_0099u32.to_le_bytes());
        b[4..8].copy_from_slice(&0x0100_0042u32.to_le_bytes());
        b[8..10].copy_from_slice(&0x0099u16.to_le_bytes());
        b[10..12].copy_from_slice(&900i16.to_le_bytes());
        b[12..14].copy_from_slice(&(0x0042u16 | TrophySolution::ENTRY_FLG_BIT).to_le_bytes());
        b[14..16].copy_from_slice(&856i16.to_le_bytes());
        b[16] = 3;
        b[17] = 0;
        b[18..18 + 9].copy_from_slice(b"Macnugget");
        b[34..34 + 5].copy_from_slice(b"Daisy");
        b
    }

    #[test]
    fn decodes_all_fields_at_lsb_offsets() {
        let t = TrophySolution::decode(&body()).expect("decode");
        assert_eq!(t.loot_unique_no, 0x0100_0099);
        assert_eq!(t.entry_unique_no, 0x0100_0042);
        assert_eq!(t.loot_act_index, 0x0099);
        assert_eq!(t.loot_point, 900);
        assert_eq!(t.entry_act_index, 0x0042);
        assert!(t.entry_lotted);
        assert_eq!(t.entry_point, 856);
        assert_eq!(t.slot, 3);
        assert_eq!(t.judge, TrophyJudge::Pending);
        assert_eq!(t.loot_name.as_deref(), Some("Macnugget"));
        assert_eq!(t.entry_name.as_deref(), Some("Daisy"));
    }

    #[test]
    fn entry_flg_does_not_bleed_into_the_act_index() {
        let mut b = body();
        b[12..14].copy_from_slice(&0x7FFFu16.to_le_bytes());
        let t = TrophySolution::decode(&b).unwrap();
        assert_eq!(t.entry_act_index, 0x7FFF);
        assert!(!t.entry_lotted, "EntryFlg clear means the player passed");
    }

    #[test]
    fn judge_flag_saturates_to_silent_clear() {
        // The client treats every JudgeFlg >= 3 alike: clear the slot, print
        // nothing (research/XiPackets/world/server/0x00D3).
        for (raw, want) in [
            (0u8, TrophyJudge::Pending),
            (1, TrophyJudge::Won),
            (2, TrophyJudge::WinnerIneligible),
            (3, TrophyJudge::SilentClear),
            (255, TrophyJudge::SilentClear),
        ] {
            let mut b = body();
            b[17] = raw;
            assert_eq!(TrophySolution::decode(&b).unwrap().judge, want);
        }
    }

    #[test]
    fn a_pass_announces_nothing() {
        let mut b = body();
        b[14..16].copy_from_slice(&0i16.to_le_bytes());
        assert!(!TrophySolution::decode(&b).unwrap().announces_lot());
        assert!(TrophySolution::decode(&body()).unwrap().announces_lot());
    }

    #[test]
    fn truncated_body_is_error() {
        let buf = vec![0u8; TrophySolution::SIZE - 1];
        assert!(matches!(
            TrophySolution::decode(&buf),
            Err(DecodeError::Truncated(n, _)) if n == TrophySolution::SIZE
        ));
    }
}
