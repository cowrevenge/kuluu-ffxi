use super::*;

/// s2c 0x00A Mog House cluster. Body offsets follow
/// vendor/server/src/map/packets/s2c/0x00a_login.h:115-127; `login_state` values are
/// the SAVE_LOGIN_STATE enum (h:50-59). `map_number` is the MH interior MODEL id
/// (GetMogHouseModelID, 0x00a_login.cpp:35-72), NOT a zone id; `mog_zone_flag` is
/// only assigned in the non-MH branch (.cpp: CanUseMisc(MISC_MOGMENU)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerLoginMyroom {
    pub login_state: u32,
    pub sub_map_number: u8,
    pub map_number: u16,
    pub exit_bit: u8,
    pub mog_zone_flag: u8,
}

impl ServerLoginMyroom {
    pub const LOGIN_STATE_MYROOM: u32 = 1;
    pub const LOGIN_STATE_GAME: u32 = 2;

    /// MyroomMapNumber sentinel "not in a Mog House" (0x00a_login.cpp non-MH branch).
    pub(crate) const MYROOM_NONE: u16 = 0x01FF;

    /// MyroomSubMapNumber value while on the MH second floor (0x00a_login.cpp MH branch).
    pub const SUB_MAP_2F: u8 = 0x02;

    /// LSB reuses LoginState MYROOM + this MyroomMapNumber for ZONE_FERETORY
    /// (Monstrosity), which is not a Mog House server-side
    /// (0x00a_login.cpp:234-239 sets it with no m_moghouseID).
    pub(crate) const MYROOM_FERETORY: u16 = 0x02D9;

    pub(crate) const LOGIN_STATE_OFFSET: usize = 0x7C;
    pub(crate) const SUB_MAP_NUMBER_OFFSET: usize = 0xA4;
    pub(crate) const MAP_NUMBER_OFFSET: usize = 0xA6;
    pub(crate) const EXIT_BIT_OFFSET: usize = 0xAA;
    pub(crate) const MOG_ZONE_FLAG_OFFSET: usize = 0xAB;
    pub(crate) const MIN_LEN: usize = Self::MOG_ZONE_FLAG_OFFSET + 1;

    fn decode(body: &[u8]) -> Option<Self> {
        if body.len() < Self::MIN_LEN {
            return None;
        }
        Some(Self {
            login_state: u32::from_le_bytes(
                body[Self::LOGIN_STATE_OFFSET..Self::LOGIN_STATE_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            ),
            sub_map_number: body[Self::SUB_MAP_NUMBER_OFFSET],
            map_number: u16::from_le_bytes(
                body[Self::MAP_NUMBER_OFFSET..Self::MAP_NUMBER_OFFSET + 2]
                    .try_into()
                    .unwrap(),
            ),
            exit_bit: body[Self::EXIT_BIT_OFFSET],
            mog_zone_flag: body[Self::MOG_ZONE_FLAG_OFFSET],
        })
    }

    /// The MH interior model id, only when the server actually placed the player
    /// in a Mog House: LoginState MYROOM excluding the [`Self::MYROOM_NONE`]
    /// sentinel and the [`Self::MYROOM_FERETORY`] alias.
    pub fn myroom_model(&self) -> Option<u16> {
        (self.login_state == Self::LOGIN_STATE_MYROOM
            && self.map_number != Self::MYROOM_NONE
            && self.map_number != Self::MYROOM_FERETORY)
            .then_some(self.map_number)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ServerLogin {
    pub unique_no: u32,
    pub act_index: u16,
    pub zone_no: u16,

    pub game_time: Option<u32>,

    pub pos_head: PosHead,

    pub music_num: Option<[u16; 5]>,

    pub myroom: Option<ServerLoginMyroom>,

    pub zone_in_event: Option<ZoneInEvent>,

    /// Weather in force as the character zones in.
    ///
    /// The server sends 0x057 WEATHER only when the weather *changes*
    /// (vendor/server/src/map/zone.cpp:672 is its sole construction site, a
    /// CHAR_INZONE broadcast), so this is the only weather a zoning character
    /// receives until the next change — which on a Vana'diel schedule can be a
    /// long wait.
    pub weather: Option<ZoneInWeather>,
}

/// Weather in force at zone-in, from the 0x00A weather block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneInWeather {
    /// `WeatherNumber` — the LSB weather id, same discriminant order as 0x057.
    pub weather_number: u16,
    /// `WeatherNumber2` — the second weather slot. Populated during a
    /// transition; not yet consumed, and not assumed to be the incoming side.
    pub weather_number2: u16,
    /// `WeatherOffsetTime`.
    pub offset_time: u32,
}

/// Zone-in cutscene carried inside s2c 0x00A LOGIN: when `currentEvent` is
/// already set at zone-in (e.g. the new-character intro, a Mog House 2F unlock
/// CS), LSB delivers it via the login packet instead of a 0x032/0x034 push
/// (vendor/server/src/map/packets/s2c/0x00a_login.cpp:183-192). The client
/// must answer with 0x05B `End` (`EventPara` = this `event_para`) or the char
/// stays InEvent server-side — zonelines/logout rejected — and the CS re-fires
/// on every subsequent login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneInEvent {
    /// `EventNum`: the zone id the event belongs to.
    pub event_num: u16,
    /// `EventPara`: the cutscene/event id (`currentEvent->eventId`).
    pub event_para: u16,
    /// `EventMode`: `currentEvent->eventFlags` low half.
    pub event_mode: u16,
}

impl ServerLogin {
    pub(crate) const SIZE: usize = 48;

