use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bevy::prelude::Resource;
use kuluu_render::SceneSource;
use kuluu_snapshot as wire;
use tokio::runtime::Handle as RtHandle;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use kuluu_session::state::{AgentEvent, EntityChanges, SessionState};
use kuluu_session::wire_translate::{entity_to_wire, event_to_viewer_event, state_to_snapshot};

// The session watch signals per folded packet event — far above frame rate in a
// crowd — so the off-main-thread translator caps itself near the 120 Hz display
// ceiling. On slower displays this allows up to ~2x the old once-per-frame
// translate rate (and its watch read-lock pressure on the session folder);
// accepted because the work left the render thread, and the cap still bounds
// folder contention (audit of kuluu-4mef).
const TRANSLATE_MIN_PERIOD: Duration = Duration::from_millis(8);

enum TranslatedFrame {
    Snapshot(Box<wire::SceneSnapshot>),
    Delta(wire::SceneDelta),
}

struct ReadyFrame {
    frame: TranslatedFrame,
    rebuild_us: u64,
    entity_count: usize,
}

// Mutex<Option> rather than a watch: poll_snapshot/drain_deltas take ownership
// of the frame, so the render thread never clones; overwriting the single slot
// keeps only the newest, which makes out-of-order delivery impossible.
type SnapshotMailbox = Arc<Mutex<Option<ReadyFrame>>>;

/// Full-snapshot triggers tracked between cycles: first frame ever, zone
/// generation bump (zone change), self char id change (new session or
/// character switch), or a new reconnect replay marker. Everything else is an
/// O(changed) entity delta built from the folder's drained batches.
#[derive(Default)]
struct ResyncTracker {
    primed: bool,
    last_zone_generation: u64,
    last_char_id: Option<u32>,
    last_reconnect_at_ms: u64,
}

impl ResyncTracker {
    fn needs_full_snapshot(&mut self, s: &SessionState) -> bool {
        let reconnect_at = s.last_reconnect.as_ref().map(|r| r.at_unix_ms).unwrap_or(0);
        let full = !self.primed
            || s.zone_generation != self.last_zone_generation
            || s.char_id != self.last_char_id
            || reconnect_at != self.last_reconnect_at_ms;
        if full {
            self.primed = true;
            self.last_zone_generation = s.zone_generation;
            self.last_char_id = s.char_id;
            self.last_reconnect_at_ms = reconnect_at;
        }
        full
    }
}

/// One translate cycle: merge the change batches drained since the last
/// cycle, then emit an O(changed) entity delta when the folder stamped ids
/// this window and nothing else changed; otherwise a full snapshot (resync,
/// non-entity state, or a watch change no batch explains — e.g. a self-look
/// latch that mutated no record). Every watch change still yields a frame.
fn translate_frame(
    state_rx: &mut watch::Receiver<SessionState>,
    changes_rx: &mut mpsc::UnboundedReceiver<EntityChanges>,
    resync: &mut ResyncTracker,
) -> ReadyFrame {
    let started = std::time::Instant::now();

    // Merge every batch drained since the previous cycle. Removals win over
    // upserts: an upsert-then-remove inside the window nets to a removal.
    let mut upserts: HashSet<u32> = HashSet::new();
    let mut removals: HashSet<u32> = HashSet::new();
    let mut other_changed = false;
    while let Ok(batch) = changes_rx.try_recv() {
        upserts.extend(batch.upserts);
        removals.extend(batch.removals);
        other_changed |= batch.other_changed;
    }
    upserts.retain(|id| !removals.contains(id));

    let (frame, entity_count) = {
        let guard = state_rx.borrow_and_update();
        let entity_count = guard.entities.len();
        if resync.needs_full_snapshot(&guard) || other_changed {
            // A full snapshot is authoritative for everything the drained
            // batches covered, so those ids need no separate emission.
            (
                TranslatedFrame::Snapshot(Box::new(state_to_snapshot(&guard))),
                entity_count,
            )
        } else if upserts.is_empty() && removals.is_empty() {
            // The watch changed but no batch explains it: emit a full snapshot
            // rather than risk dropping the change. Rare in practice.
            (
                TranslatedFrame::Snapshot(Box::new(state_to_snapshot(&guard))),
                entity_count,
            )
        } else {
            let mut delta = wire::SceneDelta::default();
            for id in &upserts {
                if let Some(idx) = guard.entity_index.get(id).copied() {
                    delta
                        .entities_upserted
                        .push(entity_to_wire(&guard.entities[idx]));
                }
            }
            delta.entities_removed.extend(removals);
            (TranslatedFrame::Delta(delta), entity_count)
        }
    };

    ReadyFrame {
        frame,
        rebuild_us: started.elapsed().as_micros() as u64,
        entity_count,
    }
}

