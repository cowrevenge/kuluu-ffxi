//! s2c 0x056 GP_SERV_COMMAND_MISSION — the mission/quest log battery (vault main_menu.md §30).
//!
//! Shapes and port numbers are LSB's own (`vendor/server/src/map/packets/s2c/
//! 0x056_mission.h`, `0x056_mission_other.{h,cpp}`, `0x056_mission_tvr.cpp`, port enums in
//! `vendor/server/src/map/enums/mission_log.h` + `quest_log.h`). Every body is 32 payload bytes,
//! a u16 Port at offset 32 and one pad word. LSB sends no counterpart: mission acceptance runs
//! through NPC/event dialogs only (C2s [V-negative]).

use thiserror::Error;

/// MISSION main port (`packet.Port = 0xFFFF` in `vendor/server/src/map/packets/s2c/0x056_mission.cpp`).
pub const PORT_MISSION_MAIN: u16 = 0xFFFF;
/// TVR port (`packet.Port = 0xFFFE` in `vendor/server/src/map/packets/s2c/0x056_mission_tvr.cpp`).
pub const PORT_TVR: u16 = 0xFFFE;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MissionError {
    #[error("MISSION frame too short: {0} bytes (need 34)")]
    TooShort(usize),
}

/// The main-port fields, verbatim per `vendor/server/src/map/packets/s2c/0x056_mission.h`.
/// CoP2 already carries `statusUpper << 16 | statusLower` as LSB built it; the addon nibbles are
/// `expansion_addon_t` (4-bit ACP/AMK/ASA) and the TalesBeginning bitfield shares that dword's
/// high half (`vendor/server/src/map/packets/s2c/0x056_mission.cpp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionMain {
    pub nation: u32,
    pub nation_mission: u32,
    pub rotz: u32,
    pub cop: u32,
    pub cop2: u32,
    pub addon_acp: u8,
    pub addon_amk: u8,
    pub addon_asa: u8,
    pub tales_beginning: u16,
    pub soa: u32,
    pub rov: u32,
}

/// One OTHER-port frame — `uint32 Data[8]` plus the port enum value that named the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionLogPort {
    pub port: u16,
    pub data: [u32; 8],
}

/// One decoded s2c 0x056 sub-frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionFrame {
    Main(MissionMain),
    Tvr(u32),
    Other(MissionLogPort),
}

pub fn parse_mission_frame(body: &[u8]) -> Result<MissionFrame, MissionError> {
    if body.len() < 34 {
        return Err(MissionError::TooShort(body.len()));
    }
    let dword = |i: usize| u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]);
    let port = u16::from_le_bytes([body[32], body[33]]);
    let data: [u32; 8] = std::array::from_fn(|i| dword(i * 4));
    Ok(match port {
        PORT_MISSION_MAIN => MissionFrame::Main(MissionMain {
            nation: data[0],
            nation_mission: data[1],
            rotz: data[2],
            cop: data[3],
            cop2: data[4],
            addon_acp: (data[5] & 0xF) as u8,
            addon_amk: ((data[5] >> 4) & 0xF) as u8,
            addon_asa: ((data[5] >> 8) & 0xF) as u8,
            tales_beginning: (data[5] >> 16) as u16,
            soa: data[6],
            rov: data[7],
        }),
        PORT_TVR => MissionFrame::Tvr(data[0]),
        _ => MissionFrame::Other(MissionLogPort { port, data }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(port: u16, dwords: [u32; 8]) -> Vec<u8> {
        let mut v = Vec::new();
        for d in dwords {
            v.extend_from_slice(&d.to_le_bytes());
        }
        v.extend_from_slice(&port.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        v
    }

    #[test]
    fn main_port_unpacks_the_nation_log_and_bitfield_dword() {
        // dword5 = addons ACP=1/AMK=2/ASA=3 in the low half, TalesBeginning high half.
        let b = body(
            PORT_MISSION_MAIN,
            [1, 7, 0, 0, 0x0002_0003, 0xBEEF_0032, 4, 5],
        );
        let f = parse_mission_frame(&b).expect("frame");
        let MissionFrame::Main(m) = f else {
            panic!("main port must decode Main, got {f:?}")
        };
        assert_eq!(
            (m.nation, m.nation_mission, m.rotz, m.cop2),
            (1, 7, 0, 0x0002_0003)
        );
        assert_eq!(
            (m.addon_acp, m.addon_amk, m.addon_asa),
            (2, 3, 0),
            "4-bit nibbles"
        );
        assert_eq!(m.tales_beginning, 0xBEEF);
        assert_eq!((m.soa, m.rov), (4, 5));
    }

    #[test]
    fn tvr_and_other_ports_pass_through_raw() {
        let tvr = parse_mission_frame(&body(PORT_TVR, [0x2A; 8])).expect("tvr");
        assert_eq!(tvr, MissionFrame::Tvr(0x2A));
        // QuestOffer::Sandoria (vendor/server/src/map/enums/quest_log.h) rides Other untouched.
        let other = parse_mission_frame(&body(0x0050, [9; 8])).expect("other");
        assert_eq!(
            other,
            MissionFrame::Other(MissionLogPort {
                port: 0x0050,
                data: [9; 8]
            })
        );
    }

    #[test]
    fn short_body_is_rejected() {
        assert_eq!(
            parse_mission_frame(&[0u8; 33]),
            Err(MissionError::TooShort(33))
        );
    }
}
