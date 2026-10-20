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
        // Found by the 2026-10-20 install-wide census below: block 0 reads
        // "Cavernous Maws" = `xi.mission.id.WOTG[0]` (vocab row (5,0)) and every
        // non-blank block labels itself with its position.
        5 => Some((196, 7)), // Wings of the Goddess
        // The add-on micro-chains, same law: each table's block 0 is the chain
        // title ("A Crystalline Prophecy"/"A Moogle Kupo d'Etat"/"A Shantotto
        // Ascension") and matches vocab rows (9,0)/(10,0)/(11,0) verbatim.
        9 => Some((222, 18)),
        10 => Some((223, 12)),
        11 => Some((223, 13)),
        _ => None,
    }
}

/// The id word measured on the one trailing slot each add-on micro-chain keeps
/// past its last numbered mission: block 12 of ROM/222/18.DAT (ACP ids run to
/// 11) and block 15 of ROM/223/13.DAT (ASA ids to 14) both read `To commence
/// …`-style prompt text under this word. No wire id reaches it, so a
/// position-keyed lookup can never surface those rows.
pub const CHAIN_SLOT_SENTINEL_ID_WORD: u32 = 1000;

/// Read one chain's catalog through the install's own tables: locate the measured
/// file, parse it with the mission-family law, and refuse anything whose entries
/// do not label themselves — every non-blank block must carry its position as
/// its id word (or [`CHAIN_SLOT_SENTINEL_ID_WORD`]).
pub fn open_chain_catalog(root: &DatRoot, chain: u8) -> Result<DmsgTable, ChainCatalog> {
    let (dir, file) = chain_catalog_file(chain).ok_or(ChainCatalog::Unmapped)?;
    let loc = root
        .locate(dir, file)
        .ok_or(ChainCatalog::Absent { dir, file })?;
    let bytes = std::fs::read(loc.path_under(root))?;
    let catalog = DmsgTable::parse_mission_family(&bytes)?;
    for i in 0..catalog.len() {
        if catalog.entry_strings(i).is_some_and(|s| !s.is_empty()) {
            if !matches!(
                catalog.entry_id_word(i),
                Some(w) if w == i as u32 || w == CHAIN_SLOT_SENTINEL_ID_WORD
            ) {
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
        // Campaign ops (8) never had a catalog; the Adoulin-era pair (12/13)
        // has tables (see the census doc comment) but they key off wire-encoded
        // id words, not positions, and are deliberately not claimed by this map.
        for chain in [8u8, 12, 13] {
            assert!(matches!(chain_catalog_file(chain), None));
        }
    }

    /// Install-gated census over the four ROM dirs that hold chain catalogs: it
    /// fully parses every fixed-block mission-family table found and prints
    /// entry counts + sample self-labeled blocks, then asserts every pinned
    /// `chain_catalog_file` pair appears in what the sweep discovers. The map
    /// was built by an install-wide sweep on 2026-10-20 (all 256 dirs; ~130
    /// fixed-block tables — mostly per-zone quest text) which found exactly the
    /// eleven pinned files plus: DE/FR twins of each era table, a "The Voracious
    /// Resurgence" set under dir 364 (no `MissionLog` id for it in LSB), and the
    /// Adoulin pair — **293/69** (SoA) and **333/3/4** (RoV) — whose blocks label
    /// themselves with WIRE-encoded ids, not positions: SoA words read
    /// `0x6E + 2*id` (`__The Geomagnetron` @ 112 for vocab row (12,1)) and RoV
    /// words read `0x6C + id`, each shared by a chapter title block and its
    /// `__`-prefixed step rows. Those two stay out of the position-keyed map
    /// until keyed lookups off their own words are built; sweeping all four dirs
    /// here keeps that law (and this warning) alive on every run.
    #[test]
    fn census_finds_exactly_the_pinned_family_tables_plus_any_unclaimed_ones() {
        let Ok(resolved) = crate::install::resolve() else {
            eprintln!("SKIP (real-DAT guard)");
            return;
        };
        let rom = resolved.path.join("ROM");
        let mut found: Vec<(u16, u8, usize)> = Vec::new();
        let dirs: Vec<u16> = vec![176, 196, 222, 223];
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(rom.join(dir.to_string())) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let Some(no) = name.strip_suffix(".DAT").and_then(|s| s.parse::<u8>().ok()) else {
                    continue;
                };
                // Two passes: a 0x40-byte peek first — the ROM tree is gigabytes
                // and only a handful of files carry this header shape.
                let Ok(peek) = std::fs::File::open(entry.path()).map(|mut f| {
                    use std::io::Read;
                    let mut buf = vec![0u8; 0x40];
                    let n = f.read(&mut buf).unwrap_or(0);
                    buf.truncate(n);
                    buf
                }) else {
                    continue;
                };
                if family_header(&peek).is_none() {
                    continue;
                }
                let Ok(bytes) = std::fs::read(entry.path()) else {
                    continue;
                };
                match DmsgTable::parse_mission_family(&bytes) {
                    Ok(table) => {
                        eprintln!(
                            "census {dir}/{no}: entries={} sample={:?}",
                            table.len(),
                            (0..table.len())
                                .filter(|&i| {
                                    table.entry_strings(i).is_some_and(|s| !s.is_empty())
                                })
                                .take(4)
                                .map(|i| {
                                    (
                                        table.entry_id_word(i),
                                        table
                                            .entry_strings(i)
                                            .and_then(|s| s.first().cloned())
                                            .unwrap_or_default(),
                                    )
                                })
                                .collect::<Vec<_>>()
                        );
                        found.push((dir, no, table.len()));
                    }
                    Err(e) => eprintln!("census {dir}/{no}: header shape but parse failed: {e:?}"),
                }
            }
        }
        for (chain, pair) in [
            (0u8, (176u16, 67u8)),
            (1, (176, 68)),
            (2, (176, 69)),
            (3, (176, 70)),
            (4, (176, 73)),
            (5, (196, 7)),
            (6, (176, 71)),
            (7, (176, 72)),
            (9, (222, 18)),
            (10, (223, 12)),
            (11, (223, 13)),
        ] {
            assert_eq!(chain_catalog_file(chain), Some(pair));
            assert!(
                found.iter().any(|(d, f, _)| (*d, *f) == pair),
                "pinned catalog {pair:?} for chain {chain} must be a family table on this install"
            );
        }
    }

    /// The plain (un-XORed) header test for the fixed-block mission layout:
    /// `d_msg` banner, words at 0x14 = {file_size, table_offset, table_size,
    /// block_size, _, num_strings} with table_size == 0. Returns the entry
    /// count so callers know what a full parse would yield.
    fn family_header(bytes: &[u8]) -> Option<usize> {
        if bytes.len() < 0x40 || &bytes[..5] != b"d_msg" {
            return None;
        }
        let word =
            |i: usize| u32::from_le_bytes(bytes[0x14 + i * 4..0x18 + i * 4].try_into().unwrap());
        let (table_size, block_size, num_strings) = (word(2), word(3), word(5));
        if table_size != 0 || !(0x20..=0x4_0000).contains(&block_size) {
            return None;
        }
        let n = usize::try_from(num_strings).ok()?;
        (n > 0 && n <= 50_000).then_some(n)
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
        // The era tables the census claimed: same fixed-block law, self-labeled
        // blocks, names matching the vocab lattice rows article-for-article.
        let wotg = open_chain_catalog(&root, 5).expect("WoTG catalog");
        assert_eq!(name(&wotg, 0), Some("Cavernous Maws"));
        assert_eq!(name(&wotg, 1), Some("Back to the Beginning"));
        let acp = open_chain_catalog(&root, 9).expect("ACP catalog");
        assert_eq!(name(&acp, 0), Some("A Crystalline Prophecy"));
        // LSB re-types the micro-chain titles without their diacritics; the DAT
        // is `Kupo d'Etat` (vocab row (10,0) says "Detat" — one typo neither
        // side can settle except here).
        let amk = open_chain_catalog(&root, 10).expect("AMK catalog");
        assert_eq!(name(&amk, 0), Some("A Moogle Kupo d'Etat"));
        let asa = open_chain_catalog(&root, 11).expect("ASA catalog");
        assert_eq!(name(&asa, 0), Some("A Shantotto Ascension"));
    }
}
