use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PosMode {
    Normal = 0x00,
    Event = 0x01,
    Clear = 0x02,
    Pop = 0x03,
    Reset = 0x05,
    Materialize = 0x06,
    Lock = 0x08,
    Unlock = 0x09,
    Rotate = 0x0A,
}

impl PosMode {
    pub fn from_u8(raw: u8) -> Option<Self> {
        Some(match raw {
            0x00 => PosMode::Normal,
            0x01 => PosMode::Event,
            0x02 => PosMode::Clear,
            0x03 => PosMode::Pop,
            0x05 => PosMode::Reset,
            0x06 => PosMode::Materialize,
            0x08 => PosMode::Lock,
            0x09 => PosMode::Unlock,
            0x0A => PosMode::Rotate,
            _ => return None,
        })
    }

    pub fn carries_position(&self) -> bool {
        matches!(
            self,
            PosMode::Normal | PosMode::Event | PosMode::Pop | PosMode::Reset | PosMode::Materialize
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForcedMove {
    pub unique_no: u32,
    pub act_index: u16,
    pub mode: PosMode,
    pub x: f32,

    pub y: f32,

    pub z: f32,
    pub heading: u8,

    pub raw_mode: u8,
}

impl ForcedMove {
    pub(crate) const SIZE: usize = 24;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let lsb_x = f32::from_le_bytes(body[0..4].try_into().unwrap());
        let lsb_y = f32::from_le_bytes(body[4..8].try_into().unwrap());
        let lsb_z = f32::from_le_bytes(body[8..12].try_into().unwrap());
        let unique_no = u32::from_le_bytes(body[12..16].try_into().unwrap());
        let act_index = u16::from_le_bytes(body[16..18].try_into().unwrap());
        let raw_mode = body[18];

        let mode = PosMode::from_u8(raw_mode).unwrap_or(PosMode::Normal);
        let heading = body[19];
        Ok(Self {
            unique_no,
            act_index,
            mode,
            x: lsb_x,
            y: lsb_z,
            z: lsb_y,
            heading,
            raw_mode,
        })
    }
}

#[cfg(test)]
mod forced_move_tests {
    use super::*;

    #[test]
    fn forced_move_decodes_normal_mode_and_swaps_axes() {
        let mut body = vec![0u8; ForcedMove::SIZE];
        body[0..4].copy_from_slice(&12.5f32.to_le_bytes());
        body[4..8].copy_from_slice(&3.25f32.to_le_bytes());
        body[8..12].copy_from_slice(&(-7.0f32).to_le_bytes());
        body[12..16].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
        body[16..18].copy_from_slice(&42u16.to_le_bytes());
        body[18] = 0x00;
        body[19] = 64;
        let fm = ForcedMove::decode(&body).expect("decode");
        assert!((fm.x - 12.5).abs() < 1e-3);
        assert!((fm.y - (-7.0)).abs() < 1e-3, "NS (our.y) ← LSB.z");
        assert!((fm.z - 3.25).abs() < 1e-3, "height (our.z) ← LSB.y");
        assert_eq!(fm.unique_no, 0xDEADBEEF);
        assert_eq!(fm.act_index, 42);
        assert_eq!(fm.mode, PosMode::Normal);
        assert!(fm.mode.carries_position());
        assert_eq!(fm.heading, 64);
    }

    #[test]
    fn forced_move_lock_unlock_clear_do_not_carry_position() {
        for raw in [0x08u8, 0x09u8, 0x02u8, 0x0A] {
            let mut body = vec![0u8; ForcedMove::SIZE];
            body[18] = raw;
            let fm = ForcedMove::decode(&body).expect("decode");
            assert!(
                !fm.mode.carries_position(),
                "mode 0x{raw:02x} must not be authoritative for position",
            );
        }
    }

    #[test]
    fn forced_move_truncated_errors() {
        let body = vec![0u8; ForcedMove::SIZE - 1];
        assert!(matches!(
            ForcedMove::decode(&body),
            Err(DecodeError::Truncated(s, n)) if s == ForcedMove::SIZE && n == ForcedMove::SIZE - 1
        ));
    }

    #[test]
    fn forced_move_unknown_mode_falls_back_to_normal() {
        let mut body = vec![0u8; ForcedMove::SIZE];
        body[18] = 0x7F;
        let fm = ForcedMove::decode(&body).expect("decode");
        assert_eq!(fm.raw_mode, 0x7F);
        assert_eq!(fm.mode, PosMode::Normal);
    }
}
