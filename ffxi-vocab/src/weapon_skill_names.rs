//! Weapon skill names, the PC-usable subset of the `<skill>` id space that
//! [`crate::tp_move_names`] merges with monster TP moves. Several mob skills
//! reuse a weapon skill's name, so only this table can answer "which id did
//! the player mean by this name".

include!(concat!(env!("OUT_DIR"), "/weapon_skill_names_table.rs"));

pub fn lookup(id: u16) -> Option<&'static str> {
    WEAPON_SKILL_NAMES
        .binary_search_by_key(&id, |&(k, _)| k)
        .ok()
        .map(|i| WEAPON_SKILL_NAMES[i].1)
}

pub fn id_for(name: &str) -> Option<u16> {
    WEAPON_SKILL_NAMES
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|&(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_weapon_skills_resolve() {
        assert_eq!(id_for("Fast Blade"), Some(32));
        assert_eq!(id_for("fast blade").and_then(lookup), Some("Fast Blade"));
        assert!(id_for("Uppercut").is_none());
    }

    /// `id_for` takes the first name that matches, so a duplicate would make
    /// the reverse lookup depend on table order.
    #[test]
    fn names_are_unique_case_insensitively() {
        let mut seen: Vec<String> = WEAPON_SKILL_NAMES
            .iter()
            .map(|(_, n)| n.to_ascii_lowercase())
            .collect();
        seen.sort();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len());
    }

    /// Every weapon skill must also survive the merge into the shared name
    /// table the battle-message `<skill>` token reads.
    #[test]
    fn every_weapon_skill_is_present_in_the_merged_table() {
        for &(id, name) in WEAPON_SKILL_NAMES {
            assert_eq!(crate::tp_move_names::lookup(id), Some(name), "id {id}");
        }
    }
}
