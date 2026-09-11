// vendor/server/src/map/enums/action/category.h ActionCategory - `action.cmd_no`, 4 bits.
pub const CATEGORY_BASIC_ATTACK: u8 = 1;
// The finish categories that key a completion effect DAT (scheduler_runtime's
// action_dat_file_id) and the start categories that carry a cast-loop routine (the "ca??" family).
pub const CATEGORY_SKILL_FINISH: u8 = 3;
pub const CATEGORY_MAGIC_FINISH: u8 = 4;
pub const CATEGORY_ABILITY_FINISH: u8 = 6;
pub const CATEGORY_SKILL_START: u8 = 7;
pub const CATEGORY_ITEM_START: u8 = 9;
pub const CATEGORY_ABILITY_START: u8 = 10;
pub const CATEGORY_MOB_SKILL_FINISH: u8 = 11;
pub const CATEGORY_RANGED_START: u8 = 12;
pub const CATEGORY_PET_SKILL_FINISH: u8 = 13;

// vendor/server/src/map/enums/action/info.h - the per-result `info` bits. Defeated means the
// action killed the target (retail flips StatusServer on the same frame as the HP packet;
// .agents/skills/retail-observe/references/2026-09-09-wormwatch-runtime.md "First non-burrow routines");
// CriticalHit marks a critical hit. It is set from outcome.isCritical and is independent of
// hitDistortion: vendor/server/src/map/action/action.cpp action_result_t::recordDamage derives
// hitDistortion purely from damage as a percent of target max HP (>=20 Heavy, >=10 Medium, >0
// Light), so a crit can land on any distortion level and a heavy recoil can be non-crit.
pub const INFO_DEFEATED: u8 = 1;
pub const INFO_CRITICAL_HIT: u8 = 2;

// vendor/server/src/map/enums/action/resolution.h — `result.resolution`, 3 bits in
// vendor/server/src/map/packets/s2c/0x028_battle2.cpp GP_SERV_COMMAND_BATTLE2::pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActionResolution {
    Hit,
    Miss,
    Guard,
    Parry,
    Block,
}

impl ActionResolution {
    pub fn from_wire(bits: u8) -> Option<Self> {
        Some(match bits {
            0 => Self::Hit,
            1 => Self::Miss,
            2 => Self::Guard,
            3 => Self::Parry,
            4 => Self::Block,
            _ => return None,
        })
    }

    pub fn to_wire(self) -> u8 {
        match self {
            Self::Hit => 0,
            Self::Miss => 1,
            Self::Guard => 2,
            Self::Parry => 3,
            Self::Block => 4,
        }
    }
}

// vendor/server/src/map/attack.h AttackAnimation. Set from `attack.GetAnimationID()` into
// `actionResult.animation` (vendor/server/src/map/entities/battleentity.cpp CBattleEntity::OnAttack) — for a basic
// attack this is the swing slot, not a skill id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AttackAnimation {
    RightAttack,
    LeftAttack,
    RightKick,
    LeftKick,
    Throw,
}

impl AttackAnimation {
    pub fn from_wire(bits: u16) -> Option<Self> {
        Some(match bits {
            0 => Self::RightAttack,
            1 => Self::LeftAttack,
            2 => Self::RightKick,
            3 => Self::LeftKick,
            4 => Self::Throw,
            _ => return None,
        })
    }

    pub fn to_wire(self) -> u16 {
        match self {
            Self::RightAttack => 0,
            Self::LeftAttack => 1,
            Self::RightKick => 2,
            Self::LeftKick => 3,
            Self::Throw => 4,
        }
    }
}

// vendor/server/src/map/enums/action/info.h - result.info, 5 bits, a bitflag set (that header's
// magic_enum::customize::enum_range<ActionInfo>::is_flags is true): Defeated and CriticalHit
// combine with each other, and the same field carries job-specific values for Dancer
// steps/flourishes, Rune Fencer elements, DRG jumps and COR rolls. A typed u8 keeps every 5-bit
// value round-trippable; an enum would have to invent combined variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionInfo(u8);

impl ActionInfo {
    pub const NONE: Self = Self(0);
    /// Field width on the wire: 0x028_battle2.cpp GP_SERV_COMMAND_BATTLE2::pack writes info(5)
    /// (vendor/server/src/map/enums/action/info.h result.info is 5 bits), so only the low five
    /// bits of a byte are part of the field.
    const FIELD_MASK: u8 = 0x1F;
    /// info.h Defeated - the action defeated the target; retail flips StatusServer on this frame.
    pub const DEFEATED: Self = Self(INFO_DEFEATED);
    /// info.h CriticalHit - set from outcome.isCritical, independent of hitDistortion.
    pub const CRITICAL_HIT: Self = Self(INFO_CRITICAL_HIT);

