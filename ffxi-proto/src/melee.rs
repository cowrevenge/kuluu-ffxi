// vendor/server/src/map/enums/action/category.h:31 — `action.cmd_no`, 4 bits.
pub const CATEGORY_BASIC_ATTACK: u8 = 1;

// vendor/server/src/map/enums/action/resolution.h — `result.resolution`, 3 bits in
// vendor/server/src/map/packets/s2c/0x028_battle2.cpp:71.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

// vendor/server/src/map/attack.h:52-59. Set from `attack.GetAnimationID()` into
// `actionResult.animation` (vendor/server/src/map/entities/battleentity.cpp:3007) — for a basic
// attack this is the swing slot, not a skill id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
