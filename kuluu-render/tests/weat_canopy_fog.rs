//! Pins the retail policy the cloud canopy's fog lane encodes, against the user's DATs.
//! Skips without a retail install.

use ffxi_dat::chunk::{walk_tree, ChunkNode};
use ffxi_dat::generator::Generator;
use ffxi_dat::weather::collect_weather_records;
use ffxi_dat::{ChunkKind, DatRoot};
use kuluu_render::zone_clouds::CLOUD_MIN_RIM;

/// The two outdoor zone DATs the sky work was measured against: `f_la` and `f_or`.
const ZONE_DATS: [u32; 2] = [202, 209];

fn zone_bytes(file_id: u32) -> Option<Vec<u8>> {
    let root = DatRoot::from_env_or_default().ok()?;
    let location = root.resolve(file_id).ok()?;
    std::fs::read(location.path_under(&root)).ok()
}

fn fourcc(name: &[u8; 4]) -> String {
    String::from_utf8_lossy(name).trim_end().to_string()
}

fn collect_weat_tags<'a>(node: &'a ChunkNode<'a>, out: &mut Vec<(String, &'a ChunkNode<'a>)>) {
    for child in &node.children {
        if child.chunk.kind != ChunkKind::Rmp as u8 {
            continue;
        }
        if child.chunk.name == *b"weat" {
            for tag in &child.children {
                if tag.chunk.kind == ChunkKind::Rmp as u8 {
                    out.push((fourcc(&tag.chunk.name), tag));
                }
            }
        }
        collect_weat_tags(child, out);
    }
}

fn canopy_fog_enabled(tag: &ChunkNode, name: &[u8; 4]) -> Option<bool> {
    tag.children
        .iter()
        .find(|c| c.chunk.kind == ChunkKind::Generator as u8 && c.chunk.name == *name)
        .and_then(|c| {
            Generator::parse_cloud_generator(*name, c.chunk.data)
                .ok()
                .flatten()
        })
        .map(|def| def.fog_enabled)
}

/// research/XIClient CMoElem.cpp:542-543 gates fog on a per-generator bit, so the fix for
/// kuluu-grbo is NOT "sky layers skip fog" — the overcast haze sheet is deliberately fogged.
/// Blanket-exempting the canopy would break exactly the weathers the bead reports.
#[test]
fn cld1_never_fogs_and_the_overcast_cld2_always_does() {
    let mut checked_cld1 = 0;
    let mut checked_cld2 = 0;
    for file_id in ZONE_DATS {
        let Some(bytes) = zone_bytes(file_id) else {
            eprintln!("zone DAT {file_id} unavailable; skipping");
            return;
        };
        let tree = walk_tree(&bytes);
        let mut tags = Vec::new();
        collect_weat_tags(&tree, &mut tags);
        assert!(!tags.is_empty(), "zone DAT {file_id} authors no weat tags");

        for (tag, node) in tags {
            if let Some(fog) = canopy_fog_enabled(node, b"cld1") {
                assert!(!fog, "{file_id} weat/{tag} cld1 is fogged");
                checked_cld1 += 1;
            }
            // The overcast family carries its cloud structure in RGB, which fog replaces
            // outright — so cld2 is authored as a fogged haze sheet there and only there.
            let overcast = matches!(tag.as_str(), "clod" | "mist" | "thdr");
            if let Some(fog) = canopy_fog_enabled(node, b"cld2") {
                assert_eq!(fog, overcast, "{file_id} weat/{tag} cld2 fog bit flipped");
                checked_cld2 += 1;
            }
        }
    }
    assert!(checked_cld1 > 0 && checked_cld2 > 0, "no canopies examined");
}

/// The WHY behind the lane: at the draw distance the client ships with, the canopy rim sits
/// past every fog distance the zone's own 0x2F records author, so fog can only ever replace
/// its colour with the horizon tint rather than blend toward it.
///
/// The rim is fixed at every draw distance, so this holds at every preset the menu
/// offers — but it is still only a supporting argument: the per-generator fog bit,
/// not the rim, is what keeps cld1 unfogged.
#[test]
fn every_authored_fog_distance_is_far_inside_the_canopy_rim() {
    let rim = CLOUD_MIN_RIM;
    for file_id in ZONE_DATS {
        let Some(bytes) = zone_bytes(file_id) else {
            eprintln!("zone DAT {file_id} unavailable; skipping");
            return;
        };
        let records = collect_weather_records(&bytes);
        assert!(
            !records.is_empty(),
            "zone DAT {file_id} has no 0x2F records"
        );
        for record in records {
            assert!(
                record.max_fog_dist_landscape < rim,
                "{file_id} minute {} fog distance {} reaches the canopy rim {rim}",
                record.time_minutes,
                record.max_fog_dist_landscape,
            );
        }
    }
}
