use super::*;

/// s2c 0x058 GP_SERV_COMMAND_ASSIST — the server's answer to an assist request
/// and, more generally, its authoritative retarget push (LSB sends it from
/// `CCharEntity::OnChangeTarget`, `CPlayerController::Engage`, the
/// auto-target-after-kill scan in `CAttackState::UpdateTarget`, and
/// `battleutils::assistTarget`). Body offsets follow the PacketData struct in
/// vendor/server/src/map/packets/s2c/0x058_assist.h: UniqueNo u32 @0 (the local
/// player), AssistNo u32 @4 (the new target), ActIndex u16 @8, padding u16 @10.
///
/// `ActIndex` carries the *player's* own targid (0x058_assist.cpp GP_SERV_COMMAND_ASSIST::GP_SERV_COMMAND_ASSIST), not the
/// target's, and retail's `RecvAssist` ignores it
/// (research/XiPackets/world/server/0x0058/README.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assist {
    pub unique_no: u32,
    pub assist_no: u32,
    pub act_index: u16,
}

impl Assist {
    pub const UNIQUE_NO_OFFSET: usize = 0;
    pub const ASSIST_NO_OFFSET: usize = 4;
    pub const ACT_INDEX_OFFSET: usize = 8;
    pub const PADDING_OFFSET: usize = 10;
    pub const SIZE: usize = Self::PADDING_OFFSET + 2;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let rd32 = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        Ok(Self {
            unique_no: rd32(Self::UNIQUE_NO_OFFSET),
            assist_no: rd32(Self::ASSIST_NO_OFFSET),
            act_index: u16::from_le_bytes([
                body[Self::ACT_INDEX_OFFSET],
                body[Self::ACT_INDEX_OFFSET + 1],
            ]),
        })
    }

    /// LSB memsets the packet buffer (s2c/base.h `GP_SERV_PACKET()`) and only
    /// fills `AssistNo` when a target exists (0x058_assist.cpp GP_SERV_COMMAND_ASSIST::GP_SERV_COMMAND_ASSIST), so
    /// `AssistNo == 0` is "no target", not entity 0.
    pub fn target(&self) -> Option<u32> {
        (self.assist_no != 0).then_some(self.assist_no)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Body laid out field-by-field at the LSB struct offsets
    /// (vendor/server/src/map/packets/s2c/0x058_assist.h).
    fn body() -> Vec<u8> {
        let mut b = vec![0u8; Assist::SIZE];
        b[Assist::UNIQUE_NO_OFFSET..Assist::UNIQUE_NO_OFFSET + 4]
            .copy_from_slice(&0x0100_0F42u32.to_le_bytes());
        b[Assist::ASSIST_NO_OFFSET..Assist::ASSIST_NO_OFFSET + 4]
            .copy_from_slice(&0x0100_07D1u32.to_le_bytes());
        b[Assist::ACT_INDEX_OFFSET..Assist::ACT_INDEX_OFFSET + 2]
            .copy_from_slice(&0x0442u16.to_le_bytes());
        b
    }

    #[test]
    fn decodes_all_fields_at_lsb_offsets() {
        let a = Assist::decode(&body()).expect("decode");
        assert_eq!(a.unique_no, 0x0100_0F42);
        assert_eq!(a.assist_no, 0x0100_07D1);
        assert_eq!(a.act_index, 0x0442);
        assert_eq!(a.target(), Some(0x0100_07D1));
    }

    /// AssistNo is a u32 at offset 4, so a decoder that mistook it for the
    /// u16 ActIndex (or read the wrong word) would pick up these bytes.
    #[test]
    fn assist_no_is_the_second_u32_not_the_first() {
        let mut b = body();
        b[Assist::UNIQUE_NO_OFFSET..Assist::UNIQUE_NO_OFFSET + 4].copy_from_slice(&[0xEE; 4]);
        let a = Assist::decode(&b).expect("decode");
        assert_eq!(a.assist_no, 0x0100_07D1);
        assert_eq!(a.unique_no, 0xEEEE_EEEE);
    }

    #[test]
    fn zero_assist_no_is_no_target() {
        let mut b = body();
        b[Assist::ASSIST_NO_OFFSET..Assist::ASSIST_NO_OFFSET + 4].fill(0);
        assert_eq!(Assist::decode(&b).unwrap().target(), None);
    }

    /// A body shorter than the LSB struct must fail loudly rather than decode a
    /// half-read target.
    #[test]
    fn truncated_body_is_error() {
        assert!(matches!(
            Assist::decode(&[0u8; Assist::SIZE - 1]),
            Err(DecodeError::Truncated(n, have))
                if n == Assist::SIZE && have == Assist::SIZE - 1
        ));
    }
}
