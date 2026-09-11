//! Choreography cues the event VM hands to its host.
//!
//! The dialog opcodes yield through [`crate::StepResult`]; the *staging* opcodes
//! (actor motion, screen fade, camera lock, music volume, event-hide, mount) do
//! not yield at all in retail — they call straight into the renderer and fall
//! through to the next instruction. A cue is that call, captured as data so a
//! host outside this crate can perform it, drained with
//! [`crate::EventVm::take_cues`].
//!
//! Cues are **event-scoped**: each one describes a change retail applies for the
//! duration of the running event, never a persisted flag.

use crate::vm::scene::EventPosition;

/// A baked `XiEvent::GetActorIndex` operand: the entity an opcode names
/// (research/XiEvents/Event VM Functions.md). Cues carry it unresolved because
/// only the host owns the entity table the reserved selectors index; this VM's
/// [`crate::EventVm`] would collapse "the local player" and "the event entity"
/// onto one target index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorLookup(pub u32);

/// Reserved lookup selectors: `…C0` local player, `…C1`–`…D1` party/alliance
/// slots, `…F1`–`…F5` party members, `…F8` the event entity, `…F9`/`…F0` the
/// local player again (research/XiEvents/Event VM Functions.md).
const LOOKUP_RESERVED: std::ops::RangeInclusive<u32> = 0x7FFF_FFC0..=0x7FFF_FFF9;
const LOOKUP_LOCAL_PLAYER_A: u32 = 0x7FFF_FFC0;
const LOOKUP_LOCAL_PLAYER_B: u32 = 0x7FFF_FFF0;
const LOOKUP_LOCAL_PLAYER_C: u32 = 0x7FFF_FFF9;
const LOOKUP_EVENT_ENTITY: u32 = 0x7FFF_FFF8;
/// A lookup with any high byte set is a literal entity server id, whose low bits
/// are the target index (same doc, default handler).
const LOOKUP_SERVER_ID_MASK: u32 = 0xFF00_0000;
const LOOKUP_TARGET_INDEX_MASK: u32 = 0x3FF;

impl ActorLookup {
    pub const LOCAL_PLAYER: Self = Self(LOOKUP_LOCAL_PLAYER_B);
    pub const EVENT_ENTITY: Self = Self(LOOKUP_EVENT_ENTITY);

    pub fn is_local_player(self) -> bool {
        matches!(
            self.0,
            LOOKUP_LOCAL_PLAYER_A | LOOKUP_LOCAL_PLAYER_B | LOOKUP_LOCAL_PLAYER_C
        )
    }

    /// True for the explicit event-entity selector and for the default
    /// handler's fallback (a non-reserved value with no high byte set).
    pub fn is_event_entity(self) -> bool {
        self.0 == LOOKUP_EVENT_ENTITY
            || (!LOOKUP_RESERVED.contains(&self.0) && self.0 & LOOKUP_SERVER_ID_MASK == 0)
    }

    /// The literal entity server id this lookup names, if it is one.
    pub fn server_id(self) -> Option<u32> {
        (!LOOKUP_RESERVED.contains(&self.0) && self.0 & LOOKUP_SERVER_ID_MASK != 0)
            .then_some(self.0)
    }

    /// Target index of the literal server id — `val & 0x3FF`.
    pub fn target_index(self) -> Option<u16> {
        self.server_id()
            .map(|id| (id & LOOKUP_TARGET_INDEX_MASK) as u16)
    }
}

/// A four-character scheduler/action key in file byte order — the operand is an
/// ASCII tag (`"fdo0"`, `"kue0"`), not a numeric id.
pub type FourCc = [u8; 4];

/// Base DAT file id opcode 0x45 adds its [`dat_id_helper`]-mapped work operand
/// to (research/XiEvents/OpCodes/0x0045.md, `FUNC_XiEvent_OpCode_0x0045` passing
/// 30704 to `CodeLOADEVENTSCHEDULER2`).
pub const SCHEDULER_DAT_ID_BASE: u32 = 30704;

/// Scheduler DAT holding the screen-fade pair (ROM/62/110.DAT).
pub const SCHEDULER_FADE_DAT_ID: u32 = 30904;