async fn run_translator(
    mut state_rx: watch::Receiver<SessionState>,
    mailbox: SnapshotMailbox,
    mut changes_rx: mpsc::UnboundedReceiver<EntityChanges>,
) {
    let mut resync = ResyncTracker::default();
    loop {
        let started = tokio::time::Instant::now();
        let ready = translate_frame(&mut state_rx, &mut changes_rx, &mut resync);
        // Take the overwritten frame out before dropping it: its Vec frees
        // must not run inside the lock the render thread polls every frame.
        let prev = mailbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(ready);
        drop(prev);
        tokio::time::sleep_until(started + TRANSLATE_MIN_PERIOD).await;
        if state_rx.changed().await.is_err() {
            break;
        }
    }
}

#[derive(Resource)]
pub struct NativeSource {
    mailbox: SnapshotMailbox,
    translator: JoinHandle<()>,
    event_rx: broadcast::Receiver<AgentEvent>,
    warned_translator_gone: bool,
    /// Newest entity-delta frame not yet handed to the render thread (see
    /// `poll_snapshot`): stashed so ingest applies it after the snapshot slot.
    pending_delta: Option<wire::SceneDelta>,

    pub last_rebuild_us: u64,
    pub last_entity_count: usize,
    pub rebuilds_total: u64,
}

impl NativeSource {
    pub fn new(
        runtime: &RtHandle,
        state_rx: watch::Receiver<SessionState>,
        event_rx: broadcast::Receiver<AgentEvent>,
        entity_changes_rx: mpsc::UnboundedReceiver<EntityChanges>,
    ) -> Self {
        let mailbox = SnapshotMailbox::default();
        let translator = runtime.spawn(run_translator(
            state_rx,
            Arc::clone(&mailbox),
            entity_changes_rx,
        ));
        Self {
            mailbox,
            translator,
            event_rx,
            warned_translator_gone: false,
            pending_delta: None,
            last_rebuild_us: 0,
            last_entity_count: 0,
            rebuilds_total: 0,
        }
    }
}

impl Drop for NativeSource {
    fn drop(&mut self) {
        self.translator.abort();
    }
}

impl SceneSource for NativeSource {
    /// Takes the newest mailbox frame. A full snapshot is returned here; an
    /// entity delta is stashed in `pending_delta` and served by
    /// [`Self::drain_deltas`] so ingest applies it after the snapshot slot.
    fn poll_snapshot(&mut self) -> Option<Box<wire::SceneSnapshot>> {
        let ready = self
            .mailbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(ready) = ready else {
            // A dead translator (panic in state_to_snapshot, or session end)
            // otherwise freezes the viewer on the last scene with no diagnostic.
            if !self.warned_translator_gone && self.translator.is_finished() {
                self.warned_translator_gone = true;
                bevy::log::warn!(
                    "snapshot translator exited; no further scene updates until reconnect"
                );
            }
            return None;
        };
        self.last_rebuild_us = ready.rebuild_us;
        self.last_entity_count = ready.entity_count;
        match ready.frame {
            TranslatedFrame::Snapshot(snap) => {
                self.rebuilds_total = self.rebuilds_total.wrapping_add(1);
                Some(snap)
            }
            TranslatedFrame::Delta(delta) => {
                // The translator emits at most one frame per cycle, so a
                // stashed delta can only be replaced by the next frame.
                self.pending_delta = Some(delta);
                None
            }
        }
    }

    fn drain_deltas(&mut self) -> Vec<wire::SceneDelta> {
        match self.pending_delta.take() {
            Some(delta) => vec![delta],
            None => Vec::new(),
        }
    }

