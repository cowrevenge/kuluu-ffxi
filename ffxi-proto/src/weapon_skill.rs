//! Weapon skill type per equippable weapon, scraped from LSB `item_weapon`.

include!(concat!(env!("OUT_DIR"), "/weapon_skill_table.rs"));

/// SKILLTYPE, vendor/server/src/map/entities/battleentity.h:144. Rods *and*
/// bait carry it — LSB's fishing gate tests `getSkillType() != SKILL_FISHING`
/// on both the ranged and ammo slots
/// (vendor/server/src/map/utils/fishingutils.cpp StartFishing).
pub const SKILL_FISHING: u8 = 48;

/// The `item_weapon.skill` of `item_id`, or `None` when the item is not a
/// weapon or carries no skill type.
pub fn lookup(item_id: u16) -> Option<u8> {
    WEAPON_SKILL
        .binary_search_by_key(&item_id, |&(k, _)| k)
        .ok()
        .map(|i| WEAPON_SKILL[i].1)
}

/// A fishing rod or a bait — anything whose weapon skill is Fishing.
pub fn is_fishing_gear(item_id: u16) -> bool {
    lookup(item_id) == Some(SKILL_FISHING)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the scrape against LSB's own fishing tables: every rod in
    /// `vendor/server/sql/fishing_rod.sql` is keyed by item id, so each one must
    /// come back as fishing gear here. A layout change in either dump breaks
    /// this instead of silently emptying the fishing gate.
    #[test]
    fn every_lsb_fishing_rod_reads_as_fishing_gear() {
        let sql = match std::fs::read_to_string("../vendor/server/sql/fishing_rod.sql") {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut checked = 0;
        for line in sql.lines() {
            let Some(rest) = line
                .trim()
                .strip_prefix("INSERT INTO `fishing_rod` VALUES (")
            else {
                continue;
            };
            let Some(id) = rest
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u16>().ok())
            else {
                continue;
            };
            assert!(
                is_fishing_gear(id),
                "fishing_rod.sql rod {id} is not SKILL_FISHING in item_weapon.sql"
            );
            checked += 1;
        }
        assert!(checked > 0, "parsed no rods out of fishing_rod.sql");
    }

    #[test]
    fn a_sword_is_not_fishing_gear() {
        // 16537 = LSB item_weapon 'xiphos', skill 3 (Sword).
        assert_eq!(lookup(16537), Some(3));
        assert!(!is_fishing_gear(16537));
    }
}
