// vendor/server/src/map/enums/action/category.h ActionCategory BasicAttack — `action.cmd_no`, 4 bits.
pub const CATEGORY_BASIC_ATTACK: u8 = 1;

// vendor/server/src/map/enums/action/info.h - the per-result `info` bits. Defeated means the
// action killed the target (retail flips StatusServer on the same frame as the HP packet, F49);
// CriticalHit is the crit flag that pairs with hitDistortion Heavy.
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

// vendor/server/src/map/packets/s2c/0x028_battle2.cpp GP_SERV_COMMAND_BATTLE2::pack - one
// result block's bits: resolution(3), kind(2), animation(12), info(5), hitDistortion(2),
// knockback(3). A body that carries no result block, or that ends mid-block, has none of them
// at all: `resolution == 0` is `Hit`, so absence must not be spelled as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeleeResult {
    pub resolution: ActionResolution,
    pub animation: AttackAnimation,
    /// vendor/server/src/map/enums/action/info.h - bit 1 `Defeated` (the action killed the
    /// target), bit 2 `CriticalHit`. Retail flips StatusServer on the same frame as the HP
    /// packet when Defeated is set; finding F49.
    pub info: u8,
    /// vendor/server/src/map/enums/action/hit_distortion.h - 0 None, 1 Light, 2 Medium,
    /// 3 Heavy (the crit case; drives `ldam`, F54).
    pub hit_distortion: u8,
    /// vendor/server/src/map/enums/action/knockback.h - 0 none .. 7 level 7. Any non-zero
    /// level plays `sway` alongside the damage reaction; finding F52.
    pub knockback: u8,
    /// The result's `kind` bits, uninterpreted.
    pub kind: u8,
}

impl MeleeResult {
    pub fn from_wire(
        resolution: u8,
        animation: u16,
        info: u8,
        hit_distortion: u8,
        knockback: u8,
        kind: u8,
    ) -> Option<Self> {
        Some(Self {
            resolution: ActionResolution::from_wire(resolution)?,
            animation: AttackAnimation::from_wire(animation)?,
            info,
            hit_distortion,
            knockback,
            kind,
        })
    }

    pub fn to_wire(self) -> (u8, u16) {
        (self.resolution.to_wire(), self.animation.to_wire())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_roundtrips_through_melee_result() {
        for resolution in 0..=4u8 {
            for animation in 0..=4u16 {
                let r = MeleeResult::from_wire(resolution, animation, 0, 0, 0, 0)
                    .expect("in-range bits");
                assert_eq!(r.to_wire(), (resolution, animation));
            }
        }
        assert_eq!(MeleeResult::from_wire(5, 0, 0, 0, 0, 0), None);
        assert_eq!(MeleeResult::from_wire(0, 5, 0, 0, 0, 0), None);
    }

    // The outcome bits ride through unvalidated: the bit reader already bounds them to their
    // field widths (info 5, hitDistortion 2, knockback 3, kind 2).
    #[test]
    fn outcome_bits_roundtrip() {
        let r = MeleeResult::from_wire(0, 1, 2, 3, 2, 1).expect("in-range bits");
        assert_eq!(r.resolution, ActionResolution::Hit);
        assert_eq!(r.animation, AttackAnimation::LeftAttack);
        assert_eq!(r.info, 2, "CriticalHit bit");
        assert_eq!(r.hit_distortion, 3, "Heavy");
        assert_eq!(r.knockback, 2, "level 2");
        assert_eq!(r.kind, 1);
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