    /// The 5 wire bits as written by 0x028_battle2.cpp GP_SERV_COMMAND_BATTLE2::pack (info(5)).
    pub fn bits(self) -> u8 {
        self.0
    }

    /// Mask to the field width: the bit reader already bounds info to 5 bits, and anything above
    /// is not representable on the wire.
    pub fn from_bits(bits: u8) -> Self {
        Self(bits & Self::FIELD_MASK)
    }

    pub fn is_defeated(self) -> bool {
        self.0 & INFO_DEFEATED != 0
    }

    pub fn is_critical_hit(self) -> bool {
        self.0 & INFO_CRITICAL_HIT != 0
    }
}

// vendor/server/src/map/enums/action/hit_distortion.h - result.scale lower 2 bits: the defender's
// recoil after a physical attack. recordDamage derives it purely from damage as a percent of the
// target's max HP (>=20 Heavy, >=10 Medium, >0 Light; vendor/server/src/map/action/action.cpp
// action_result_t::recordDamage), independent of the crit bit in info.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HitDistortion {
    None,
    Light,
    Medium,
    Heavy,
}

impl HitDistortion {
    pub fn from_wire(bits: u8) -> Option<Self> {
        Some(match bits {
            0 => Self::None,
            1 => Self::Light,
            2 => Self::Medium,
            3 => Self::Heavy,
            _ => return None,
        })
    }

    pub fn to_wire(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Light => 1,
            Self::Medium => 2,
            Self::Heavy => 3,
        }
    }
}

// vendor/server/src/map/enums/action/knockback.h - result.scale upper 3 bits (the C++ enum is
// named Knockback): any non-zero level plays `sway` alongside the damage reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KnockbackLevel {
    None,
    Level1,
    Level2,
    Level3,
    Level4,
    Level5,
    Level6,
    Level7,
}

impl KnockbackLevel {
    pub fn from_wire(bits: u8) -> Option<Self> {
        Some(match bits {
            0 => Self::None,
            1 => Self::Level1,
            2 => Self::Level2,
            3 => Self::Level3,
            4 => Self::Level4,
            5 => Self::Level5,
            6 => Self::Level6,
            7 => Self::Level7,
            _ => return None,
        })
    }

    pub fn to_wire(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Level1 => 1,
            Self::Level2 => 2,
            Self::Level3 => 3,
            Self::Level4 => 4,
            Self::Level5 => 5,
            Self::Level6 => 6,
            Self::Level7 => 7,
        }
    }

    /// Any non-zero level: the sway-alongside rule (rabbit_tester s7d).
    pub fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

// The first result block's outcome, typed. `None` means "no result block was read" (or the block
// carried out-of-range enum bits), never zero: resolution 0 is Hit, so absence must not be spelled
// as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResultOutcome {
    pub resolution: ActionResolution,
    pub info: ActionInfo,
    pub hit_distortion: HitDistortion,
    pub knockback: KnockbackLevel,
}

impl ResultOutcome {
    /// From the raw bits of 0x028_battle2.cpp GP_SERV_COMMAND_BATTLE2::pack (resolution(3),
    /// info(5), hitDistortion(2), knockback(3)); kind(2) and animation(12) are not outcome data.
    pub fn from_wire(resolution: u8, info: u8, hit_distortion: u8, knockback: u8) -> Option<Self> {
        Some(Self {
            resolution: ActionResolution::from_wire(resolution)?,
            info: ActionInfo::from_bits(info),
            hit_distortion: HitDistortion::from_wire(hit_distortion)?,
            knockback: KnockbackLevel::from_wire(knockback)?,
        })
    }
}

// vendor/server/src/map/packets/s2c/0x028_battle2.cpp GP_SERV_COMMAND_BATTLE2::pack - one result block's
// bits: resolution(3), kind(2, skipped), animation(12), info(5), hitDistortion(2), knockback(3). A body that
// carries no result block, or that ends mid-block, has none of them at all: `resolution == 0`
// is `Hit`, so absence must not be spelled as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeleeResult {
    pub resolution: ActionResolution,
    pub animation: AttackAnimation,
    /// vendor/server/src/map/enums/action/info.h - bit 1 `Defeated` (the action killed the
    /// target), bit 2 `CriticalHit`. Defeated latches the death path on this frame
    /// (rabbit_tester s9).
    pub info: ActionInfo,
    /// vendor/server/src/map/enums/action/hit_distortion.h - recordDamage sets it from damage as a
    /// percent of the target's max HP, independent of the crit bit in `info`.
    pub hit_distortion: HitDistortion,
    /// vendor/server/src/map/enums/action/knockback.h - any non-zero level plays `sway` alongside
    /// the damage reaction (rabbit_tester s7d).
    pub knockback: KnockbackLevel,
}

