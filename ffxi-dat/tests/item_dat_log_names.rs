//! Pins the count==5 string-table layout (2=log name singular, 3=log name
//! plural) against a real retail install's item DATs, including a log name
//! that is not a case-fold of the display name. Self-skips without an install.

use ffxi_dat::item_dat::ItemTable;

const FIRE_CRYSTAL: u16 = 4096;
const CHAMOMILE: u16 = 636;
const DEFENDING_RING: u16 = 13566;

fn open() -> Option<ItemTable> {
    let root = ffxi_dat::archive::open_test_install()?;
    let table = ItemTable::open(root.root());
    if table.is_empty() {
        eprintln!("SKIP: install has no usable item DATs");
        return None;
    }
    Some(table)
}

#[test]
fn a_common_noun_logs_lowercase() {
    let Some(table) = open() else { return };
    let item = table.lookup(FIRE_CRYSTAL).expect("fire crystal decodes");
    assert_eq!(item.name, "Fire Crystal");
    assert_eq!(item.log_name, "fire crystal");
    assert_eq!(item.log_name_plural, "fire crystals");
}

#[test]
fn a_log_name_is_its_own_string_not_a_case_fold() {
    let Some(table) = open() else { return };
    let item = table.lookup(CHAMOMILE).expect("chamomile decodes");
    assert_eq!(item.name, "Chamomile");
    assert_eq!(item.log_name, "sprig of chamomile");
}

#[test]
fn equipment_carries_log_names_too() {
    let Some(table) = open() else { return };
    let item = table
        .lookup(DEFENDING_RING)
        .expect("defending ring decodes");
    assert_eq!(item.log_name, "defending ring");
}
