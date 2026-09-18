use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::client_profile::ClientProfile;
use crate::ftable::{FTable, SubPath, FTABLE_BYTES_PER_FILE_ID};
use crate::vtable::VTable;
use crate::{DatError, Result};

/// XIClient's LoadFileTables stops at INDEX_ROM_MAX, which is 13
/// (research/XIClient/src/XIClient/include/Constants/Values.h); POLUtils
/// DoFullFileScan (vendor/POLUtils/MassExtractor/Program.cs) probes up to
/// ROM19, and so does this crate so a newer install than that client still
/// resolves.
const MAX_ROM_INDEX: u8 = 19;

pub const DAT_PATH_ENV: &str = "FFXI_DAT_PATH";

pub const DEFAULT_INSTALL_DIR: &str = "vendor/game-files/SquareEnix/FINAL FANTASY XI";

/// Named installs live side by side here so one checkout can target several
/// client generations; `cargo xtask ffxi-client link --target <name>` wires them.
pub const TARGETS_DIR: &str = "vendor/game-files/targets";

/// Selects a named install under [`TARGETS_DIR`]. `FFXI_DAT_PATH` wins when
/// both are set, so an explicit path is never silently redirected.
pub const CLIENT_TARGET_ENV: &str = "FFXI_CLIENT_TARGET";

pub const INSTALL_SUBDIR: &str = "SquareEnix/FINAL FANTASY XI";

pub fn target_install_dir(targets_dir: &Path, name: &str) -> PathBuf {
    targets_dir.join(name).join(INSTALL_SUBDIR)
}

fn is_install(dir: &Path) -> bool {
    dir.join("VTABLE.DAT").exists()
}

/// Workspace-relative paths are tried against the cwd first; cargo runs each
/// test binary with cwd set to its own package root, so the workspace root
/// resolved from this crate's manifest dir is the fallback (absent in a
/// shipped binary, which is why cwd is still tried first).
fn workspace_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        bases.push(cwd);
    }
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        bases.push(root.to_path_buf());
    }
    bases
}

/// The checkout's [`TARGETS_DIR`], if the checkout is reachable.
pub fn workspace_targets_dir() -> Option<PathBuf> {
    workspace_bases()
        .into_iter()
        .map(|b| b.join(TARGETS_DIR))
        .find(|p| p.is_dir())
}

/// The named checkout target, if it holds an install.
pub fn workspace_target(name: &str) -> Option<PathBuf> {
    workspace_bases()
        .into_iter()
        .map(|b| target_install_dir(&b.join(TARGETS_DIR), name))
        .find(|p| is_install(p))
}

/// The checkout's [`DEFAULT_INSTALL_DIR`], if it holds an install.
pub fn workspace_default() -> Option<PathBuf> {
    workspace_bases()
        .into_iter()
        .map(|b| b.join(DEFAULT_INSTALL_DIR))
        .find(|p| is_install(p))
}

/// Overlay roots searched before the base install, in order, separated by the
/// platform path separator. A startup override; see [`discover_overlays`] for
/// where the list otherwise comes from.
pub const OVERLAY_ENV: &str = "FFXI_DAT_OVERLAYS";

fn overlays_from_env() -> Option<Vec<PathBuf>> {
    let raw = env::var_os(OVERLAY_ENV)?;
    Some(
        env::split_paths(&raw)
            .filter(|p| !p.as_os_str().is_empty())
            .collect(),
    )
}

/// XI-Pivot's config, relative to the game directory that contains the install.
const PIVOT_INI: &str = "config/pivot/pivot.ini";
/// Where Pivot keeps the overlay directories, relative to the same place.
const PIVOT_DAT_DIR: &str = "polplugins/DATs";

/// The game directory holding Pivot's config and overlays, given a DAT root of
/// `<game>/SquareEnix/FINAL FANTASY XI`. A root that is itself a symlink (the
/// checkout default pointing into a named target) is followed first, since
/// the config sits beside the real tree, not the link.
fn game_dir(install_root: &Path) -> Option<PathBuf> {
    let real = match std::fs::read_link(install_root) {
        Ok(target) => install_root
            .parent()
            .map(|p| p.join(&target))
            .unwrap_or(target),
        Err(_) => install_root.to_path_buf(),
    };
    real.parent()?.parent().map(Path::to_path_buf)
}

