include!(concat!(env!("OUT_DIR"), "/spell_animation_table.rs"));
include!(concat!(env!("OUT_DIR"), "/ability_animation_table.rs"));

// research/xim SpellTables.kt / AbilityTable.kt: a skill's completion animation
// is a global file-table entry at base_offset + per-skill animation index, where
// the per-skill index is the `animation` column of spell_list.sql / abilities.sql.
const SPELL_FILE_TABLE_OFFSET: u32 = 0xAF0;
const ABILITY_FILE_TABLE_OFFSET: u32 = 0x113C;
const TRUST_FILE_ID: u32 = 0xE9B;
const TRUST_SPELL_ID_MIN: u16 = 896;

fn lookup(table: &[(u16, u16)], id: u16) -> Option<u16> {
    table
        .binary_search_by_key(&id, |&(k, _)| k)
        .ok()
        .map(|i| table[i].1)
}

// Every completion effect is `<table base> + animation index`, and s2c 0x028 carries that index
// per result — LSB fills it straight from the action's own animation column (magic_state.cpp,
// charentity.cpp:1602/1923). The scraped `*_ANIMATION` tables hold the same column keyed by
// action id, and stand in only when a truncated body carried no result to read it from.
//
// The action id is NOT the index. research/xim AbilityTable.kt getAnimationId adds
// `animInfo.animationId` for every branch, and the two diverge widely (Sneak Attack is ability
// 44 / animation 17, Mighty Strikes 16 / 33) — keying by id lands on an unrelated ability's DAT.
pub fn spell_file_id(spell_id: u32, animation: Option<u16>) -> Option<u32> {
    let id = u16::try_from(spell_id).ok()?;
    if id >= TRUST_SPELL_ID_MIN {
        return Some(TRUST_FILE_ID);
    }
    let index = animation.or_else(|| lookup(SPELL_ANIMATION, id))?;
    Some(SPELL_FILE_TABLE_OFFSET + index as u32)
}

pub fn ability_file_id(ability_id: u32, animation: Option<u16>) -> Option<u32> {
    let index = animation.or_else(|| lookup(ABILITY_ANIMATION, u16::try_from(ability_id).ok()?))?;
    Some(ABILITY_FILE_TABLE_OFFSET + index as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_nonempty_and_sorted() {
        assert!(SPELL_ANIMATION.len() >= 400);
        assert!(ABILITY_ANIMATION.len() >= 100);
        assert!(SPELL_ANIMATION.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(ABILITY_ANIMATION.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn packet_animation_wins_over_the_scraped_column() {
        assert_eq!(
            spell_file_id(1, Some(200)),
            Some(SPELL_FILE_TABLE_OFFSET + 200)
        );
        assert_eq!(
            ability_file_id(44, Some(17)),
            Some(ABILITY_FILE_TABLE_OFFSET + 17)
        );
    }

    #[test]
    fn a_result_less_body_falls_back_to_the_scraped_column() {
        let index = lookup(SPELL_ANIMATION, 1).unwrap();
        assert_eq!(
            spell_file_id(1, None),
            Some(SPELL_FILE_TABLE_OFFSET + index as u32)
        );
        // Sneak Attack: abilities.sql animation 17, not its ability id 44.
        assert_eq!(
            ability_file_id(44, None),
            Some(ABILITY_FILE_TABLE_OFFSET + 17)
        );
    }

    #[test]
    fn trust_spells_share_one_file() {
        assert_eq!(spell_file_id(900, Some(5)), Some(TRUST_FILE_ID));
    }

    #[test]
    fn out_of_range_is_none() {
        assert_eq!(spell_file_id(0xF_FFFF, None), None);
        assert_eq!(ability_file_id(0xF_FFFF, None), None);
    }
}
