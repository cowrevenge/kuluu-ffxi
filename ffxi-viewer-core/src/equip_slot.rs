//! The 16 SAVE_EQUIP_KIND equipment slots, as SceneSnapshot.equipped[16] and
//! MenuKind::EquipSlot index them. Presentation (the retail equipment-window
//! grid and cursor movement) stays in `hud::equipment_screen`.

pub const SLOT_NAMES: [&str; 16] = [
    "Main", "Sub", "Ranged", "Ammo", "Head", "Body", "Hands", "Legs", "Feet", "Neck", "Waist",
    "L.Ear", "R.Ear", "L.Ring", "R.Ring", "Back",
];

// The labels retail prints in each equipment cell, under the icon
// (retail capture 2026-08-04, HorizonXI /check window).
const SLOT_ABBR: [&str; 16] = [
    "Main", "Sub", "Range", "Ammo", "Head", "Body", "Hands", "Legs", "Feet", "Neck", "Waist",
    "Ear1", "Ear2", "Ring1", "Ring2", "Back",
];

// Discriminants are the internal slot indices used by SceneSnapshot.equipped[16]
// and MenuKind::EquipSlot — `repr(u8)` lets `slot as usize` recover that index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EquipmentIndex {
    Main = 0,
    Sub = 1,
    Range = 2,
    Ammo = 3,
    Head = 4,
    Body = 5,
    Hands = 6,
    Legs = 7,
    Feet = 8,
    Neck = 9,
    Waist = 10,
    LeftEar = 11,
    RightEar = 12,
    LeftRing = 13,
    RightRing = 14,
    Back = 15,
}

impl EquipmentIndex {
    pub const ALL: [EquipmentIndex; 16] = [
        Self::Main,
        Self::Sub,
        Self::Range,
        Self::Ammo,
        Self::Head,
        Self::Body,
        Self::Hands,
        Self::Legs,
        Self::Feet,
        Self::Neck,
        Self::Waist,
        Self::LeftEar,
        Self::RightEar,
        Self::LeftRing,
        Self::RightRing,
        Self::Back,
    ];

    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }

    pub fn name(self) -> &'static str {
        SLOT_NAMES[self as usize]
    }

    pub fn abbr(self) -> &'static str {
        SLOT_ABBR[self as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_discriminant_ordered() {
        for (i, &slot) in EquipmentIndex::ALL.iter().enumerate() {
            assert_eq!(slot as usize, i, "ALL must be in discriminant order");
            assert_eq!(EquipmentIndex::from_index(i as u8), Some(slot));
        }
        assert_eq!(EquipmentIndex::from_index(16), None);
    }

    #[test]
    fn slot_names_and_abbr_aligned() {
        assert_eq!(SLOT_NAMES.len(), 16);
        assert_eq!(SLOT_ABBR.len(), 16);
        assert_eq!(SLOT_NAMES[10], "Waist");
        assert_eq!(SLOT_ABBR[10], "Waist");
        assert_eq!(SLOT_ABBR[11], "Ear1", "retail numbers the paired slots");
        assert_eq!(SLOT_ABBR[13], "Ring1");
    }
}
