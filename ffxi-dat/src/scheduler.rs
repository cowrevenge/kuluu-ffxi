use crate::{DatError, Result};

// The effect list of a routine whose control-flow section (section 1) is empty. Kept as the
// fallback for chunks whose section table reads implausibly — see `effect_section_start`.
pub const SCHEDULER_HEADER_LEN: usize = 64;

// research/xim EffectRoutineParser.kt:41-57 — after four zero dwords the routine header holds
// three u32 section offsets (section 1 = control-flow setup, 2 = the effect list, 3 = trailer),
// each measured from the CHUNK header, which begins `CHUNK_HEADER_LEN` before `body`.
const SECTION_TABLE_OFFSET: usize = 0x10;
const SECTION2_SLOT: usize = SECTION_TABLE_OFFSET + 4;
const CHUNK_HEADER_LEN: usize = 0x10;
// Three section offsets plus `totalDelay` (EffectRoutineParser.kt:46-50).
const SECTION_TABLE_LEN: usize = 0x10;

// research/xim EffectRoutineParser.kt:65 — `numInputs = (unkCombo and 0x1F) - 1`, counted from
// the dword that carries the opcode, so the stage spans `unkCombo & 0x1F` dwords in total.
const STAGE_LENGTH_MASK: u16 = 0x1F;

// research/xim EffectRoutineParser.kt:81,96-98 / :275-285.
const END_ROUTINE_OPCODE: u8 = 0x00;
const RANDOM_BLOCK_OPEN: u8 = 0x3D;
const RANDOM_BLOCK_CLOSE: u8 = 0x3E;

