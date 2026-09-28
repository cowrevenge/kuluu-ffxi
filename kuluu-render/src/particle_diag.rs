//! Particle-reader diagnostics: one collector that answers, for every generator chunk in an
//! effect DAT, what the data says, what the reader understood, and which pipeline it goes down.
//!
//! Parse-time categories (unknown opcodes with their raw payload, decoded-but-inert opcodes,
//! unknown linked-data kinds) come from [`EffectCoverageReport`]. Route, resource and visibility
//! categories are computed statically against [`ActionAssets`] with the same lookup order the
//! spawn path uses (`particle_sim::resolve_mesh`: dir-scoped tier first, flat map after), so a
//! headless walk of a DAT reports exactly what a live run would — without one.

use std::collections::BTreeMap;

use ffxi_dat::particle_gen::{GeneratorOpcodeOutcome, GeneratorSection};
use ffxi_dat::scheduler::{Scheduler, StageKind, NO_LOCAL_DIR};

use crate::scheduler_runtime::{ActionAssets, EffectCoverageReport};

/// One deduped diagnostics row: how many times the key occurred and one example generator +
/// directory to find it by hand.
#[derive(Debug, Clone)]
pub struct DiagEntry {
    pub count: u32,
    pub example_gen: [u8; 4],
    pub example_dir: [u8; 4],
    /// Key-specific detail (opcode size + raw args, missing-resource description, ...).
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct ParticleDiag {
    /// No section arm decoded the block. Key (section, opcode); detail carries size + raw args.
    pub unknown_opcode: BTreeMap<(GeneratorSection, u8), DiagEntry>,
    /// An arm read the block at the right size but no runtime behaviour consumes what it stored.
    pub parsed_but_inert: BTreeMap<(GeneratorSection, u8), DiagEntry>,
    /// StandardParticleSetup named a linked-data kind byte the reader does not recognise; the
    /// chunk is refused outright. Key = raw kind byte.
    pub unknown_kind: BTreeMap<u8, DiagEntry>,
    /// Declared kind and the pipeline it goes down disagree (a mesh-typed generator routed to
    /// rumble is the shipped shape). Key = route description.
    pub kind_route_mismatch: BTreeMap<String, DiagEntry>,
    /// The resource a def names does not exist in the DAT. Key = what was searched for.
    pub missing_resource: BTreeMap<String, DiagEntry>,
    /// Reached the draw path with alpha 0 for its whole life — rumble-like behaviour hides here.
    /// Key = why it is invisible.
    pub never_visible: BTreeMap<String, DiagEntry>,
    /// Sound generators: sep chunk missing, or authored near/far 0 taking the Calc3D class
    /// defaults (count only). Key = which case.
    pub sound: BTreeMap<String, DiagEntry>,
    /// Generators carrying a rumble track (sec2 0x82) and why it fired — or did not — in this
    /// run. Key = reason.
    pub rumble: BTreeMap<String, DiagEntry>,
}

fn bump<K: Ord>(
    map: &mut BTreeMap<K, DiagEntry>,
    key: K,
    gen: [u8; 4],
    dir: [u8; 4],
    detail: String,
) {
    match map.get_mut(&key) {
        Some(entry) => entry.count += 1,
        None => {
            map.insert(
                key,
                DiagEntry {
                    count: 1,
                    example_gen: gen,
                    example_dir: dir,
                    detail,
                },
            );
        }
    }
}

fn id4(bytes: [u8; 4]) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The raw args of a dropped block as hex, capped so one row stays on one line.
fn hex_sample(args: &[u8], cap: usize) -> String {
    let shown: Vec<String> = args.iter().take(cap).map(|b| format!("{b:02x}")).collect();
    if args.len() > cap {
        format!("{} … ({} bytes)", shown.join(" "), args.len())
    } else {
        shown.join(" ")
    }
}

// The decoded-but-inert table: every section arm that stores a field no runtime path reads,
// verified against the consumers in particle_sim.rs / zone_clouds.rs / element_sort.rs (a field
// copied into LiveGenerator and never read does not count). Adding an engine consumer for one of
// these must remove it here.
pub fn inert_reason(section: GeneratorSection, opcode: u8) -> Option<&'static str> {
    Some(match (section, opcode) {
        (GeneratorSection::Initializers, 0x1D) => "sprite-sheet init word never read",
        (GeneratorSection::Initializers, 0x29) => "scale_z_track: the engine sprite is 2-D",
        (GeneratorSection::Initializers, 0x32) => {
            "haze_offset_x on a mesh def: only distortion defs consume it"
        }
        (GeneratorSection::Initializers, 0x44 | 0x53) => "child_generator: no child-particle path",
        (GeneratorSection::Initializers, 0x45) => "parent_position_copy: no child-particle path",
        (GeneratorSection::Initializers, 0x46) => "parent_velocity: no child-particle path",
        (GeneratorSection::Initializers, 0x47 | 0x79) => "parent_rotate: no child-particle path",
        (GeneratorSection::Initializers, 0x48) => "parent_color: no child-particle path",
        (GeneratorSection::Initializers, 0x49) => "parent_scale: no child-particle path",
        (GeneratorSection::Initializers, 0x4A) => "parent_tex_coord: no child-particle path",
        (GeneratorSection::Initializers, 0x4E | 0x4F) => {
            "fixed_point_position_variance: reconstruction only"
        }
        (GeneratorSection::Initializers, 0x51) => "velocity_y_track: no per-frame velocity track",
        (GeneratorSection::Initializers, 0x54) => "point_list_position: no spline runtime",
        (GeneratorSection::Initializers, 0x55 | 0x59 | 0x5B | 0x5D | 0x5F) => {
            "specular element not modelled"
        }
        (GeneratorSection::Initializers, 0x56) => "batching_setup: retail walk has no case",
        (GeneratorSection::Initializers, 0x8E) => "foot_mark: spawn-snap not implemented",
        (GeneratorSection::Updaters, 0x25 | 0x33) => "child-generator updater: no child path",
        (GeneratorSection::Updaters, 0x34) => "point-list position updater: no spline runtime",
        (GeneratorSection::Updaters, 0x36 | 0x37 | 0x3B) => {
            "specular progress updater: specular element not modelled"
        }
        (GeneratorSection::Initializers, 0x33..=0x37) => {
            "weighted-mesh weight track: draw path deferred"
        }
        (GeneratorSection::Updaters, 0x1E..=0x22) => {
            "weighted-mesh weight applier: draw path deferred"
        }
        (GeneratorSection::ElementDie, 0x01) => "emit child on expiry: no child-particle path",
        _ => return None,
    })
}

