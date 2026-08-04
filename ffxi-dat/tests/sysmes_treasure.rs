//! Composes the treasure-pool messages out of a real retail install's
//! system-message table, so the wording is pinned to the data rather than to
//! English literals in our source. Self-skips without an install.

use ffxi_dat::sysmes::{treasure, SpanKind, SysMesDat, SysMesParams};

fn open() -> Option<SysMesDat> {
    let root = ffxi_dat::archive::open_test_install()?;
    let dat = SysMesDat::open(&root);
    if dat.is_none() {
        eprintln!("SKIP: install has no usable system-message table at ROM/27/76.DAT");
    }
    dat
}

fn plain(dat: &SysMesDat, index: usize, params: &SysMesParams) -> String {
    dat.message(index, params)
        .unwrap_or_else(|| panic!("system-message entry {index} missing"))
        .to_plain()
}

#[test]
fn find_on_a_defeated_mob() {
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("lizard tail"),
        target_name: Some("Rock Lizard"),
        target_article: true,
        ..Default::default()
    };
    assert_eq!(
        plain(&dat, treasure::FIND_ON, &params),
        "You find a lizard tail on the Rock Lizard."
    );
}

#[test]
fn a_named_mob_keeps_no_article() {
    // NamedFlag clears the "[the /]" alternative's first branch — retail says
    // "on Leaping Lizzy.", not "on the Leaping Lizzy."
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("pair of bounding boots"),
        target_name: Some("Leaping Lizzy"),
        target_article: false,
        ..Default::default()
    };
    assert_eq!(
        plain(&dat, treasure::FIND_ON, &params),
        "You find a pair of bounding boots on Leaping Lizzy."
    );
}

#[test]
fn the_item_name_is_the_only_coloured_span() {
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("pair of bounding boots"),
        target_name: Some("Leaping Lizzy"),
        ..Default::default()
    };
    let line = dat.message(treasure::FIND_ON, &params).expect("entry 16");
    let coloured: Vec<&str> = line.lines[0]
        .iter()
        .filter(|s| s.kind == SpanKind::Item)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(coloured, vec!["pair of bounding boots"]);
    assert!(
        line.lines[0].len() >= 3,
        "the item must sit between text spans so it can take retail's green: {:?}",
        line.lines[0]
    );
}

#[test]
fn find_in_a_container() {
    let Some(dat) = open() else { return };
    let mut params = SysMesParams {
        items: item0("map of the Sanctuary of Zi'Tah"),
        ..Default::default()
    };
    params.strings[1] = Some("treasure chest");
    assert_eq!(
        plain(&dat, treasure::FIND_IN, &params),
        "You find a map of the Sanctuary of Zi'Tah in the treasure chest."
    );
}

#[test]
fn a_lot_announces_its_roll() {
    let Some(dat) = open() else { return };
    let mut params = SysMesParams {
        items: item0("lizard tail"),
        ..Default::default()
    };
    params.strings[3] = Some("Macnugget");
    params.strings[2] = Some("856");
    assert_eq!(
        plain(&dat, treasure::LOT, &params),
        "Macnugget's lot for the lizard tail: 856 points."
    );
}

#[test]
fn someone_else_obtains_the_item() {
    let Some(dat) = open() else { return };
    let mut params = SysMesParams {
        items: item0("pair of bounding boots"),
        ..Default::default()
    };
    params.strings[2] = Some("Macnugget");
    assert_eq!(
        plain(&dat, treasure::OBTAINS_ITEM, &params),
        "Macnugget obtains a pair of bounding boots."
    );
}

#[test]
fn you_obtain_the_item() {
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("lizard tail"),
        ..Default::default()
    };
    assert_eq!(
        plain(&dat, treasure::YOU_OBTAIN, &params),
        "You obtain a lizard tail."
    );
}

#[test]
fn you_cast_lots() {
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("lizard tail"),
        ..Default::default()
    };
    assert_eq!(
        plain(&dat, treasure::YOU_CAST_LOTS, &params),
        "You cast lots for the lizard tail."
    );
}

#[test]
fn found_gil_carries_its_unit() {
    let Some(dat) = open() else { return };
    let mut params = SysMesParams::default();
    params.strings[0] = Some("Macnugget");
    params.numbers[0] = 1_200;
    assert_eq!(
        plain(&dat, treasure::OBTAINS_GIL, &params),
        "Macnugget obtains 1,200 gil."
    );
}

#[test]
fn an_ineligible_winner_loses_the_item_over_two_lines() {
    let Some(dat) = open() else { return };
    let mut params = SysMesParams {
        items: item0("lizard tail"),
        ..Default::default()
    };
    params.strings[2] = Some("Macnugget");
    let line = dat
        .message(treasure::OTHER_INELIGIBLE, &params)
        .expect("entry 15");
    assert_eq!(
        line.lines.len(),
        2,
        "retail prints the loss as its own log line"
    );
    assert_eq!(
        line.to_plain(),
        "Macnugget does not meet the necessary requirements to obtain the lizard tail.\n\
         Lizard tail lost."
    );
}

#[test]
fn you_are_the_ineligible_winner() {
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("lizard tail"),
        ..Default::default()
    };
    assert_eq!(
        plain(&dat, treasure::YOU_INELIGIBLE, &params),
        "You do not meet the requirements to obtain the lizard tail.\nLizard tail lost."
    );
}

#[test]
fn an_unclaimed_item_is_lost() {
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("lizard tail"),
        ..Default::default()
    };
    assert_eq!(
        plain(&dat, treasure::WAS_LOST, &params),
        "A lizard tail was lost."
    );
}

#[test]
fn treasure_entries_carry_a_log_mode() {
    let Some(dat) = open() else { return };
    let params = SysMesParams {
        items: item0("lizard tail"),
        ..Default::default()
    };
    for index in [
        treasure::FIND_ON,
        treasure::FIND_IN,
        treasure::LOT,
        treasure::OBTAINS_ITEM,
        treasure::OBTAINS_GIL,
        treasure::YOU_CAST_LOTS,
        treasure::YOU_OBTAIN,
        treasure::WAS_LOST,
        treasure::YOU_INELIGIBLE,
        treasure::OTHER_INELIGIBLE,
    ] {
        let line = dat.message(index, &params).expect("entry present");
        assert!(
            line.log_mode.is_some(),
            "entry {index} has no 0x1F log mode, so it would render uncoloured"
        );
    }
}

fn item0(name: &str) -> [Option<&str>; ffxi_dat::sysmes::PARAM_SLOTS] {
    let mut items = [None; ffxi_dat::sysmes::PARAM_SLOTS];
    items[0] = Some(name);
    items
}
