use super::*;

#[derive(Debug, Clone, Copy)]
pub struct PosHead {
    pub unique_no: u32,

    pub act_index: u16,

    pub send_flag: u8,

    pub dir: u8,

    pub x: f32,

    pub z: f32,

    pub y: f32,

    pub flags0: u32,

    pub speed: u8,

    pub speed_base: u8,

    pub hpp: u8,

    pub server_status: u8,
    pub flags1: u32,
    pub flags2: u32,
    pub flags3: u32,

    pub bt_target_id: u32,
}

impl PosHead {
    pub const SIZE: usize = 40;

    pub const SIZE_WITH_BT_TARGET: usize = 44;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let bt_target_id = if body.len() >= Self::SIZE_WITH_BT_TARGET {
            u32::from_le_bytes(body[40..44].try_into().unwrap())
        } else {
            0
        };
        Ok(Self {
            unique_no: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            act_index: u16::from_le_bytes(body[4..6].try_into().unwrap()),
            send_flag: body[6],
            dir: body[7],
            x: f32::from_le_bytes(body[8..12].try_into().unwrap()),
            z: f32::from_le_bytes(body[12..16].try_into().unwrap()),
            y: f32::from_le_bytes(body[16..20].try_into().unwrap()),
            flags0: u32::from_le_bytes(body[20..24].try_into().unwrap()),
            speed: body[24],
            speed_base: body[25],
            hpp: body[26],
            server_status: body[27],
            flags1: u32::from_le_bytes(body[28..32].try_into().unwrap()),
            flags2: u32::from_le_bytes(body[32..36].try_into().unwrap()),
            flags3: u32::from_le_bytes(body[36..40].try_into().unwrap()),
            bt_target_id,
        })
    }

    // Head-look target = the targid the entity has selected, packed into Flags0
    // bits 17..31. Both 0x0D (char_update.cpp `Flags0.facetarget = m_TargID`) and
    // 0x0E (entity_update.cpp `ref<uint16>(0x1A) = m_TargID << 1`) write it here.
    // Distinct from bt_target_id (the combat-claim UniqueNo).
    const FACETARGET_SHIFT: u32 = 17;
    const FACETARGET_MASK: u32 = 0x7FFF;

    pub fn facetarget(&self) -> u16 {
        ((self.flags0 >> Self::FACETARGET_SHIFT) & Self::FACETARGET_MASK) as u16
    }

    pub fn decode_char_npc(body: &[u8]) -> Result<(Self, u32), DecodeError> {
        let head = Self::decode(body)?;
        Ok((head, head.bt_target_id))
    }

    pub const UPDATE_DESPAWN: u8 = 0x20;

    pub fn is_entity_despawn(opcode: u16, body: &[u8]) -> bool {
        use crate::map::s2c;
        (opcode == s2c::CHAR_PC || opcode == s2c::CHAR_NPC)
            && body
                .get(6)
                .copied()
                .is_some_and(|mask| mask & Self::UPDATE_DESPAWN != 0)
    }

    pub fn try_extract_name(opcode: u16, body: &[u8]) -> Option<String> {
        use crate::map::s2c;

        const NAME_FLAG: u8 = 0x08;
        if body.len() < 7 || body[6] & NAME_FLAG == 0 {
            return None;
        }
        let slot: &[u8] = if opcode == s2c::CHAR_PC {
            const NAME_START: usize = 0x56;
            if body.len() <= NAME_START {
                return None;
            }
            &body[NAME_START..]
        } else if opcode == s2c::CHAR_NPC {
            const STANDARD_START: usize = 0x30;
            const RENAMED_START: usize = 0x31;
            if body.len() <= STANDARD_START {
                return None;
            }
            let start = if body[STANDARD_START] == 0x01 {
                RENAMED_START
            } else {
                STANDARD_START
            };
            if body.len() <= start {
                return None;
            }
            let end = body.len().min(start + 16);
            &body[start..end]
        } else {
            return None;
        };
        read_name_slot(slot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookData {
    Standard {
        modelid: u16,
    },

    Equipped {
        face: u8,

        race: u8,
        head: u16,
        body: u16,
        hands: u16,
        legs: u16,
        feet: u16,
        main: u16,
        sub: u16,
        ranged: u16,
    },

    Door {
        size: u16,
    },

    Transport {
        size: u16,
    },
}

impl LookData {
    pub const LOOK_BODY_OFFSET: usize = 0x2C;

    pub fn decode_char_npc(body: &[u8]) -> Option<Self> {
        let off = Self::LOOK_BODY_OFFSET;
        if body.len() < off + 4 {
            return None;
        }
        let size = u16::from_le_bytes([body[off], body[off + 1]]);

        match size {
            0 | 5 | 6 => {
                let modelid = u16::from_le_bytes([body[off + 2], body[off + 3]]);
                Some(LookData::Standard { modelid })
            }
            1 | 7 => {
                if body.len() < off + 20 {
                    return None;
                }
                Some(LookData::Equipped {
                    face: body[off + 2],
                    race: body[off + 3],
                    head: u16::from_le_bytes([body[off + 4], body[off + 5]]),
                    body: u16::from_le_bytes([body[off + 6], body[off + 7]]),
                    hands: u16::from_le_bytes([body[off + 8], body[off + 9]]),
                    legs: u16::from_le_bytes([body[off + 10], body[off + 11]]),
                    feet: u16::from_le_bytes([body[off + 12], body[off + 13]]),
                    main: u16::from_le_bytes([body[off + 14], body[off + 15]]),
                    sub: u16::from_le_bytes([body[off + 16], body[off + 17]]),
                    ranged: u16::from_le_bytes([body[off + 18], body[off + 19]]),
                })
            }
            2 => Some(LookData::Door { size }),
            3 | 4 => Some(LookData::Transport { size }),
            _ => None,
        }
    }

    pub const CHAR_PC_GRAP_OFFSET: usize = 0x44;

    pub fn decode_char_pc(body: &[u8]) -> Option<Self> {
        let off = Self::CHAR_PC_GRAP_OFFSET;
        if body.len() < off + 18 {
            return None;
        }
        let slot0 = u16::from_le_bytes([body[off], body[off + 1]]);
        if slot0 == 0 {
            return None;
        }
        let face = (slot0 & 0x00FF) as u8;
        let race = ((slot0 >> 8) & 0x00FF) as u8;

        let read_slot = |i: usize| -> u16 {
            let p = off + 2 * i;
            u16::from_le_bytes([body[p], body[p + 1]]) & 0x0FFF
        };
        Some(LookData::Equipped {
            face,
            race,
            head: read_slot(1),
            body: read_slot(2),
            hands: read_slot(3),
            legs: read_slot(4),
            feet: read_slot(5),
            main: read_slot(6),
            sub: read_slot(7),
            ranged: read_slot(8),
        })
    }
}

/// NPC/MOB appearance-state from the General block of the 0x0E `CHAR_NPC`
/// packet, alongside the [`LookData`] at 0x2C. Offsets per
/// `vendor/server/src/map/packets/entity_update.cpp` (`updateWith`), with
/// body[0] == LSB packet 0x04: `animation` at LSB 0x1F → body[0x1B],
/// `status` at LSB 0x20 → body[0x1C], `animationsub` at LSB 0x2A → body[0x26].
///
/// `animationsub != 0` is the server's "active sub-animation effect" signal that
/// drives brazier/lamp/torch flames. On spawn LSB sets 0x2A to `4 | animationsub`
/// (bit 2 is a spawn flag), so the raw byte is kept and consumers mask 0x04 for
/// the bare selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NpcState {
    pub animation: u8,
    pub animationsub: u8,
    pub status: u8,
}

impl NpcState {
    pub const ANIMATION_OFFSET: usize = 0x1B;
    pub const STATUS_OFFSET: usize = 0x1C;
    pub const ANIMATIONSUB_OFFSET: usize = 0x26;

    /// Decode the appearance-state bytes from a `CHAR_NPC` (0x0E) body. Returns
    /// `None` if the body is too short to reach `animationsub` (the furthest of
    /// the three fields). Callers should only trust `animation`/`animationsub`
    /// when the packet's General/UPDATE_HP send-flag bit (0x04) is set — the
    /// server only refreshes them in that block — whereas `status` (0x20) is
    /// written on every update.
    pub fn decode_char_npc(body: &[u8]) -> Option<Self> {
        if body.len() <= Self::ANIMATIONSUB_OFFSET {
            return None;
        }
        Some(Self {
            animation: body[Self::ANIMATION_OFFSET],
            animationsub: body[Self::ANIMATIONSUB_OFFSET],
            status: body[Self::STATUS_OFFSET],
        })
    }

    /// Decode appearance-state from a `CHAR_PC` (0x0D) body. PCs share the
    /// `GP_SERV_POS_HEAD` prefix, so `animation` (`server_status`) sits at the
    /// same body[0x1B] — but unlike `CHAR_NPC` the 0x1C/0x26 bytes fall inside
    /// the PC `Flags1`/`Flags3` bitfields, so only `animation` is meaningful
    /// (`animationsub`/`status` left zero). Drives PC death pose / cast / sit.
    /// vendor/server/src/map/packets/char_update.cpp (`GP_SERV_CHAR_PC`).
    /// Trust only when the General send-flag bit (0x04) is set.
    pub fn decode_char_pc(body: &[u8]) -> Option<Self> {
        if body.len() <= Self::ANIMATION_OFFSET {
            return None;
        }
        Some(Self {
            animation: body[Self::ANIMATION_OFFSET],
            animationsub: 0,
            status: 0,
        })
    }

    /// `status` (LSB 0x20 → body[0x1C]) alone, for `CHAR_NPC`. Unlike the General
    /// block's `animation`/`animationsub`, the server writes this byte on every
    /// update regardless of the UPDATE_HP send-flag, so it is valid on pos-only /
    /// status-only ticks. vendor/server/src/map/packets/entity_update.cpp.
    pub fn decode_char_npc_status(body: &[u8]) -> Option<u8> {
        body.get(Self::STATUS_OFFSET).copied()
    }
}

const _: () = {
    assert!(NpcState::ANIMATION_OFFSET < NpcState::STATUS_OFFSET);
    assert!(NpcState::STATUS_OFFSET < NpcState::ANIMATIONSUB_OFFSET);
    assert!(NpcState::ANIMATIONSUB_OFFSET < LookData::LOOK_BODY_OFFSET);
};

#[cfg(test)]
mod despawn_tests {
    use super::*;
    use crate::map::s2c;

    fn body_with_updatemask(mask: u8) -> Vec<u8> {
        let mut body = vec![0u8; PosHead::SIZE_WITH_BT_TARGET];
        body[6] = mask;
        body
    }

    #[test]
    fn lsb_despawn_byte_0x30_on_char_npc_is_despawn() {
        let body = body_with_updatemask(0x30);
        assert!(PosHead::is_entity_despawn(s2c::CHAR_NPC, &body));
    }

    #[test]
    fn despawn_bit_alone_is_despawn() {
        let body = body_with_updatemask(PosHead::UPDATE_DESPAWN);
        assert!(PosHead::is_entity_despawn(s2c::CHAR_NPC, &body));
    }

    #[test]
    fn spawn_and_normal_updatemasks_are_not_despawn() {
        for mask in [0x0F, 0x57, 0x01, 0x07, 0x08, 0x10, 0x1F] {
            assert_eq!(mask & PosHead::UPDATE_DESPAWN, 0, "test mask sanity");
            let body = body_with_updatemask(mask);
            assert!(
                !PosHead::is_entity_despawn(s2c::CHAR_NPC, &body),
                "CHAR_NPC updatemask 0x{mask:02x} must not be treated as despawn",
            );
            assert!(
                !PosHead::is_entity_despawn(s2c::CHAR_PC, &body),
                "CHAR_PC SendFlg 0x{mask:02x} must not be treated as despawn",
            );
        }
    }

    #[test]
    fn despawn_bit_on_char_pc_is_despawn() {
        let body = body_with_updatemask(PosHead::UPDATE_DESPAWN);
        assert!(PosHead::is_entity_despawn(s2c::CHAR_PC, &body));
    }

    #[test]
    fn truncated_body_is_not_despawn() {
        assert!(!PosHead::is_entity_despawn(s2c::CHAR_NPC, &[]));
        assert!(!PosHead::is_entity_despawn(s2c::CHAR_NPC, &[0u8; 4]));
        assert!(!PosHead::is_entity_despawn(s2c::CHAR_PC, &[0u8; 4]));
    }
}

fn read_name_slot(slot: &[u8]) -> Option<String> {
    let n = slot.iter().position(|&b| b == 0).unwrap_or(slot.len());
    if n < 3 {
        return None;
    }
    let bytes = &slot[..n];
    if !bytes.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        return None;
    }
    Some(String::from_utf8_lossy(bytes).into_owned())
}

