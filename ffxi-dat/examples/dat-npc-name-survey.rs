use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::ExitCode;

use ffxi_dat::npc_names::{split_id, NpcNameTable};
use ffxi_dat::DatRoot;

const LSB_NPC_LIST_SQL: &str = "vendor/server/sql/npc_list.sql";

const NPC_LIST_INSERT_PREFIX: &str = "INSERT INTO `npc_list` VALUES (";

const SAMPLE_ROWS: usize = 12;

// Score both NPC-name addressing schemes against LSB's npc_list.sql: the record index the low
// 12 bits of an entity id used to be read as, versus the id each record embeds at 0x1C. Every
// disagreement is then classified so era skew (kuluu-j0nd: the vendored LSB pin is newer than
// this install's client) can be told apart from a real lookup bug.
#[derive(Default)]
struct Tally {
    rows: usize,
    new_ok: usize,
    new_wrong: usize,
    new_none: usize,
    old_ok: usize,
    old_wrong: usize,
    old_none: usize,
    // Rows where the two schemes return different names: every row outside this subset scores
    // identically for both, so this is where the whole verdict is decided.
    contested: usize,
    contested_new_ok: usize,
    contested_old_ok: usize,
    ids_embedded: usize,
    ids_duplicated: usize,
    ids_duplicated_named: usize,
    flips: Vec<Flip>,
}

