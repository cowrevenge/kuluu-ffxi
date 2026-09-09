use bevy::prelude::*;
use kuluu_snapshot::ViewerEvent;

use crate::snapshot::{EventLog, SceneState};

#[derive(Resource, Default)]
pub struct EngagementChatCursor {
    pos: usize,
}

pub fn report_engagement_events_system(
    events: Res<EventLog>,
    mut cursor: ResMut<EngagementChatCursor>,
    scene_state: Res<SceneState>,
    mut toasts: MessageWriter<crate::snapshot::ToastEvent>,
) {
    let len = events.recent.len();

    if cursor.pos > len {
        cursor.pos = 0;
    }
    for i in cursor.pos..len {
        let line = match &events.recent[i] {
            ViewerEvent::ZoneChanged { from, to } => Some(match (from, *to) {
                (_, kuluu_snapshot::ZONE_UNKNOWN) => "→ Leaving the zone".to_string(),
                (Some(prev), to) => format!("→ Zone change: 0x{prev:04X} → 0x{to:04X}"),
                (None, to) => format!("→ Zone entered: 0x{to:04X}"),
            }),
            ViewerEvent::EngagedBy { entity_id } => {
                let name = scene_state
                    .snapshot
                    .entities
                    .iter()
                    .find(|e| e.id == *entity_id)
                    .and_then(|e| e.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("0x{:08X}", entity_id));
                Some(format!("⚔ Engaged by {} (0x{:08X})", name, entity_id))
            }
            ViewerEvent::LowHp { pct } => Some(format!("❤ Low HP: self at {}%", pct)),
            _ => None,
        };
        if let Some(text) = line {
            toasts.write(crate::snapshot::ToastEvent::debug(text));
        }
    }
    cursor.pos = len;
}

#[derive(Resource, Default)]
pub struct SpeedSuppressionLatch {
    prev_suppressed: Option<bool>,
}

pub fn report_speed_state_system(
    mut latch: ResMut<SpeedSuppressionLatch>,
    scene_state: Res<SceneState>,
    mut toasts: MessageWriter<crate::snapshot::ToastEvent>,
) {
    let snap = &scene_state.snapshot;
    let Some(self_id) = snap.self_char_id else {
        return;
    };
    let Some(self_entity) = snap.entities.iter().find(|e| e.id == self_id) else {
        return;
    };
    let speed = self_entity.speed;
    let speed_base = self_entity.speed_base;
    let suppressed_now = speed == 0;

    let line = match latch.prev_suppressed {
        None => None,
        Some(prev) if prev == suppressed_now => None,
        Some(_prev) => Some(if suppressed_now {
            "✋ Speed suppressed (Bind/Stun/Sleep?)".to_string()
        } else {
            format!("✋ Speed restored ({}/{})", speed, speed_base)
        }),
    };
    latch.prev_suppressed = Some(suppressed_now);

    if let Some(text) = line {
        toasts.write(crate::snapshot::ToastEvent::debug(text));
    }
}

pub struct DebugChatPlugin;

impl Plugin for DebugChatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EngagementChatCursor>()
            .init_resource::<SpeedSuppressionLatch>()
            .add_systems(
                Update,
                (report_engagement_events_system, report_speed_state_system),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{chat_line_visible, drain_toast_events, ToastEvent};
    use kuluu_snapshot::{Entity as SnapEntity, EntityKind, SceneSnapshot, Vec3};

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<ToastEvent>()
            .init_resource::<EventLog>()
            .init_resource::<SceneState>()
            .init_resource::<EngagementChatCursor>()
            .init_resource::<SpeedSuppressionLatch>()
            .add_systems(
                Update,
                (
                    report_engagement_events_system,
                    report_speed_state_system,
                    drain_toast_events,
                )
                    .chain(),
            );
        app
    }

    fn self_entity(id: u32, speed: u8) -> SnapEntity {
        SnapEntity {
            id,
            act_index: 1,
            kind: EntityKind::Pc,
            name: Some("Self".into()),
            pos: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            heading: 0,
            hp_pct: Some(100),
            bt_target_id: 0,
            face_target: 0,
            name_vis: None,
            claim_id: 0,
            speed,
            speed_base: 40,
            look: None,
            animation: 0,
            animationsub: 0,
            mount: None,
            status: 0,
            char_flags: Default::default(),
            monstrosity: false,
        }
    }

    fn emitted(app: &App) -> Vec<&kuluu_snapshot::ChatLine> {
        app.world()
            .resource::<SceneState>()
            .local_toasts
            .iter()
            .collect()
    }

    #[test]
    fn engagement_lines_are_suppressed_without_dev_hud() {
        let mut app = app();
        {
            let mut events = app.world_mut().resource_mut::<EventLog>();
            events.push(ViewerEvent::EngagedBy { entity_id: 0x1234 });
            events.push(ViewerEvent::LowHp { pct: 12 });
            events.push(ViewerEvent::ZoneChanged {
                from: Some(0x0100),
                to: 0x0101,
            });
        }
        app.update();

        let lines = emitted(&app);
        assert_eq!(lines.len(), 3, "got {lines:?}");
        for line in lines {
            assert!(
                !chat_line_visible(line.channel, false),
                "{:?} must stay out of player chat",
                line.text
            );
            assert!(chat_line_visible(line.channel, true));
        }
    }

    #[test]
    fn speed_transition_lines_are_suppressed_without_dev_hud() {
        let mut app = app();
        let mut snap = SceneSnapshot {
            self_char_id: Some(7),
            ..Default::default()
        };
        snap.entities.push(self_entity(7, 40));
        app.world_mut().resource_mut::<SceneState>().snapshot = snap;
        app.update();
        assert!(emitted(&app).is_empty(), "first frame only latches");

        {
            let mut state = app.world_mut().resource_mut::<SceneState>();
            state.snapshot.entities[0].speed = 0;
        }
        app.update();

        let lines = emitted(&app);
        assert_eq!(lines.len(), 1, "got {lines:?}");
        assert!(!chat_line_visible(lines[0].channel, false));
        assert!(chat_line_visible(lines[0].channel, true));
    }
}
