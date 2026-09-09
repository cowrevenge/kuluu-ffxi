use ffxi_event::vm::scene::{EventPosition, SceneAction, EVENT_COORD_UNITS, EVENT_HEADING_UNITS};
use tokio::sync::broadcast;

use super::codec::{build_subpacket_event_position, build_subpacket_pos};
use crate::event_dialog::DialogSession;
use crate::state::{AgentEvent, Position, Vec3};

const WIRE_HEADING_UNITS: f32 = (u8::MAX as u16 + 1) as f32;

pub(super) fn event_position(position: Position) -> EventPosition {
    EventPosition {
        x: (position.pos.x * EVENT_COORD_UNITS) as i32,
        y: (position.pos.z * EVENT_COORD_UNITS) as i32,
        z: (position.pos.y * EVENT_COORD_UNITS) as i32,
        heading: (f32::from(position.heading) * EVENT_HEADING_UNITS / WIRE_HEADING_UNITS) as i32,
    }
}

pub(super) fn session_position(position: EventPosition, previous: Position) -> Position {
    Position {
        pos: Vec3 {
            x: position.x as f32 / EVENT_COORD_UNITS,
            y: position.z as f32 / EVENT_COORD_UNITS,
            z: position.y as f32 / EVENT_COORD_UNITS,
        },
        heading: (position.heading as f32 / EVENT_HEADING_UNITS * WIRE_HEADING_UNITS) as i32 as u8,
        ..previous
    }
}

pub(super) fn drain_scene_actions(
    dialog: &mut DialogSession,
    identity: (u32, u16, u16),
    zone: u16,
    sequence: &mut u16,
    position: &mut Position,
    events: &broadcast::Sender<AgentEvent>,
) -> Vec<u8> {
    let mut payload = Vec::new();
    for action in dialog.take_scene_actions() {
        match action {
            SceneAction::PlayerPosition(next) => {
                *position = session_position(next, *position);
                let _ = events.send(AgentEvent::PositionChanged { pos: *position });
                // vendor/server/src/map/packets/c2s/0x015_pos.cpp GP_CLI_COMMAND_POS::process
                // accepts scripted walking in-event; the final POS must precede EVENTEND.
                payload.extend(build_subpacket_pos(
                    *sequence,
                    position.pos.x,
                    position.pos.y,
                    position.pos.z,
                    position.heading,
                    0,
                ));
            }
            SceneAction::PositionUpdate {
                position: next,
                end_para,
            } => {
                payload.extend(build_subpacket_event_position(
                    *sequence,
                    identity,
                    zone,
                    end_para,
                    session_position(next, *position),
                ));
            }
        }
        *sequence = sequence.wrapping_add(1);
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_coordinates_and_heading_roundtrip_through_session_axes() {
        let authored = EventPosition {
            x: 33_762,
            y: -2_558,
            z: -31_432,
            heading: 3072,
        };
        let converted = session_position(authored, Position::default());
        assert!((converted.pos.x - 33.762).abs() < 0.001);
        assert!((converted.pos.y + 31.432).abs() < 0.001);
        assert!((converted.pos.z + 2.558).abs() < 0.001);
        assert_eq!(converted.heading, 192);
        assert_eq!(event_position(converted), authored);
    }

    #[test]
    fn position_update_packet_matches_lsb_eventendxzy_layout() {
        let position = Position {
            pos: Vec3 {
                x: 33.762,
                y: -31.432,
                z: -2.558,
            },
            heading: 192,
            ..Position::default()
        };
        let packet = build_subpacket_event_position(19, (17_793_078, 54, 221), 248, 7, position);
        assert_eq!(packet.len(), 32);
        assert_eq!(u16::from_le_bytes(packet[2..4].try_into().unwrap()), 19);
        for (offset, expected) in [
            (4, position.pos.x),
            (8, position.pos.z),
            (12, position.pos.y),
        ] {
            assert_eq!(
                f32::from_le_bytes(packet[offset..offset + 4].try_into().unwrap()),
                expected
            );
        }
        assert_eq!(
            u32::from_le_bytes(packet[16..20].try_into().unwrap()),
            17_793_078
        );
        assert_eq!(u32::from_le_bytes(packet[20..24].try_into().unwrap()), 7);
        assert_eq!(u16::from_le_bytes(packet[24..26].try_into().unwrap()), 248);
        assert_eq!(u16::from_le_bytes(packet[26..28].try_into().unwrap()), 221);
        assert_eq!(u16::from_le_bytes(packet[28..30].try_into().unwrap()), 54);
        assert_eq!(
            packet[30],
            ffxi_proto::map::event_position_wire::UPDATE_PENDING as u8
        );
        assert_eq!(packet[31], 192);
    }
}
