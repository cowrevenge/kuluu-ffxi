use super::*;

// vendor/server/src/map/packets/s2c/0x057_weather.h:32-37 (StartTime u32, WeatherNumber, WeatherOffsetTime u16)
#[derive(Debug, Clone, Copy)]
pub struct WeatherPacket {
    pub start_time: u32,
    pub weather_number: u16,
    pub offset_time: u16,
}

impl WeatherPacket {
    pub(crate) const SIZE: usize = 8;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            start_time: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            weather_number: u16::from_le_bytes(body[4..6].try_into().unwrap()),
            offset_time: u16::from_le_bytes(body[6..8].try_into().unwrap()),
        })
    }
}

#[cfg(test)]
mod weather_packet_tests {
    use super::*;

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
}
