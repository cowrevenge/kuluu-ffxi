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
    pub const MYROOM_NONE: u16 = 0x01FF;

    /// MyroomSubMapNumber value while on the MH second floor (0x00a_login.cpp MH branch).
    pub const SUB_MAP_2F: u8 = 0x02;

    /// LSB reuses LoginState MYROOM + this MyroomMapNumber for ZONE_FERETORY
    /// (Monstrosity), which is not a Mog House server-side
    /// (0x00a_login.cpp:234-239 sets it with no m_moghouseID).
    pub const MYROOM_FERETORY: u16 = 0x02D9;

    pub const LOGIN_STATE_OFFSET: usize = 0x7C;
    pub const SUB_MAP_NUMBER_OFFSET: usize = 0xA4;
    pub const MAP_NUMBER_OFFSET: usize = 0xA6;
    pub const EXIT_BIT_OFFSET: usize = 0xAA;
    pub const MOG_ZONE_FLAG_OFFSET: usize = 0xAB;
    pub const MIN_LEN: usize = Self::MOG_ZONE_FLAG_OFFSET + 1;

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
    pub const SIZE: usize = 48;

    pub const MUSIC_NUM_OFFSET: usize = 0x52;
    pub const MUSIC_NUM_SIZE: usize = 5 * 2;

    pub const GAME_TIME_OFFSET: usize = 0x38;

    pub const EVENT_NUM_OFFSET: usize = 0x5E;
    pub const EVENT_PARA_OFFSET: usize = 0x60;
    pub const EVENT_MODE_OFFSET: usize = 0x62;

    /// `PosHead.server_status` while a zone-in event is pending — the packet's
    /// event fields are only written then, and event id 0 is a real cutscene
    /// (Bastok Markets intro), so presence keys off the status byte
    /// (0x00a_login.cpp:191, ANIMATION_EVENT in
    /// vendor/server/src/map/entities/baseentity.h:66).
    pub const SERVER_STATUS_EVENT: u8 = 4;

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
        Ok(Self {
            unique_no: pos_head.unique_no,
            act_index: pos_head.act_index,
            zone_no: zone_u32 as u16,
            game_time,
            pos_head,
            music_num,
            myroom: ServerLoginMyroom::decode(body),
            zone_in_event,
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
    pub const SIZE: usize = 24;

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