pub fn is_inert_opcode(section: GeneratorSection, opcode: u8) -> bool {
    inert_reason(section, opcode).is_some()
}

/// Decoded opcodes the engine deliberately does not model — XIM reads and discards them, or
/// the engine's sprite has no axis for them (research/xim ParticleGeneratorParser.kt). They are
/// parsed correctly by design, so they stay out of ParsedButInert.
pub fn known_inert(section: GeneratorSection, opcode: u8) -> bool {
    matches!(
        (section, opcode),
        // XIM reads the sprite-sheet init word and ignores it.
        (GeneratorSection::Initializers, 0x1D)
            // The engine's sprite is 2-D; scale.z has no axis to drive.
            | (GeneratorSection::Initializers, 0x29)
    )
}

/// The route a particle def goes down at spawn, mirroring `spawn_particle_generators`: rumble
/// first (sec2 0x82 + sec3 0x5F), then the mesh/sheet draw path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnRoute {
    Rumble,
    Draw,
}

impl ParticleDiag {
    /// One table per category, rows sorted by count descending; empty categories say why they
    /// are empty so an all-empty output reads as "not wired", not "complete".
    pub fn render_table(&self) -> String {
        let mut out = String::new();

        for (title, map, empty_note) in [
            (
                "== UnknownOpcode (no section arm decoded the block) ==",
                &self.unknown_opcode,
                "(none — every walked block reached an arm)",
            ),
            (
                "== ParsedButInert (decoded at the right size, no runtime consumer) ==",
                &self.parsed_but_inert,
                "(none — every decoded block has a consumer)",
            ),
        ] {
            out.push_str(&format!("\n{title}\n"));
            let mut rows: Vec<(&(GeneratorSection, u8), &DiagEntry)> = map.iter().collect();
            if rows.is_empty() {
                out.push_str(empty_note);
                out.push('\n');
                continue;
            }
            rows.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));
            for ((sec, op), e) in rows {
                out.push_str(&format!(
                    "{:4}  {:?}/{:#04x}  {}  e.g. gen={} dir={}\n",
                    e.count,
                    sec,
                    op,
                    e.detail,
                    id4(e.example_gen),
                    id4(e.example_dir)
                ));
            }
        }

        out.push_str("\n== UnknownKind (linked-data kind byte not recognised; chunk refused) ==\n");
        if self.unknown_kind.is_empty() {
            out.push_str("(none — every StandardParticleSetup named a known kind)\n");
        }
        let mut kinds: Vec<(&u8, &DiagEntry)> = self.unknown_kind.iter().collect();
        kinds.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));
        for (kind, e) in kinds {
            out.push_str(&format!(
                "{:4}  kind byte {:#04x}  {}  e.g. gen={} dir={}\n",
                e.count,
                kind,
                e.detail,
                id4(e.example_gen),
                id4(e.example_dir)
            ));
        }

        for (title, map, empty_note) in [
            (
                "== KindRouteMismatch (declared kind vs pipeline taken) ==",
                &self.kind_route_mismatch,
                "(none — every def took the pipeline its kind declares)",
            ),
            (
                "== MissingResource (named resource absent from the DAT) ==",
                &self.missing_resource,
                "(none — every named mesh/sheet/texture resolved)",
            ),
            (
                "== NeverVisible (draw path, alpha 0 for the whole life) ==",
                &self.never_visible,
                "(none — no draw-path generator is invisible its whole life)",
            ),
            (
                "== Sound ==",
                &self.sound,
                "(no sound generators in this DAT)",
            ),
            (
                "== Rumble (sec2 0x82 carriers) ==",
                &self.rumble,
                "(no rumble carriers in this DAT)",
            ),
        ] {
            out.push_str(&format!("\n{title}\n"));
            if map.is_empty() {
                out.push_str(empty_note);
                out.push('\n');
                continue;
            }
            let mut keyed: Vec<(&String, &DiagEntry)> = map.iter().collect();
            keyed.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));
            for (key, e) in keyed {
                let detail = if e.detail.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", e.detail)
                };
                out.push_str(&format!(
                    "{:4}  {}{}  e.g. gen={} dir={}\n",
                    e.count,
                    key,
                    detail,
                    id4(e.example_gen),
                    id4(e.example_dir)
                ));
            }
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_draw_resource(
    assets: &ActionAssets,
    dir: [u8; 4],
    def: &ffxi_dat::particle_gen::ParticleGeneratorDef,
) -> (String, Option<String>) {
    // The same lookup order as particle_sim::resolve_mesh — dir-scoped tier first, flat map
    // after — so the static answer matches what a live spawn would find.
    use ffxi_dat::particle_gen::ParticleMeshKind;
    match def.mesh_kind {
        ParticleMeshKind::StaticMesh | ParticleMeshKind::WeightedMesh => {
            let Some(d3m) = assets.d3m(dir, &def.mesh_id) else {
                return (
                    format!("0x1F mesh '{}' (dir {} + flat)", id4(def.mesh_id), id4(dir)),
                    None,
                );
            };
            let (namespace, local) = d3m.texture_name_tokens();
            let found = (!local.is_empty())
                .then(|| {
                    assets
                        .images_by_qualified_name
                        .get(&(namespace.clone(), local.clone()))
                        .or_else(|| assets.images_by_name.get(&local))
                })
                .flatten()
                .or_else(|| assets.images.get(&d3m.texture_dat_id()));
            match found {
                Some(_) => (
                    format!("0x1F mesh '{}' + 0x20 texture", id4(def.mesh_id)),
                    Some(format!("mesh {} / texture ok", id4(def.mesh_id))),
                ),
                None => (
                    format!(
                        "texture for 0x1F mesh '{}' (qualified/local/dat-id)",
                        id4(def.mesh_id)
                    ),
                    None,
                ),
            }
        }
        ParticleMeshKind::SpriteSheet => {
            let Some(ss) = assets.sprite_sheet(dir, &def.mesh_id) else {
                return (
                    format!(
                        "0x21 sheet '{}' (dir {} + flat)",
                        id4(def.mesh_id),
                        id4(dir)
                    ),
                    None,
                );
            };
            match assets
                .images_by_qualified_name
                .get(&(ss.category.clone(), ss.id.clone()))
                .or_else(|| assets.images_by_name.get(&ss.id))
            {
                Some(_) => (
                    format!("0x21 sheet '{}' + 0x20 texture", id4(def.mesh_id)),
                    Some(format!("sheet {} / texture ok", id4(def.mesh_id))),
                ),
                None => (
                    format!(
                        "texture for 0x21 sheet '{}' (qualified/local)",
                        id4(def.mesh_id)
                    ),
                    None,
                ),
            }
        }
    }
}