/// Screen fade-out scheduler tag, emitted by [`EventCue::Scheduler`].
pub const SCHEDULER_TAG_FADE_OUT: FourCc = *b"fdo0";
/// Screen fade-in scheduler tag, emitted by [`EventCue::Scheduler`].
pub const SCHEDULER_TAG_FADE_IN: FourCc = *b"fdi0";

/// [`EventCue::Scheduler::duration`] value meaning "play the DAT-authored
/// timing verbatim" — the overwhelming majority of authored call sites.
pub const SCHEDULER_DURATION_FROM_DAT: u16 = 0;

/// `GameStatus` values opcode 0x7E writes to the target's `StatusEvent`
/// (research/XIClient/src/XIClient/include/World/Actor/GameStatus.h; the case-to-value mapping is
/// research/XiEvents/OpCodes/0x007E.md).
pub const STATUS_EVENT_IDLE: u8 = 0;
pub const STATUS_EVENT_CHOCOBO: u8 = 5;
pub const STATUS_EVENT_MOUNT: u8 = 85;

/// Highest music-volume table index (`FUNC_YmMusicServer_Volume`'s first
/// argument indexes a volume table; it is not a percentage).
pub const MUSIC_VOLUME_MAX: u8 = 127;

/// `FUNC_DatIdHelper` (research/XiEvents/OpCodes/0x0045.md): the two folded
/// bands of the scheduler DAT id space.
pub fn dat_id_helper(param: i32) -> i32 {
    const HIGH_BAND: i32 = 600;
    const HIGH_BAND_OFFSET: i32 = 39643;
    const MID_BAND: i32 = 300;
    const MID_BAND_OFFSET: i32 = 25937;
    if param >= HIGH_BAND {
        return param.wrapping_add(HIGH_BAND_OFFSET);
    }
    if param >= MID_BAND {
        return param.wrapping_add(MID_BAND_OFFSET);
    }
    param
}

// The 0x5B event motion resource bands (research/XiEvents/OpCodes/0x005B.md,
// FUNC_XiSkeletonActor_ReadEventMotionRes call sites): the operand selects a
// base DAT id by which band it falls in.
const EVENT_MOTION_BAND_1: i32 = 512;
const EVENT_MOTION_BAND_2: i32 = 1024;
const EVENT_MOTION_BAND_3: i32 = 2048;
const EVENT_MOTION_BAND_4: i32 = 3072;
const EVENT_MOTION_BASE_0: i32 = 32104;
const EVENT_MOTION_BASE_1: i32 = 49135;
const EVENT_MOTION_BASE_2: i32 = 56345;
const EVENT_MOTION_BASE_3: i32 = 59739;
const EVENT_MOTION_BASE_4: i32 = 66339;

/// DAT id of the event motion resource a 0x5B operand names.
pub fn event_motion_dat_id(param: i32) -> u32 {
    let base = if param < EVENT_MOTION_BAND_1 {
        EVENT_MOTION_BASE_0
    } else if param < EVENT_MOTION_BAND_2 {
        EVENT_MOTION_BASE_1
    } else if param < EVENT_MOTION_BAND_3 {
        EVENT_MOTION_BASE_2
    } else if param < EVENT_MOTION_BAND_4 {
        EVENT_MOTION_BASE_3
    } else {
        EVENT_MOTION_BASE_4
    };
    param.wrapping_add(base) as u32
}

/// DAT id of the 0x66 Tpc motion package `param` names: the same base as the
/// 0x5B low band with no banding (research/cexi-docs/cutscene_authoring.md,
/// Dialogue + gestures; package 0 is the default humanoid talk set).
pub fn tpc_motion_dat_id(param: i32) -> u32 {
    param.wrapping_add(EVENT_MOTION_BASE_0) as u32
}

/// The 0x5B "no action" key: retail loads the motion resource and skips
/// SetAction when the key is zero or these bytes.
pub const NO_ACTION_KEY: FourCc = *b"xxxx";

