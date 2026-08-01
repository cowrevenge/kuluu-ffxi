pub mod animation;

mod widescan;
pub use widescan::*;

mod entity;
pub use entity::*;
mod login;
pub use login::*;
mod status;
pub use status::*;
mod emote;
pub use emote::*;
mod fishing;
pub use fishing::*;
mod messages;
pub use messages::*;
mod movement;
pub use movement::*;
mod party;
pub use party::*;
mod inventory;
pub use inventory::*;
mod delivery;
pub use delivery::*;
mod abilities;
pub use abilities::*;
mod equip_inspect;
pub use equip_inspect::*;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("expected at least {0} bytes, have {1}")]
    Truncated(usize, usize),
    #[error("unrecognized discriminant 0x{0:02x}")]
    UnknownDiscriminant(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_data_decodes_standard_modelid() {
        let mut buf = vec![0u8; 0x40];
        buf[0x2C..0x2E].copy_from_slice(&0u16.to_le_bytes());
        buf[0x2E..0x30].copy_from_slice(&0x1234u16.to_le_bytes());
        assert_eq!(
            LookData::decode_char_npc(&buf),
            Some(LookData::Standard { modelid: 0x1234 })
        );
    }

    #[test]
    fn look_data_decodes_equipped_look_t() {
        let mut buf = vec![0u8; 0x50];
        buf[0x2C..0x2E].copy_from_slice(&1u16.to_le_bytes());
        buf[0x2E] = 0x07;
        buf[0x2F] = 0x03;
        for (i, v) in [
            0xA001u16, 0xA002, 0xA003, 0xA004, 0xA005, 0xA006, 0xA007, 0xA008,
        ]
        .iter()
        .enumerate()
        {
            buf[0x30 + 2 * i..0x32 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        assert_eq!(
            LookData::decode_char_npc(&buf),
            Some(LookData::Equipped {
                face: 0x07,
                race: 0x03,
                head: 0xA001,
                body: 0xA002,
                hands: 0xA003,
                legs: 0xA004,
                feet: 0xA005,
                main: 0xA006,
                sub: 0xA007,
                ranged: 0xA008,
            })
        );
    }

    #[test]
    fn look_data_truncated_returns_none() {
        let buf = vec![0u8; 0x20];
        assert_eq!(LookData::decode_char_npc(&buf), None);
    }

    #[test]
    fn look_data_unknown_sentinel_returns_none() {
        let mut buf = vec![0u8; 0x40];
        buf[0x2C..0x2E].copy_from_slice(&0x00FFu16.to_le_bytes());
        assert_eq!(LookData::decode_char_npc(&buf), None);
    }

    #[test]
    fn look_data_decodes_pc_grapidtbl() {
        let mut buf = vec![0u8; 0x60];
        let off = LookData::CHAR_PC_GRAP_OFFSET;

        buf[off..off + 2].copy_from_slice(&0x0107u16.to_le_bytes());

        let gear: [u16; 8] = [0x111, 0x222, 0x333, 0x444, 0x555, 0x666, 0x777, 0x888];
        for (i, raw) in gear.iter().enumerate() {
            let slot_idx = i + 1;
            let masked = *raw | ((slot_idx as u16) << 12);
            let p = off + 2 * slot_idx;
            buf[p..p + 2].copy_from_slice(&masked.to_le_bytes());
        }
        assert_eq!(
            LookData::decode_char_pc(&buf),
            Some(LookData::Equipped {
                face: 0x07,
                race: 0x01,
                head: 0x111,
                body: 0x222,
                hands: 0x333,
                legs: 0x444,
                feet: 0x555,
                main: 0x666,
                sub: 0x777,
                ranged: 0x888,
            })
        );
    }

    #[test]
    fn look_data_pc_zero_modelid_returns_none() {
        let buf = vec![0u8; 0x60];
        assert_eq!(LookData::decode_char_pc(&buf), None);
    }

    #[test]
    fn look_data_pc_truncated_returns_none() {
        let mut buf = vec![0u8; 0x55];

        buf[LookData::CHAR_PC_GRAP_OFFSET..LookData::CHAR_PC_GRAP_OFFSET + 2]
            .copy_from_slice(&0x0107u16.to_le_bytes());
        assert_eq!(LookData::decode_char_pc(&buf), None);
    }

    #[test]
    fn npc_state_decodes_lsb_general_block_offsets() {
        let mut body = vec![0u8; 0x30];
        body[NpcState::ANIMATION_OFFSET] = 0x21;
        body[NpcState::STATUS_OFFSET] = 0x02;
        body[NpcState::ANIMATIONSUB_OFFSET] = 0x05;
        assert_eq!(
            NpcState::decode_char_npc(&body),
            Some(NpcState {
                animation: 0x21,
                animationsub: 0x05,
                status: 0x02,
            })
        );
    }

    #[test]
    fn npc_state_matches_fireworks_effect_npc() {
        const SPAWN_FLAG: u8 = 0x04;
        let mut body = vec![0u8; 0x48];
        body[NpcState::ANIMATION_OFFSET] = 0;
        body[NpcState::STATUS_OFFSET] = 2;
        body[NpcState::ANIMATIONSUB_OFFSET] = SPAWN_FLAG | 1;
        let st = NpcState::decode_char_npc(&body).expect("decode");
        assert_eq!(st.animation, 0);
        assert_eq!(st.status, 2);
        assert_ne!(st.animationsub, 0);
        assert_eq!(st.animationsub & !SPAWN_FLAG, 1);
    }

    #[test]
    fn npc_state_truncated_returns_none() {
        assert_eq!(NpcState::decode_char_npc(&[0u8; 0x26]), None);
        assert!(NpcState::decode_char_npc(&[0u8; 0x27]).is_some());
    }

    #[test]
    fn npc_state_status_readable_without_general_block() {
        let mut body = vec![0u8; NpcState::STATUS_OFFSET + 1];
        body[NpcState::STATUS_OFFSET] = 3;
        assert_eq!(
            NpcState::decode_char_npc(&body),
            None,
            "full NpcState needs the General block at ANIMATIONSUB_OFFSET"
        );
        assert_eq!(
            NpcState::decode_char_npc_status(&body),
            Some(3),
            "status alone reads from a body reaching only STATUS_OFFSET"
        );

        assert_eq!(
            NpcState::decode_char_npc_status(&[0u8; NpcState::STATUS_OFFSET]),
            None,
            "body not reaching 0x1C yields no status"
        );
    }

    #[test]
    fn npc_state_char_pc_reads_only_animation() {
        const DEATH: u8 = 3;
        let mut body = vec![0u8; PosHead::SIZE];
        body[NpcState::ANIMATION_OFFSET] = DEATH;
        // Bytes that are status/animationsub for CHAR_NPC are PC bitfield bits
        // here; decode_char_pc must ignore them.
        body[NpcState::STATUS_OFFSET] = 0xFF;
        let st = NpcState::decode_char_pc(&body).expect("decode");
        assert_eq!(st.animation, DEATH);
        assert_eq!(st.status, 0);
        assert_eq!(st.animationsub, 0);
    }

    #[test]
    fn npc_state_char_pc_truncated_returns_none() {
        assert_eq!(
            NpcState::decode_char_pc(&[0u8; NpcState::ANIMATION_OFFSET]),
            None
        );
        assert!(NpcState::decode_char_pc(&[0u8; NpcState::ANIMATION_OFFSET + 1]).is_some());
    }

    #[test]
    fn pos_head_minimal_decode() {
        let mut buf = vec![0u8; PosHead::SIZE];
        buf[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf[4..6].copy_from_slice(&0x0042u16.to_le_bytes());
        buf[6] = 0b0000_0001;
        buf[7] = 64;
        buf[8..12].copy_from_slice(&123.5f32.to_le_bytes());
        buf[12..16].copy_from_slice(&(-12.0f32).to_le_bytes());
        buf[16..20].copy_from_slice(&7.25f32.to_le_bytes());
        buf[24] = 25;
        buf[25] = 25;
        buf[26] = 100;
        buf[27] = 1;

        let h = PosHead::decode(&buf).unwrap();
        assert_eq!(h.unique_no, 0xDEAD_BEEF);
        assert_eq!(h.act_index, 0x42);
        assert_eq!(h.send_flag, 1);
        assert_eq!(h.dir, 64);
        assert_eq!(h.x, 123.5);
        assert_eq!(h.z, -12.0);
        assert_eq!(h.y, 7.25);
        assert_eq!(h.speed, 25);
        assert_eq!(h.speed_base, 25);
        assert_eq!(h.hpp, 100);
    }

    /// Pins the 2F-unlock byte to LSB's full-packet offset 0x27 minus the 4-byte
    /// sub-packet header (vendor/server/src/map/packets/char_sync.cpp:61).
    #[test]
    fn char_sync_2f_flag_sits_at_lsb_packet_byte_0x27() {
        assert_eq!(CharSync::MH_2F_UNLOCKED_OFFSET, 0x27 - 4);
    }

    #[test]
    fn pos_head_truncated_errors() {
        let buf = vec![0u8; PosHead::SIZE - 1];
        assert!(matches!(
            PosHead::decode(&buf),
            Err(DecodeError::Truncated(_, _))
        ));
    }

    #[test]
    fn pos_head_extracts_bt_target_id_when_present() {
        let mut buf = vec![0u8; PosHead::SIZE_WITH_BT_TARGET];
        buf[0..4].copy_from_slice(&0xCAFE_F00Du32.to_le_bytes());
        buf[40..44].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        let h = PosHead::decode(&buf).unwrap();
        assert_eq!(h.unique_no, 0xCAFE_F00D);
        assert_eq!(h.bt_target_id, 0xDEAD_BEEF);
    }

    #[test]
    fn pos_head_extracts_facetarget_from_flags0() {
        // facetarget occupies Flags0 bits 17..31; targid 0x1A2 must round-trip
        // and not bleed into the low MovTime/RunMode/GroundFlag/KingFlag bits.
        let mut buf = vec![0u8; PosHead::SIZE_WITH_BT_TARGET];
        let flags0 = (0x01A2u32 << 17) | 0x0001_FFFF;
        buf[20..24].copy_from_slice(&flags0.to_le_bytes());
        let h = PosHead::decode(&buf).unwrap();
        assert_eq!(h.facetarget(), 0x01A2);
    }

    #[test]
    fn pos_head_zero_flags0_has_no_facetarget() {
        let buf = vec![0u8; PosHead::SIZE];
        let h = PosHead::decode(&buf).unwrap();
        assert_eq!(h.facetarget(), 0);
    }

    #[test]
    fn decode_char_npc_extracts_claim_id() {
        let mut buf = vec![0u8; PosHead::SIZE_WITH_BT_TARGET];
        buf[0..4].copy_from_slice(&0xAABB_CCDDu32.to_le_bytes());
        buf[4..6].copy_from_slice(&0x07F0u16.to_le_bytes());
        buf[40..44].copy_from_slice(&0x0123_4567u32.to_le_bytes());
        let (head, claim_id) = PosHead::decode_char_npc(&buf).unwrap();
        assert_eq!(head.unique_no, 0xAABB_CCDD);
        assert_eq!(head.act_index, 0x07F0);
        assert_eq!(claim_id, 0x0123_4567);
    }

    #[test]
    fn decode_char_npc_unclaimed_yields_zero_claim() {
        let buf = vec![0u8; PosHead::SIZE];
        let (_, claim_id) = PosHead::decode_char_npc(&buf).unwrap();
        assert_eq!(claim_id, 0);
    }

    #[test]
    fn pos_head_legacy_40_byte_body_yields_zero_bt_target() {
        let buf = vec![0u8; PosHead::SIZE];
        let h = PosHead::decode(&buf).unwrap();
        assert_eq!(h.bt_target_id, 0);
    }

    #[test]
    fn try_extract_name_recovers_char_npc_with_update_name() {
        use crate::map::s2c;

        let mut buf = vec![0u8; 64];
        buf[6] = 0x08;
        buf[0x30..0x30 + 9].copy_from_slice(b"Sigli-Sea");
        let name = PosHead::try_extract_name(s2c::CHAR_NPC, &buf);
        assert_eq!(name.as_deref(), Some("Sigli-Sea"));
    }

    #[test]
    fn try_extract_name_returns_none_without_update_name() {
        use crate::map::s2c;

        let mut buf = vec![0u8; 64];
        buf[0x30..0x30 + 5].copy_from_slice(b"Junk!");
        assert!(PosHead::try_extract_name(s2c::CHAR_NPC, &buf).is_none());
    }

    #[test]
    fn try_extract_name_char_npc_renamed_low_targid_shift() {
        use crate::map::s2c;

        let mut buf = vec![0u8; 68];
        buf[6] = 0x08;
        buf[0x30] = 0x01;
        buf[0x31..0x31 + 12].copy_from_slice(b"Big Bad Bee\0");
        let name = PosHead::try_extract_name(s2c::CHAR_NPC, &buf);
        assert_eq!(name.as_deref(), Some("Big Bad Bee"));
    }

    #[test]
    fn try_extract_name_char_pc_uses_fixed_offset_with_send_flag() {
        use crate::map::s2c;

        let mut buf = vec![0u8; 0x60];
        buf[6] = 0x08;
        buf[0x56..0x56 + 6].copy_from_slice(b"Cleric");
        let name = PosHead::try_extract_name(s2c::CHAR_PC, &buf);
        assert_eq!(name.as_deref(), Some("Cleric"));
    }

    #[test]
    fn try_extract_name_char_pc_rejects_when_send_flag_clear() {
        use crate::map::s2c;

        let mut buf = vec![0u8; 0x60];
        buf[6] = 0x01;
        buf[0x56..0x56 + 6].copy_from_slice(b"Junked");
        assert!(PosHead::try_extract_name(s2c::CHAR_PC, &buf).is_none());
    }

    #[test]
    fn entity_set_name_decodes_trust_name() {
        let mut buf = vec![0u8; 0x28];
        buf[0] = 0x03;
        buf[1] = 0x05;
        buf[2..4].copy_from_slice(&0x07F2u16.to_le_bytes());
        buf[4..8].copy_from_slice(&0x0123_45F2u32.to_le_bytes());
        buf[8..10].copy_from_slice(&0x0042u16.to_le_bytes());
        buf[0x14..0x14 + 13].copy_from_slice(b"Mihli Aliapoh");

        let ent = EntitySetName::decode(&buf).unwrap();
        assert_eq!(ent.targid, 0x07F2);
        assert_eq!(ent.id, 0x0123_45F2);
        assert_eq!(ent.master_targid, 0x0042);
        assert_eq!(ent.name.as_deref(), Some("Mihli Aliapoh"));
    }

    #[test]
    fn entity_set_name_short_name_rejected() {
        let mut buf = vec![0u8; 0x28];
        buf[0] = 0x03;
        buf[4..8].copy_from_slice(&0x42u32.to_le_bytes());
        buf[0x14..0x14 + 2].copy_from_slice(b"Mi");

        let ent = EntitySetName::decode(&buf).unwrap();
        assert!(ent.name.is_none());
    }

    #[test]
    fn entity_set_name_truncated_errors() {
        let buf = vec![0u8; EntitySetName::SIZE - 1];
        assert!(matches!(
            EntitySetName::decode(&buf),
            Err(DecodeError::Truncated(_, _))
        ));
    }

    #[test]
    fn pet_sync_decodes_full_pet_record() {
        let mut buf = vec![0u8; 0x28];
        buf[0] = 0x04;
        buf[2..4].copy_from_slice(&0x0001u16.to_le_bytes());
        buf[4..8].copy_from_slice(&0x0010_0001u32.to_le_bytes());
        buf[8..10].copy_from_slice(&0x07A5u16.to_le_bytes());
        buf[0x0A] = 87;
        buf[0x0B] = 60;
        buf[0x0C..0x0E].copy_from_slice(&1234u16.to_le_bytes());
        buf[0x10..0x14].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf[0x14..0x14 + 11].copy_from_slice(b"Crab Family");

        let pet = PetSync::decode(&buf).unwrap();
        assert_eq!(pet.owner_targid, 0x0001);
        assert_eq!(pet.owner_id, 0x0010_0001);
        assert_eq!(pet.pet_targid, 0x07A5);
        assert_eq!(pet.hp_pct, 87);
        assert_eq!(pet.mp_pct, 60);
        assert_eq!(pet.tp, 1234);
        assert_eq!(pet.bt_target_id, 0xDEAD_BEEF);
        assert_eq!(pet.name.as_deref(), Some("Crab Family"));
    }

    #[test]
    fn pet_sync_despawn_variant_skips_pet_fields() {
        let mut buf = vec![0u8; 0x18];
        buf[0] = 0x04;
        buf[2..4].copy_from_slice(&0x0001u16.to_le_bytes());
        buf[4..8].copy_from_slice(&0x0010_0001u32.to_le_bytes());

        let pet = PetSync::decode(&buf).unwrap();
        assert_eq!(pet.owner_targid, 0x0001);
        assert_eq!(pet.owner_id, 0x0010_0001);
        assert_eq!(pet.pet_targid, 0);
        assert_eq!(pet.hp_pct, 0);
        assert!(pet.name.is_none());
    }

    #[test]
    fn pet_sync_truncated_below_owner_header_errors() {
        let buf = vec![0u8; PetSync::DESPAWN_SIZE - 1];
        assert!(matches!(
            PetSync::decode(&buf),
            Err(DecodeError::Truncated(_, _))
        ));
    }

    #[test]
    fn char_sync_decodes_ids() {
        let mut buf = vec![0u8; CharSync::SIZE];
        buf[0] = 0x02;
        buf[1] = 0x09;
        buf[2..4].copy_from_slice(&0x07F0u16.to_le_bytes());
        buf[4..8].copy_from_slice(&0x0123_4567u32.to_le_bytes());

        let sync = CharSync::decode(&buf).unwrap();
        assert_eq!(sync.targid, 0x07F0);
        assert_eq!(sync.id, 0x0123_4567);
        assert_eq!(
            sync.mh_2f_unlocked, None,
            "minimal body does not reach the 2F byte"
        );
    }

    #[test]
    fn char_sync_reads_mh_2f_unlock_bit() {
        // char_sync.cpp builds a 0x28-byte packet → 0x24-byte body.
        let mut buf = vec![0u8; 0x24];
        buf[0] = CharSync::SUB_TYPE;
        buf[4..8].copy_from_slice(&0x0123_4567u32.to_le_bytes());

        let sync = CharSync::decode(&buf).unwrap();
        assert_eq!(sync.mh_2f_unlocked, Some(false));

        buf[CharSync::MH_2F_UNLOCKED_OFFSET] = 1;
        let sync = CharSync::decode(&buf).unwrap();
        assert_eq!(sync.mh_2f_unlocked, Some(true));
    }
}