/// Overlay directory names from a `pivot.ini`, ordered by their `[overlays]`
/// index.
///
/// Pivot indexes them `0=`, `1=`, … and we search in that order, first match
/// wins. That precedence is NOT confirmed against Pivot's source (none is
/// vendored) and the shipped `pivotSettingsHolder.ini` comment contradicts its
/// own entries; it is unobservable on the horizonxi-2023 target
/// (vendor/game-files/targets/hxi), where no two overlays claim the same path.
fn parse_pivot_ini(ini: &str) -> (Option<PathBuf>, Vec<String>) {
    let mut root_path = None;
    let mut entries: Vec<(u32, String)> = Vec::new();
    let mut in_overlays = false;
    for line in ini.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_overlays = line.eq_ignore_ascii_case("[overlays]");
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if in_overlays {
            if let Ok(index) = key.parse::<u32>() {
                entries.push((index, value.to_string()));
            }
        } else if key.eq_ignore_ascii_case("root_path") {
            root_path = Some(PathBuf::from(value));
        }
    }
    entries.sort_by_key(|(index, _)| *index);
    (
        root_path,
        entries.into_iter().map(|(_, name)| name).collect(),
    )
}

/// Overlay roots for an install, honouring the config a private server already
/// ships: [`OVERLAY_ENV`] first, else XI-Pivot's own `pivot.ini` beside the
/// install. Empty when neither applies, which is the vanilla path.
///
/// `root_path` in a real `pivot.ini` is the Windows path Pivot was configured
/// with (measured: `C:\Program Files (x86)\...\polplugins\DATs`), so it is only
/// honoured when it resolves on this machine; otherwise the overlays are taken
/// from the install's own `polplugins/DATs`.
pub fn discover_overlays(install_root: &Path) -> Vec<PathBuf> {
    if let Some(from_env) = overlays_from_env() {
        return from_env;
    }
    let Some(game_dir) = game_dir(install_root) else {
        return Vec::new();
    };
    let Ok(ini) = std::fs::read_to_string(game_dir.join(PIVOT_INI)) else {
        return Vec::new();
    };
    let (root_path, names) = parse_pivot_ini(&ini);
    let dat_dir = root_path
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| game_dir.join(PIVOT_DAT_DIR));
    names
        .into_iter()
        .map(|name| dat_dir.join(name))
        .filter(|p| p.is_dir())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatLocation {
    pub rom_dir: String,
    pub sub_path: SubPath,
}

impl DatLocation {
    /// Where this file lives, honouring `root`'s overlay search path before the
    /// base install.
    pub fn path_under(&self, root: &DatRoot) -> PathBuf {
        root.path_of(self)
    }

    /// The raw `<root>/<ROMn>/<dir>/<file>.DAT` join, with no overlay search.
    pub fn join_under(&self, root: &Path) -> PathBuf {
        self.join_under_ext(root, "DAT")
    }

    fn join_under_ext(&self, root: &Path, ext: &str) -> PathBuf {
        root.join(&self.rom_dir)
            .join(self.sub_path.dir.to_string())
            .join(format!("{}.{ext}", self.sub_path.file))
    }

    /// First existing spelling of this file under `dir`. Retail runs on
    /// Windows, whose fopen is case-insensitive, so an install (or a
    /// hand-assembled overlay) can mix `.DAT` with `.dat` and the real client
    /// never notices; only a case-sensitive filesystem — a Linux user's
    /// wine/launcher-managed install — can tell them apart.
    fn find_under(&self, dir: &Path) -> Option<PathBuf> {
        ["DAT", "dat"]
            .into_iter()
            .map(|ext| self.join_under_ext(dir, ext))
            .find(|p| p.is_file())
    }
}

#[derive(Debug)]
struct AppTables {
    rom_index: u8,
    rom_dir: String,
    vtable: VTable,
    ftable: FTable,
}

/// A ROM left out of the merge because a table is not the size the base
/// tables dictate. LoadFileTables
/// (research/XIClient/src/XIClient/source/System/FileIO/FileIOVirtualFileSystem.cpp)
/// aborts the whole load on this; a user-assembled install is better served by
/// the ROMs that do fit, with the rejected one reported through
/// [`DatRoot::skipped_tables`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSizeMismatch {
    pub rom_dir: String,
    pub path: PathBuf,
    pub len: u64,
    pub expected: u64,
}

impl fmt::Display for TableSizeMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} skipped: {} is {} bytes, the base tables dictate {}",
            self.rom_dir,
            self.path.display(),
            self.len,
            self.expected
        )
    }
}

