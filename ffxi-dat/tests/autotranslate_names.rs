use ffxi_dat::{
    archive::open_test_install, autotranslate_names::InstalledNames, item_dat::ItemTable,
    KeyItemTable,
};
use ffxi_proto::autotranslate::{decode_with, NameResolver};

const MANDAU_TAG: [u8; 6] = [0xFD, 0x07, 0x02, 0x47, 0x5F, 0xFD];
const ZERUHN_REPORT_TAG: [u8; 6] = [0xFD, 0x15, 0x02, 0xFF, 0x01, 0xFD];

#[test]
fn installed_names_override_only_item_families() {
    let Some(root) = open_test_install() else {
        return;
    };
    let names = InstalledNames::open_from_root(&root);
    let expected = match root.profile().name() {
        "horizonxi-2023" => "{Onion Greataxe}",
        "retail-2026-09" => "{Mandau}",
        other => {
            eprintln!("unmeasured profile: {other}");
            return;
        }
    };
    assert_eq!(decode_with(&MANDAU_TAG, &names), expected);
    assert_eq!(decode_with(&ZERUHN_REPORT_TAG, &names), "{Zeruhn report}");
    assert_eq!(
        names.key_item_name(395).as_deref(),
        Some("map of the Zeruhn Mines")
    );
    assert_eq!(
        decode_with(&[0xFD, 0x02, 0x02, 0x01, 0x01, 0xFD], &names),
        "{Nice to meet you.}"
    );
    let items = ItemTable::open_from_root(&root);
    for (id, name) in [
        (0x2001, "Harlequin Head"),
        (0x7000, "Maze Tabula M01"),
        (0xF001, "Rabbit"),
        (0x7403, "Rabbit Ins. I"),
    ] {
        assert_eq!(items.name(id).as_deref(), Some(name));
    }
    assert!(items.skipped().is_empty(), "{:?}", items.skipped());
    assert!(!KeyItemTable::open_from_root(&root).is_empty());
}
