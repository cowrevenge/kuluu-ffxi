//! Inflates the installed `FFXiMain.dll` code section and pins it to its
//! [`KNOWN_CLIENTS`] row: the image hash and length, the stub the entry point
//! runs, and the bytes at a disassembly RVA this tree cites. Self-skips without
//! an install. The unpacked image is never written anywhere.

use ffxi_dat::client_profile::{ClientProfile, KnownClient, FFXIMAIN_DLL, KNOWN_CLIENTS};
use ffxi_dat::pol1::{self, TextImage};
use sha2::{Digest, Sha256};

// FFXiMain.dll: the routine ffxi-proto/src/decode/login.rs cites on
// `ZoneInVoyage::decode` (horizonxi-2023 RVA 0xFA0F7, retail-2026-09 RVA 0xFB3A7)
// reads the ship-timer fields out of the s2c 0x00A LOGIN body
// `ZONE_IN_VOYAGE_READ_OFFSET` bytes past its entry: `mov dx,[ebx+0x7c];
// mov esi,[ebx+0x78]`. Identical bytes on both rows because neither operand
// carries a build-dependent displacement.
const ZONE_IN_VOYAGE_ENTRY_RVA: &[(&str, u32)] = &[
    ("horizonxi-2023", 0x000F_A0F7),
    ("retail-2026-09", 0x000F_B3A7),
];
const ZONE_IN_VOYAGE_READ_OFFSET: u32 = 0xB;
const ZONE_IN_VOYAGE_READ: &[u8] = &[0x66, 0x8B, 0x53, 0x7C, 0x8B, 0x73, 0x78];

fn installed_dll() -> Option<(&'static KnownClient, Vec<u8>)> {
    let root = ffxi_dat::archive::open_test_install()?;
    let profile = ClientProfile::probe(root.root());
    let Some(known) = profile.known else {
        eprintln!("SKIP: installed client is not a KNOWN_CLIENTS row: {profile}");
        return None;
    };
    match std::fs::read(root.root().join(FFXIMAIN_DLL)) {
        Ok(bytes) => Some((known, bytes)),
        Err(e) => {
            eprintln!("SKIP: {} is unreadable: {e}", FFXIMAIN_DLL);
            None
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn the_installed_client_unpacks_to_its_recorded_text_image() {
    let Some((known, dll)) = installed_dll() else {
        return;
    };
    let Some(measured) = known.unpacked_text else {
        eprintln!("SKIP: {} carries no unpacked .text measurement", known.name);
        return;
    };
    let text = pol1::unpack_text(&dll).unwrap_or_else(|e| panic!("{}: {e}", known.name));
    assert_eq!(text.len(), measured.virtual_size as usize, "{}", known.name);
    assert_eq!(sha256_hex(&text), measured.sha256, "{}", known.name);
    assert_eq!(
        pol1::entry_point_rva(&dll),
        Some(measured.pol1_stub_rva),
        "{}",
        known.name
    );
}

#[test]
fn the_cited_zone_in_voyage_rva_holds_the_instructions_it_is_cited_for() {
    let Some((known, dll)) = installed_dll() else {
        return;
    };
    let Some((_, entry)) = ZONE_IN_VOYAGE_ENTRY_RVA
        .iter()
        .find(|(row, _)| *row == known.name)
    else {
        eprintln!("SKIP: {} cites no ZoneInVoyage address", known.name);
        return;
    };
    let image = TextImage::unpack(&dll).unwrap_or_else(|e| panic!("{}: {e}", known.name));
    let rva = entry + ZONE_IN_VOYAGE_READ_OFFSET;
    let found = image
        .at_rva(rva, ZONE_IN_VOYAGE_READ.len())
        .unwrap_or_else(|e| panic!("{}: {e}", known.name));
    assert_eq!(found, ZONE_IN_VOYAGE_READ, "{} {rva:#x}", known.name);
}

#[test]
fn every_cited_row_is_a_known_client() {
    for (row, _) in ZONE_IN_VOYAGE_ENTRY_RVA {
        let known = KNOWN_CLIENTS
            .iter()
            .find(|k| k.name == *row)
            .unwrap_or_else(|| panic!("{row} is not a KNOWN_CLIENTS row"));
        assert!(
            known.unpacked_text.is_some(),
            "{row} cites a code address but records no unpacked .text"
        );
    }
}
