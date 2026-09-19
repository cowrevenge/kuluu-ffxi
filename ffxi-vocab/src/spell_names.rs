include!(concat!(env!("OUT_DIR"), "/spell_names_table.rs"));

pub fn lookup(id: u16) -> Option<&'static str> {
    SPELL_NAMES
        .binary_search_by_key(&id, |&(k, _)| k)
        .ok()
        .map(|i| SPELL_NAMES[i].1)
}

pub fn id_for(name: &str) -> Option<u16> {
    SPELL_NAMES
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|&(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_spells_resolve() {
        assert_eq!(lookup(1), Some("Cure"));

        assert_eq!(lookup(144), Some("Fire"));
    }

    #[test]
    fn names_resolve_back_to_their_id_ignoring_case() {
        assert_eq!(id_for("Cure"), Some(1));
        assert_eq!(id_for("cure ii"), Some(2));
        assert!(id_for("Not A Spell").is_none());
    }

    /// `id_for` takes the first name that matches, so a duplicate would make
    /// the reverse lookup depend on table order.
    #[test]
    fn names_are_unique_case_insensitively() {
        let mut seen: Vec<String> = SPELL_NAMES
            .iter()
            .map(|(_, n)| n.to_ascii_lowercase())
            .collect();
        seen.sort();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len());
    }

    #[test]
    fn unknown_id_returns_none() {
        assert!(lookup(0xFFFF).is_none());
    }

    #[test]
    fn table_size_is_reasonable() {
        assert!(
            SPELL_NAMES.len() >= 500,
            "SPELL_NAMES.len() = {} (expected at least 500)",
            SPELL_NAMES.len()
        );
    }
}
