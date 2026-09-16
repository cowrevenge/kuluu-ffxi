use super::*;
use crate::s2c_layout::weather as lsb;

// vendor/server/src/map/packets/s2c/0x057_weather.h GP_SERV_COMMAND_WEATHER PacketData (StartTime u32, WeatherNumber, WeatherOffsetTime u16)
#[derive(Debug, Clone, Copy)]
pub struct WeatherPacket {
    pub start_time: u32,
    pub weather_number: u16,
    pub offset_time: u16,
}

impl WeatherPacket {
    pub(crate) const START_TIME_OFFSET: usize = 0;
    pub(crate) const WEATHER_NUMBER_OFFSET: usize = 4;
    pub(crate) const OFFSET_TIME_OFFSET: usize = 6;
    pub(crate) const SIZE: usize = 8;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let rd32 = |o: usize| u32::from_le_bytes(body[o..o + 4].try_into().unwrap());
        let rd16 = |o: usize| u16::from_le_bytes(body[o..o + 2].try_into().unwrap());
        Ok(Self {
            start_time: rd32(Self::START_TIME_OFFSET),
            weather_number: rd16(Self::WEATHER_NUMBER_OFFSET),
            offset_time: rd16(Self::OFFSET_TIME_OFFSET),
        })
    }
}

pin_s2c_offset!(
    WeatherPacket::START_TIME_OFFSET,
    lsb::START_TIME,
    "GP_SERV_COMMAND_WEATHER.StartTime"
);
pin_s2c_offset!(
    WeatherPacket::WEATHER_NUMBER_OFFSET,
    lsb::WEATHER_NUMBER,
    "GP_SERV_COMMAND_WEATHER.WeatherNumber"
);
pin_s2c_offset!(
    WeatherPacket::OFFSET_TIME_OFFSET,
    lsb::WEATHER_OFFSET_TIME,
    "GP_SERV_COMMAND_WEATHER.WeatherOffsetTime"
);
pin_s2c_offset!(
    WeatherPacket::SIZE,
    lsb::SIZE,
    "GP_SERV_COMMAND_WEATHER.PacketData size"
);

#[cfg(test)]
mod weather_packet_tests {
    use super::*;

    #[test]
    fn weather_packet_decodes_fields() {
        let mut buf = [0u8; WeatherPacket::SIZE];
        buf[WeatherPacket::START_TIME_OFFSET..][..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf[WeatherPacket::WEATHER_NUMBER_OFFSET..][..2].copy_from_slice(&6u16.to_le_bytes());
        buf[WeatherPacket::OFFSET_TIME_OFFSET..][..2].copy_from_slice(&0x0123u16.to_le_bytes());
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
