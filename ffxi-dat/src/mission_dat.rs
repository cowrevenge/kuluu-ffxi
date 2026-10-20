//! Mission-chain name/briefing catalogs, read from the install — never
//! transcribed. Every chain with a client catalog is one fixed-block `d_msg`
//! file ([`crate::dmsg::DmsgTable::parse_mission_family`]) whose entries carry their own id word at
//! +0x1C: the block position IS the wire mission id — measured on all six English
//! tables (ROM/176/67..73.DAT) and re-checked here at every open, so callers key
//! lookups off the catalog's own ids with no hand-built index map. The chain
//! numbering is `MissionLog` (`vendor/server/src/map/enums/mission_log.h`).
//!
//! Which file serves which chain was measured this session against the retail
//! install by matching LSB's scraped per-chain names to block contents; the FR
//! twins live under dir 178 (same block law, translated text) and kuluu stays on
//! the English set like every other GetText read. WoTG and the add-on-era chains
//! have no measured table yet — they report `ChainCatalog::Unmapped`.

use crate::archive::DatRoot;
use crate::dmsg::{DmsgError, DmsgTable};

/// The chain's English catalog is not a fixed-block mission table on this
/// install (unknown chain, or an install without the file).
#[derive(Debug, thiserror::Error)]
pub enum ChainCatalog {
    /// No measured client catalog serves this chain.
    #[error("no measured mission-name catalog maps this chain")]
    Unmapped,
    /// The install's tables do not carry this chain's measured file at all.
    #[error("catalog file {dir}/{file}.DAT absent on this install")]
    Absent { dir: u16, file: u8 },
    /// The table's own id words disagree with its block positions; refusing to
    /// label anything rather than mislabel it.
    #[error("mission-name catalog entries do not carry their own ids (+0x1C word != position)")]
    IdsDisagree,
    #[error(transparent)]
    Read {
        #[from]
        source: std::io::Error,
    },
    #[error(transparent)]
    Parse {
        #[from]
        source: DmsgError,
    },
}

/// The measured `<dir>/<no>` pair of the English catalog serving `chain`, or
/// `None` where retail keeps no fixed-block table for it (WoTG and every chain
/// past CoP had none found).
pub fn chain_catalog_file(chain: u8) -> Option<(u16, u8)> {
    match chain {
        0 => Some((176, 67)), // Sandoria (block 0 = "Smash the Orcish Scouts")
        1 => Some((176, 68)), // Bastok (block 0 = "The Zeruhn Report")
        2 => Some((176, 69)), // Windurst
        3 => Some((176, 70)), // Zilart
        6 => Some((176, 71)), // CoP
        7 => Some((176, 72)), // Assault ops (block N = `xi.assault.mission` id N)
        4 => Some((176, 73)), // ToAU main chain
        _ => None,
    }
}

/// Read one chain's catalog through the install's own tables: locate the measured
/// file, parse it with the mission-family law, and refuse anything whose entries
/// do not label themselves.
pub fn open_chain_catalog(root: &DatRoot, chain: u8) -> Result<DmsgTable, ChainCatalog> {
    let (dir, file) = chain_catalog_file(chain).ok_or(ChainCatalog::Unmapped)?;
    let loc = root
        .locate(dir, file)
        .ok_or(ChainCatalog::Absent { dir, file })?;
    let bytes = std::fs::read(loc.path_under(root))?;
    let catalog = DmsgTable::parse_mission_family(&bytes)?;
    for i in 0..catalog.len() {
        if catalog.entry_strings(i).is_some_and(|s| !s.is_empty()) {
            if catalog.entry_id_word(i) != Some(i as u32) {
                return Err(ChainCatalog::IdsDisagree);
            }
        }
    }
    Ok(catalog)
}

/// The name entry `id` carries, or `None` when the block is blank.
pub fn name(catalog: &DmsgTable, id: usize) -> Option<&str> {
    catalog
        .entry_strings(id)
        .and_then(|s| s.first())
        .filter(|s| !s.is_empty())
        .map(String::as_str)
}

/// The briefing entry `id` carries (`Area: …\nObjective: …` for assault ops,
/// `Mission Orders: …` for story missions), or `None` when the block has none.
pub fn description(catalog: &DmsgTable, id: usize) -> Option<&str> {
    catalog
        .entry_strings(id)
        .and_then(|s| s.get(1))
        .filter(|s| !s.is_empty())
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmapped_chains_report_unmapped() {
        // WoTG (5), the add-ons (9-11), and the Adoulin pair (12/13): none was measured.
        for chain in [5u8, 8, 9, 10, 11, 12, 13] {
            assert!(matches!(chain_catalog_file(chain), None));
        }
    }

    /// Gated on a retail install: the measured names read back through exactly
    /// the chain/id pairing LSB uses, and every non-blank block labels itself.
    #[test]
    fn real_chain_catalogs_name_and_self_label() {
        let Some(root) = crate::archive::open_test_install() else {
            return;
        };
        let sandoria = open_chain_catalog(&root, 0).expect("sandoria catalog");
        assert_eq!(name(&sandoria, 0), Some("Smash the Orcish Scouts"));
        let bastok = open_chain_catalog(&root, 1).expect("bastok catalog");
        assert_eq!(name(&bastok, 0), Some("The Zeruhn Report"));
        let assault = open_chain_catalog(&root, 7).expect("assault catalog");
        // `xi.assault.mission.EXCAVATION_DUTY = 21` (vendor/server/scripts/enum/assault.lua).
        assert_eq!(name(&assault, 21), Some("Excavation Duty"));
        assert!(description(&assault, 21).is_some_and(|d| d.starts_with("Area: Lebros Cavern")));
    }
}
