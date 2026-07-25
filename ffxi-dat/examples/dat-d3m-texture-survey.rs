use std::collections::HashMap;
use std::process::ExitCode;

use ffxi_dat::{chunk::walk, kind::ChunkKind, texture, DatRoot};

const DEFAULT_LAST_FILE_ID: u32 = 12_000;

const SAMPLE_ROWS: usize = 20;

// Score how a StaticMesh particle's 16-byte qualified texture name resolves against the Img
// chunks sharing its DAT file. research/xim DatResource.kt:488-493 is the authority: full
// (namespace, local) match, then local-only; never the chunk DatId. A link is scored by the
// *identity* of the Img chunk each key lands on — its ordinal within the file — because two
// keys can both hit and still point at different textures, which is the case that changes what
// the screen draws.
type ChunkOrdinal = usize;

#[derive(Default)]
struct FileIndex {
    by_dat_id: HashMap<[u8; 4], ChunkOrdinal>,
    by_qualified: HashMap<(String, String), ChunkOrdinal>,
    by_local: HashMap<String, ChunkOrdinal>,
    // The same two name maps built as they were before the flag widening, i.e. only 0xA1 Imgs
    // claim a name. Isolates how much of the change is the widening rather than the tier order.
    by_qualified_dxt: HashMap<(String, String), ChunkOrdinal>,
    by_local_dxt: HashMap<String, ChunkOrdinal>,
    label: HashMap<ChunkOrdinal, String>,
    // Two chunks can be distinct and still decode to the same pixels, in which case swapping
    // one for the other changes nothing on screen.
    pixels: HashMap<ChunkOrdinal, u64>,
}

fn pixel_digest(tex: &texture::DecodedTexture) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tex.width.hash(&mut h);
    tex.height.hash(&mut h);
    tex.rgba.hash(&mut h);
    h.finish()
}

impl FileIndex {
    // Mirrors scheduler_runtime::parse_action_bytes: an Img only enters any map if it decodes,
    // and a later chunk overwrites an earlier one under the same key.
    fn insert(&mut self, ordinal: ChunkOrdinal, dat_id: [u8; 4], body: &[u8]) {
        let Ok(decoded) = texture::decode_texture(body) else {
            return;
        };
        self.pixels.insert(ordinal, pixel_digest(&decoded));
        let named = texture::extract_texture_tokens(body);
        self.label.insert(
            ordinal,
            match &named {
                Some((ns, local)) => format!(
                    "{}[{ns}/{local}]",
                    String::from_utf8_lossy(&dat_id).trim_end()
                ),
                None => format!("{}[unnamed]", String::from_utf8_lossy(&dat_id).trim_end()),
            },
        );
        self.by_dat_id.insert(dat_id, ordinal);
        let Some((ns, local)) = named else {
            return;
        };
        self.by_qualified
            .insert((ns.clone(), local.clone()), ordinal);
        self.by_local.insert(local.clone(), ordinal);
        if body[0] == texture::FLG_DXT {
            self.by_qualified_dxt.insert((ns, local.clone()), ordinal);
            self.by_local_dxt.insert(local, ordinal);
        }
    }

    fn resolve_by_dat_id(&self, d3m: &ffxi_dat::d3m::D3m) -> Option<ChunkOrdinal> {
        self.by_dat_id.get(&d3m.texture_dat_id()).copied()
    }

    fn resolve_by_name_then_dat_id(&self, d3m: &ffxi_dat::d3m::D3m) -> Option<ChunkOrdinal> {
        let (ns, local) = d3m.texture_name_tokens();
        self.by_qualified
            .get(&(ns, local.clone()))
            .or_else(|| self.by_local.get(&local))
            .copied()
            .or_else(|| self.resolve_by_dat_id(d3m))
    }

    fn resolve_dxt_names_only(&self, d3m: &ffxi_dat::d3m::D3m) -> Option<ChunkOrdinal> {
        let (ns, local) = d3m.texture_name_tokens();
        self.by_qualified_dxt
            .get(&(ns, local.clone()))
            .or_else(|| self.by_local_dxt.get(&local))
            .copied()
            .or_else(|| self.resolve_by_dat_id(d3m))
    }

    fn label(&self, ordinal: Option<ChunkOrdinal>) -> String {
        match ordinal {
            Some(o) => self.label.get(&o).cloned().unwrap_or_else(|| o.to_string()),
            None => "none".to_string(),
        }
    }
}

#[derive(Default)]
struct Tally {
    d3m_with_texture: usize,
    by_dat_id: usize,
    by_name_then_dat_id: usize,
    resolved_by_both: usize,
    disagree_same_pixels: usize,
    disagree: Vec<String>,
    disagree_families: HashMap<String, usize>,
    disagree_from_flag_widening: Vec<String>,
    name_only_wins: usize,
    dat_id_only_wins: Vec<String>,
    img_flags: HashMap<u8, usize>,
    unnamed_flag_sample: HashMap<u8, String>,
}

fn family(local: &str) -> String {
    let stem: String = local
        .chars()
        .take_while(|c| !c.is_ascii_digit())
        .collect::<String>();
    if stem.is_empty() {
        local.to_string()
    } else {
        stem
    }
}

