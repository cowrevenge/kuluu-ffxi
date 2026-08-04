//! Zone-local footstep sound tables (`fses` walk, `fser` run).
//!
//! A step's sound is chosen from the terrain under the foot plus two bytes off
//! the actor's equipped-feet CIB, composed into a 4-char DatId that indexes a
//! table of 0x3D sound pointers held in the zone DAT.
//!
//! The composition below is **measured from the retail install**, not taken from
//! a reference. research/cexi-docs/sounds/footsteps.md describes the shape and is
//! tier 4; it reads the terrain digit as a decimal int and flags what happens
//! past terrain 9 as an open question. The shipped bytes answer it: the digit is
//! a hex nibble. Measured over the first 60 zones in `ZONE_DAT_TABLE` — 22,596
//! `fses` sound pointers — every name is `'0'`, a terrain nibble in `1..=a`, a
//! base-36 digit in `1..=r`, and `'1'..='3'`. Against 13,720 CIBs the two
//! footstep bytes land in exactly the matching ranges: `footstep_material`
//! `0..=27` (plus 35 and a 255 sentinel) and `footstep_size` `{0,1,2}` (plus
//! 255), so `material` is the base-36 digit and `size + 1` the last char.

use std::collections::HashMap;

use crate::chunk::ChunkNode;
use crate::kind::ChunkKind;
use crate::sep::Sep;

/// Walk table directory name.
pub const WALK_TABLE: [u8; 4] = *b"fses";
/// Run table directory name. Same entry names as [`WALK_TABLE`], different
/// sound ids — `0111` resolves to 100001 walking and 100011 running.
pub const RUN_TABLE: [u8; 4] = *b"fser";

/// `footstep_material` / `footstep_size` sentinel for an item that carries no
/// footstep data. 10,826 of 13,720 CIBs use it.
pub const FOOTSTEP_NONE: u8 = 0xFF;

/// Measured span of the base-36 material digit: `'1'..'9'` then `'a'..'r'`, the
/// 27 distinct third characters present in the shipped tables.
const MATERIAL_MIN: u8 = 1;
const MATERIAL_MAX: u8 = 27;

/// Terrain nibble span. 0 is the generic `Object` surface and ships no entry.
const TERRAIN_MIN: u8 = 1;
const TERRAIN_MAX: u8 = 10;

/// The last character is `size + 1`, so a size of 0 is authored as `'1'`.
const SIZE_MAX: u8 = 2;
const SIZE_CHAR_BIAS: u8 = 1;

const RADIX: u32 = 36;

fn digit36(v: u8) -> Option<u8> {
    char::from_digit(u32::from(v), RADIX).map(|c| c as u8)
}

/// The `fses`/`fser` entry name for a step, or `None` when any component is
/// outside the range the tables author (including the [`FOOTSTEP_NONE`]
/// sentinel, which is how an item says "no footstep").
pub fn footstep_dat_id(terrain: u8, material: u8, size: u8) -> Option<[u8; 4]> {
    if !(TERRAIN_MIN..=TERRAIN_MAX).contains(&terrain)
        || !(MATERIAL_MIN..=MATERIAL_MAX).contains(&material)
        || size > SIZE_MAX
    {
        return None;
    }
    Some([
        b'0',
        digit36(terrain)?,
        digit36(material)?,
        b'0' + size + SIZE_CHAR_BIAS,
    ])
}

/// One zone's footstep sound pointers, keyed by the [`footstep_dat_id`] name.
#[derive(Debug, Clone, Default)]
pub struct FootstepTables {
    pub walk: HashMap<[u8; 4], u32>,
    pub run: HashMap<[u8; 4], u32>,
}