/// The kind byte a def's mesh kind came from — the reverse of `LinkedDataKind::from_byte` for
/// the three kinds that claim the particle path.
#[cfg(not(target_arch = "wasm32"))]
fn kind_byte(def: &ffxi_dat::particle_gen::ParticleGeneratorDef) -> u8 {
    use ffxi_dat::particle_gen::{LinkedDataKind, ParticleMeshKind};
    match def.mesh_kind {
        ParticleMeshKind::StaticMesh => LinkedDataKind::STATIC_MESH,
        ParticleMeshKind::SpriteSheet => LinkedDataKind::SPRITE_SHEET,
        ParticleMeshKind::WeightedMesh => LinkedDataKind::WEIGHTED_MESH,
    }
}

/// Walk one parsed effect DAT and fill every category that static analysis can reach. The
/// spawn-time-only facts (a rumble that never fired because no pad was present) are reported as
/// such in the Rumble rows.
#[cfg(not(target_arch = "wasm32"))]
pub fn analyze(assets: &ActionAssets, report: &EffectCoverageReport) -> ParticleDiag {
    use ffxi_dat::particle_gen::ps2_float_rescale;

    let mut diag = ParticleDiag::default();

    for d in &report.dropped_opcode_details {
        bump(
            &mut diag.unknown_opcode,
            (d.section, d.opcode),
            d.name,
            d.dir,
            format!(
                "size {} words; args {}",
                d.size_words,
                hex_sample(&d.args_sample, 16)
            ),
        );
    }

    for &(name, section, opcode, outcome) in &report.generator_opcodes {
        if outcome == GeneratorOpcodeOutcome::Decoded
            && is_inert_opcode(section, opcode)
            && !known_inert(section, opcode)
        {
            let dir = assets
                .particle_def_dirs
                .get(&name)
                .copied()
                .unwrap_or(NO_LOCAL_DIR);
            bump(
                &mut diag.parsed_but_inert,
                (section, opcode),
                name,
                dir,
                inert_reason(section, opcode).unwrap_or("inert").to_string(),
            );
        }
    }

    for &(name, kind, linked_id, dir) in &report.unknown_linked_data_kinds {
        bump(
            &mut diag.unknown_kind,
            kind,
            name,
            dir,
            format!("linked id '{}'", id4(linked_id)),
        );
    }

    for ((dir, name), def) in &assets.particle_defs_by_dir {
        let rumble_armed = def.rumble_track.is_some() && def.rumble_falloff.is_some();
        if let Some(track_id) = def.rumble_track {
            bump(
                &mut diag.rumble,
                "headless walk: no gamepad layer; fires in-game with a pad and vibration on"
                    .to_string(),
                *name,
                *dir,
                format!(
                    "track '{}' near {} far {}",
                    id4(track_id),
                    def.rumble_falloff.map(|f| f[0]).unwrap_or(f32::NAN),
                    def.rumble_falloff.map(|f| f[1]).unwrap_or(f32::NAN)
                ),
            );
        }
        if rumble_armed {
            bump(
                &mut diag.kind_route_mismatch,
                "mesh-typed generator routed to rumble (sec2 0x82 + sec3 0x5F); never draws"
                    .to_string(),
                *name,
                *dir,
                format!("kind byte {:#04x}", kind_byte(def)),
            );
            continue;
        }

        let init_alpha = ps2_float_rescale(def.init_color[3]);
        if init_alpha == 0.0 {
            match def.alpha_track {
                None => bump(
                    &mut diag.never_visible,
                    "init alpha 0, no alpha track".to_string(),
                    *name,
                    *dir,
                    format!("kind byte {:#04x}", kind_byte(def)),
                ),
                Some(track_id) => {
                    let all_zero = assets.keyframes.get(&track_id).is_some_and(|t| {
                        t.points.iter().all(|&(_, v)| ps2_float_rescale(v) == 0.0)
                    });
                    if all_zero {
                        bump(
                            &mut diag.never_visible,
                            format!("alpha track '{}' all zero", id4(track_id)),
                            *name,
                            *dir,
                            format!("kind byte {:#04x}", kind_byte(def)),
                        );
                    }
                }
            }
        }

        let (searched, found) = resolve_draw_resource(assets, *dir, def);
        if found.is_none() {
            bump(
                &mut diag.missing_resource,
                searched,
                *name,
                *dir,
                String::new(),
            );
        }
    }

    for (&name, def) in &assets.sound_defs {
        let dir = NO_LOCAL_DIR;
        if !assets.seps.contains_key(&def.sep_id) {
            bump(
                &mut diag.sound,
                format!("sep chunk '{}' missing", id4(def.sep_id)),
                name,
                dir,
                String::new(),
            );
        }
        if def.far == 0.0 || def.near == 0.0 {
            bump(
                &mut diag.sound,
                "authored near/far 0 -> Calc3D class defaults (count only)".to_string(),
                name,
                dir,
                format!("near {} far {}", def.near, def.far),
            );
        }
    }

    diag
}