    pub(crate) const MUSIC_NUM_OFFSET: usize = 0x52;
    pub(crate) const MUSIC_NUM_SIZE: usize = 5 * 2;

    pub(crate) const GAME_TIME_OFFSET: usize = 0x38;

    pub(crate) const EVENT_NUM_OFFSET: usize = 0x5E;
    pub(crate) const EVENT_PARA_OFFSET: usize = 0x60;
    pub(crate) const EVENT_MODE_OFFSET: usize = 0x62;
    // vendor/server/src/map/packets/s2c/0x00A_login.h:107-111 — WeatherNumber,
    // WeatherNumber2, WeatherTime, WeatherTime2, WeatherOffsetTime, immediately
    // after EventMode. The offset chain is pinned at both ends by constants this
    // decoder already uses: MusicNum[5] at 0x52 runs to SubMapNumber at 0x5C,
    // and past the weather block sit ShipStart/ShipEnd/IsMonstrosity, landing
    // exactly on LOGIN_STATE_OFFSET 0x7C.
    pub(crate) const WEATHER_NUMBER_OFFSET: usize = 0x64;
    pub(crate) const WEATHER_NUMBER2_OFFSET: usize = 0x66;
    pub(crate) const WEATHER_OFFSET_TIME_OFFSET: usize = 0x70;

    /// `PosHead.server_status` while a zone-in event is pending — the packet's
    /// event fields are only written then, and event id 0 is a real cutscene
    /// (Bastok Markets intro), so presence keys off the status byte
    /// (0x00a_login.cpp:191, ANIMATION_EVENT in
    /// vendor/server/src/map/entities/baseentity.h:66).
    pub(crate) const SERVER_STATUS_EVENT: u8 = 4;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }

        let zone_u32 = u32::from_le_bytes(body[44..48].try_into().unwrap());
        let pos_head = PosHead::decode(&body[..PosHead::SIZE_WITH_BT_TARGET])?;
        let game_time = if body.len() >= Self::GAME_TIME_OFFSET + 4 {
            Some(u32::from_le_bytes(
                body[Self::GAME_TIME_OFFSET..Self::GAME_TIME_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            ))
        } else {
            None
        };

        let music_num = if body.len() >= Self::MUSIC_NUM_OFFSET + Self::MUSIC_NUM_SIZE {
            let base = Self::MUSIC_NUM_OFFSET;
            Some([
                u16::from_le_bytes([body[base], body[base + 1]]),
                u16::from_le_bytes([body[base + 2], body[base + 3]]),
                u16::from_le_bytes([body[base + 4], body[base + 5]]),
                u16::from_le_bytes([body[base + 6], body[base + 7]]),
                u16::from_le_bytes([body[base + 8], body[base + 9]]),
            ])
        } else {
            None
        };
        let zone_in_event = (pos_head.server_status == Self::SERVER_STATUS_EVENT
            && body.len() >= Self::EVENT_MODE_OFFSET + 2)
            .then(|| ZoneInEvent {
                event_num: u16::from_le_bytes(
                    body[Self::EVENT_NUM_OFFSET..Self::EVENT_NUM_OFFSET + 2]
                        .try_into()
                        .unwrap(),
                ),
                event_para: u16::from_le_bytes(
                    body[Self::EVENT_PARA_OFFSET..Self::EVENT_PARA_OFFSET + 2]
                        .try_into()
                        .unwrap(),
                ),
                event_mode: u16::from_le_bytes(
                    body[Self::EVENT_MODE_OFFSET..Self::EVENT_MODE_OFFSET + 2]
                        .try_into()
                        .unwrap(),
                ),
            });
        let weather = (body.len() >= Self::WEATHER_OFFSET_TIME_OFFSET + 4).then(|| {
            let u16_at = |off: usize| u16::from_le_bytes(body[off..off + 2].try_into().unwrap());
            ZoneInWeather {
                weather_number: u16_at(Self::WEATHER_NUMBER_OFFSET),
                weather_number2: u16_at(Self::WEATHER_NUMBER2_OFFSET),
                offset_time: u32::from_le_bytes(
                    body[Self::WEATHER_OFFSET_TIME_OFFSET..Self::WEATHER_OFFSET_TIME_OFFSET + 4]
                        .try_into()
                        .unwrap(),
                ),
            }
        });
        Ok(Self {
            unique_no: pos_head.unique_no,
            act_index: pos_head.act_index,
            zone_no: zone_u32 as u16,
            game_time,
            pos_head,
            music_num,
            myroom: ServerLoginMyroom::decode(body),
            zone_in_event,
            weather,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ServerLogout {
    pub logout_state: u32,
    pub new_server_ip: u32,
    pub new_server_port: u16,
    pub error_code: u32,
}

impl ServerLogout {
    pub(crate) const SIZE: usize = 24;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            logout_state: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            new_server_ip: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            new_server_port: u32::from_le_bytes(body[8..12].try_into().unwrap()) as u16,
            error_code: u32::from_le_bytes(body[20..24].try_into().unwrap()),
        })
    }

    pub fn is_zone_change(&self) -> bool {
        self.logout_state == 2
    }
}

#[cfg(test)]
mod server_login_tests {
    // The weather block sits between EventMode and ShipStart in
    // vendor/server/src/map/packets/s2c/0x00A_login.h:107-111. Pin the offsets
    // against the two constants that bracket it, so a future field insertion
    // cannot silently slide weather onto the ship or event fields.
    #[test]
    fn zone_in_weather_offsets_sit_between_event_mode_and_login_state() {
        use super::ServerLogin as L;
        assert_eq!(L::WEATHER_NUMBER_OFFSET, L::EVENT_MODE_OFFSET + 2);
        assert_eq!(L::WEATHER_NUMBER2_OFFSET, L::WEATHER_NUMBER_OFFSET + 2);
        // WeatherTime + WeatherTime2 are the two u32s we skip.
        assert_eq!(
            L::WEATHER_OFFSET_TIME_OFFSET,
            L::WEATHER_NUMBER2_OFFSET + 2 + 4 + 4
        );
        // ShipStart u32, ShipEnd u16, IsMonstrosity u16, then LoginState.
        assert_eq!(
            L::WEATHER_OFFSET_TIME_OFFSET + 4 + 4 + 2 + 2,
            super::ServerLoginMyroom::LOGIN_STATE_OFFSET
        );
    }

    use super::*;

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
}

#[cfg(test)]
mod server_logout_tests {
    use super::*;

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
}