    fn drain_events(&mut self) -> Vec<wire::ViewerEvent> {
        let mut out = Vec::new();
        loop {
            match self.event_rx.try_recv() {
                Ok(ev) => {
                    if let Some(translated) = event_to_viewer_event(ev) {
                        out.push(translated);
                    }
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => break,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuluu_session::state::{
        ChatChannel, ChatLine, ContainerInfo, Entity, EntityKind, EquippedRef, ItemSlot,
        PartyMember, ReactorGoalSnapshot, ReconnectInfo, Stage, Vec3,
    };

    fn populated_state() -> SessionState {
        let mut s = SessionState {
            stage: Stage::InZone,
            account_id: Some(1),
            char_id: Some(0x1000_0001),
            character: Some("Sylvie".into()),
            zone_id: Some(230),
            current_goal: Some(ReactorGoalSnapshot::Engaged {
                target_id: 0x1000_0102,
                attack_issued: true,
            }),
            last_reconnect: Some(ReconnectInfo {
                downtime_ms: 800,
                at_unix_ms: 1_700_000_002_000,
            }),
            current_weather: Some(4),
            status_icons: vec![13, 33],
            status_icon_expiries: vec![600, 0],
            ability_recasts: vec![(16, 45), (0, 0)],
            spells_known: vec![1, 2, 17],
            job_abilities_known: vec![16],
            weaponskills_known: vec![32],
            key_items: vec![342],
            key_items_seen: vec![342],
            ..Default::default()
        };

        s.entities = vec![
            Entity {
                id: 0x1000_0001,
                act_index: 0x001,
                kind: EntityKind::Pc,
                name: Some("Sylvie".into()),
                pos: Vec3 {
                    x: 12.5,
                    y: -1.0,
                    z: 34.0,
                },
                heading: 96,
                hp_pct: Some(100),
                bt_target_id: 0,
                name_vis: None,
                face_target: 0x102,
                claim_id: 0,
                speed: 40,
                speed_base: 40,
                look: Some(ffxi_proto::decode::LookData::Equipped {
                    face: 2,
                    race: 1,
                    head: 0x1000,
                    body: 0x2001,
                    hands: 0x3002,
                    legs: 0x4003,
                    feet: 0x5004,
                    main: 0x6005,
                    sub: 0,
                    ranged: 0,
                }),
                npc_state: None,
                status: 1,
                char_flags: Default::default(),
                mount_id: None,
            },
            Entity {
                id: 0x1000_0102,
                act_index: 0x102,
                kind: EntityKind::Mob,
                name: Some("Wild Rabbit".into()),
                pos: Vec3 {
                    x: 15.0,
                    y: -1.2,
                    z: 30.0,
                },
                heading: 12,
                hp_pct: Some(72),
                bt_target_id: 0x1000_0001,
                name_vis: None,
                face_target: 0x001,
                claim_id: 0x1000_0001,
                speed: 40,
                speed_base: 40,
                look: Some(ffxi_proto::decode::LookData::Standard { modelid: 0x0119 }),
                npc_state: Some(ffxi_proto::decode::NpcState {
                    animation: 1,
                    animationsub: 0,
                    status: 1,
                }),
                status: 1,
                char_flags: Default::default(),
                mount_id: None,
            },
        ];

        s.party = vec![PartyMember {
            id: 0x1000_0001,
            act_index: 0x001,
            name: Some("Sylvie".into()),
            hp: 512,
            mp: 128,
            tp: 1000,
            hp_pct: 100,
            mp_pct: 90,
            zone_no: 230,
            main_job: 1,
            main_job_lv: 12,
            sub_job: 5,
            sub_job_lv: 6,
            is_party_leader: true,
            is_alliance_leader: false,
            in_mog_house: false,
            party_no: 0,
        }];

        s.chat = (0..8)
            .map(|i| ChatLine {
                spans: Vec::new(),
                channel: if i % 2 == 0 {
                    ChatChannel::Say
                } else {
                    ChatChannel::Battle
                },
                sender: format!("Speaker{i}"),
                text: format!("line {i}"),
                server_ts: 1000 + i,
            })
            .collect();

        let mut inv0 = ContainerInfo {
            capacity: 30,
            slots: Vec::new(),
        };
        inv0.slots.push(ItemSlot {
            index: 3,
            item_no: 16448,
            quantity: 1,
            locked: true,
            price: 0,
            charges_remaining: None,
            next_use_vana_ts: None,
        });
        s.inventory.containers.insert(0, inv0);
        s.equipment[0] = Some(EquippedRef {
            container: 0,
            container_index: 3,
        });

        s
    }

    fn normalized(mut snap: wire::SceneSnapshot) -> serde_json::Value {
        snap.producer_monotonic_ms = 0;
        serde_json::to_value(&snap).expect("SceneSnapshot serializes")
    }

    #[test]
    fn translator_task_matches_synchronous_translate() {
        let state = populated_state();
        let expected = state_to_snapshot(&state);
        assert!(!expected.entities.is_empty() && !expected.chat.is_empty());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let (state_tx, state_rx) = watch::channel(state);
        let mailbox = SnapshotMailbox::default();
        let (_changes_tx, changes_rx) = mpsc::unbounded_channel::<EntityChanges>();

        let translated = rt.block_on(async {
            let task = tokio::spawn(run_translator(state_rx, Arc::clone(&mailbox), changes_rx));
            let got = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(t) = mailbox
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take()
                    {
                        break t;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("translator publishes within timeout");
            task.abort();
            got
        });
        drop(state_tx);

        let snap = match translated.frame {
            TranslatedFrame::Snapshot(snap) => *snap,
            TranslatedFrame::Delta(_) => panic!("first frame must be a full snapshot"),
        };
        assert_eq!(snap.entities.len(), expected.entities.len());
        assert_eq!(normalized(snap), normalized(expected));
    }

    #[test]
    fn final_state_is_delivered_after_sender_drops() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let (state_tx, state_rx) = watch::channel(populated_state());
        let mailbox = SnapshotMailbox::default();
        let (_changes_tx, changes_rx) = mpsc::unbounded_channel::<EntityChanges>();

        rt.block_on(async {
            let task = tokio::spawn(run_translator(state_rx, Arc::clone(&mailbox), changes_rx));
            state_tx.send_modify(|s| s.zone_id = Some(999));
            drop(state_tx);
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .expect("translator exits after sender drop")
                .expect("translator did not panic");
        });

        let last = mailbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .expect("final snapshot published");
        // The direct send_modify bypasses the folder, so no batch explains the
        // watch change: the defensive full-snapshot path must deliver it.
        let snap = match last.frame {
            TranslatedFrame::Snapshot(snap) => *snap,
            TranslatedFrame::Delta(_) => panic!("unexplained watch change must be a full snapshot"),
        };
        assert_eq!(snap.zone_id, Some(999), "last unseen state delivered");
    }

    fn mob_entity(id: u32) -> Entity {
        Entity {
            id,
            act_index: 1,
            kind: EntityKind::Mob,
            name: Some("Antlion".into()),
            pos: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            heading: 0,
            hp_pct: Some(50),
            bt_target_id: 0,
            face_target: 0,
            name_vis: None,
            claim_id: 0,
            speed: 0,
            speed_base: 0,
            look: None,
            npc_state: None,
            status: 1,
            char_flags: Default::default(),
            mount_id: None,
        }
    }

    /// Simulates one folder fold: apply the event to the watched state and
    /// drain its pending sets into a batch on the changes channel — exactly
    /// what run_event_folder does around send_if_modified.
    fn fold_and_drain(
        state_tx: &watch::Sender<SessionState>,
        changes_tx: &mpsc::UnboundedSender<EntityChanges>,
        event: AgentEvent,
    ) {
        let changed = state_tx.send_modify(|s| s.apply_event(&event));
        if changed {
            let (upserts, removals) = state_tx.borrow_mut().take_pending_entities();
            if !upserts.is_empty() || !removals.is_empty() {
                changes_tx
                    .send(EntityChanges {
                        upserts,
                        removals,
                        other_changed: false,
                    })
                    .expect("receiver alive");
            }
        }
    }

    #[test]
    fn translate_frame_emits_entity_delta_for_stamped_changes() {
        let mut s = SessionState::default();
        s.apply_event(&AgentEvent::Connected {
            account_id: 1,
            char_id: 7,
            character: "Cow".into(),
            zone_id: 103,
        });
        let (state_tx, state_rx) = watch::channel(s);
        let (changes_tx, changes_rx) = mpsc::unbounded_channel();
        let mut resync = ResyncTracker::default();

        // First cycle is always a full snapshot.
        let first = translate_frame(&mut state_rx, &mut changes_rx, &mut resync);
        assert!(matches!(first.frame, TranslatedFrame::Snapshot(_)));

        fold_and_drain(
            &state_tx,
            &changes_tx,
            AgentEvent::EntityUpserted {
                entity: mob_entity(42),
                pos_present: true,
            },
        );
        let second = translate_frame(&mut state_rx, &mut changes_rx, &mut resync);
        match second.frame {
            TranslatedFrame::Delta(delta) => {
                assert_eq!(delta.entities_upserted.len(), 1);
                assert_eq!(delta.entities_upserted[0].id, 42);
                assert!(delta.entities_removed.is_empty());
            }
            TranslatedFrame::Snapshot(_) => panic!("steady-state change must be a delta"),
        }

        // A second upsert of the same entity stays a one-id delta.
        let mut moved = mob_entity(42);
        moved.pos.z += 5.0;
        fold_and_drain(
            &state_tx,
            &changes_tx,
            AgentEvent::EntityUpserted {
                entity: moved,
                pos_present: true,
            },
        );
        match translate_frame(&mut state_rx, &mut changes_rx, &mut resync).frame {
            TranslatedFrame::Delta(delta) => assert_eq!(delta.entities_upserted.len(), 1),
            TranslatedFrame::Snapshot(_) => panic!("steady-state change must be a delta"),
        }
    }

    #[test]
    fn translate_frame_removal_wins_over_same_window_upsert() {
        let mut s = SessionState::default();
        s.apply_event(&AgentEvent::Connected {
            account_id: 1,
            char_id: 7,
            character: "Cow".into(),
            zone_id: 103,
        });
        let (state_tx, state_rx) = watch::channel(s);
        let (changes_tx, changes_rx) = mpsc::unbounded_channel();
        let mut resync = ResyncTracker::default();
        translate_frame(&mut state_rx, &mut changes_rx, &mut resync); // prime

        fold_and_drain(
            &state_tx,
            &changes_tx,
            AgentEvent::EntityUpserted {
                entity: mob_entity(9),
                pos_present: true,
            },
        );
        fold_and_drain(&state_tx, &changes_tx, AgentEvent::EntityRemoved { id: 9 });

        match translate_frame(&mut state_rx, &mut changes_rx, &mut resync).frame {
            TranslatedFrame::Delta(delta) => {
                assert!(
                    delta.entities_upserted.is_empty(),
                    "voided upsert must not be emitted"
                );
                assert_eq!(delta.entities_removed, vec![9]);
            }
            TranslatedFrame::Snapshot(_) => panic!("steady-state change must be a delta"),
        }
    }

    #[test]
    fn translate_frame_other_changed_forces_full_snapshot() {
        let mut s = SessionState::default();
        s.apply_event(&AgentEvent::Connected {
            account_id: 1,
            char_id: 7,
            character: "Cow".into(),
            zone_id: 103,
        });
        let (state_tx, state_rx) = watch::channel(s);
        let (changes_tx, changes_rx) = mpsc::unbounded_channel();
        let mut resync = ResyncTracker::default();
        translate_frame(&mut state_rx, &mut changes_rx, &mut resync); // prime

        // A chat line is not an entity change: the batch flags other_changed.
        state_tx.send_modify(|s| {
            s.apply_event(&AgentEvent::ChatLine {
                line: ChatLine {
                    spans: Vec::new(),
                    channel: ChatChannel::Say,
                    sender: "T".into(),
                    text: "hi".into(),
                    server_ts: 1,
                },
            });
        });
        let (upserts, removals) = state_tx.borrow_mut().take_pending_entities();
        changes_tx
            .send(EntityChanges {
                upserts,
                removals,
                other_changed: true,
            })
            .expect("receiver alive");

        match translate_frame(&mut state_rx, &mut changes_rx, &mut resync).frame {
            TranslatedFrame::Snapshot(snap) => assert_eq!(snap.chat.len(), 1),
            TranslatedFrame::Delta(_) => panic!("non-entity change must be a full snapshot"),
        }
    }

    /// The 0x5D master volume is fanned across the music slots, so the count
    /// here and the one the renderer indexes have to be the same number.
    /// Lives here (not in kuluu-session) because it is the one assertion that
    /// needs both sides of the session/renderer boundary in scope.
    #[test]
    fn the_music_slot_count_matches_the_renderer_mixer() {
        assert_eq!(
            kuluu_session::state::MUSIC_SLOT_COUNT as usize,
            kuluu_render::audio::SLOT_COUNT
        );
    }
}