#[derive(Debug, Clone, Copy)]
pub struct CharSync {
    pub targid: u16,
    pub id: u32,
    /// MogExpansionFlag: MH second floor unlocked (`mhflag & 0x20`), byte 0x27 of the
    /// full packet = body 0x23. vendor/server/src/map/packets/char_sync.cpp:61.
    /// `None` when the packet is too short to carry it.
    pub mh_2f_unlocked: Option<bool>,
}

impl CharSync {
    pub const SUB_TYPE: u8 = 0x02;
    pub const SIZE: usize = 8;

    pub const MH_2F_UNLOCKED_OFFSET: usize = 0x23;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            targid: u16::from_le_bytes(body[2..4].try_into().unwrap()),
            id: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            mh_2f_unlocked: body.get(Self::MH_2F_UNLOCKED_OFFSET).map(|&b| b != 0),
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntitySetName {
    pub targid: u16,
    pub id: u32,
    pub master_targid: u16,
    pub name: Option<String>,
}

impl EntitySetName {
    pub const SUB_TYPE: u8 = 0x03;

    pub const SIZE: usize = 0x14;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let name = read_name_slot(&body[0x14..]);
        Ok(Self {
            targid: u16::from_le_bytes(body[2..4].try_into().unwrap()),
            id: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            master_targid: u16::from_le_bytes(body[8..10].try_into().unwrap()),
            name,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PetSync {
    pub owner_targid: u16,
    pub owner_id: u32,
    pub pet_targid: u16,
    pub hp_pct: u8,
    pub mp_pct: u8,
    pub tp: u16,
    pub bt_target_id: u32,
    pub name: Option<String>,
}

impl PetSync {
    pub const DESPAWN_SIZE: usize = 8;

    pub const FULL_HEADER_SIZE: usize = 0x14;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::DESPAWN_SIZE {
            return Err(DecodeError::Truncated(Self::DESPAWN_SIZE, body.len()));
        }
        let owner_targid = u16::from_le_bytes(body[2..4].try_into().unwrap());
        let owner_id = u32::from_le_bytes(body[4..8].try_into().unwrap());
        if body.len() < Self::FULL_HEADER_SIZE {
            return Ok(Self {
                owner_targid,
                owner_id,
                pet_targid: 0,
                hp_pct: 0,
                mp_pct: 0,
                tp: 0,
                bt_target_id: 0,
                name: None,
            });
        }
        let name = read_name_slot(&body[0x14..]);
        Ok(Self {
            owner_targid,
            owner_id,
            pet_targid: u16::from_le_bytes(body[8..10].try_into().unwrap()),
            hp_pct: body[0x0A],
            mp_pct: body[0x0B],
            tp: u16::from_le_bytes(body[0x0C..0x0E].try_into().unwrap()),
            bt_target_id: u32::from_le_bytes(body[0x10..0x14].try_into().unwrap()),
            name,
        })
    }
}
