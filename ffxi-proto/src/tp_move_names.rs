//! Names for the `<skill>` token of a battle message — weapon skills and monster TP moves.
//!
//! One id space split across two LSB tables: ids < 256 are weapon skills PCs and mobs share
//! (`weapon_skills`), ids >= 256 are monster-only TP moves (`mob_skills`). LSB itself makes
//! that split — a mob skill under 256 finishes as `ActionCategory::SkillFinish`, at or above
//! it as `MobSkillFinish` (vendor/server/src/map/entities/battleentity.cpp:2655-2662).
//!
//! Retail reads these from its own table (`ROM/27/80.DAT`, a xor-0x80 string table — see
//! research/xim `MobAbilityTable.kt`), which differs from LSB's snake_case identifiers in
//! punctuation only.

include!(concat!(env!("OUT_DIR"), "/tp_move_names_table.rs"));

pub fn lookup(id: u16) -> Option<&'static str> {
    TP_MOVE_NAMES
        .binary_search_by_key(&id, |&(k, _)| k)
        .ok()
        .map(|i| TP_MOVE_NAMES[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_skills_resolve() {
        assert_eq!(lookup(1), Some("Combo"));
        assert_eq!(lookup(160), Some("Shining Strike"));
    }

    #[test]
    fn monster_tp_moves_resolve() {
        assert_eq!(lookup(584), Some("Uppercut"));
    }

    #[test]
    fn weapon_skill_only_ids_survive_the_merge() {
        // 163 is in weapon_skills but not mob_skills; merging must not drop it.
        assert!(lookup(163).is_some());
    }

    #[test]
    fn unknown_id_returns_none() {
        assert!(lookup(0xFFFF).is_none());
    }

    #[test]
    fn table_is_sorted_and_reasonably_sized() {
        assert!(TP_MOVE_NAMES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(
            TP_MOVE_NAMES.len() >= 2000,
            "TP_MOVE_NAMES.len() = {}",
            TP_MOVE_NAMES.len()
        );
    }
}