/// One line per generator a routine's particle stages name: kind byte, route taken, the resource
/// it resolved (and which chunk type), and its opcode census — handled vs inert vs dropped. The
/// same lines an in-game VfxTrace run prints, computed from the parse alone.
#[cfg(not(target_arch = "wasm32"))]
pub fn trace_routine(
    schedulers: &[Scheduler],
    assets: &ActionAssets,
    report: &EffectCoverageReport,
    routine_name: [u8; 4],
) -> Vec<String> {
    let Some(sched) = schedulers.iter().find(|s| s.name == routine_name) else {
        return vec![format!("routine '{}' not found", id4(routine_name))];
    };
    let mut lines = vec![format!(
        "routine '{}': {} stages, tracing particle stages",
        id4(sched.name),
        sched.stages.len()
    )];
    for t in &sched.stages {
        if t.stage.kind != StageKind::Particle {
            continue;
        }
        let id = t.stage.id;
        let dir = t.stage.local_dir;

        let (route, resource) = if let Some((def_dir, def)) = assets.particle_def_scoped(dir, &id) {
            let rumble_armed = def.rumble_track.is_some() && def.rumble_falloff.is_some();
            if rumble_armed {
                (
                    "rumble",
                    format!(
                        "track '{}' near {} far {}",
                        id4(def.rumble_track.unwrap()),
                        def.rumble_falloff.map(|f| f[0]).unwrap_or(f32::NAN),
                        def.rumble_falloff.map(|f| f[1]).unwrap_or(f32::NAN)
                    ),
                )
            } else {
                let (searched, found) = resolve_draw_resource(assets, def_dir, def);
                (
                    "draw",
                    match found {
                        Some(ok) => format!("{searched} — {ok}"),
                        None => format!("{searched} — MISSING"),
                    },
                )
            }
        } else if let Some(sound) = assets.sound_defs.get(&id) {
            (
                "sound",
                format!(
                    "sep '{}' -> se_id {}",
                    id4(sound.sep_id),
                    assets
                        .seps
                        .get(&sound.sep_id)
                        .map(|s| format!("{:#06x}", s.se_id))
                        .unwrap_or_else(|| "MISSING".into())
                ),
            )
        } else if let Some(dist) = assets.distortion_defs.get(&id) {
            (
                "haze",
                format!(
                    "envelope '{}' haze_x {:.3}",
                    dist.envelope_track.map(id4).unwrap_or_else(|| "-".into()),
                    dist.haze_offset_x
                ),
            )
        } else {
            ("noop", format!("no def in any tier (dir {})", id4(dir)))
        };

        let census = report
            .generator_opcodes
            .iter()
            .filter(|(name, _, _, _)| *name == id)
            .fold((0u32, 0u32, 0u32), |(h, i, d), (_, sec, op, outcome)| {
                if *outcome == GeneratorOpcodeOutcome::Dropped {
                    (h, i, d + 1)
                } else if is_inert_opcode(*sec, *op) && !known_inert(*sec, *op) {
                    (h, i + 1, d)
                } else {
                    (h + 1, i, d)
                }
            });

        let kind = assets
            .particle_def_scoped(dir, &id)
            .map(|(_, def)| format!("{:#04x}", kind_byte(def)))
            .or_else(|| {
                assets
                    .sound_defs
                    .get(&id)
                    .map(|_| format!("{:#04x}", ffxi_dat::particle_gen::LinkedDataKind::AUDIO))
            })
            .or_else(|| {
                assets.distortion_defs.get(&id).map(|_| {
                    format!(
                        "{:#04x}",
                        ffxi_dat::particle_gen::LinkedDataKind::DISTORTION
                    )
                })
            })
            .unwrap_or_else(|| "-".into());

        lines.push(format!(
            "{}  kind={}  route={:<7} {}  opcodes {}h/{}i/{}d",
            id4(id),
            kind,
            route,
            resource,
            census.0,
            census.1,
            census.2
        ));
    }
    lines
}