/// One staging effect the running event asked for. Emitted in execution order.
/// Not `Eq`: [`EventCue::ActorMove`] carries a float speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventCue {
    /// 0x2C SCHEDULOR: play action `key` on `actor1`, with `actor2` as the
    /// action's partner (research/XiEvents/OpCodes/0x002C.md).
    ActorMotion {
        actor1: ActorLookup,
        actor2: ActorLookup,
        key: FourCc,
    },
    /// 0x45 LOADEVENTSCHEDULER2: run scheduler `tag` out of DAT file `dat_id`
    /// over the two actors (research/XiEvents/OpCodes/0x0045.md). `duration` is
    /// the authored override, [`SCHEDULER_DURATION_FROM_DAT`] for none.
    Scheduler {
        dat_id: u32,
        actor1: ActorLookup,
        actor2: ActorLookup,
        tag: FourCc,
        duration: u16,
    },
    /// 0x5B/0x66 LOADEXTSCHEDULER: load event motion resource `motion_dat_id`
    /// into `actor1`'s skeleton, then play action `key` on it with `actor2` as
    /// partner (research/XiEvents/OpCodes/0x005B.md). `tpc` marks the 0x66
    /// per-actor package form; the id is resolved in both cases.
    ExtScheduler {
        motion_dat_id: u32,
        tpc: bool,
        actor1: ActorLookup,
        actor2: ActorLookup,
        key: FourCc,
    },
    /// 0x4E EVENTHIDE: set/clear the target's event-hide render flag
    /// (research/XiEvents/OpCodes/0x004E.md).
    ActorHide { target: ActorLookup, hide: bool },
    /// 0x46 DEFCAMERA: take the camera (and the cutscene HUD) away from the
    /// player, or give it back (research/XiEvents/OpCodes/0x0046.md). Retail's
    /// restore reads saved global camera state, so the cue carries none.
    CameraLock { lock: bool },
    /// 0x5D MUSICVOLUME: ease the playing track to volume table index `volume`
    /// over `fade_frames` (research/XiEvents/OpCodes/0x005D.md).
    MusicVolume { volume: u8, fade_frames: u16 },
    /// 0x7E CHOCOBO/MOUNT: put the target on or off a mount by writing its
    /// `StatusEvent` (research/XiEvents/OpCodes/0x007E.md). `mount_id` is
    /// carried only by the non-chocobo mount cases.
    Mount {
        target: ActorLookup,
        status_event: u8,
        mount_id: Option<u16>,
    },
    /// 0x1F MOVE case 0 on a non-player actor: walk the event entity to `goal`
    /// at `speed` (research/XiEvents/OpCodes/0x001F.md). The host arms the
    /// arrival hold from its own distance and speed; the VM never measures it.
    ActorMove {
        actor: ActorLookup,
        goal: EventPosition,
        speed: f32,
    },
    /// 0x37 on a non-player actor: set the event entity's position (teleport,
    /// no hold; research/XiEvents/OpCodes/0x0037.md).
    ActorPlace {
        actor: ActorLookup,
        position: EventPosition,
    },
    /// 0x39 on a non-player actor: set the event entity's facing from its work
    /// operand (research/XiEvents/OpCodes/0x0039.md).
    ActorFace { actor: ActorLookup, heading: i32 },
    /// 0x4A DTURA, 0x79 lookat case 0 and the motion half of 0x1E
    /// look-and-talk: turn `actor` toward `target`
    /// (research/XiEvents/OpCodes/0x004A.md, 0x0079.md, 0x001E.md).
    ActorLookAt {
        actor: ActorLookup,
        target: ActorLookup,
    },
    /// 0x5E / 0x6B stop action: kill the current action on `actor` and return
    /// it to idle; `key` names the routine slot to clear when the operand is a
    /// nonzero tag (research/XiEvents/OpCodes/0x005E.md, 0x006B.md).
    ActorStopAction {
        actor: ActorLookup,
        key: Option<FourCc>,
    },
}