impl MeleeResult {
    pub fn from_wire(
        resolution: u8,
        animation: u16,
        info: u8,
        hit_distortion: u8,
        knockback: u8,
    ) -> Option<Self> {
        Some(Self {
            resolution: ActionResolution::from_wire(resolution)?,
            animation: AttackAnimation::from_wire(animation)?,
            info: ActionInfo::from_bits(info),
            hit_distortion: HitDistortion::from_wire(hit_distortion)?,
            knockback: KnockbackLevel::from_wire(knockback)?,
        })
    }

    /// Every wire bit of the result block in 0x028_battle2.cpp pack order (resolution(3),
    /// animation(12), info(5), hitDistortion(2), knockback(3)). Lossless: from_wire(to_wire(x)) == x
    /// for every constructible MeleeResult, so no outcome data is dropped on the way to the wire.
    pub fn to_wire(self) -> (u8, u16, u8, u8, u8) {
        (
            self.resolution.to_wire(),
            self.animation.to_wire(),
            self.info.bits(),
            self.hit_distortion.to_wire(),
            self.knockback.to_wire(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_roundtrips_through_melee_result() {
        for resolution in 0..=4u8 {
            for animation in 0..=4u16 {
                let r =
                    MeleeResult::from_wire(resolution, animation, 0, 0, 0).expect("in-range bits");
                assert_eq!(r.to_wire(), (resolution, animation, 0, 0, 0));
            }
        }
        assert_eq!(MeleeResult::from_wire(5, 0, 0, 0, 0), None);
        assert_eq!(MeleeResult::from_wire(0, 5, 0, 0, 0), None);
    }

    // The outcome bits are validated against the pinned enums: hitDistortion is a 2-bit field and
    // knockback a 3-bit one on the wire, so out-of-range values cannot round-trip.
    #[test]
    fn outcome_bits_roundtrip() {
        let r = MeleeResult::from_wire(0, 1, 2, 3, 2).expect("in-range bits");
        assert_eq!(r.resolution, ActionResolution::Hit);
        assert_eq!(r.animation, AttackAnimation::LeftAttack);
        assert_eq!(r.info, ActionInfo::CRITICAL_HIT, "CriticalHit bit");
        assert_eq!(r.hit_distortion, HitDistortion::Heavy);
        assert_eq!(r.knockback, KnockbackLevel::Level2);
    }

    // from_wire(to_wire(x)) == x for a table of non-zero outcomes: the lossless property the
    // snapshot contract relies on.
    #[test]
    fn melee_result_to_wire_is_lossless() {
        let cases = [
            MeleeResult::from_wire(0, 1, 2, 3, 2).expect("in-range bits"),
            MeleeResult::from_wire(4, 4, 3, 2, 7).expect("in-range bits"),
            MeleeResult::from_wire(1, 2, 1, 1, 5).expect("in-range bits"),
        ];
        for r in cases {
            let (resolution, animation, info, hit_distortion, knockback) = r.to_wire();
            assert_eq!(
                MeleeResult::from_wire(resolution, animation, info, hit_distortion, knockback),
                Some(r)
            );
        }
    }

    #[test]
    fn result_outcome_from_wire_validates_the_enums() {
        let o = ResultOutcome::from_wire(0, 3, 3, 2).expect("in-range bits");
        assert_eq!(o.resolution, ActionResolution::Hit);
        assert!(o.info.is_defeated());
        assert!(o.info.is_critical_hit());
        assert_eq!(o.hit_distortion, HitDistortion::Heavy);
        assert_eq!(o.knockback, KnockbackLevel::Level2);
        // Out-of-range enum bits read as "no outcome", not a zeroed one.
        assert_eq!(ResultOutcome::from_wire(5, 0, 0, 0), None);
        assert_eq!(ResultOutcome::from_wire(0, 0, 4, 0), None);
        assert_eq!(ResultOutcome::from_wire(0, 0, 0, 8), None);
    }

    #[test]
    fn wire_values_match_lsb_enums() {
        assert_eq!(ActionResolution::from_wire(0), Some(ActionResolution::Hit));
        assert_eq!(
            ActionResolution::from_wire(4),
            Some(ActionResolution::Block)
        );
        assert_eq!(ActionResolution::from_wire(5), None);
        assert_eq!(
            AttackAnimation::from_wire(0),
            Some(AttackAnimation::RightAttack)
        );
        assert_eq!(AttackAnimation::from_wire(4), Some(AttackAnimation::Throw));
        assert_eq!(AttackAnimation::from_wire(5), None);
    }
}
