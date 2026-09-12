//! Pins the count==5 string-table layout (2=log name singular, 3=log name
//! plural) and the equipment tail against a real install's item DATs on
//! either block layout, including a log name that is not a case-fold of the
//! display name. Self-skips without an install.

use ffxi_dat::item_dat::ItemTable;

const FIRE_CRYSTAL: u16 = 4096;
const CHAMOMILE: u16 = 636;
const DEFENDING_RING: u16 = 13566;
const CESTI: u16 = 16385;
const GIL: u16 = 0xFFFF;
const ICON_SIDE: u32 = 32;

fn open() -> Option<ItemTable> {
    let root = ffxi_dat::archive::open_test_install()?;
    let table = ItemTable::open(root.root());
    if table.is_empty() {
        eprintln!("SKIP: install has no usable item DATs");
        return None;
    }
    assert!(
        table.skipped().is_empty(),
        "item DATs skipped: {:?}",
        table.skipped()
    );
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

#[test]
fn armor_tail_decodes_level_slots_races_and_jobs() {
    let Some(table) = open() else { return };
    let item = table
        .lookup(DEFENDING_RING)
        .expect("defending ring decodes");
    assert_eq!(item.name, "Defending Ring");
    assert_eq!(item.level, 70);
    assert_eq!(item.slot_mask, 0x6000);
    assert_eq!(item.races_mask, 0x1FE);
    assert_eq!(item.jobs_mask, 0x007F_FFFE);
}

#[test]
fn weapon_tail_decodes_level_slots_and_jobs() {
    let Some(table) = open() else { return };
    let item = table.lookup(CESTI).expect("cesti decodes");
    assert_eq!(item.name, "Cesti");
    assert_eq!(item.level, 1);
    assert_eq!(item.slot_mask, 1);
    assert_eq!(item.jobs_mask, 0x0008_0BE6);
}

#[test]
fn the_currency_dat_resolves_gil() {
    let Some(table) = open() else { return };
    let item = table.lookup(GIL).expect("gil decodes");
    assert_eq!(item.name, "Gil");
}

#[test]
fn icons_decode_at_retail_size() {
    let Some(table) = open() else { return };
    let icon = table.icon(FIRE_CRYSTAL).expect("fire crystal icon decodes");
    assert_eq!((icon.width, icon.height), (ICON_SIDE, ICON_SIDE));
}