impl FootstepTables {
    /// Harvest both tables from a walked zone DAT tree. Empty when the zone
    /// ships no `fses` — which retail treats as "this zone has no footsteps"
    /// rather than as an error.
    pub fn from_tree(root: &ChunkNode<'_>) -> Self {
        Self {
            walk: collect_table(root, WALK_TABLE),
            run: collect_table(root, RUN_TABLE),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.walk.is_empty() && self.run.is_empty()
    }

    /// Sound id for a step, falling back to the walk table when the run table
    /// omits an entry the walk table has.
    pub fn sound_id(&self, id: &[u8; 4], running: bool) -> Option<u32> {
        if running {
            self.run.get(id).or_else(|| self.walk.get(id)).copied()
        } else {
            self.walk.get(id).copied()
        }
    }
}

fn collect_table(node: &ChunkNode<'_>, name: [u8; 4]) -> HashMap<[u8; 4], u32> {
    let mut out = HashMap::new();
    collect_into(node, name, &mut out);
    out
}

fn collect_into(node: &ChunkNode<'_>, name: [u8; 4], out: &mut HashMap<[u8; 4], u32>) {
    for child in &node.children {
        if child.chunk.kind == ChunkKind::Rmp as u8 && child.chunk.name == name {
            for entry in &child.children {
                if entry.chunk.kind != ChunkKind::Sep as u8 {
                    continue;
                }
                if let Ok(sep) = Sep::parse(entry.chunk.name, entry.chunk.data) {
                    out.insert(entry.chunk.name, sep.se_id);
                }
            }
        }
        collect_into(child, name, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_the_measured_id_shape() {
        assert_eq!(footstep_dat_id(1, 1, 0), Some(*b"0111"));
        // Terrain is a hex nibble, not a decimal int: terrain 10 is 'a', which is
        // the question research/cexi-docs/sounds/footsteps.md leaves open.
        assert_eq!(footstep_dat_id(10, 1, 0), Some(*b"0a11"));
        // Material runs past 9 into the same base-36 alphabet.
        assert_eq!(footstep_dat_id(1, 10, 0), Some(*b"01a1"));
        assert_eq!(footstep_dat_id(1, 27, 0), Some(*b"01r1"));
        // Last char is size + 1.
        assert_eq!(footstep_dat_id(1, 1, 2), Some(*b"0113"));
    }

    #[test]
    fn rejects_components_the_tables_do_not_author() {
        assert_eq!(footstep_dat_id(0, 1, 0), None, "terrain 0 ships no entry");
        assert_eq!(footstep_dat_id(11, 1, 0), None);
        assert_eq!(footstep_dat_id(1, 0, 0), None);
        assert_eq!(footstep_dat_id(1, 28, 0), None);
        assert_eq!(footstep_dat_id(1, 1, 3), None);
        // The "no footstep data" sentinel must not compose an id.
        assert_eq!(footstep_dat_id(1, FOOTSTEP_NONE, 0), None);
        assert_eq!(footstep_dat_id(1, 1, FOOTSTEP_NONE), None);
    }

    // Retail-byte guard (skips without an install). Pins the composition against
    // the shipped tables: every name they author must be one this function can
    // produce, or the encoding is wrong somewhere.
    #[test]
    fn real_dat_every_shipped_entry_is_reachable_from_the_composition() {
        let Ok(root) = crate::DatRoot::from_env_or_default() else {
            return;
        };
        let reachable: std::collections::HashSet<[u8; 4]> = (TERRAIN_MIN..=TERRAIN_MAX)
            .flat_map(|t| {
                (MATERIAL_MIN..=MATERIAL_MAX)
                    .flat_map(move |m| (0..=SIZE_MAX).filter_map(move |s| footstep_dat_id(t, m, s)))
            })
            .collect();

        let mut zones = 0u32;
        let mut entries = 0u32;
        for &(_zone, file_id) in crate::zone_dat::ZONE_DAT_TABLE.iter().take(60) {
            let Ok(loc) = root.resolve(file_id) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
                continue;
            };
            let tables = FootstepTables::from_tree(&crate::chunk::walk_tree(&bytes));
            if tables.walk.is_empty() {
                continue;
            }
            zones += 1;
            for name in tables.walk.keys() {
                entries += 1;
                assert!(
                    reachable.contains(name),
                    "shipped entry {:?} cannot be composed",
                    String::from_utf8_lossy(name)
                );
            }
        }
        assert!(zones > 20, "expected a real corpus, saw {zones} zones");
        assert!(
            entries > 1000,
            "expected real tables, saw {entries} entries"
        );
    }

    // The two tables are the walk/run split, not duplicates.
    #[test]
    fn real_dat_walk_and_run_differ() {
        let Ok(root) = crate::DatRoot::from_env_or_default() else {
            return;
        };
        let differs = crate::zone_dat::ZONE_DAT_TABLE
            .iter()
            .take(60)
            .any(|&(_z, f)| {
                let Ok(loc) = root.resolve(f) else {
                    return false;
                };
                let Ok(bytes) = std::fs::read(loc.path_under(&root)) else {
                    return false;
                };
                let t = FootstepTables::from_tree(&crate::chunk::walk_tree(&bytes));
                !t.walk.is_empty()
                    && !t.run.is_empty()
                    && t.walk
                        .iter()
                        .any(|(k, v)| t.run.get(k).is_some_and(|r| r != v))
            });
        assert!(differs, "fser never differs from fses — check the harvest");
    }
}
