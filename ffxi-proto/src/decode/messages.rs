use super::*;

#[derive(Debug, Clone, Copy)]
pub struct SystemMessage {
    pub para: u32,
    pub para2: u32,
    pub message_id: u16,
}

impl SystemMessage {
    pub(crate) const SIZE: usize = 12;

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
    pub(crate) const NUM_COUNT: usize = 4;
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

#[cfg(test)]
mod system_message_tests {
    use super::*;

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
}