impl EventCue {
    pub(crate) fn resolve_event_actor(self, actor: ActorLookup) -> Self {
        let resolve = |target: ActorLookup| {
            if target.is_event_entity() {
                actor
            } else {
                target
            }
        };
        match self {
            Self::ActorMotion {
                actor1,
                actor2,
                key,
            } => Self::ActorMotion {
                actor1: resolve(actor1),
                actor2: resolve(actor2),
                key,
            },
            Self::Scheduler {
                dat_id,
                actor1,
                actor2,
                tag,
                duration,
            } => Self::Scheduler {
                dat_id,
                actor1: resolve(actor1),
                actor2: resolve(actor2),
                tag,
                duration,
            },
            Self::ExtScheduler {
                motion_dat_id,
                tpc,
                actor1,
                actor2,
                key,
            } => Self::ExtScheduler {
                motion_dat_id,
                tpc,
                actor1: resolve(actor1),
                actor2: resolve(actor2),
                key,
            },
            Self::ActorHide { target, hide } => Self::ActorHide {
                target: resolve(target),
                hide,
            },
            Self::Mount {
                target,
                status_event,
                mount_id,
            } => Self::Mount {
                target: resolve(target),
                status_event,
                mount_id,
            },
            Self::ActorMove { actor, goal, speed } => Self::ActorMove {
                actor: resolve(actor),
                goal,
                speed,
            },
            Self::ActorPlace { actor, position } => Self::ActorPlace {
                actor: resolve(actor),
                position,
            },
            Self::ActorFace { actor, heading } => Self::ActorFace {
                actor: resolve(actor),
                heading,
            },
            Self::ActorLookAt { actor, target } => Self::ActorLookAt {
                actor: resolve(actor),
                target: resolve(target),
            },
            Self::ActorStopAction { actor, key } => Self::ActorStopAction {
                actor: resolve(actor),
                key,
            },
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fade pair's DAT id is the one the 0x45 base plus its authored work
    /// operand resolves to; both consts must keep agreeing.
    #[test]
    fn fade_scheduler_dat_id_is_the_base_plus_its_authored_operand() {
        const FADE_WORK_OPERAND: i32 = 200;
        assert_eq!(
            SCHEDULER_DAT_ID_BASE as i32 + dat_id_helper(FADE_WORK_OPERAND),
            SCHEDULER_FADE_DAT_ID as i32
        );
    }

    #[test]
    fn fade_tags_are_the_authored_ascii_fourccs() {
        assert_eq!(&SCHEDULER_TAG_FADE_OUT, b"fdo0");
        assert_eq!(&SCHEDULER_TAG_FADE_IN, b"fdi0");
        for tag in [SCHEDULER_TAG_FADE_OUT, SCHEDULER_TAG_FADE_IN] {
            assert!(tag.iter().all(|b| (0x20..0x7F).contains(b)), "{tag:?}");
        }
    }

    #[test]
    fn dat_id_helper_folds_at_its_two_band_edges() {
        assert_eq!(dat_id_helper(0), 0);
        assert_eq!(dat_id_helper(299), 299);
        assert_eq!(dat_id_helper(300), 300 + 25937);
        assert_eq!(dat_id_helper(599), 599 + 25937);
        assert_eq!(dat_id_helper(600), 600 + 39643);
    }

    #[test]
    fn event_motion_dat_id_picks_its_base_at_each_band_edge() {
        // Each band edge lands on the next base exactly once; the value just
        // below an edge stays in the current band.
        assert_eq!(event_motion_dat_id(0), 32104);
        assert_eq!(event_motion_dat_id(511), 32104 + 511);
        assert_eq!(event_motion_dat_id(512), 49135 + 512);
        assert_eq!(event_motion_dat_id(1023), 49135 + 1023);
        assert_eq!(event_motion_dat_id(1024), 56345 + 1024);
        assert_eq!(event_motion_dat_id(2047), 56345 + 2047);
        assert_eq!(event_motion_dat_id(2048), 59739 + 2048);
        assert_eq!(event_motion_dat_id(3071), 59739 + 3071);
        assert_eq!(event_motion_dat_id(3072), 66339 + 3072);
    }

    #[test]
    fn actor_lookup_separates_the_player_the_event_entity_and_server_ids() {
        assert!(ActorLookup::LOCAL_PLAYER.is_local_player());
        assert!(!ActorLookup::LOCAL_PLAYER.is_event_entity());
        assert!(ActorLookup(0x7FFF_FFC0).is_local_player());
        assert!(ActorLookup(0x7FFF_FFF9).is_local_player());

        assert!(ActorLookup::EVENT_ENTITY.is_event_entity());
        assert!(!ActorLookup::EVENT_ENTITY.is_local_player());
        assert_eq!(ActorLookup::EVENT_ENTITY.server_id(), None);

        // The chocobo renter in Southern San d'Oria's rental cutscene.
        let npc = ActorLookup(0x010E_6032);
        assert_eq!(npc.server_id(), Some(0x010E_6032));
        assert_eq!(npc.target_index(), Some(0x032));
        assert!(!npc.is_local_player() && !npc.is_event_entity());

        // No high byte and not reserved: the default handler's fallback.
        assert!(ActorLookup(0x0000_0042).is_event_entity());
    }
}
