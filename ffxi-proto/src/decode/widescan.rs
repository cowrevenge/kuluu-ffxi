use super::DecodeError;

/// One s2c 0x0F4 wide-scan (tracking) list entry.
/// vendor/server/src/map/packets/s2c/0x0f4_tracking_list.h PacketData: a packed
/// u32 (ActIndex:16, Level:8, Type:3, unused:5), int16 x, int16 z, uint8 sName[16].
/// `Type` = objtype/2 → 0 char, 1 npc, 2 mob (0x0f4_tracking_list.cpp).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidescanEntry {
    pub act_index: u16,
    pub level: u8,
    /// Marker category: 0 = char (black), 1 = npc (green), 2 = mob (red).
    pub kind: u8,
    /// Entity X minus self X in server units (0x0f4_tracking_list.cpp).
    pub rel_x: i16,
    /// Entity Z minus self Z in server units.
    pub rel_z: i16,
    /// sName[16]; LSB currently leaves this empty (0x0f4_tracking_list.cpp TODO).
    pub name: String,
}

impl WidescanEntry {
    pub(crate) const SIZE: usize = 24;

    /// Type occupies 3 bits of the packed u32 (0x0f4_tracking_list.h Type:3).
    const TYPE_MASK: u32 = 0x07;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let packed = u32::from_le_bytes(body[0..4].try_into().unwrap());
        let name_bytes = &body[8..24];
        let n = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        Ok(Self {
            act_index: (packed & 0xFFFF) as u16,
            level: ((packed >> 16) & 0xFF) as u8,
            kind: ((packed >> 24) & Self::TYPE_MASK) as u8,
            rel_x: i16::from_le_bytes([body[4], body[5]]),
            rel_z: i16::from_le_bytes([body[6], body[7]]),
            name: String::from_utf8_lossy(&name_bytes[..n]).into_owned(),
        })
    }
}

/// s2c 0x0F5 tracking-position update for the currently tracked entity.
/// vendor/server/src/map/packets/s2c/0x0f5_tracking_pos.h PacketData: float x/y/z,
/// uint8 Level, uint8 unused, uint16 ActIndex, GP_TRACKING_POS_STATE State (uint8).
/// Coordinates are raw server values (LSB does not swap Y/Z here; see the .cpp TODO).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidescanPos {
    pub act_index: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `State == Lose` — the tracked entity left wide-scan range / despawned.
    pub lost: bool,
}

impl WidescanPos {
    pub(crate) const SIZE: usize = 17;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let rdf = |o: usize| f32::from_le_bytes(body[o..o + 4].try_into().unwrap());
        Ok(Self {
            x: rdf(0),
            y: rdf(4),
            z: rdf(8),
            act_index: u16::from_le_bytes([body[14], body[15]]),
            lost: body[16] == crate::map::tracking::pos_state::LOSE,
        })
    }
}

/// s2c 0x0F6 wide-scan list framing.
/// vendor/server/src/map/packets/s2c/0x0f6_tracking_state.h PacketData: a single
/// GP_TRACKING_STATE byte. ListStart brackets a fresh list, ListEnd closes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidescanState {
    pub list_start: bool,
    pub list_end: bool,
}

impl WidescanState {
    pub(crate) const SIZE: usize = 1;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        use crate::map::tracking::list_state;
        Ok(Self {
            list_start: body[0] == list_state::LIST_START,
            list_end: body[0] == list_state::LIST_END,
        })
    }
}

#[cfg(test)]
mod widescan_tests {
    use super::*;

    fn entry_bytes(act: u16, level: u8, kind: u8, rx: i16, rz: i16, name: &str) -> Vec<u8> {
        let mut body = vec![0u8; WidescanEntry::SIZE];
        let packed = (act as u32) | ((level as u32) << 16) | ((kind as u32 & 0x07) << 24);
        body[0..4].copy_from_slice(&packed.to_le_bytes());
        body[4..6].copy_from_slice(&rx.to_le_bytes());
        body[6..8].copy_from_slice(&rz.to_le_bytes());
        let nb = name.as_bytes();
        let n = nb.len().min(15);
        body[8..8 + n].copy_from_slice(&nb[..n]);
        body
    }

    #[test]
    fn widescan_entry_roundtrip() {
        let body = entry_bytes(0x1234, 42, 2, -300, 512, "Mandragora");
        let e = WidescanEntry::decode(&body).expect("decode");
        assert_eq!(e.act_index, 0x1234);
        assert_eq!(e.level, 42);
        assert_eq!(e.kind, 2);
        assert_eq!(e.rel_x, -300);
        assert_eq!(e.rel_z, 512);
        assert_eq!(e.name, "Mandragora");
    }

    #[test]
    fn widescan_entry_empty_name_is_empty_string() {
        let body = entry_bytes(1, 1, 1, 0, 0, "");
        let e = WidescanEntry::decode(&body).expect("decode");
        assert_eq!(e.name, "");
    }

    #[test]
    fn widescan_entry_truncated_errors() {
        let body = vec![0u8; WidescanEntry::SIZE - 1];
        assert!(matches!(
            WidescanEntry::decode(&body),
            Err(DecodeError::Truncated(s, n)) if s == WidescanEntry::SIZE && n == WidescanEntry::SIZE - 1
        ));
    }

    #[test]
    fn widescan_pos_roundtrip_start_and_lose() {
        use crate::map::tracking::pos_state;
        let mut body = vec![0u8; WidescanPos::SIZE];
        body[0..4].copy_from_slice(&12.5f32.to_le_bytes());
        body[4..8].copy_from_slice(&(-3.0f32).to_le_bytes());
        body[8..12].copy_from_slice(&64.0f32.to_le_bytes());
        body[12] = 1;
        body[14..16].copy_from_slice(&0x0ABCu16.to_le_bytes());
        body[16] = pos_state::START;
        let p = WidescanPos::decode(&body).expect("decode");
        assert!((p.x - 12.5).abs() < 1e-3);
        assert!((p.y - (-3.0)).abs() < 1e-3);
        assert!((p.z - 64.0).abs() < 1e-3);
        assert_eq!(p.act_index, 0x0ABC);
        assert!(!p.lost, "State Start is not a loss");

        body[16] = pos_state::LOSE;
        let lost = WidescanPos::decode(&body).expect("decode");
        assert!(lost.lost, "State Lose flags a loss");
    }

    #[test]
    fn widescan_state_list_start_end_are_disjoint() {
        use crate::map::tracking::list_state;
        let start = WidescanState::decode(&[list_state::LIST_START]).expect("decode");
        assert!(start.list_start && !start.list_end);
        let end = WidescanState::decode(&[list_state::LIST_END]).expect("decode");
        assert!(!end.list_start && end.list_end);
        let none = WidescanState::decode(&[list_state::NONE]).expect("decode");
        assert!(!none.list_start && !none.list_end);
    }
}
