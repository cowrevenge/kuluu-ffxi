include!(concat!(env!("OUT_DIR"), "/ability_names_table.rs"));

pub fn lookup(id: u16) -> Option<&'static str> {
    ABILITY_NAMES
        .binary_search_by_key(&id, |&(k, _)| k)
        .ok()
        .map(|i| ABILITY_NAMES[i].1)
}

pub fn id_for(name: &str) -> Option<u16> {
    ABILITY_NAMES
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|&(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_abilities_resolve() {
        assert_eq!(lookup(16), Some("Mighty Strikes"));

        assert_eq!(lookup(22), Some("Invincible"));
    }

    #[test]
    fn names_resolve_back_to_their_id_ignoring_case() {
        assert_eq!(id_for("Flee"), id_for("flee"));
        assert_eq!(id_for("Flee").and_then(lookup), Some("Flee"));
        assert_eq!(id_for("Mighty Strikes"), Some(16));
        assert!(id_for("Not An Ability").is_none());
    }

    /// `id_for` takes the first name that matches, so a duplicate would make
    /// the reverse lookup depend on table order.
    #[test]
    fn names_are_unique_case_insensitively() {
        let mut seen: Vec<String> = ABILITY_NAMES
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
            ABILITY_NAMES.len() >= 400,
            "ABILITY_NAMES.len() = {} (expected at least 400)",
            ABILITY_NAMES.len()
        );
    }
}
