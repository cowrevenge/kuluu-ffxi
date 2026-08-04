use super::*;

/// s2c 0x0CA GP_SERV_COMMAND_INSPECT_MESSAGE — the checked PC's bazaar message
/// and title. LSB pushes it immediately before the 0x0C9 EQUIPMENT/GENERAL
/// batches, so it carries no target id of its own
/// (vendor/server/src/map/packets/c2s/0x0dd_equip_inspect.cpp:134-136); the
/// caller correlates it with the /check it just sent.
/// vendor/server/src/map/packets/s2c/0x0ca_inspect_message.h:36-44.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectMessage {
    pub message: String,
    /// `sName` — the checked PC's name, the only correlation key in the packet.
    pub name: String,
    /// `DesignationNo` — the target's equipped title id (`profile.title`).
    pub title: u32,
    pub bazaar: bool,
    pub race: u8,
}

impl InspectMessage {
    const MESSAGE_OFFSET: usize = 0;
    const MESSAGE_LEN: usize = 123;
    const FLAGS_OFFSET: usize = Self::MESSAGE_OFFSET + Self::MESSAGE_LEN;
    const NAME_OFFSET: usize = Self::FLAGS_OFFSET + 1;
    const NAME_LEN: usize = 16;
    const TITLE_OFFSET: usize = Self::NAME_OFFSET + Self::NAME_LEN;
    pub const SIZE: usize = Self::TITLE_OFFSET + 4;

    // `uint8 BazaarFlag:1; uint8 MyFlag:1; uint8 Race:6` — little-endian
    // bitfields allocate from the low bit (0x0ca_inspect_message.h:39-41).
    const BAZAAR_FLAG: u8 = 1 << 0;
    const RACE_SHIFT: u32 = 2;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let flags = body[Self::FLAGS_OFFSET];
        Ok(Self {
            message: nul_terminated(
                &body[Self::MESSAGE_OFFSET..Self::MESSAGE_OFFSET + Self::MESSAGE_LEN],
            ),
            name: nul_terminated(&body[Self::NAME_OFFSET..Self::NAME_OFFSET + Self::NAME_LEN]),
            title: u32::from_le_bytes(
                body[Self::TITLE_OFFSET..Self::TITLE_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            ),
            bazaar: flags & Self::BAZAAR_FLAG != 0,
            race: flags >> Self::RACE_SHIFT,
        })
    }
}

fn nul_terminated(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(message: &str, name: &str, title: u32, flags: u8) -> Vec<u8> {
        let mut buf = vec![0u8; InspectMessage::SIZE];
        buf[..message.len()].copy_from_slice(message.as_bytes());
        buf[InspectMessage::FLAGS_OFFSET] = flags;
        let name_at = InspectMessage::NAME_OFFSET;
        buf[name_at..name_at + name.len()].copy_from_slice(name.as_bytes());
        let title_at = InspectMessage::TITLE_OFFSET;
        buf[title_at..title_at + 4].copy_from_slice(&title.to_le_bytes());
        buf
    }

    #[test]
    fn decodes_message_name_and_title() {
        // LSB's constructor sets BazaarFlag=1, MyFlag=1, Race=1 => 0b000001_1_1.
        let raw = body("Selling cheap!", "Aliya", 315, 0b0000_0111);
        let m = InspectMessage::decode(&raw).expect("decode");
        assert_eq!(m.message, "Selling cheap!");
        assert_eq!(m.name, "Aliya");
        assert_eq!(m.title, 315);
        assert!(m.bazaar);
        assert_eq!(m.race, 1);
    }

    #[test]
    fn empty_message_decodes_empty_not_padded() {
        let m = InspectMessage::decode(&body("", "Aliya", 0, 0)).expect("decode");
        assert!(m.message.is_empty());
        assert!(!m.bazaar);
    }

    #[test]
    fn a_message_filling_the_field_keeps_every_byte() {
        let full = "x".repeat(InspectMessage::MESSAGE_LEN);
        let m = InspectMessage::decode(&body(&full, "Aliya", 0, 0)).expect("decode");
        assert_eq!(m.message.len(), InspectMessage::MESSAGE_LEN);
    }

    #[test]
    fn truncated_body_errors() {
        assert!(matches!(
            InspectMessage::decode(&[0u8; InspectMessage::SIZE - 1]),
            Err(DecodeError::Truncated(n, _)) if n == InspectMessage::SIZE
        ));
    }
}
