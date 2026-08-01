use super::*;

/// s2c 0x037 GP_SERV_SERVERSTATUS (char status). Only the fields we consume are
/// decoded: the subject id, its HP%, the death/homepoint counters, the animation
/// (`server_status`) byte, and the fishing hook-delay timer.
/// vendor/server/src/map/packets/char_status.cpp
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharStatus {
    pub unique_no: u32,
    pub hpp: u8,
    pub dead_counter1: u32,
    pub dead_counter2: u32,
    /// The self player's animation byte (ANIMATIONTYPE); see [`animation`]. Mirrors the
    /// `server_status` that 0x0D broadcasts for other players.
    pub server_status: u8,
    /// Frames the client waits before the cast settles and it requests a hook check.
    /// Only meaningful while `server_status == animation::FISHING_START`. 0 if the packet
    /// was truncated before this field.
    pub fishing_timer: u8,
    /// Movement speed, 0 while bound
    /// (vendor/server/src/map/packets/char_status.cpp `Flags1.Speed`).
    pub speed: u16,
}

impl CharStatus {
    pub const UNIQUE_NO_OFFSET: usize = 0x20;
    pub const FLAGS0_OFFSET: usize = 0x24;
    pub const SPEED_OFFSET: usize = 0x28;
    pub const SERVER_STATUS_OFFSET: usize = 0x2C;
    pub const DEAD_COUNTER1_OFFSET: usize = 0x38;
    pub const DEAD_COUNTER2_OFFSET: usize = 0x3C;
    pub const FISHING_TIMER_OFFSET: usize = 0x46;
    pub const SPEED_MASK: u16 = 0x0FFF;
    pub const MIN_LEN: usize = Self::DEAD_COUNTER2_OFFSET + 4;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let need = Self::MIN_LEN;
        if body.len() < need {
            return Err(DecodeError::Truncated(need, body.len()));
        }
        let rd = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        let flags0 = rd(Self::FLAGS0_OFFSET);
        Ok(Self {
            unique_no: rd(Self::UNIQUE_NO_OFFSET),
            // flags0_t bitfield: hpp occupies bits 16..24.
            hpp: ((flags0 >> 16) & 0xFF) as u8,
            dead_counter1: rd(Self::DEAD_COUNTER1_OFFSET),
            dead_counter2: rd(Self::DEAD_COUNTER2_OFFSET),
            server_status: body[Self::SERVER_STATUS_OFFSET],
            fishing_timer: body.get(Self::FISHING_TIMER_OFFSET).copied().unwrap_or(0),
            speed: u16::from_le_bytes([body[Self::SPEED_OFFSET], body[Self::SPEED_OFFSET + 1]])
                & Self::SPEED_MASK,
        })
    }

    /// Seconds until the server force-warps a KO'd player home. LSB sends
    /// dead_counter1 = 60 * (6min + (60min - timeSinceDeath)); the leading 6min is fixed
    /// padding, so stripping it (`dead_counter1/60 - 360`) yields the real time left,
    /// which hits 0 when the server-side CDeathState completes at death + 60min.
    /// vendor/server/src/map/packets/char_status.cpp,
    /// charentity.cpp::GetTimeUntilDeathHomepoint, ai/states/death_state.cpp
    pub fn seconds_until_homepoint(&self) -> u32 {
        (self.dead_counter1 / 60).saturating_sub(360)
    }
}

const _: () = assert!(CharStatus::SPEED_OFFSET + 2 <= CharStatus::MIN_LEN);

/// s2c 0x061 GP_SERV_COMMAND_CLISTATUS — the self-character stat block.
/// Field offsets follow the `CLISTATUS` struct in
/// vendor/server/src/map/packets/s2c/0x061_clistatus.h (mirror of
/// research/XiPackets/world/server/0x0061). `bp_base`/`bp_adj` are STR, DEX, VIT,
/// AGI, INT, MND, CHR in order; `bp_adj` is the signed gear/buff delta retail shows
/// as the "+N" beside each stat. `def_elem` is Fire, Ice, Wind, Earth, Lightning,
/// Water, Light, Dark. The struct declares `atk`/`def` as int16_t, but they are
/// sourced from the non-negative ATT()/DEF() (.cpp:63-64), so we read them as u16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliStatus {
    pub hp_max: u32,
    pub mp_max: u32,
    pub mjob_no: u8,
    pub mjob_lv: u8,
    pub sjob_no: u8,
    pub sjob_lv: u8,
    pub bp_base: [u16; 7],
    pub bp_adj: [i16; 7],
    pub attack: u16,
    pub defense: u16,
    pub def_elem: [i16; 8],
    pub ilvl: u8,
}