fn check_table_sizes(
    rom_dir: &str,
    vtable: &VTable,
    ftable: &FTable,
    file_id_count: u32,
) -> std::result::Result<(), TableSizeMismatch> {
    let mismatch = |path: &Path, len: u64, expected: u64| TableSizeMismatch {
        rom_dir: rom_dir.to_string(),
        path: path.to_path_buf(),
        len,
        expected,
    };
    if vtable.len() != file_id_count {
        return Err(mismatch(
            vtable.source(),
            u64::from(vtable.len()),
            u64::from(file_id_count),
        ));
    }
    let ftable_bytes = u64::from(ftable.len()) * FTABLE_BYTES_PER_FILE_ID as u64;
    let expected_ftable_bytes = u64::from(vtable.len()) * FTABLE_BYTES_PER_FILE_ID as u64;
    if ftable_bytes != expected_ftable_bytes {
        return Err(mismatch(
            ftable.source(),
            ftable_bytes,
            expected_ftable_bytes,
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub struct DatRoot {
    root: PathBuf,
    profile: ClientProfile,
    /// Ascending `rom_index`; [`DatRoot::resolve`] walks it backwards.
    apps: Vec<AppTables>,
    skipped: Vec<TableSizeMismatch>,
    /// Behind a lock because the renderer shares one `Arc<DatRoot>`: swapping
    /// overlays must be visible through that handle without rebuilding the root
    /// (which would re-read every VTABLE/FTABLE) or replacing the `Arc` at every
    /// holder. A read per DAT open is nothing against the file I/O that follows.
    overlays: RwLock<Vec<PathBuf>>,
}

impl DatRoot {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let mut apps: Vec<AppTables> = Vec::new();
        let mut skipped = Vec::new();

        for i in 1..=MAX_ROM_INDEX {
            let (rom_dir, vt_path, ft_path) = appid_paths(&root, i);
            if !vt_path.exists() {
                continue;
            }
            let vtable = VTable::load(&vt_path)?;
            let ftable = FTable::load(&ft_path)?;
            let file_id_count = apps.first().map_or(vtable.len(), |base| base.vtable.len());
            if let Err(mismatch) = check_table_sizes(&rom_dir, &vtable, &ftable, file_id_count) {
                skipped.push(mismatch);
                continue;
            }
            apps.push(AppTables {
                rom_index: i,
                rom_dir,
                vtable,
                ftable,
            });
        }

        if apps.is_empty() {
            return Err(DatError::Io {
                path: root.join("VTABLE.DAT"),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no VTABLE.DAT or VTABLEN.DAT found under root",
                ),
            });
        }

        let overlays = RwLock::new(discover_overlays(&root));
        // The profile's item-layout probe resolves a file id, so it needs the
        // assembled tables and overlay search path: build the root, then fill it in.
        let mut root = Self {
            root,
            profile: ClientProfile::default(),
            apps,
            skipped,
            overlays,
        };
        root.profile = ClientProfile::probe_in(&root);
        Ok(root)
    }

    /// Replace the overlay search path. Every constructor already seeds it from
    /// [`discover_overlays`]; this is for callers that configure it directly.
    pub fn with_overlays(self, overlays: Vec<PathBuf>) -> Self {
        self.set_overlays(overlays);
        self
    }

    /// Swap the overlay search path on a live root, so a settings change takes
    /// effect without a restart. Callers holding DAT-derived caches must drop
    /// them — this only changes which file a later resolve reads. The
    /// scheduler's zone-scene memo ([`crate::scheduler::clear_zone_scene_cache`])
    /// is cleared here automatically; every other DAT-derived cache is the
    /// holder's to drop.
    pub fn set_overlays(&self, overlays: Vec<PathBuf>) {
        *self
            .overlays
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = overlays;
        crate::scheduler::clear_zone_scene_cache();
    }

    pub fn overlays(&self) -> Vec<PathBuf> {
        self.overlays
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Resolve a location to a real file: each overlay in order, then the base
    /// install under either `.DAT` spelling. The raw `.DAT` join is returned
    /// unconditionally when no spelling exists, so a missing file still
    /// surfaces as a read error at the install path the caller expects.
    pub fn path_of(&self, loc: &DatLocation) -> PathBuf {
        self.overlays
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find_map(|overlay| loc.find_under(overlay))
            .or_else(|| loc.find_under(&self.root))
            .unwrap_or_else(|| loc.join_under(&self.root))
    }

    pub fn from_env() -> Result<Self> {
        let root = env::var_os(DAT_PATH_ENV).ok_or(DatError::EnvMissing)?;
        Self::open(PathBuf::from(root))
    }

    /// `FFXI_DAT_PATH`, else the checkout target named by `FFXI_CLIENT_TARGET`,
    /// else the checkout default. Product-side sources (the launcher's saved
    /// choice, the per-user client directory) are settled into `FFXI_DAT_PATH`
    /// by kuluu before this runs.
    pub fn from_env_or_default() -> Result<Self> {
        if let Some(root) = env::var_os(DAT_PATH_ENV) {
            return Self::open(PathBuf::from(root));
        }
        if let Some(name) = env::var_os(CLIENT_TARGET_ENV) {
            let name = name.to_string_lossy();
            return match workspace_target(&name) {
                Some(p) => Self::open(p),
                None => Err(DatError::TargetMissing {
                    name: name.into_owned(),
                }),
            };
        }
        Self::open(workspace_default().ok_or(DatError::EnvMissing)?)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn profile(&self) -> &ClientProfile {
        &self.profile
    }

    pub fn app_summary(&self) -> Vec<(String, u32, u32)> {
        self.apps
            .iter()
            .map(|a| (a.rom_dir.clone(), a.vtable.len(), a.ftable.len()))
            .collect()
    }

    /// ROMs whose tables were rejected at open; empty on a well-formed install.
    pub fn skipped_tables(&self) -> &[TableSizeMismatch] {
        &self.skipped
    }

    /// Size of the file-id space: the base VTABLE's length, which every
    /// accepted expansion table matches.
    pub fn file_id_count(&self) -> u32 {
        self.apps.first().map_or(0, |base| base.vtable.len())
    }

    /// The highest ROM claiming `file_id` owns it. LoadFileTables
    /// (research/XIClient/src/XIClient/source/System/FileIO/FileIOVirtualFileSystem.cpp)
    /// merges ROM2..INDEX_ROM_MAX ascending into the base tables and overwrites
    /// the owner on every claim, so a later ROM's copy shadows an earlier one.
    pub fn resolve(&self, file_id: u32) -> Result<DatLocation> {
        for app in self.apps.iter().rev() {
            if app.vtable.contains(file_id, app.rom_index) {
                let sub_path = app.ftable.sub_path(file_id)?;
                return Ok(DatLocation {
                    rom_dir: app.rom_dir.clone(),
                    sub_path,
                });
            }
        }
        Err(DatError::FileNotPresent { file_id })
    }
}

/// Test-support entry point, `pub` only so real-DAT guards in sibling crates can
/// share it. Opens `FFXI_DAT_PATH` if set and usable, else the checkout target
/// named by `FFXI_CLIENT_TARGET`, else the default install resolved relative to
/// the crate (works regardless of the test CWD, unlike
/// [`DatRoot::from_env_or_default`]'s relative path). `None` — with a printed
/// reason, so a vacuous pass is never mistaken for a real one — when no install
/// is present.
#[doc(hidden)]
pub fn open_test_install() -> Option<DatRoot> {
    match DatRoot::from_env() {
        Ok(root) => return Some(root),
        Err(DatError::EnvMissing) => {}
        // A stale FFXI_DAT_PATH in a shell must not turn every real-DAT test into a silent
        // skip, so say so and still try the vendored install.
        Err(e) => eprintln!(
            "real-DAT guard: {DAT_PATH_ENV} unusable ({e}); trying the vendored install instead"
        ),
    }
    if let Some(name) = env::var_os(CLIENT_TARGET_ENV) {
        let name = name.to_string_lossy();
        match workspace_target(&name).map(DatRoot::open) {
            Some(Ok(root)) => return Some(root),
            Some(Err(e)) => eprintln!(
                "real-DAT guard: {CLIENT_TARGET_ENV}={name} unusable ({e}); trying the vendored install instead"
            ),
            None => eprintln!(
                "real-DAT guard: {CLIENT_TARGET_ENV}={name} names no install under {TARGETS_DIR}; trying the vendored install instead"
            ),
        }
    }
    let default = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(DEFAULT_INSTALL_DIR);
    if !default.join("VTABLE.DAT").exists() {
        eprintln!(
            "SKIP (real-DAT guard): no retail install — {DAT_PATH_ENV} unset and {} has no VTABLE.DAT",
            default.display()
        );
        return None;
    }
    match DatRoot::open(&default) {
        Ok(root) => Some(root),
        Err(e) => {
            eprintln!(
                "SKIP (real-DAT guard): {} is not a usable install: {e}",
                default.display()
            );
            None
        }
    }
}

fn appid_paths(root: &Path, i: u8) -> (String, PathBuf, PathBuf) {
    if i == 1 {
        (
            "ROM".to_string(),
            root.join("VTABLE.DAT"),
            root.join("FTABLE.DAT"),
        )
    } else {
        let rd = format!("ROM{}", i);
        (
            rd.clone(),
            root.join(&rd).join(format!("VTABLE{}.DAT", i)),
            root.join(&rd).join(format!("FTABLE{}.DAT", i)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct SynthApp {
        rom_index: u8,
        vtable: Vec<u8>,
        ftable_words: Vec<u16>,
    }

    fn synth_root(apps: &[SynthApp]) -> (tempfile::TempDir, DatRoot) {
        let dir = tempfile::tempdir().unwrap();
        for app in apps {
            let (_rom_dir, vt_path, ft_path) = appid_paths(dir.path(), app.rom_index);
            if let Some(parent) = vt_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&vt_path, &app.vtable).unwrap();
            let mut ft_bytes = Vec::with_capacity(app.ftable_words.len() * 2);
            for w in &app.ftable_words {
                ft_bytes.extend_from_slice(&w.to_le_bytes());
            }
            fs::write(&ft_path, ft_bytes).unwrap();
        }
        let root = DatRoot::open(dir.path()).unwrap();
        (dir, root)
    }

    // Ids 1 and 2 are claimed by the base ROM; ROM2 re-claims id 1 and ROM3
    // re-claims both, each with its own FTABLE entry. The merge order in
    // LoadFileTables makes the highest claimant the owner.
    #[test]
    fn resolve_picks_highest_appid_that_claims_file_id() {
        let (_tmp, root) = synth_root(&[
            SynthApp {
                rom_index: 1,
                vtable: vec![0, 1, 1, 0, 0],
                ftable_words: vec![0x0000, 0x0080, 0x00FF, 0x0000, 0x0000],
            },
            SynthApp {
                rom_index: 2,
                vtable: vec![0, 2, 0, 2, 0],
                ftable_words: vec![0x0000, 0x0100, 0x0000, 0x0001, 0x0000],
            },
            SynthApp {
                rom_index: 3,
                vtable: vec![0, 3, 3, 0, 3],
                ftable_words: vec![0x0000, 0x0180, 0x0181, 0x0000, 0xFFFF],
            },
        ]);

        let loc1 = root.resolve(1).unwrap();
        assert_eq!(loc1.rom_dir, "ROM3");
        assert_eq!(loc1.sub_path, SubPath { dir: 3, file: 0 });

        let loc2 = root.resolve(2).unwrap();
        assert_eq!(loc2.rom_dir, "ROM3");
        assert_eq!(loc2.sub_path, SubPath { dir: 3, file: 1 });

        let loc3 = root.resolve(3).unwrap();
        assert_eq!(loc3.rom_dir, "ROM2");
        assert_eq!(loc3.sub_path, SubPath { dir: 0, file: 1 });

        let loc4 = root.resolve(4).unwrap();
        assert_eq!(loc4.rom_dir, "ROM3");
        assert_eq!(
            loc4.sub_path,
            SubPath {
                dir: 511,
                file: 127
            }
        );
        assert!(root.skipped_tables().is_empty());
        assert_eq!(root.file_id_count(), 5);
    }

    #[test]
    fn expansion_vtable_of_another_length_is_skipped() {
        let (_tmp, root) = synth_root(&[
            SynthApp {
                rom_index: 1,
                vtable: vec![0, 1, 1],
                ftable_words: vec![0x0000, 0x0080, 0x00FF],
            },
            SynthApp {
                rom_index: 2,
                vtable: vec![0, 2, 2, 2],
                ftable_words: vec![0x0000, 0x0100, 0x0101, 0x0102],
            },
            SynthApp {
                rom_index: 3,
                vtable: vec![0, 0, 3],
                ftable_words: vec![0x0000, 0x0000, 0x0180],
            },
        ]);

        let [skipped] = root.skipped_tables() else {
            panic!("exactly ROM2 must be skipped: {:?}", root.skipped_tables());
        };
        assert_eq!(skipped.rom_dir, "ROM2");
        assert_eq!(skipped.path, root.root().join("ROM2").join("VTABLE2.DAT"));
        assert_eq!((skipped.len, skipped.expected), (4, 3));
        assert_eq!(root.file_id_count(), 3);

        assert_eq!(root.resolve(1).unwrap().rom_dir, "ROM");
        assert_eq!(root.resolve(2).unwrap().rom_dir, "ROM3");
        assert!(matches!(
            root.resolve(3),
            Err(DatError::FileNotPresent { file_id: 3 })
        ));
    }

    #[test]
    fn ftable_not_twice_its_vtable_is_skipped() {
        let (_tmp, root) = synth_root(&[
            SynthApp {
                rom_index: 1,
                vtable: vec![0, 1, 1],
                ftable_words: vec![0x0000, 0x0080, 0x00FF],
            },
            SynthApp {
                rom_index: 2,
                vtable: vec![0, 2, 2],
                ftable_words: vec![0x0000, 0x0100],
            },
        ]);

        let [skipped] = root.skipped_tables() else {
            panic!("exactly ROM2 must be skipped: {:?}", root.skipped_tables());
        };
        assert_eq!(skipped.path, root.root().join("ROM2").join("FTABLE2.DAT"));
        assert_eq!((skipped.len, skipped.expected), (4, 6));
        assert_eq!(root.resolve(1).unwrap().rom_dir, "ROM");
        assert_eq!(root.app_summary().len(), 1);
    }

    // Real-install guard: every ROM's tables fit the base id space (no ROM
    // skipped), and wherever more than one ROM claims an id the highest wins.
    // The horizonxi-2023 target ships a ROM10 that re-claims base-ROM ids;
    // retail-2026-09 has no multi-claims, where this passes vacuously.
    #[test]
    fn installed_tables_all_fit_and_highest_claim_wins() {
        let Some(root) = open_test_install() else {
            return;
        };
        assert!(
            root.skipped_tables().is_empty(),
            "{:?}",
            root.skipped_tables()
        );
        let summary = root.app_summary();
        let id_count = root.file_id_count();
        assert!(id_count > 0);
        for (rom_dir, vtable_len, ftable_len) in &summary {
            assert_eq!(*vtable_len, id_count, "{rom_dir} VTABLE");
            assert_eq!(*ftable_len, id_count, "{rom_dir} FTABLE");
        }

        let mut multi_claims = 0u32;
        for file_id in 0..id_count {
            let claimants: Vec<&str> = root
                .apps
                .iter()
                .filter(|app| app.vtable.contains(file_id, app.rom_index))
                .map(|app| app.rom_dir.as_str())
                .collect();
            let Some(highest) = claimants.last() else {
                continue;
            };
            if claimants.len() > 1 {
                multi_claims += 1;
            }
            assert_eq!(
                root.resolve(file_id).unwrap().rom_dir,
                *highest,
                "file id {file_id} claimed by {claimants:?}"
            );
        }
        eprintln!(
            "{}: {} ROMs, {id_count} ids, {multi_claims} claimed by more than one ROM",
            root.root().display(),
            summary.len()
        );
    }

    #[test]
    fn resolve_returns_missing_when_no_app_claims_it() {
        let (_tmp, root) = synth_root(&[SynthApp {
            rom_index: 1,
            vtable: vec![1, 1],
            ftable_words: vec![0x0000, 0x0080],
        }]);
        assert!(matches!(
            root.resolve(5),
            Err(DatError::FileNotPresent { file_id: 5 })
        ));
    }

    #[test]
    fn path_under_assembles_correct_layout() {
        let (tmp, root) = synth_root(&[SynthApp {
            rom_index: 2,
            vtable: vec![0, 0, 2],
            ftable_words: vec![0x0000, 0x0000, 0xFFFF],
        }]);
        let loc = root.resolve(2).unwrap();
        let p = loc.path_under(&root);
        assert_eq!(p, tmp.path().join("ROM2").join("511").join("127.DAT"));
    }

    /// Writes `<overlay>/ROM2/511/127.<ext>` — the path file id 2 resolves to in
    /// `overlay_root()` — and returns it.
    fn write_overlay_entry(overlay: &Path, ext: &str, body: &[u8]) -> PathBuf {
        let dir = overlay.join("ROM2").join("511");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("127.{ext}"));
        fs::write(&path, body).unwrap();
        path
    }

    fn overlay_root() -> (tempfile::TempDir, DatRoot) {
        synth_root(&[SynthApp {
            rom_index: 2,
            vtable: vec![0, 0, 2],
            ftable_words: vec![0x0000, 0x0000, 0xFFFF],
        }])
    }

    /// What a reader of file id 2 actually gets. Asserting on bytes rather than
    /// on the path keeps these honest on a case-insensitive filesystem, where
    /// `127.DAT` and `127.dat` name the same file.
    fn served_bytes(root: &DatRoot) -> Vec<u8> {
        fs::read(root.resolve(2).unwrap().path_under(root)).unwrap()
    }

    #[test]
    fn overlays_take_precedence_in_order() {
        let (_tmp, root) = overlay_root();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write_overlay_entry(first.path(), "DAT", b"first");
        write_overlay_entry(second.path(), "DAT", b"second");

        let root = root.with_overlays(vec![first.path().into(), second.path().into()]);
        assert_eq!(served_bytes(&root), b"first");
    }

    #[test]
    fn an_overlay_that_lacks_the_file_falls_through() {
        let (tmp, root) = overlay_root();
        let empty = tempfile::tempdir().unwrap();
        let backing = tempfile::tempdir().unwrap();
        write_overlay_entry(backing.path(), "DAT", b"backing");

        let with_both = root.with_overlays(vec![empty.path().into(), backing.path().into()]);
        assert_eq!(served_bytes(&with_both), b"backing");

        // Nothing claims it: the base install path, so a missing file still
        // reports against the install the caller expects.
        let only_empty = with_both.with_overlays(vec![empty.path().into()]);
        assert_eq!(
            only_empty.resolve(2).unwrap().path_under(&only_empty),
            tmp.path().join("ROM2").join("511").join("127.DAT")
        );
    }

    // HorizonXI's XI-Pivot overlays (horizonoverrides, xiview) mix both
    // spellings, which only matters where the filesystem is case-sensitive.
    #[test]
    fn overlay_matches_a_lowercase_extension() {
        let (_tmp, root) = overlay_root();
        let overlay = tempfile::tempdir().unwrap();
        write_overlay_entry(overlay.path(), "dat", b"lower");

        let root = root.with_overlays(vec![overlay.path().into()]);
        assert_eq!(served_bytes(&root), b"lower");
    }

    // The base install mixes spellings too: retail's Windows fopen is
    // case-insensitive, so an install assembled by wine or a third-party
    // launcher can carry `127.dat` and the real client never notices. A
    // case-sensitive filesystem (a Linux user's install) must not turn that
    // file into a missing body part.
    #[test]
    fn base_install_matches_a_lowercase_extension() {
        let (tmp, root) = overlay_root();
        write_overlay_entry(tmp.path(), "dat", b"base-lower");
        assert_eq!(served_bytes(&root), b"base-lower");
    }

    /// The shape XI-Pivot actually ships, from the horizonxi-2023 target
    /// (vendor/game-files/targets/hxi) — including the Windows `root_path` that
    /// cannot resolve off Windows.
    const REAL_PIVOT_INI: &str = "\
[settings]
root_path=C:\\Program Files (x86)\\HorizonXI\\HorizonXI\\Game\\polplugins\\DATs
debug_log=false
redirect_fopens=true
[overlays]
0=horizonmusic
1=horizonoverrides
2=xiview
";

    #[test]
    fn pivot_ini_parses_in_index_order() {
        let (root_path, names) = parse_pivot_ini(REAL_PIVOT_INI);
        assert_eq!(
            root_path,
            Some(PathBuf::from(
                "C:\\Program Files (x86)\\HorizonXI\\HorizonXI\\Game\\polplugins\\DATs"
            ))
        );
        assert_eq!(names, ["horizonmusic", "horizonoverrides", "xiview"]);
    }

    #[test]
    fn pivot_ini_ignores_comments_and_orders_by_index_not_file_order() {
        let (_, names) = parse_pivot_ini(
            "; a comment\n[overlays]\n2=third\n0=first\n; another\n1=second\n[settings]\nroot_path=x\n",
        );
        assert_eq!(names, ["first", "second", "third"]);
    }

    /// Builds `<game>/SquareEnix/FINAL FANTASY XI` plus the Pivot config and
    /// overlay dirs beside it, and returns the game dir and install root.
    fn synth_pivot_install(ini: &str, overlay_dirs: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let game = tempfile::tempdir().unwrap();
        let install = game.path().join("SquareEnix").join("FINAL FANTASY XI");
        fs::create_dir_all(&install).unwrap();
        let ini_path = game.path().join(PIVOT_INI);
        fs::create_dir_all(ini_path.parent().unwrap()).unwrap();
        fs::write(&ini_path, ini).unwrap();
        for d in overlay_dirs {
            fs::create_dir_all(game.path().join(PIVOT_DAT_DIR).join(d)).unwrap();
        }
        (game, install)
    }

    #[test]
    fn discovery_falls_back_to_the_local_dat_dir_when_root_path_is_a_windows_path() {
        let (game, install) = synth_pivot_install(
            REAL_PIVOT_INI,
            &["horizonmusic", "horizonoverrides", "xiview"],
        );
        let dats = game.path().join(PIVOT_DAT_DIR);
        assert_eq!(
            discover_overlays(&install),
            vec![
                dats.join("horizonmusic"),
                dats.join("horizonoverrides"),
                dats.join("xiview"),
            ]
        );
    }

    // A name in pivot.ini with no directory behind it must not become a search
    // path that silently matches nothing.
    #[test]
    fn discovery_drops_overlays_with_no_directory() {
        let (game, install) = synth_pivot_install(REAL_PIVOT_INI, &["xiview"]);
        assert_eq!(
            discover_overlays(&install),
            vec![game.path().join(PIVOT_DAT_DIR).join("xiview")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_follows_a_symlinked_install_root_to_the_real_game_dir() {
        let (game, install) = synth_pivot_install(REAL_PIVOT_INI, &["xiview"]);
        let link_home = tempfile::tempdir().unwrap();
        let link_parent = link_home.path().join("SquareEnix");
        fs::create_dir_all(&link_parent).unwrap();
        let link = link_parent.join("FINAL FANTASY XI");
        std::os::unix::fs::symlink(&install, &link).unwrap();
        assert_eq!(
            discover_overlays(&link),
            vec![game.path().join(PIVOT_DAT_DIR).join("xiview")]
        );
    }

    #[test]
    fn discovery_yields_nothing_without_a_pivot_config() {
        let game = tempfile::tempdir().unwrap();
        let install = game.path().join("SquareEnix").join("FINAL FANTASY XI");
        fs::create_dir_all(&install).unwrap();
        assert!(discover_overlays(&install).is_empty());
    }

    #[test]
    fn overlays_can_be_swapped_on_a_live_shared_root() {
        let (_tmp, root) = overlay_root();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write_overlay_entry(first.path(), "DAT", b"first");
        write_overlay_entry(second.path(), "DAT", b"second");

        let root = std::sync::Arc::new(root.with_overlays(vec![first.path().into()]));
        assert_eq!(served_bytes(&root), b"first");

        // Through the shared handle, with no rebuild and no &mut.
        let shared = std::sync::Arc::clone(&root);
        shared.set_overlays(vec![second.path().into()]);
        assert_eq!(served_bytes(&root), b"second");

        shared.set_overlays(Vec::new());
        assert!(root.overlays().is_empty());
    }

    // Real-install guard: whatever pivot.ini the user's server ships, discovery
    // must return exactly the directories it names that exist, in index order.
    // Self-skips on a vanilla install with no Pivot, and on a shell that has
    // already overridden the list.
    #[test]
    fn discovery_matches_the_installed_pivot_config() {
        if env::var_os(OVERLAY_ENV).is_some() {
            eprintln!("SKIP: {OVERLAY_ENV} is set, which bypasses pivot discovery");
            return;
        }
        let Some(root) = open_test_install() else {
            return;
        };
        let Some(game_dir) = game_dir(root.root()) else {
            return;
        };
        let Ok(ini) = fs::read_to_string(game_dir.join(PIVOT_INI)) else {
            eprintln!("SKIP: no {PIVOT_INI} beside the install (vanilla, not a private server)");
            return;
        };

        let (root_path, names) = parse_pivot_ini(&ini);
        let dat_dir = root_path
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| game_dir.join(PIVOT_DAT_DIR));
        let expected: Vec<PathBuf> = names
            .iter()
            .map(|n| dat_dir.join(n))
            .filter(|p| p.is_dir())
            .collect();

        assert_eq!(discover_overlays(root.root()), expected);
        assert_eq!(
            root.overlays(),
            expected,
            "DatRoot::open must seed overlays from the same discovery"
        );
        assert!(
            !names.is_empty(),
            "a pivot.ini with no [overlays] entries is not a useful fixture"
        );
    }

    #[test]
    fn no_overlays_is_the_base_install() {
        let (tmp, root) = overlay_root();
        assert!(root.overlays().is_empty(), "unset env means vanilla");
        assert_eq!(
            root.resolve(2).unwrap().path_under(&root),
            tmp.path().join("ROM2").join("511").join("127.DAT")
        );
    }

    #[test]
    fn empty_install_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = DatRoot::open(dir.path()).unwrap_err();
        assert!(matches!(err, DatError::Io { .. }));
    }
}
