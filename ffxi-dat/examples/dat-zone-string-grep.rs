//! Grep a zone's dialog string table, reporting entry indexes and the fishing
//! message offset each one would be if it is a fishing line.
//!
//! usage: cargo run -p ffxi-dat --example dat-zone-string-grep <needle> [zone_id ...]
//! With no zone ids, scans every zone that has a string DAT.

use std::process::ExitCode;

use ffxi_dat::dmsg::StringDat;
use ffxi_dat::zone_dat::zone_id_to_string_file_id;
use ffxi_dat::DatRoot;

const MAX_ZONE_ID: u16 = 300;

fn load(root: &DatRoot, zone: u16) -> Option<StringDat> {
    let file_id = zone_id_to_string_file_id(zone)?;
    let loc = root.resolve(file_id).ok()?;
    let bytes = std::fs::read(loc.path_under(root)).ok()?;
    StringDat::parse(&bytes).ok()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(needle) = args.first() else {
        eprintln!("usage: dat-zone-string-grep <needle> [zone_id ...]");
        return ExitCode::from(1);
    };
    let zones: Vec<u16> = if args.len() > 1 {
        args[1..].iter().filter_map(|s| s.parse().ok()).collect()
    } else {
        (0..=MAX_ZONE_ID).collect()
    };

    let root = match DatRoot::from_env_or_default() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("no FFXI install: {e}");
            return ExitCode::from(1);
        }
    };

    let needle_lc = needle.to_lowercase();
    for zone in zones {
        let Some(dat) = load(&root, zone) else {
            continue;
        };
        for index in 0..dat.len() {
            let Some(text) = dat.text(index) else {
                continue;
            };
            if !text.to_lowercase().contains(&needle_lc) {
                continue;
            }
            let is_menu = dat.menu(index).is_some();
            println!(
                "zone {zone} entry {index}{}: {:?}",
                if is_menu { " [MENU]" } else { "" },
                text.chars().take(160).collect::<String>()
            );
        }
    }
    ExitCode::SUCCESS
}