struct Flip {
    zone_id: u16,
    npc_id: u32,
    expected: String,
    got: Option<String>,
    class: FlipClass,
    expected_name_elsewhere: Option<(usize, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum FlipClass {
    // The id is on more than one record and a duplicate carries the LSB name: our first-named
    // -wins tie-break picked the wrong one. This is the only class that is our bug.
    DuplicateIdTieBreak,
    // No record claims the id, and the record the old scheme read embeds no id at all, so the
    // id map cannot see it. Recoverable without trusting a shifted index.
    IdAbsentSlotRecordHasNoId,
    // No record claims the id, and the record the old scheme read embeds a *different* id: the
    // table itself says that name belongs to another entity.
    IdAbsentSlotRecordClaimsAnotherId,
    // Another record claims the id under a different name.
    IdClaimedByAnotherRecord,
}

impl FlipClass {
    fn label(self) -> &'static str {
        match self {
            Self::DuplicateIdTieBreak => "duplicate id, tie-break picked the wrong record",
            Self::IdAbsentSlotRecordHasNoId => "id absent, slot record embeds no id",
            Self::IdAbsentSlotRecordClaimsAnotherId => "id absent, slot record claims another id",
            Self::IdClaimedByAnotherRecord => "id claimed by a different record",
        }
    }
}

fn next_sql_string(chars: &mut std::str::Chars<'_>) -> Option<String> {
    chars.by_ref().find(|&c| c == '\'')?;
    let mut out = String::new();
    loop {
        match chars.next()? {
            '\\' => out.push(chars.next()?),
            '\'' => return Some(out),
            c => out.push(c),
        }
    }
}

fn parse_npc_list_row(line: &str) -> Option<(u32, String)> {
    let rest = line.strip_prefix(NPC_LIST_INSERT_PREFIX)?;
    let (id, rest) = rest.split_once(',')?;
    let id: u32 = id.trim().parse().ok()?;
    let mut chars = rest.chars();
    next_sql_string(&mut chars)?;
    let display_name = next_sql_string(&mut chars)?;
    Some((id, display_name))
}

fn old_lookup(table: &NpcNameTable, npc_id: u32) -> Option<&str> {
    let (zone, slot) = split_id(npc_id)?;
    if zone != table.zone_id() {
        return None;
    }
    table.lookup_by_slot(slot)
}

fn names_in(table: &NpcNameTable) -> HashMap<&str, Vec<usize>> {
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for index in 1..table.len().min(usize::from(u16::MAX)) {
        if let Some(name) = table.lookup_by_slot(index as u16) {
            by_name.entry(name).or_default().push(index);
        }
    }
    by_name
}

fn classify(table: &NpcNameTable, npc_id: u32, expected: &str) -> FlipClass {
    let Some((_, slot)) = split_id(npc_id) else {
        return FlipClass::IdClaimedByAnotherRecord;
    };
    let carriers: Vec<usize> = (0..table.len())
        .filter(|&i| table.record_id(i) == Some(npc_id))
        .collect();
    if carriers.len() > 1
        && carriers
            .iter()
            .any(|&i| table.lookup_by_slot(i as u16) == Some(expected))
    {
        return FlipClass::DuplicateIdTieBreak;
    }
    if !carriers.is_empty() {
        return FlipClass::IdClaimedByAnotherRecord;
    }
    if table.record_id(usize::from(slot)).unwrap_or(0) == 0 {
        FlipClass::IdAbsentSlotRecordHasNoId
    } else {
        FlipClass::IdAbsentSlotRecordClaimsAnotherId
    }
}

fn expected_name_elsewhere(
    table: &NpcNameTable,
    names: &HashMap<&str, Vec<usize>>,
    npc_id: u32,
    expected: &str,
) -> Option<(usize, u32)> {
    let index = *names
        .get(expected)?
        .iter()
        .find(|&&i| table.record_id(i) != Some(npc_id))?;
    Some((index, table.record_id(index).unwrap_or(0)))
}

fn dump_zone(table: &NpcNameTable, rows: &[(u32, String)]) {
    println!(
        "== zone {} detail ({} lsb rows)",
        table.zone_id(),
        rows.len()
    );
    println!("    lsb id     lsb name                       dat name (by embedded id) | record carrying the lsb name");
    for (id, expected) in rows {
        let carrier = (0..table.len())
            .find(|&i| table.lookup_by_slot(i as u16) == Some(expected.as_str()))
            .map(|i| format!("record {i} id {:#010x}", table.record_id(i).unwrap_or(0)))
            .unwrap_or_else(|| "absent".to_string());
        println!(
            "    {id:#010x} {expected:<30} {:<25} | {carrier}",
            table.lookup_by_id(*id).unwrap_or("-"),
        );
    }
}

fn main() -> ExitCode {
    let detail_zone: Option<u16> = std::env::args().nth(1).and_then(|a| a.parse().ok());
    let root = match DatRoot::from_env_or_default() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("could not open DAT root: {e}");
            return ExitCode::from(1);
        }
    };
    let sql_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has a workspace parent")
        .join(LSB_NPC_LIST_SQL);
    let sql = match std::fs::read_to_string(&sql_path) {
        Ok(sql) => sql,
        Err(e) => {
            eprintln!("could not read {}: {e}", sql_path.display());
            return ExitCode::from(1);
        }
    };

    let mut by_zone: HashMap<u16, Vec<(u32, String)>> = HashMap::new();
    for (id, name) in sql.lines().filter_map(parse_npc_list_row) {
        let Some((zone, _)) = split_id(id) else {
            continue;
        };
        by_zone.entry(zone).or_default().push((id, name));
    }

    let mut zones: Vec<u16> = by_zone.keys().copied().collect();
    zones.sort_unstable();

    let mut t = Tally::default();
    let mut zones_with_table = 0usize;
    let mut zones_net_worse: Vec<(u16, usize, usize)> = Vec::new();
    let mut shift_zones: Vec<(u16, usize, usize)> = Vec::new();
    for zone in zones {
        let Ok(table) = NpcNameTable::open(&root, zone) else {
            continue;
        };
        zones_with_table += 1;
        let names = names_in(&table);

        let (mut zone_new_ok, mut zone_old_ok) = (0usize, 0usize);
        let mut rows = by_zone.remove(&zone).unwrap_or_default();
        rows.sort_unstable_by_key(|(id, _)| *id);
        if detail_zone == Some(zone) {
            dump_zone(&table, &rows);
        }
        for (id, expected) in &rows {
            t.rows += 1;
            let new = table.lookup_by_id(*id);
            let old = old_lookup(&table, *id);
            match new {
                Some(n) if n == expected => {
                    t.new_ok += 1;
                    zone_new_ok += 1;
                }
                Some(_) => t.new_wrong += 1,
                None => t.new_none += 1,
            }
            match old {
                Some(n) if n == expected => {
                    t.old_ok += 1;
                    zone_old_ok += 1;
                }
                Some(_) => t.old_wrong += 1,
                None => t.old_none += 1,
            }
            if new != old {
                t.contested += 1;
                t.contested_new_ok += usize::from(new == Some(expected.as_str()));
                t.contested_old_ok += usize::from(old == Some(expected.as_str()));
            }
            if old == Some(expected.as_str()) && new != Some(expected.as_str()) {
                t.flips.push(Flip {
                    zone_id: zone,
                    npc_id: *id,
                    expected: expected.clone(),
                    got: new.map(str::to_string),
                    class: classify(&table, *id, expected),
                    expected_name_elsewhere: expected_name_elsewhere(&table, &names, *id, expected),
                });
            }
        }
        if zone_new_ok < zone_old_ok {
            zones_net_worse.push((zone, zone_old_ok, zone_new_ok));
        }

        let mut carriers: HashMap<u32, Vec<usize>> = HashMap::new();
        for index in 0..table.len() {
            if let Some(id) = table.record_id(index).filter(|&id| id != 0) {
                carriers.entry(id).or_default().push(index);
            }
        }
        t.ids_embedded += carriers.len();
        for indices in carriers.values().filter(|v| v.len() > 1) {
            t.ids_duplicated += 1;
            let named = indices
                .iter()
                .filter(|&&i| table.lookup_by_slot(i as u16).is_some())
                .count();
            t.ids_duplicated_named += usize::from(named > 1);
        }

        let named_records = names.values().map(Vec::len).sum::<usize>();
        let shifted = (0..table.len())
            .filter(|&i| match table.record_id(i) {
                Some(id) if id != 0 => split_id(id).map(|(_, s)| usize::from(s)) != Some(i),
                _ => false,
            })
            .count();
        if shifted > 0 {
            shift_zones.push((zone, shifted, named_records));
        }
    }

    println!("zones with an npc-name table : {zones_with_table}");
    println!("npc_list.sql rows checked    : {}", t.rows);
    println!(
        "  embedded-id addressing     : ok {} / wrong {} / none {}",
        t.new_ok, t.new_wrong, t.new_none
    );
    println!(
        "  record-index addressing    : ok {} / wrong {} / none {}",
        t.old_ok, t.old_wrong, t.old_none
    );
    println!("  right -> not-right flips   : {}", t.flips.len());
    println!(
        "  rows the two schemes answer differently: {} (embedded-id right {}, record-index right {})",
        t.contested, t.contested_new_ok, t.contested_old_ok
    );
    println!(
        "  distinct embedded ids {} / on more than one record {} / on more than one *named* record {}",
        t.ids_embedded, t.ids_duplicated, t.ids_duplicated_named
    );

    let mut per_class: HashMap<FlipClass, Vec<&Flip>> = HashMap::new();
    for flip in &t.flips {
        per_class.entry(flip.class).or_default().push(flip);
    }
    let mut classes: Vec<_> = per_class.into_iter().collect();
    classes.sort_unstable_by_key(|(c, _)| *c);
    for (class, flips) in classes {
        println!("    {:<44} {}", class.label(), flips.len());
        let zones: HashSet<u16> = flips.iter().map(|f| f.zone_id).collect();
        let mut zones: Vec<u16> = zones.into_iter().collect();
        zones.sort_unstable();
        println!("      zones: {zones:?}");
        println!(
            "      lsb name lives on another record in the same table: {}/{}",
            flips
                .iter()
                .filter(|f| f.expected_name_elsewhere.is_some())
                .count(),
            flips.len()
        );
        for f in flips.iter().take(SAMPLE_ROWS) {
            let elsewhere = match f.expected_name_elsewhere {
                Some((index, id)) => format!(" (lsb name is record {index} id {id:#010x})"),
                None => " (lsb name absent from this table)".to_string(),
            };
            println!(
                "      zone {:<4} {:#010x} lsb {:?} -> {:?}{elsewhere}",
                f.zone_id, f.npc_id, f.expected, f.got
            );
        }
    }

    let mut deltas: HashMap<i64, usize> = HashMap::new();
    for f in &t.flips {
        if let Some((_, id)) = f.expected_name_elsewhere {
            *deltas
                .entry(i64::from(id) - i64::from(f.npc_id))
                .or_default() += 1;
        }
    }
    let mut deltas: Vec<(i64, usize)> = deltas.into_iter().collect();
    deltas.sort_unstable_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("  flip id drift (dat id of the lsb name minus the lsb id), most common first:");
    for (delta, n) in deltas.iter().take(SAMPLE_ROWS) {
        println!("    {delta:+} : {n}");
    }

    println!("zones where the new scheme is net worse:");
    for (zone, old_ok, new_ok) in &zones_net_worse {
        println!("    zone {zone:<4} old {old_ok} -> new {new_ok}");
    }
    println!("zones whose records embed an id that is not their own index:");
    for (zone, shifted, named) in shift_zones.iter().take(SAMPLE_ROWS) {
        println!("    zone {zone:<4} {shifted} shifted records, {named} named records");
    }
    println!("    ({} such zones)", shift_zones.len());
    ExitCode::SUCCESS
}