fn effect_section_start(body: &[u8]) -> usize {
    let Some(raw) = body
        .get(SECTION2_SLOT..SECTION2_SLOT + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
    else {
        return SCHEDULER_HEADER_LEN;
    };
    let raw = raw as usize;
    let start = raw.saturating_sub(CHUNK_HEADER_LEN);
    // The effect list can never begin inside the header that describes it, and a truncated
    // chunk must not send the cursor past the end of the body.
    let first_body_offset = SECTION_TABLE_OFFSET + SECTION_TABLE_LEN;
    if raw >= CHUNK_HEADER_LEN + first_body_offset && start < body.len() {
        start
    } else {
        SCHEDULER_HEADER_LEN
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerStage {
    pub kind: StageKind,

    pub raw_type: u8,

    pub delay_frames: u16,

    pub duration_frames: u16,

    pub id: [u8; 4],

    // research/xim EffectRoutineParser.kt:115-130 (opcode 0x05). Half-frame units (divide
    // by 2 for real frames). Zero when the stage is shorter than the motion payload.
    pub max_loops: u16,
    pub transition_in: u16,
    pub transition_out: u16,

    // research/xim EffectRoutineParser.kt:275-285,553-559 — stages between a 0x3D and its 0x3E
    // are children of one RandomChildRoutine, not siblings on the timeline: retail runs exactly
    // one of them per activation (`vatk`'s four atk1..atk4 grunts). Members of the same block
    // share a group index; `None` is an ordinary unconditional stage.
    pub random_group: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Motion,

    SoundOnTarget,

    SoundOnCaster,

    Particle,

    SubRoutine,

    // research/xim EffectRoutineParser.kt:136-140 — LinkedEffectRoutine(useTarget = true): the
    // child sequence's source actor is the primary target, so it resolves its ids against the
    // TARGET's resource dirs (the victim's own hit grunt / flinch), not the caster's.
    SubRoutineOnTarget,

    BlockingSubRoutine,

    StopParticle,

    DamageCallback,

    Unknown,
}

// research/xim EffectRoutineParser.kt:64,141-154 — opcode 0x0A is overloaded: a
// 32-byte stage (length_words 8, XIM numArgs 7) is a Source (caster) sound emitter,
// while any other length is a LinkedEffectRoutine sub-routine. Disambiguate by length.
const SOUND_EMITTER_LENGTH_WORDS: usize = 8;

impl StageKind {
    fn from_stage(b: u8, length_words: usize) -> Self {
        match b {
            // Opcodes empirically confirmed against retail spell DATs (e.g. Cure = file 0xAF1):
            // 0x02 spawns a particle generator, 0x03 calls a sub-routine, 0x05 plays motion,
            // 0x0B/0x53 play sound on target/caster.
            0x02 => Self::Particle,
            0x03 => Self::SubRoutine,
            0x05 => Self::Motion,
            // research/xim EffectRoutineParser.kt:136-140.
            0x09 => Self::SubRoutineOnTarget,
            0x0A if length_words == SOUND_EMITTER_LENGTH_WORDS => Self::SoundOnCaster,
            0x0A => Self::SubRoutine,
            0x0B => Self::SoundOnTarget,
            // research/xim EffectRoutineParser.kt:253-258 — StopParticleGeneratorRoutine, id =
            // the generator DatId to stop (ROM/0/0.DAT `stbk` stops the cast aura's gn10..gn13).
            0x2D => Self::StopParticle,
            // research/xim EffectRoutineParser.kt:219-222 — DamageCallbackRoutine, the stage the
            // damage/battle-message callback is invoked on (EffectRoutineInstance.kt:956-959).
            // Every spell routine tail-calls a `mdam` sub-routine that holds exactly this stage.
            0x2B => Self::DamageCallback,
            // research/xim EffectRoutineParser.kt:270-274 — LinkedEffectRoutine with
            // `blocking = true`: the same sub-routine call as 0x03, except the parent stalls
            // until the child finishes (EffectRoutineInstance.kt:400 `blockers += newSequences`).
            0x3B | 0x3C => Self::BlockingSubRoutine,
            0x53 => Self::SoundOnCaster,
            // research/xim EffectRoutineParser.kt:371-375 — a plain LinkedEffectRoutine, the
            // form every melee routine uses (`ati0` links the weapon's `skaz` whoosh, `atk0`
            // the race/face `vatk` grunt).
            0x57 => Self::SubRoutine,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Scheduler {
    pub name: [u8; 4],
    pub stages: Vec<TimedStage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedStage {
    pub frame: u32,
    pub stage: SchedulerStage,
}

impl Scheduler {
    pub fn parse(name: [u8; 4], body: &[u8]) -> Result<Self> {
        if body.len() < SCHEDULER_HEADER_LEN {
            return Err(DatError::TruncatedChunk {
                offset: 0,
                needed: SCHEDULER_HEADER_LEN,
                available: body.len(),
            });
        }
        let mut stages = Vec::new();
        let mut cursor = effect_section_start(body);
        let mut running_frame: u32 = 0;
        let mut open_group: Option<u16> = None;
        let mut next_group: u16 = 0;

        while cursor + 4 <= body.len() {
            let raw_type = body[cursor];
            // research/xim EffectRoutineParser.kt:63-68 — opcode(8), unkCombo(16), unk0(8); the
            // stage spans `(unkCombo & 0x1F)` dwords including the opcode dword itself.
            let length_words = (u16::from_le_bytes([body[cursor + 1], body[cursor + 2]])
                & STAGE_LENGTH_MASK) as usize;
            let stage_bytes = length_words.saturating_mul(4);
            if stage_bytes < 4 || cursor + stage_bytes > body.len() {
                break;
            }

            if stage_bytes >= 12 && cursor + 12 <= body.len() {
                let delay = u16::from_le_bytes([body[cursor + 4], body[cursor + 5]]);
                let duration = u16::from_le_bytes([body[cursor + 6], body[cursor + 7]]);
                let id = [
                    body[cursor + 8],
                    body[cursor + 9],
                    body[cursor + 10],
                    body[cursor + 11],
                ];
                let kind = StageKind::from_stage(raw_type, length_words);
                // research/xim EffectRoutineParser.kt:115-130: after id(+8) and a zero32(+12)
                // and two floats(+16,+20), the 0x05 motion payload carries transitionIn(+24),
                // a zero u16(+26), transitionOut(+28), maxLoop(+30).
                let read_u16 =
                    |off: usize| u16::from_le_bytes([body[cursor + off], body[cursor + off + 1]]);
                let (max_loops, transition_in, transition_out) =
                    if kind == StageKind::Motion && stage_bytes >= 32 {
                        (read_u16(30), read_u16(24), read_u16(28))
                    } else {
                        (0, 0, 0)
                    };
                // research/xim EffectRoutineInstance.kt runEffects: `storedFrames -=
                // head.delay` happens as each effect is popped and run, so a stage's
                // delay gates the stages AFTER it — never itself. Fire frame is the
                // sum of PRIOR delays (first stage always fires at 0: a lone Motion
                // with delay 152, e.g. the emote bow routine, plays immediately).
                stages.push(TimedStage {
                    frame: running_frame,
                    stage: SchedulerStage {
                        kind,
                        raw_type,
                        delay_frames: delay,
                        duration_frames: duration,
                        id,
                        max_loops,
                        transition_in,
                        transition_out,
                        random_group: open_group,
                    },
                });
                // A random block's children are collected into the 0x3D marker rather than
                // appended to the parent timeline (EffectRoutineParser.kt:553-559), so only
                // the marker's own delay advances the parent clock.
                if open_group.is_none() {
                    running_frame = running_frame.saturating_add(delay as u32);
                }
            }
            match raw_type {
                RANDOM_BLOCK_OPEN => {
                    open_group = Some(next_group);
                    next_group = next_group.saturating_add(1);
                }
                RANDOM_BLOCK_CLOSE => open_group = None,
                _ => {}
            }
            cursor += stage_bytes;
            // EffectRoutineParser.kt:81 — opcode 0x00 ends the section; section 3 follows it in
            // the same chunk and would otherwise be misread as more effect stages.
            if raw_type == END_ROUTINE_OPCODE {
                break;
            }
        }
        Ok(Self { name, stages })
    }

    pub fn sound_events(&self) -> impl Iterator<Item = SoundEvent> + '_ {
        self.stages.iter().filter_map(|t| match t.stage.kind {
            StageKind::SoundOnCaster => Some(SoundEvent {
                frame: t.frame,
                id: t.stage.id,
                on_caster: true,
            }),
            StageKind::SoundOnTarget => Some(SoundEvent {
                frame: t.frame,
                id: t.stage.id,
                on_caster: false,
            }),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundEvent {
    pub frame: u32,
    pub id: [u8; 4],
    pub on_caster: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_payload_recovers_loop_and_transition() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];
        // 40-byte (10-word) motion opcode: header, delay, duration, id, then the 0x05 tail.
        body.extend_from_slice(&[0x05, 0x0A, 0, 0]); // opcode, length=10 words
        body.extend_from_slice(&0u16.to_le_bytes()); // +4 delay
        body.extend_from_slice(&64u16.to_le_bytes()); // +6 duration
        body.extend_from_slice(b"mae0"); // +8 id
        body.extend_from_slice(&0u32.to_le_bytes()); // +12 zero32
        body.extend_from_slice(&1.0f32.to_le_bytes()); // +16 float
        body.extend_from_slice(&1.0f32.to_le_bytes()); // +20 float
        body.extend_from_slice(&8u16.to_le_bytes()); // +24 transitionIn
        body.extend_from_slice(&0u16.to_le_bytes()); // +26 zero
        body.extend_from_slice(&12u16.to_le_bytes()); // +28 transitionOut
        body.extend_from_slice(&3u16.to_le_bytes()); // +30 maxLoop
        body.extend_from_slice(&0u32.to_le_bytes()); // +32 unk0
        body.extend_from_slice(&0u32.to_le_bytes()); // +36 unk1

        let s = Scheduler::parse(*b"mae0", &body).unwrap();
        assert_eq!(s.stages.len(), 1);
        let st = s.stages[0].stage;
        assert_eq!(st.kind, StageKind::Motion);
        assert_eq!(&st.id, b"mae0");
        assert_eq!(st.transition_in, 8);
        assert_eq!(st.transition_out, 12);
        assert_eq!(st.max_loops, 3);
    }

    #[test]
    fn short_motion_stage_has_zero_tail() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];
        body.extend_from_slice(&[0x05, 0x03, 0, 0]);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&20u16.to_le_bytes());
        body.extend_from_slice(b"mot0");
        let s = Scheduler::parse(*b"sdam", &body).unwrap();
        let st = s.stages[0].stage;
        assert_eq!(st.max_loops, 0);
        assert_eq!(st.transition_in, 0);
        assert_eq!(st.transition_out, 0);
    }

    #[test]
    fn parses_motion_then_sound_caster() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];

        body.extend_from_slice(&[0x05, 0x03, 0, 0]);
        body.extend_from_slice(&30u16.to_le_bytes());
        body.extend_from_slice(&20u16.to_le_bytes());
        body.extend_from_slice(b"mot0");

        body.extend_from_slice(&[0x53, 0x03, 0, 0]);
        body.extend_from_slice(&15u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(b"snd0");

        let s = Scheduler::parse(*b"sdam", &body).unwrap();
        assert_eq!(s.stages.len(), 2);
        assert_eq!(s.stages[0].stage.kind, StageKind::Motion);
        assert_eq!(
            s.stages[0].frame, 0,
            "first stage fires at 0 despite its own delay"
        );
        assert_eq!(s.stages[0].stage.delay_frames, 30);
        assert_eq!(s.stages[1].stage.kind, StageKind::SoundOnCaster);
        assert_eq!(
            s.stages[1].frame, 30,
            "second stage fires after the first stage's delay"
        );
        assert_eq!(&s.stages[1].stage.id, b"snd0");

        let snd: Vec<_> = s.sound_events().collect();
        assert_eq!(snd.len(), 1);
        assert!(snd[0].on_caster);
        assert_eq!(snd[0].frame, 30);
    }

    // Boost's effect DAT (ROM/16/0.DAT) plays its caster sound via opcode 0x0A with
    // length_words 8 (32-byte stage); a 0x0A of any other length is a sub-routine link.
    #[test]
    fn opcode_0a_len8_is_sound_else_subroutine() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];
        // 0x0A, length 8 words (32 bytes): a caster sound emitter.
        body.extend_from_slice(&[0x0A, 0x08, 0, 0]);
        body.extend_from_slice(&0u16.to_le_bytes()); // +4 delay
        body.extend_from_slice(&0u16.to_le_bytes()); // +6 duration
        body.extend_from_slice(b"7047"); // +8 id -> se_id 7047
        body.extend(std::iter::repeat_n(0u8, 20)); // pad to 32 bytes
                                                   // 0x0A, length 3 words (12 bytes): a sub-routine link, not a sound.
        body.extend_from_slice(&[0x0A, 0x03, 0, 0]);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(b"sub0");

        let s = Scheduler::parse(*b"main", &body).unwrap();
        assert_eq!(s.stages[0].stage.kind, StageKind::SoundOnCaster);
        assert_eq!(&s.stages[0].stage.id, b"7047");
        assert_eq!(s.stages[1].stage.kind, StageKind::SubRoutine);
    }

    /// A lone Motion stage with a large delay (the emote-DAT shape, e.g. HumeM
    /// bow = `Motion delay=152`) fires at frame 0 — the delay only pads the
    /// routine tail (research/xim EffectRoutineInstance.kt runEffects).
    #[test]
    fn lone_delayed_motion_fires_immediately() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];
        body.extend_from_slice(&[0x05, 0x03, 0, 0]);
        body.extend_from_slice(&152u16.to_le_bytes());
        body.extend_from_slice(&152u16.to_le_bytes());
        body.extend_from_slice(b"bow?");
        let s = Scheduler::parse(*b"em00", &body).unwrap();
        assert_eq!(s.stages[0].frame, 0);
        assert_eq!(s.stages[0].stage.duration_frames, 152);
    }

    #[test]
    fn opcode_3c_is_blocking_subroutine_and_2d_is_stop_particle() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];
        body.extend_from_slice(&[0x3C, 0x04, 0, 0]);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(b"shbk");
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[0x3B, 0x04, 0, 0]);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(b"wash");
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[0x2D, 0x04, 0, 0]);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(b"gn13");
        body.extend_from_slice(&0u32.to_le_bytes());

        let s = Scheduler::parse(*b"main", &body).unwrap();
        assert_eq!(s.stages[0].stage.kind, StageKind::BlockingSubRoutine);
        assert_eq!(&s.stages[0].stage.id, b"shbk");
        assert_eq!(s.stages[1].stage.kind, StageKind::BlockingSubRoutine);
        assert_eq!(&s.stages[1].stage.id, b"wash");
        assert_eq!(s.stages[2].stage.kind, StageKind::StopParticle);
        assert_eq!(&s.stages[2].stage.id, b"gn13");
    }

    // Retail-byte guard (skips without an install): Poison's effect DAT links the caster's
    // cast-complete routine with 0x3C, and the global system-effect dir stops the cast aura's
    // four generators with 0x2D. Both were dropped as Unknown before kuluu-ky8c.
    #[test]
    fn real_dat_spell_main_links_caster_finish_routine() {
        const POISON_FILE: u32 = 3020;
        const GLOBAL_EFFECT_DIR_FILE: u32 = 0;

        let Ok(root) = crate::DatRoot::from_env_or_default() else {
            return;
        };
        let read = |id: u32| -> Option<Vec<u8>> {
            let loc = root.resolve(id).ok()?;
            std::fs::read(loc.path_under(root.root())).ok()
        };
        let Some(poison) = read(POISON_FILE) else {
            return;
        };
        let scheds = |bytes: &[u8]| -> Vec<Scheduler> {
            crate::resource_dir::ResourceDir::from_bytes(bytes.to_vec()).collect_schedulers()
        };
        let main = scheds(&poison)
            .into_iter()
            .find(|s| &s.name == b"main")
            .expect("poison DAT has a main routine");
        let link = main
            .stages
            .iter()
            .find(|t| t.stage.raw_type == 0x3C)
            .expect("main links a caster routine with 0x3C");
        assert_eq!(link.stage.kind, StageKind::BlockingSubRoutine);
        assert_eq!(&link.stage.id, b"shbk");

        let Some(global) = read(GLOBAL_EFFECT_DIR_FILE) else {
            return;
        };
        let stbk = scheds(&global)
            .into_iter()
            .find(|s| &s.name == b"stbk")
            .expect("global effect dir has the stbk stop routine");
        let stopped: Vec<[u8; 4]> = stbk
            .stages
            .iter()
            .filter(|t| t.stage.kind == StageKind::StopParticle)
            .map(|t| t.stage.id)
            .collect();
        for gen_id in [b"gn10", b"gn11", b"gn12", b"gn13"] {
            assert!(
                stopped.contains(gen_id),
                "stbk stops {}",
                String::from_utf8_lossy(gen_id)
            );
        }
    }

    // research/xim EffectRoutineParser.kt:219-222 (0x2B DamageCallbackRoutine) and :270-274
    // (0x3B/0x3C LinkedEffectRoutine with blocking = true, unlike the 0x03 link).
    #[test]
    fn damage_callback_and_blocking_subroutine_opcodes() {
        assert_eq!(StageKind::from_stage(0x2B, 3), StageKind::DamageCallback);
        assert_eq!(
            StageKind::from_stage(0x3C, 4),
            StageKind::BlockingSubRoutine
        );
        assert_eq!(
            StageKind::from_stage(0x3B, 4),
            StageKind::BlockingSubRoutine
        );
        assert_eq!(StageKind::from_stage(0x03, 3), StageKind::SubRoutine);
        assert_ne!(
            StageKind::from_stage(0x3C, 4),
            StageKind::from_stage(0x03, 3)
        );
    }

    // Retail-byte guard (skips without an install): the global effect dir's `mdam` routine —
    // the sub-routine every spell's target routine tail-calls — IS the damage callback, a
    // single 0x2B stage. It decoded as Unknown before kuluu-k6tz.
    #[test]
    fn real_dat_global_mdam_routine_is_a_damage_callback() {
        const GLOBAL_EFFECT_DIR_FILE: u32 = 0;

        let Ok(root) = crate::DatRoot::from_env_or_default() else {
            return;
        };
        let Ok(loc) = root.resolve(GLOBAL_EFFECT_DIR_FILE) else {
            return;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            return;
        };
        let mdam = crate::resource_dir::ResourceDir::from_bytes(bytes)
            .collect_schedulers()
            .into_iter()
            .find(|s| &s.name == b"mdam")
            .expect("global effect dir has the mdam routine");
        assert!(
            mdam.stages
                .iter()
                .any(|t| t.stage.kind == StageKind::DamageCallback),
            "mdam holds the 0x2B damage callback"
        );
    }

    #[test]
    fn unknown_type_is_preserved() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];
        body.extend_from_slice(&[0xAB, 0x03, 0, 0]);
        body.extend_from_slice(&5u16.to_le_bytes());
        body.extend_from_slice(&5u16.to_le_bytes());
        body.extend_from_slice(b"????");
        let s = Scheduler::parse(*b"sch0", &body).unwrap();
        assert_eq!(s.stages[0].stage.kind, StageKind::Unknown);
        assert_eq!(s.stages[0].stage.raw_type, 0xAB);
    }

    #[test]
    fn truncated_stage_stops_scan_without_panic() {
        let mut body = vec![0u8; SCHEDULER_HEADER_LEN];

        // STAGE_LENGTH_MASK words is the longest stage the encoding can express (124 bytes),
        // far past this 12-byte tail.
        body.extend_from_slice(&[0x05, STAGE_LENGTH_MASK as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let s = Scheduler::parse(*b"trun", &body).unwrap();
        assert_eq!(s.stages.len(), 0);
    }
}