impl CliStatus {
    // vendor/server/src/map/packets/s2c/0x061_clistatus.h:45-82 — the four job bytes
    // sit between mpmax (@4) and exp_now (@12).
    const MJOB_NO_OFFSET: usize = 8;
    const MJOB_LV_OFFSET: usize = 9;
    const SJOB_NO_OFFSET: usize = 10;
    const SJOB_LV_OFFSET: usize = 11;
    const ILVL_OFFSET: usize = 81;
    const NEEDED: usize = Self::ILVL_OFFSET + 1;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::NEEDED {
            return Err(DecodeError::Truncated(Self::NEEDED, body.len()));
        }
        let rd32 = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        let rd16 = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
        let rdi16 = |o: usize| i16::from_le_bytes([body[o], body[o + 1]]);
        let mut bp_base = [0u16; 7];
        let mut bp_adj = [0i16; 7];
        for i in 0..7 {
            bp_base[i] = rd16(16 + i * 2);
            bp_adj[i] = rdi16(30 + i * 2);
        }
        let mut def_elem = [0i16; 8];
        for (i, e) in def_elem.iter_mut().enumerate() {
            *e = rdi16(48 + i * 2);
        }
        Ok(Self {
            hp_max: rd32(0),
            mp_max: rd32(4),
            mjob_no: body[Self::MJOB_NO_OFFSET],
            mjob_lv: body[Self::MJOB_LV_OFFSET],
            sjob_no: body[Self::SJOB_NO_OFFSET],
            sjob_lv: body[Self::SJOB_LV_OFFSET],
            bp_base,
            bp_adj,
            attack: rd16(44),
            defense: rd16(46),
            def_elem,
            ilvl: body[Self::ILVL_OFFSET],
        })
    }
}

/// s2c 0x01B GP_SERV_COMMAND_JOB_INFO — per-job levels + unlocked-jobs bitmask for
/// the self character. Body offsets follow the GP_MYROOM_DANCER struct in
/// vendor/server/src/map/packets/s2c/0x01b_job_info.h:28-62 (filled in .cpp:30-57).
/// `job_levels` reads `job_lev2` (the full `jobs.job[24]` memcpy, index = JOBTYPE);
/// the legacy `job_lev[16]` @0x0C truncates at 16 jobs and is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobInfo {
    pub mjob_no: u8,
    pub sjob_no: u8,
    /// Bit N set = JOBTYPE N unlocked. Bit 0 is the subjob-feature flag, not a job
    /// (`sjobflg = jobs.unlocked & 1`, 0x01b_job_info.cpp).
    pub unlocked: u32,
    pub sub_job_unlocked: bool,
    pub job_levels: [u8; Self::MAX_JOBTYPE],
    pub hp_max: i32,
    pub mp_max: i32,
    pub sjobflg: u8,
}

impl JobInfo {
    /// MAX_JOBTYPE, vendor/server/src/map/entities/battleentity.h (JOBTYPE 1=WAR..23=MON).
    pub const MAX_JOBTYPE: usize = 24;

    pub const MJOB_NO_OFFSET: usize = 0x04;
    pub const SJOB_NO_OFFSET: usize = 0x07;
    pub const UNLOCKED_OFFSET: usize = 0x08;
    pub const HP_MAX_OFFSET: usize = 0x38;
    pub const MP_MAX_OFFSET: usize = 0x3C;
    pub const SJOBFLG_OFFSET: usize = 0x40;
    pub const JOB_LEVELS_OFFSET: usize = 0x44;
    pub const MIN_LEN: usize = Self::JOB_LEVELS_OFFSET + Self::MAX_JOBTYPE;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::MIN_LEN {
            return Err(DecodeError::Truncated(Self::MIN_LEN, body.len()));
        }
        let rdi32 = |o: usize| i32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        let unlocked = u32::from_le_bytes(
            body[Self::UNLOCKED_OFFSET..Self::UNLOCKED_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        let mut job_levels = [0u8; Self::MAX_JOBTYPE];
        job_levels.copy_from_slice(
            &body[Self::JOB_LEVELS_OFFSET..Self::JOB_LEVELS_OFFSET + Self::MAX_JOBTYPE],
        );
        Ok(Self {
            mjob_no: body[Self::MJOB_NO_OFFSET],
            sjob_no: body[Self::SJOB_NO_OFFSET],
            unlocked,
            sub_job_unlocked: unlocked & 1 != 0,
            job_levels,
            hp_max: rdi32(Self::HP_MAX_OFFSET),
            mp_max: rdi32(Self::MP_MAX_OFFSET),
            sjobflg: body[Self::SJOBFLG_OFFSET],
        })
    }
}
