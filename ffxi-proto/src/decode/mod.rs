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
    #[error("opcode 0x{got:03x} does not match expected 0x{expected:03x}")]
    OpcodeMismatch { expected: u16, got: u16 },
    #[error("unrecognized discriminant 0x{0:02x}")]
    UnknownDiscriminant(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the GP_SERV_COMMAND_PBX_RESULT layout to LSB's PacketData struct
    /// (vendor/server/src/map/packets/s2c/0x04b_pbx_result.h): header fields at
    /// 0-11, GP_POST_BOX_STATE at 12 (Stat, name[16], request id/time, sub id,
    /// ItemNo, Kind, Stack, Data[28]) in the 0x58 full form.
    #[test]
    fn pbx_result_reads_lsb_offsets_full_form() {
        let mut buf = vec![0u8; PbxResult::FULL_SIZE];
        buf[0] = 0x06; // Command = Recv
        buf[1] = 1; // BoxNo = Incoming
        buf[2] = 3; // PostWorkNo
        buf[3] = 0xFF; // ItemWorkNo = -1
        buf[4..8].copy_from_slice(&(-1i32).to_le_bytes()); // ItemStacks
        buf[8] = 0x01; // Result = OK
        buf[9] = 1; // ResParam1 = count
        buf[10] = 0xFF; // ResParam2 = -1
        buf[11] = 0xFF; // ResParam3 = -1
        buf[12..16].copy_from_slice(&7u32.to_le_bytes()); // Stat = incoming
        buf[16..20].copy_from_slice(b"Atti"); // From (NUL-padded)
        buf[40..44].copy_from_slice(&0i32.to_le_bytes()); // sub id
        buf[44..46].copy_from_slice(&5075u16.to_le_bytes()); // ItemNo
        buf[52..56].copy_from_slice(&2u32.to_le_bytes()); // Stack
        buf[56] = 0xAB; // Data[0] (m_extra)

        let r = PbxResult::decode(&buf).expect("decodes");
        assert_eq!(r.command, 0x06);
        assert_eq!(r.box_no, 1);
        assert_eq!(r.post_work_no, 3);
        assert_eq!(r.item_work_no, -1);
        assert_eq!(r.item_stacks, -1);
        assert_eq!(r.result, 0x01);
        assert_eq!(r.res_param1, 1);
        assert_eq!(r.res_param2, -1);
        let s = r.state.expect("full form carries GP_POST_BOX_STATE");
        assert_eq!(s.stat, 7);
        assert_eq!(s.counterpart.as_deref(), Some("Atti"));
        assert_eq!(s.item_no, 5075);
        assert_eq!(s.stack, 2);
        assert_eq!(s.extra[0], 0xAB);
    }

    /// The short 0x14 form (4-arg LSB ctor) has no box state; a Check response
    /// carries the new-item count in ResParam2 (Incoming) / ResParam3 (Outgoing)
    /// (0x04b_pbx_result.cpp:44-54).
    #[test]
    fn pbx_result_short_form_check_counts() {
        let mut buf = vec![0u8; PbxResult::SHORT_SIZE];
        buf[0] = 0x05; // Check
        buf[1] = 1; // Incoming
        buf[2] = 0xFF;
        buf[3] = 0xFF;
        buf[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        buf[8] = 0x01; // Result = OK
        buf[9] = 0xFF;
        buf[10] = 2; // ResParam2 = 2 new items
        buf[11] = 0xFF;

        let r = PbxResult::decode(&buf).expect("decodes");
        assert_eq!(r.command, 0x05);
        assert_eq!(r.res_param2, 2);
        assert!(r.state.is_none(), "short form has no state");

        assert!(PbxResult::decode(&buf[..12]).is_err(), "truncated rejects");
    }

    #[test]
    fn clistatus_reads_stat_block_offsets() {
        let mut buf = vec![0u8; 84];
        buf[0..4].copy_from_slice(&1946u32.to_le_bytes()); // hp_max
        buf[4..8].copy_from_slice(&1295u32.to_le_bytes()); // mp_max
        buf[8] = 5; // mjob_no (RDM)
        buf[9] = 75; // mjob_lv
        buf[10] = 4; // sjob_no (BLM)
        buf[11] = 37; // sjob_lv
        for i in 0..7 {
            buf[16 + i * 2..18 + i * 2].copy_from_slice(&((10 + i as u16) * 5).to_le_bytes());
            buf[30 + i * 2..32 + i * 2].copy_from_slice(&((i as i16 + 1) * 7).to_le_bytes());
        }
        buf[44..46].copy_from_slice(&1048u16.to_le_bytes()); // attack
        buf[46..48].copy_from_slice(&1006u16.to_le_bytes()); // defense
        buf[48..50].copy_from_slice(&(-15i16).to_le_bytes()); // fire resist
        buf[81] = 119; // ilvl

        let cs = CliStatus::decode(&buf).expect("decodes");
        assert_eq!(cs.hp_max, 1946);
        assert_eq!(cs.mp_max, 1295);
        assert_eq!(cs.mjob_no, 5);
        assert_eq!(cs.mjob_lv, 75);
        assert_eq!(cs.sjob_no, 4);
        assert_eq!(cs.sjob_lv, 37);
        assert_eq!(cs.bp_base[0], 50, "STR base");
        assert_eq!(cs.bp_base[6], 80, "CHR base");
        assert_eq!(cs.bp_adj[0], 7, "STR gear delta");
        assert_eq!(cs.attack, 1048);
        assert_eq!(cs.defense, 1006);
        assert_eq!(cs.def_elem[0], -15, "fire resist signed");
        assert_eq!(cs.ilvl, 119);
        assert!(
            CliStatus::decode(&buf[..80]).is_err(),
            "truncation rejected"
        );
    }

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

    #[test]
    fn server_login_decodes_zone_no() {
        let mut buf = vec![0u8; ServerLogin::SIZE];
        buf[0..4].copy_from_slice(&0x0123_4567u32.to_le_bytes());
        buf[4..6].copy_from_slice(&0x00FFu16.to_le_bytes());
        buf[44..48].copy_from_slice(&230u32.to_le_bytes());
        let l = ServerLogin::decode(&buf).unwrap();
        assert_eq!(l.unique_no, 0x0123_4567);
        assert_eq!(l.act_index, 0x00FF);
        assert_eq!(l.zone_no, 230);
    }

    #[test]
    fn server_login_zone_in_event_keys_off_status_byte_not_event_id() {
        let mut buf = vec![0u8; 0x100];
        buf[44..48].copy_from_slice(&234u32.to_le_bytes());
        buf[ServerLogin::EVENT_NUM_OFFSET..ServerLogin::EVENT_NUM_OFFSET + 2]
            .copy_from_slice(&234u16.to_le_bytes());
        // Bastok Markets intro cutscene is event id 0 — a zeroed EventPara must
        // still decode as an event when the status byte says so.
        buf[ServerLogin::EVENT_MODE_OFFSET..ServerLogin::EVENT_MODE_OFFSET + 2]
            .copy_from_slice(&32u16.to_le_bytes());

        let no_event = ServerLogin::decode(&buf).unwrap();
        assert_eq!(no_event.zone_in_event, None);

        buf[27] = ServerLogin::SERVER_STATUS_EVENT;
        let with_event = ServerLogin::decode(&buf).unwrap();
        assert_eq!(
            with_event.zone_in_event,
            Some(ZoneInEvent {
                event_num: 234,
                event_para: 0,
                event_mode: 32,
            })
        );
    }

    #[test]
    fn server_login_truncated_errors() {
        let buf = vec![0u8; ServerLogin::SIZE - 1];
        assert!(matches!(
            ServerLogin::decode(&buf),
            Err(DecodeError::Truncated(48, _))
        ));
    }

    #[test]
    fn server_login_myroom_cluster_roundtrips() {
        let mut buf = vec![0u8; 0x100];
        buf[44..48].copy_from_slice(&230u32.to_le_bytes());
        buf[ServerLoginMyroom::LOGIN_STATE_OFFSET..ServerLoginMyroom::LOGIN_STATE_OFFSET + 4]
            .copy_from_slice(&ServerLoginMyroom::LOGIN_STATE_MYROOM.to_le_bytes());
        buf[ServerLoginMyroom::SUB_MAP_NUMBER_OFFSET] = ServerLoginMyroom::SUB_MAP_2F;
        buf[ServerLoginMyroom::MAP_NUMBER_OFFSET..ServerLoginMyroom::MAP_NUMBER_OFFSET + 2]
            .copy_from_slice(&617u16.to_le_bytes());
        buf[ServerLoginMyroom::EXIT_BIT_OFFSET] = 3;
        buf[ServerLoginMyroom::MOG_ZONE_FLAG_OFFSET] = 1;

        let l = ServerLogin::decode(&buf).unwrap();
        let myroom = l.myroom.expect("full-size body carries the cluster");
        assert_eq!(myroom.login_state, ServerLoginMyroom::LOGIN_STATE_MYROOM);
        assert_eq!(myroom.sub_map_number, ServerLoginMyroom::SUB_MAP_2F);
        assert_eq!(myroom.map_number, 617);
        assert_eq!(myroom.exit_bit, 3);
        assert_eq!(myroom.mog_zone_flag, 1);
        assert_eq!(myroom.myroom_model(), Some(617));
    }

    #[test]
    fn server_login_truncated_body_yields_no_myroom() {
        let mut buf = vec![0u8; ServerLoginMyroom::MIN_LEN - 1];
        buf[44..48].copy_from_slice(&230u32.to_le_bytes());
        let l = ServerLogin::decode(&buf).unwrap();
        assert_eq!(l.zone_no, 230);
        assert!(l.myroom.is_none());
    }

    #[test]
    fn server_login_myroom_jeuno_model_decodes() {
        let mut buf = vec![0u8; 0x100];
        buf[44..48].copy_from_slice(&243u32.to_le_bytes());
        buf[ServerLoginMyroom::LOGIN_STATE_OFFSET..ServerLoginMyroom::LOGIN_STATE_OFFSET + 4]
            .copy_from_slice(&ServerLoginMyroom::LOGIN_STATE_MYROOM.to_le_bytes());
        buf[ServerLoginMyroom::MAP_NUMBER_OFFSET..ServerLoginMyroom::MAP_NUMBER_OFFSET + 2]
            .copy_from_slice(&0x0100u16.to_le_bytes());

        let myroom = ServerLogin::decode(&buf).unwrap().myroom.unwrap();
        assert_eq!(myroom.myroom_model(), Some(0x0100));
    }

    #[test]
    fn server_login_myroom_model_gated_on_state_and_sentinel() {
        let mut buf = vec![0u8; 0x100];
        buf[ServerLoginMyroom::LOGIN_STATE_OFFSET..ServerLoginMyroom::LOGIN_STATE_OFFSET + 4]
            .copy_from_slice(&ServerLoginMyroom::LOGIN_STATE_GAME.to_le_bytes());
        buf[ServerLoginMyroom::MAP_NUMBER_OFFSET..ServerLoginMyroom::MAP_NUMBER_OFFSET + 2]
            .copy_from_slice(&ServerLoginMyroom::MYROOM_NONE.to_le_bytes());
        let myroom = ServerLogin::decode(&buf).unwrap().myroom.unwrap();
        assert_eq!(myroom.login_state, ServerLoginMyroom::LOGIN_STATE_GAME);
        assert_eq!(myroom.myroom_model(), None, "GAME state carries no model");

        buf[ServerLoginMyroom::LOGIN_STATE_OFFSET..ServerLoginMyroom::LOGIN_STATE_OFFSET + 4]
            .copy_from_slice(&ServerLoginMyroom::LOGIN_STATE_MYROOM.to_le_bytes());
        let myroom = ServerLogin::decode(&buf).unwrap().myroom.unwrap();
        assert_eq!(
            myroom.myroom_model(),
            None,
            "MYROOM with the 0x01FF sentinel carries no model"
        );

        buf[ServerLoginMyroom::MAP_NUMBER_OFFSET..ServerLoginMyroom::MAP_NUMBER_OFFSET + 2]
            .copy_from_slice(&ServerLoginMyroom::MYROOM_FERETORY.to_le_bytes());
        let myroom = ServerLogin::decode(&buf).unwrap().myroom.unwrap();
        assert_eq!(
            myroom.myroom_model(),
            None,
            "Feretory MYROOM alias is not a Mog House"
        );
    }

    /// Pins the myroom cluster to LSB's GP_SERV_COMMAND_LOGIN PacketData layout
    /// (vendor/server/src/map/packets/s2c/0x00a_login.h:96-131; body offsets, no
    /// sub-packet header) so an offset edit can't pass the roundtrip tests, which
    /// build buffers through these same consts.
    #[test]
    fn myroom_cluster_offsets_and_sentinels_match_lsb_login_layout() {
        assert_eq!(ServerLoginMyroom::LOGIN_STATE_OFFSET, 0x7C);
        assert_eq!(ServerLoginMyroom::SUB_MAP_NUMBER_OFFSET, 0xA4);
        assert_eq!(ServerLoginMyroom::MAP_NUMBER_OFFSET, 0xA6);
        assert_eq!(ServerLoginMyroom::EXIT_BIT_OFFSET, 0xAA);
        assert_eq!(ServerLoginMyroom::MOG_ZONE_FLAG_OFFSET, 0xAB);
        assert_eq!(ServerLoginMyroom::LOGIN_STATE_MYROOM, 1, "SAVE_LOGIN_STATE");
        assert_eq!(ServerLoginMyroom::LOGIN_STATE_GAME, 2, "SAVE_LOGIN_STATE");
        assert_eq!(ServerLoginMyroom::MYROOM_NONE, 0x01FF);
        assert_eq!(ServerLoginMyroom::SUB_MAP_2F, 0x02);
        assert_eq!(ServerLoginMyroom::MYROOM_FERETORY, 0x02D9);
    }

    /// Pins JobInfo to LSB's GP_MYROOM_DANCER layout
    /// (vendor/server/src/map/packets/s2c/0x01b_job_info.h:28-45; job_lev2, not
    /// the legacy job_lev[16] @0x0C) and MAX_JOBTYPE
    /// (vendor/server/src/map/entities/battleentity.h:100), since the decode
    /// tests build buffers through these same consts.
    #[test]
    fn job_info_offsets_match_gp_myroom_dancer_layout() {
        assert_eq!(JobInfo::MJOB_NO_OFFSET, 0x04);
        assert_eq!(JobInfo::SJOB_NO_OFFSET, 0x07);
        assert_eq!(JobInfo::UNLOCKED_OFFSET, 0x08);
        assert_eq!(JobInfo::HP_MAX_OFFSET, 0x38);
        assert_eq!(JobInfo::MP_MAX_OFFSET, 0x3C);
        assert_eq!(JobInfo::SJOBFLG_OFFSET, 0x40);
        assert_eq!(JobInfo::JOB_LEVELS_OFFSET, 0x44);
        assert_eq!(JobInfo::MAX_JOBTYPE, 24);
    }

    /// Pins the 2F-unlock byte to LSB's full-packet offset 0x27 minus the 4-byte
    /// sub-packet header (vendor/server/src/map/packets/char_sync.cpp:61).
    #[test]
    fn char_sync_2f_flag_sits_at_lsb_packet_byte_0x27() {
        assert_eq!(CharSync::MH_2F_UNLOCKED_OFFSET, 0x27 - 4);
    }

    #[test]
    fn server_login_carries_pos_head_for_spawn_seed() {
        let mut buf = vec![0u8; ServerLogin::SIZE];
        buf[0..4].copy_from_slice(&0x0123_4567u32.to_le_bytes());
        buf[4..6].copy_from_slice(&0x00FFu16.to_le_bytes());
        buf[7] = 96;
        buf[8..12].copy_from_slice(&(-115.5f32).to_le_bytes());
        buf[12..16].copy_from_slice(&(7.25f32).to_le_bytes());
        buf[16..20].copy_from_slice(&(280.0f32).to_le_bytes());
        buf[24] = 40;
        buf[25] = 40;
        buf[44..48].copy_from_slice(&230u32.to_le_bytes());
        let l = ServerLogin::decode(&buf).unwrap();
        assert_eq!(l.pos_head.x, -115.5);
        assert_eq!(l.pos_head.z, 7.25);
        assert_eq!(l.pos_head.y, 280.0);
        assert_eq!(l.pos_head.dir, 96);
        assert_eq!(l.pos_head.speed, 40);
        assert_eq!(l.pos_head.speed_base, 40);
    }

    #[test]
    fn system_message_decodes() {
        let mut buf = vec![0u8; SystemMessage::SIZE];
        buf[0..4].copy_from_slice(&30u32.to_le_bytes());
        buf[4..8].copy_from_slice(&0u32.to_le_bytes());
        buf[8..10].copy_from_slice(&7u16.to_le_bytes());
        let m = SystemMessage::decode(&buf).unwrap();
        assert_eq!(m.para, 30);
        assert_eq!(m.para2, 0);
        assert_eq!(m.message_id, 7);
    }

    #[test]
    fn system_message_truncated_errors() {
        let buf = vec![0u8; SystemMessage::SIZE - 1];
        assert!(matches!(
            SystemMessage::decode(&buf),
            Err(DecodeError::Truncated(12, _))
        ));
    }

    #[test]
    fn server_logout_zone_change() {
        let mut buf = vec![0u8; ServerLogout::SIZE];
        buf[0..4].copy_from_slice(&2u32.to_le_bytes());
        buf[4..8].copy_from_slice(&0x6F00_A8C0u32.to_le_bytes());
        buf[8..12].copy_from_slice(&54230u32.to_le_bytes());
        let l = ServerLogout::decode(&buf).unwrap();
        assert!(l.is_zone_change());
        assert_eq!(l.new_server_port, 54230);
        assert_eq!(l.new_server_ip, 0x6F00_A8C0);
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
    fn party_attrs_group_attr_decodes() {
        let mut buf = vec![0u8; 36];
        buf[0..4].copy_from_slice(&0x0001_0042u32.to_le_bytes());
        buf[4..8].copy_from_slice(&1500u32.to_le_bytes());
        buf[8..12].copy_from_slice(&500u32.to_le_bytes());
        buf[12..16].copy_from_slice(&1234u32.to_le_bytes());
        buf[16..18].copy_from_slice(&0x0042u16.to_le_bytes());
        buf[18] = 75;
        buf[19] = 50;
        buf[20] = 0;
        buf[21] = 1;
        buf[22..24].copy_from_slice(&234u16.to_le_bytes());
        buf[28] = 6;
        buf[29] = 75;
        buf[30] = 1;
        buf[31] = 37;

        let p = PartyAttrs::decode_group_attr(&buf).unwrap();
        assert_eq!(p.unique_no, 0x0001_0042);
        assert_eq!(p.hp, 1500);
        assert_eq!(p.mp, 500);
        assert_eq!(p.tp, 1234);
        assert_eq!(p.act_index, 0x42);
        assert_eq!(p.hpp, 75);
        assert_eq!(p.mpp, 50);
        assert_eq!(p.moghouse_flg, 1);
        assert_eq!(p.zone_no, 234);
        assert_eq!(p.mjob_no, 6);
        assert_eq!(p.mjob_lv, 75);
        assert_eq!(p.sjob_no, 1);
        assert_eq!(p.sjob_lv, 37);
    }

    #[test]
    fn party_attrs_group_list_decodes_with_name_and_leader() {
        let mut buf = vec![0u8; 56];
        buf[0..4].copy_from_slice(&0x0010_0001u32.to_le_bytes());
        buf[4..8].copy_from_slice(&2000u32.to_le_bytes());
        buf[8..12].copy_from_slice(&100u32.to_le_bytes());
        buf[12..16].copy_from_slice(&0u32.to_le_bytes());

        buf[16..20].copy_from_slice(&0x0000_0005u32.to_le_bytes());
        buf[20..22].copy_from_slice(&0x0007u16.to_le_bytes());
        buf[22] = 1;
        buf[23] = 1;
        buf[24] = 0;
        buf[25] = 100;
        buf[26] = 100;
        buf[28..30].copy_from_slice(&230u16.to_le_bytes());
        buf[30] = 1;
        buf[31] = 75;
        buf[36..36 + 6].copy_from_slice(b"Vanari");

        let (attrs, extra) = PartyAttrs::decode_group_list(&buf).unwrap();
        assert_eq!(attrs.unique_no, 0x0010_0001);
        assert_eq!(attrs.hp, 2000);
        assert_eq!(attrs.act_index, 7);
        assert_eq!(attrs.zone_no, 230);
        assert_eq!(attrs.moghouse_flg, 1);
        assert_eq!(extra.member_number, 1);
        assert!(extra.is_party_leader);
        assert!(!extra.is_alliance_leader);
        assert_eq!(extra.name.as_deref(), Some("Vanari"));
    }

    #[test]
    fn party_attrs_group_list_truncated_errors() {
        let buf = vec![0u8; 40];
        assert!(matches!(
            PartyAttrs::decode_group_list(&buf),
            Err(DecodeError::Truncated(52, 40))
        ));
    }

    #[test]
    fn item_max_decodes_legacy_and_wide_capacity() {
        let mut buf = vec![0u8; ItemMax::SIZE];

        buf[0] = 81;

        buf[1] = 81;
        let wide_off = 18 + 14 + 2;
        buf[wide_off..wide_off + 2].copy_from_slice(&201u16.to_le_bytes());

        let wide_off = 18 + 14 + 10 * 2;
        buf[wide_off..wide_off + 2].copy_from_slice(&81u16.to_le_bytes());

        let m = ItemMax::decode(&buf).unwrap();
        assert_eq!(
            m.capacities[1], 200,
            "Mog Safe: wide takes precedence, +1 inverted"
        );
        assert_eq!(m.capacities[10], 80, "Wardrobe2: wide-only, +1 inverted");
        assert_eq!(
            m.capacities[17], 0,
            "Recycle Bin: zeroed (disabled sentinel)"
        );
    }

    /// LSB's only ItemNum2 = 0 emitter is a DISABLED container (a lapsed Mog
    /// Locker lease keeps its legacy byte sized), so once any wide value is
    /// present a zero must stay zero rather than fall back per-slot
    /// (vendor/server/src/map/packets/s2c/0x01c_item_max.cpp:52-57).
    #[test]
    fn item_max_wide_zero_is_the_disable_sentinel_not_a_fallback() {
        let mut buf = vec![0u8; ItemMax::SIZE];
        buf[0] = 31;
        buf[4] = 31; // lapsed locker: legacy still sized...
        let wide_off = 18 + 14;
        buf[wide_off..wide_off + 2].copy_from_slice(&31u16.to_le_bytes());
        // ...but ItemNum2[LOC_MOGLOCKER] stays 0.

        let m = ItemMax::decode(&buf).unwrap();
        assert_eq!(m.capacities[0], 30);
        assert_eq!(m.capacities[4], 0, "wide 0 = disabled, no legacy fallback");
    }

    #[test]
    fn item_max_falls_back_to_legacy_only_when_wide_is_absent() {
        let mut buf = vec![0u8; ItemMax::SIZE];

        buf[0] = 81;
        buf[4] = 21;

        let m = ItemMax::decode(&buf).unwrap();
        assert_eq!(m.capacities[0], 80, "pre-widening server: legacy decoded");
        assert_eq!(m.capacities[4], 20, "moglocker: legacy decoded with -1");
        assert_eq!(
            m.capacities[1], 0,
            "fully-disabled stays at 0, no underflow"
        );
    }

    #[test]
    fn item_max_truncated_errors() {
        let buf = vec![0u8; ItemMax::SIZE - 1];
        assert!(matches!(
            ItemMax::decode(&buf),
            Err(DecodeError::Truncated(96, _))
        ));
    }

    #[test]
    fn item_same_decodes_state_and_flags() {
        let mut buf = vec![0u8; ItemSame::SIZE];
        buf[0] = 0;
        buf[4..8].copy_from_slice(&0xCAFEu32.to_le_bytes());
        let s = ItemSame::decode(&buf).unwrap();
        assert_eq!(s.state, ItemSameState::StillLoading);
        assert_eq!(s.flags, 0xCAFE);

        buf[0] = 1;
        let s = ItemSame::decode(&buf).unwrap();
        assert_eq!(s.state, ItemSameState::AllLoaded);
    }

    #[test]
    fn item_num_decodes() {
        let mut buf = vec![0u8; ItemNum::SIZE];
        buf[0..4].copy_from_slice(&12345u32.to_le_bytes());
        buf[4] = 0;
        buf[5] = 7;
        buf[6] = 1;
        let n = ItemNum::decode(&buf).unwrap();
        assert_eq!(n.quantity, 12345);
        assert_eq!(n.category, 0);
        assert_eq!(n.index, 7);
        assert_eq!(n.lock_flg, 1);
    }

    #[test]
    fn item_list_decodes() {
        let mut buf = vec![0u8; ItemList::SIZE];
        buf[0..4].copy_from_slice(&1u32.to_le_bytes());
        buf[4..6].copy_from_slice(&4112u16.to_le_bytes());
        buf[6] = 5;
        buf[7] = 12;
        buf[8] = 0;
        let l = ItemList::decode(&buf).unwrap();
        assert_eq!(l.quantity, 1);
        assert_eq!(l.item_no, 4112);
        assert_eq!(l.category, 5);
        assert_eq!(l.index, 12);
    }

    #[test]
    fn item_attr_decodes_with_extdata() {
        let mut buf = vec![0u8; ItemAttr::SIZE];
        buf[0..4].copy_from_slice(&1u32.to_le_bytes());
        buf[4..8].copy_from_slice(&500_000u32.to_le_bytes());
        buf[8..10].copy_from_slice(&8000u16.to_le_bytes());
        buf[10] = 0;
        buf[11] = 3;
        buf[12] = 0;
        for (i, b) in buf[13..37].iter_mut().enumerate() {
            *b = i as u8;
        }
        let a = ItemAttr::decode(&buf).unwrap();
        assert_eq!(a.quantity, 1);
        assert_eq!(a.price, 500_000);
        assert_eq!(a.item_no, 8000);
        assert_eq!(a.category, 0);
        assert_eq!(a.index, 3);
        assert_eq!(a.extdata[0], 0);
        assert_eq!(a.extdata[23], 23);
    }

    #[test]
    fn item_attr_truncated_errors() {
        let buf = vec![0u8; ItemAttr::SIZE - 1];
        assert!(matches!(
            ItemAttr::decode(&buf),
            Err(DecodeError::Truncated(37, _))
        ));
    }

    fn item_attr_with_extdata(ext: [u8; 24]) -> ItemAttr {
        let mut buf = vec![0u8; ItemAttr::SIZE];
        buf[13..37].copy_from_slice(&ext);
        ItemAttr::decode(&buf).unwrap()
    }

    #[test]
    fn charge_info_none_for_non_charged_header() {
        let mut ext = [0u8; 24];
        ext[0] = 0x00;
        assert_eq!(item_attr_with_extdata(ext).charge_info(), None);
    }

    #[test]
    fn charge_info_reads_charges_next_use_and_ready() {
        // 0x020_item_attr.cpp:47-68 — header 0x01, charges at [1], ready bit
        // 0x40 in flags-hi [3], next-use vana timestamp at [4..8].
        let mut ext = [0u8; 24];
        ext[0] = 0x01;
        ext[1] = 2;
        ext[3] = 0x90;
        ext[4..8].copy_from_slice(&123_456u32.to_le_bytes());
        let ci = item_attr_with_extdata(ext).charge_info().unwrap();
        assert_eq!(ci.charges, 2);
        assert_eq!(ci.next_use_vana_ts, 123_456);
        assert!(!ci.ready);
    }

    #[test]
    fn charge_info_ready_bit_set_and_zero_timestamp() {
        let mut ext = [0u8; 24];
        ext[0] = 0x01;
        ext[1] = 1;
        ext[3] = 0xC0;
        let ci = item_attr_with_extdata(ext).charge_info().unwrap();
        assert_eq!(ci.charges, 1);
        assert_eq!(ci.next_use_vana_ts, 0);
        assert!(ci.ready);
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

    #[test]
    fn job_info_decodes_synthetic_body() {
        let mut buf = vec![0u8; 0x80];
        buf[JobInfo::MJOB_NO_OFFSET] = 5; // RDM
        buf[JobInfo::SJOB_NO_OFFSET] = 4; // BLM
                                          // bit 0 = subjob feature, bits 1..6 = WAR..THF unlocked.
        let unlocked: u32 = 0b0111_1111;
        buf[JobInfo::UNLOCKED_OFFSET..JobInfo::UNLOCKED_OFFSET + 4]
            .copy_from_slice(&unlocked.to_le_bytes());
        buf[JobInfo::HP_MAX_OFFSET..JobInfo::HP_MAX_OFFSET + 4]
            .copy_from_slice(&1946i32.to_le_bytes());
        buf[JobInfo::MP_MAX_OFFSET..JobInfo::MP_MAX_OFFSET + 4]
            .copy_from_slice(&1295i32.to_le_bytes());
        buf[JobInfo::SJOBFLG_OFFSET] = 1;
        for j in 0..JobInfo::MAX_JOBTYPE {
            buf[JobInfo::JOB_LEVELS_OFFSET + j] = j as u8 * 3;
        }
        // Legacy truncated job_lev[16] @0x0C left zeroed: proves we read job_lev2.
        let info = JobInfo::decode(&buf).unwrap();
        assert_eq!(info.mjob_no, 5);
        assert_eq!(info.sjob_no, 4);
        assert_eq!(info.unlocked, unlocked);
        assert!(info.sub_job_unlocked);
        assert_eq!(info.hp_max, 1946);
        assert_eq!(info.mp_max, 1295);
        assert_eq!(info.sjobflg, 1);
        assert_eq!(info.job_levels[1], 3, "WAR");
        assert_eq!(
            info.job_levels[22], 66,
            "RUN — beyond the legacy 16-job array"
        );
    }

    #[test]
    fn job_info_truncated_errors() {
        let buf = vec![0u8; JobInfo::MIN_LEN - 1];
        assert!(matches!(
            JobInfo::decode(&buf),
            Err(DecodeError::Truncated(_, _))
        ));
    }

    #[test]
    fn job_info_without_subjob_flag() {
        let mut buf = vec![0u8; JobInfo::MIN_LEN];
        buf[JobInfo::UNLOCKED_OFFSET..JobInfo::UNLOCKED_OFFSET + 4]
            .copy_from_slice(&0b0000_0110u32.to_le_bytes());
        let info = JobInfo::decode(&buf).unwrap();
        assert!(!info.sub_job_unlocked);
        assert_eq!(info.sjobflg, 0);
    }

    #[test]
    fn weather_packet_decodes_fields() {
        let mut buf = [0u8; WeatherPacket::SIZE];
        buf[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf[4..6].copy_from_slice(&6u16.to_le_bytes());
        buf[6..8].copy_from_slice(&0x0123u16.to_le_bytes());
        let w = WeatherPacket::decode(&buf).unwrap();
        assert_eq!(w.start_time, 0xDEAD_BEEF);
        assert_eq!(w.weather_number, 6);
        assert_eq!(w.offset_time, 0x0123);
    }

    #[test]
    fn weather_packet_truncated_returns_err() {
        let buf = [0u8; WeatherPacket::SIZE - 1];
        assert!(matches!(
            WeatherPacket::decode(&buf),
            Err(DecodeError::Truncated(WeatherPacket::SIZE, n)) if n == WeatherPacket::SIZE - 1
        ));
    }

    #[test]
    fn equip_list_decodes_field_order() {
        let buf = [0x05u8, 0x04, 0x08, 0x00];
        let e = EquipList::decode(&buf).expect("decode");
        assert_eq!(e.container_index, 5);
        assert_eq!(e.equip_slot, 4);
        assert_eq!(e.container, 8);
    }

    #[test]
    fn equip_list_truncated_returns_err() {
        let buf = [0u8; EquipList::SIZE - 1];
        assert!(matches!(
            EquipList::decode(&buf),
            Err(DecodeError::Truncated(EquipList::SIZE, n)) if n == EquipList::SIZE - 1
        ));
    }

    #[test]
    fn magic_data_known_ids_picks_set_bits() {
        let mut buf = [0u8; MagicData::SIZE];

        buf[0] = 0b1000_0001;
        buf[1] = 0b0000_0001;
        buf[2] = 0b0000_0010;
        buf[127] = 0b1000_0000;
        let m = MagicData::decode(&buf).unwrap();
        assert_eq!(m.known_ids(), vec![0, 7, 8, 17, 1023]);
        assert!(m.is_known(0));
        assert!(m.is_known(7));
        assert!(m.is_known(1023));
        assert!(!m.is_known(1));

        assert!(!m.is_known(u16::MAX));
    }

    #[test]
    fn magic_data_truncated_returns_err() {
        let buf = [0u8; MagicData::SIZE - 1];
        assert!(matches!(
            MagicData::decode(&buf),
            Err(DecodeError::Truncated(MagicData::SIZE, n)) if n == MagicData::SIZE - 1
        ));
    }

    #[test]
    fn command_data_splits_into_four_bitsets() {
        let mut buf = [0u8; CommandData::SIZE];

        buf[0] = 0xA1;
        buf[64] = 0xA2;
        buf[128] = 0xA3;
        buf[192] = 0xA4;
        let c = CommandData::decode(&buf).unwrap();
        assert_eq!(c.weapon_skills[0], 0xA1);
        assert_eq!(c.job_abilities[0], 0xA2);
        assert_eq!(c.pet_abilities[0], 0xA3);
        assert_eq!(c.traits[0], 0xA4);

        assert_eq!(c.weapon_skills.len(), 64);
        assert_eq!(c.job_abilities.len(), 64);
        assert_eq!(c.pet_abilities.len(), 64);
        assert_eq!(c.traits.len(), 32);
    }

    #[test]
    fn command_data_truncated_returns_err() {
        let buf = [0u8; CommandData::SIZE - 1];
        assert!(matches!(
            CommandData::decode(&buf),
            Err(DecodeError::Truncated(CommandData::SIZE, n)) if n == CommandData::SIZE - 1
        ));
    }

    #[test]
    fn char_status_decodes_death_counter_and_homepoint_seconds() {
        // Full wire body: GP_SERV_SERVERSTATUS is 0x60 incl. the 4-byte sub-header, so
        // the body (which `sub.data` exposes) is 0x5C. Sizing to that — rather than just
        // past dead_counter2 — keeps the fields anchored if a trailing field shifts.
        let mut body = vec![0u8; 0x5C];
        body[CharStatus::UNIQUE_NO_OFFSET..CharStatus::UNIQUE_NO_OFFSET + 4]
            .copy_from_slice(&0x000B_C5EBu32.to_le_bytes());
        // Flags0 with hpp (bits 16..24) == 0 → KO'd.
        body[CharStatus::FLAGS0_OFFSET..CharStatus::FLAGS0_OFFSET + 4]
            .copy_from_slice(&0u32.to_le_bytes());
        // 60 * (360 + 1800): 30 min until the forced home-point warp.
        body[CharStatus::DEAD_COUNTER1_OFFSET..CharStatus::DEAD_COUNTER1_OFFSET + 4]
            .copy_from_slice(&129_600u32.to_le_bytes());
        body[CharStatus::DEAD_COUNTER2_OFFSET..CharStatus::DEAD_COUNTER2_OFFSET + 4]
            .copy_from_slice(&0x1122_3344u32.to_le_bytes());
        body[CharStatus::SERVER_STATUS_OFFSET] = animation::FISHING_START;
        body[CharStatus::FISHING_TIMER_OFFSET] = 42;

        let cs = CharStatus::decode(&body).unwrap();
        assert_eq!(cs.unique_no, 0x000B_C5EB);
        assert_eq!(cs.hpp, 0);
        assert_eq!(cs.dead_counter1, 129_600);
        assert_eq!(cs.dead_counter2, 0x1122_3344);
        assert_eq!(cs.seconds_until_homepoint(), 1800);
        assert_eq!(cs.server_status, animation::FISHING_START);
        assert_eq!(cs.fishing_timer, 42);
    }

    #[test]
    fn char_status_fishing_timer_zero_when_truncated_before_field() {
        // dead_counter2 is the last guaranteed field; fishing_timer sits past it and must
        // default to 0 rather than panic when the body stops short.
        let body = vec![0u8; CharStatus::DEAD_COUNTER2_OFFSET + 4];
        let cs = CharStatus::decode(&body).unwrap();
        assert_eq!(cs.fishing_timer, 0);
    }

    #[test]
    fn fish_packet_decodes_minigame_params() {
        let mut body = vec![0u8; FishPacket::SIZE];
        body[0..2].copy_from_slice(&200u16.to_le_bytes()); // stamina
        body[2..4].copy_from_slice(&5u16.to_le_bytes()); // arrow_delay
        body[4..6].copy_from_slice(&130u16.to_le_bytes()); // regen
        body[6..8].copy_from_slice(&3u16.to_le_bytes()); // move_frequency
        body[8..10].copy_from_slice(&40u16.to_le_bytes()); // arrow_damage
        body[10..12].copy_from_slice(&10u16.to_le_bytes()); // arrow_regen
        body[12..14].copy_from_slice(&30u16.to_le_bytes()); // time
        body[14] = 0b11; // angler_sense: both bits set
        body[16..20].copy_from_slice(&0x0000_0064u32.to_le_bytes()); // intuition

        let f = FishPacket::decode(&body).unwrap();
        assert_eq!(f.stamina, 200);
        assert_eq!(f.arrow_delay, 5);
        assert_eq!(f.regen, 130);
        assert_eq!(f.move_frequency, 3);
        assert_eq!(f.arrow_damage, 40);
        assert_eq!(f.arrow_regen, 10);
        assert_eq!(f.time, 30);
        assert_eq!(f.intuition, 100);
        assert!(f.shows_intuition());

        assert!(matches!(
            FishPacket::decode(&[0u8; FishPacket::SIZE - 1]),
            Err(DecodeError::Truncated(n, _)) if n == FishPacket::SIZE
        ));
    }

    #[test]
    fn char_status_homepoint_seconds_boundaries() {
        let secs = |dc1: u32| {
            CharStatus {
                unique_no: 0,
                hpp: 0,
                dead_counter1: dc1,
                dead_counter2: 0,
                server_status: 0,
                fishing_timer: 0,
                speed: 0,
            }
            .seconds_until_homepoint()
        };
        // Fresh death: 60 * (6min + 60min) → full 60 min remaining.
        assert_eq!(secs(60 * (360 + 3600)), 3600);
        // At/below the 6-min padding floor, saturate at 0 instead of wrapping.
        assert_eq!(secs(60 * 360), 0);
        assert_eq!(secs(0), 0);
    }

    #[test]
    fn char_status_extracts_hpp_from_flags0() {
        let mut body = vec![0u8; CharStatus::DEAD_COUNTER2_OFFSET + 4];
        body[CharStatus::FLAGS0_OFFSET..CharStatus::FLAGS0_OFFSET + 4]
            .copy_from_slice(&(75u32 << 16).to_le_bytes());
        assert_eq!(CharStatus::decode(&body).unwrap().hpp, 75);
    }

    #[test]
    fn char_status_truncated_returns_err() {
        let need = CharStatus::DEAD_COUNTER2_OFFSET + 4;
        let buf = vec![0u8; need - 1];
        assert!(matches!(
            CharStatus::decode(&buf),
            Err(DecodeError::Truncated(n, have)) if n == need && have == need - 1
        ));
    }

    #[test]
    fn char_status_decodes_speed_and_masks_high_nibble() {
        let mut body = vec![0u8; CharStatus::MIN_LEN];
        body[CharStatus::SPEED_OFFSET..CharStatus::SPEED_OFFSET + 2]
            .copy_from_slice(&0xA078u16.to_le_bytes());
        assert_eq!(CharStatus::decode(&body).unwrap().speed, 0x078);
    }

    #[test]
    fn char_status_base_speed_decodes() {
        let mut body = vec![0u8; CharStatus::MIN_LEN];
        body[CharStatus::SPEED_OFFSET..CharStatus::SPEED_OFFSET + 2]
            .copy_from_slice(&0x0032u16.to_le_bytes());
        assert_eq!(CharStatus::decode(&body).unwrap().speed, 50);
    }

    #[test]
    fn char_status_bound_speed_zero_decodes() {
        let body = vec![0u8; CharStatus::MIN_LEN];
        assert_eq!(CharStatus::decode(&body).unwrap().speed, 0);
    }
}