fn main() -> ExitCode {
    let last_file_id = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(DEFAULT_LAST_FILE_ID);
    let root = match DatRoot::from_env_or_default() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("could not open DAT root: {e}");
            return ExitCode::from(1);
        }
    };

    let mut t = Tally::default();
    for file_id in 0..=last_file_id {
        let Ok(loc) = root.resolve(file_id) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            continue;
        };

        let mut index = FileIndex::default();
        let mut d3ms = Vec::new();
        // ChunkWalker does not advance its cursor when a header is truncated, so `.flatten()`
        // spins forever on a short tail (ROM3/0/66.DAT is 4 zero bytes). Stop at the first error.
        for (ordinal, c) in walk(&bytes).map_while(Result::ok).enumerate() {
            match ChunkKind::from_u8(c.kind) {
                Some(ChunkKind::Img) => {
                    let Some(&flag) = c.data.first() else {
                        continue;
                    };
                    *t.img_flags.entry(flag).or_default() += 1;
                    if texture::extract_texture_tokens(c.data).is_none() {
                        t.unnamed_flag_sample.entry(flag).or_insert_with(|| {
                            String::from_utf8_lossy(&c.data[1..0x11.min(c.data.len())]).to_string()
                        });
                    }
                    index.insert(ordinal, c.name, c.data);
                }
                Some(ChunkKind::D3m) => {
                    if let Ok(d) = ffxi_dat::d3m::D3m::parse(c.name, c.data) {
                        d3ms.push(d);
                    }
                }
                _ => {}
            }
        }

        for d in &d3ms {
            let (ns, local) = d.texture_name_tokens();
            if ns.is_empty() && local.is_empty() {
                continue;
            }
            t.d3m_with_texture += 1;
            let old = index.resolve_by_dat_id(d);
            let new = index.resolve_by_name_then_dat_id(d);
            let new_without_widening = index.resolve_dxt_names_only(d);
            t.by_dat_id += usize::from(old.is_some());
            t.by_name_then_dat_id += usize::from(new.is_some());
            let d3m_name = String::from_utf8_lossy(&d.name).trim_end().to_string();
            if old.is_some() && new.is_some() {
                t.resolved_by_both += 1;
                let same_pixels = old
                    .and_then(|o| index.pixels.get(&o))
                    .zip(new.and_then(|n| index.pixels.get(&n)))
                    .is_some_and(|(a, b)| a == b);
                if old != new && same_pixels {
                    t.disagree_same_pixels += 1;
                }
                if old != new && !same_pixels {
                    t.disagree.push(format!(
                        "file {file_id} d3m {d3m_name} tex {ns:?}/{local:?}: {} -> {}",
                        index.label(old),
                        index.label(new)
                    ));
                    *t.disagree_families.entry(family(&local)).or_default() += 1;
                }
            }
            let widening_same_pixels = new_without_widening
                .and_then(|o| index.pixels.get(&o))
                .zip(new.and_then(|n| index.pixels.get(&n)))
                .is_some_and(|(a, b)| a == b);
            if new != new_without_widening && !widening_same_pixels {
                t.disagree_from_flag_widening.push(format!(
                    "file {file_id} d3m {d3m_name} tex {ns:?}/{local:?}: {} -> {}",
                    index.label(new_without_widening),
                    index.label(new)
                ));
            }
            t.name_only_wins += usize::from(new.is_some() && old.is_none());
            if old.is_some() && new.is_none() {
                t.dat_id_only_wins.push(format!(
                    "file {file_id} d3m {d3m_name} tex {ns:?}/{local:?}"
                ));
            }
        }
    }

    println!("scanned file ids 0..={last_file_id}");
    let mut flags: Vec<_> = t.img_flags.into_iter().collect();
    flags.sort_unstable();
    println!(
        "img chunks by flag byte            : {:?}",
        flags
            .iter()
            .map(|(f, n)| (format!("{f:#04x}"), *n))
            .collect::<Vec<_>>()
    );
    let mut unnamed: Vec<_> = t.unnamed_flag_sample.into_iter().collect();
    unnamed.sort_unstable();
    for (flag, sample) in unnamed {
        println!("  flag {flag:#04x} keeps no name; header bytes 1..0x11 = {sample:?}");
    }
    println!(
        "d3m meshes carrying a texture name : {}",
        t.d3m_with_texture
    );
    println!("  resolved by 4-byte DatId         : {}", t.by_dat_id);
    println!(
        "  resolved by name then DatId      : {}",
        t.by_name_then_dat_id
    );
    println!(
        "  resolved by both keys            : {}",
        t.resolved_by_both
    );
    println!(
        "  ... of those, landing on a different Img chunk that decodes to different pixels : {}",
        t.disagree.len()
    );
    println!(
        "  ... landing on a different Img chunk with identical pixels (no visual change)    : {}",
        t.disagree_same_pixels
    );
    let mut families: Vec<(String, usize)> = t.disagree_families.into_iter().collect();
    families.sort_unstable_by_key(|(name, n)| (std::cmp::Reverse(*n), name.clone()));
    println!(
        "      by texture family: {:?}",
        families.iter().take(SAMPLE_ROWS).collect::<Vec<_>>()
    );
    for line in t.disagree.iter().take(SAMPLE_ROWS) {
        println!("      {line}");
    }
    println!(
        "  changed by the 0xA1 -> named-flag widening alone : {}",
        t.disagree_from_flag_widening.len()
    );
    for line in t.disagree_from_flag_widening.iter().take(SAMPLE_ROWS) {
        println!("      {line}");
    }
    println!("  name-only wins over DatId        : {}", t.name_only_wins);
    println!(
        "  DatId-only wins over name        : {}",
        t.dat_id_only_wins.len()
    );
    for line in t.dat_id_only_wins.iter().take(SAMPLE_ROWS) {
        println!("      {line}");
    }
    ExitCode::SUCCESS
}
