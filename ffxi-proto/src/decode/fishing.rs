use super::*;

/// s2c 0x115 GP_SERV_COMMAND_FISH. The server sends this once a fish bites to hand the
/// client every parameter it needs to simulate the catch mini-game locally.
/// vendor/server/src/map/packets/s2c/0x115_fish.h, research/XiPackets/world/server/0x0115
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishPacket {
    /// The fish's starting (and maximum) stamina.
    pub stamina: u16,
    /// Base reaction window for an arrow press (client adjusts by intuition).
    pub arrow_delay: u16,
    /// Per-tick stamina regen, biased by 128 server-side (`regen - 128`).
    pub regen: u16,
    /// How often the fish thrashes left/right (client scales by 20).
    pub move_frequency: u16,
    /// Stamina removed on a correct, on-time arrow press.
    pub arrow_damage: u16,
    /// Stamina restored on a missed/late arrow press.
    pub arrow_regen: u16,
    /// Time limit to land the fish, in seconds (client scales by 60 → frames).
    pub time: u16,
    /// Angler-sense flags: bit0 alters the music/arrow timing, bit1 triggers the
    /// "intuition" light-bulb animation when the fish is first hooked.
    pub angler_sense: u8,
    /// Fishing intuition; reflected back to the server and used for the golden arrows.
    pub intuition: u32,
}

impl FishPacket {
    pub(crate) const SIZE: usize = 20;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let rd16 = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
        Ok(Self {
            stamina: rd16(0),
            arrow_delay: rd16(2),
            regen: rd16(4),
            move_frequency: rd16(6),
            arrow_damage: rd16(8),
            arrow_regen: rd16(10),
            time: rd16(12),
            angler_sense: body[14],
            intuition: u32::from_le_bytes([body[16], body[17], body[18], body[19]]),
        })
    }

    /// `true` when the angler-sense bit that drives the "intuition" hook animation
    /// (the light bulb) is set. vendor `fish->sense2`.
    pub fn shows_intuition(&self) -> bool {
        (self.angler_sense >> 1) & 1 == 1
    }
}

#[cfg(test)]
mod fish_packet_tests {
    use super::*;

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
}
