//! Project automation, invoked via the `cargo xtask` alias (.cargo/config.toml).
//!
//! FFXI installs are managed by the product binary (`kuluu install <verb>`),
//! not here; the checkout holds no game files.
//!
//! ## `cargo xtask install-hooks [--check]`
//!
//! Activate the versioned git hooks in `.githooks/` for this clone by pointing
//! `core.hooksPath` at it (the fmt+clippy pre-push gate plus Beads lifecycle
//! hooks). Mirrors
//! `scripts/install-hooks.sh`, kept as a compile-free fast path. It's per-clone
//! because `git config` writes the uncommitted `.git/config` — git won't let a
//! repo auto-enable its own hooks. `--check` only verifies (non-zero exit when
//! inactive) so the README / CI / a setup doctor can assert the gate is live.

mod dlss;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Where the install verbs went; kept so an old habit gets a pointer, not a
/// usage dump.
const INSTALL_CLI: &str = "cargo run -p kuluu -- install <list|which|use|path|link|get|update>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dlss") => match dlss::run(&args[1..], &workspace_root()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("ffxi-client" | "game" | "install") => {
            eprintln!("FFXI installs are managed by the product binary:\n  {INSTALL_CLI}");
            ExitCode::FAILURE
        }
        Some("install-hooks") => match cmd_install_hooks(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some(other) => {
            eprintln!("unknown xtask `{other}`\n");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage: cargo xtask install-hooks [--check]\n\
         \x20      cargo xtask dlss <check|build>\n\
         \n\
         Activate the versioned git hooks (.githooks/) for this clone.\n\
         --check     verify the versioned hooks are active; non-zero exit if not\n\
         \n\
         FFXI installs: {INSTALL_CLI}"
    );
}

// --- git hook activation ---

/// The hooks dir we point `core.hooksPath` at, relative to the workspace root.
const HOOKS_DIR: &str = ".githooks";

/// Activate (or, with `--check`, verify) the versioned git hooks for this clone.
/// Sets `core.hooksPath=.githooks` so the pre-push gate and Beads lifecycle hooks
/// run. The same effect as `scripts/install-hooks.sh`; both write byte-identical
/// config so they can't drift.
fn cmd_install_hooks(args: &[String]) -> Result<(), String> {
    let mut check = false;
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            s => {
                return Err(format!(
                    "unknown flag `{s}` (install-hooks takes only --check)"
                ))
            }
        }
    }

    require_tool("git")?;
    let workspace = workspace_root();
    // Guard against running outside the repo: the hook we're enabling must exist.
    if !workspace.join(HOOKS_DIR).join("pre-push").is_file() {
        return Err(format!(
            "{HOOKS_DIR}/pre-push not found under {} — run this from the ffxi repo",
            show(&workspace)
        ));
    }

    if check {
        return match git_config_get(&workspace, "core.hooksPath")?.as_deref() {
            Some(HOOKS_DIR) => {
                println!("ok: git hooks active (core.hooksPath={HOOKS_DIR})");
                Ok(())
            }
            Some(other) => Err(format!(
                "git hooks NOT active: core.hooksPath={other} (expected {HOOKS_DIR})\n\
                 run: cargo xtask install-hooks"
            )),
            None => Err("git hooks NOT active: core.hooksPath is unset\n\
                 run: cargo xtask install-hooks"
                .to_string()),
        };
    }

    git_config_set(&workspace, "core.hooksPath", HOOKS_DIR)?;
    make_executable(&workspace.join(HOOKS_DIR));
    println!("installed: core.hooksPath={HOOKS_DIR} (checks and Beads integration active)");
    println!("bypass a push with: git push --no-verify");
    Ok(())
}

/// Set a git config key in the workspace repo (writes `.git/config`).
fn git_config_set(workspace: &Path, key: &str, value: &str) -> Result<(), String> {
    let status = Command::new("git")
        .current_dir(workspace)
        .args(["config", key, value])
        .status()
        .map_err(|e| format!("running git config: {e}"))?;
    if !status.success() {
        return Err(format!("`git config {key} {value}` failed ({status})"));
    }
    Ok(())
}

/// Read a git config key; `None` if unset (`git config --get` exits 1 for that).
fn git_config_get(workspace: &Path, key: &str) -> Result<Option<String>, String> {
    let out = Command::new("git")
        .current_dir(workspace)
        .args(["config", "--get", key])
        .output()
        .map_err(|e| format!("running git config: {e}"))?;
    if !out.status.success() {
        return Ok(None);
    }
    let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!val.is_empty()).then_some(val))
}

/// `chmod +x` every file in the hooks dir so git can run them. No-op on Windows,
/// where git ignores the unix exec bit.
#[cfg(unix)]
fn make_executable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&p) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o111);
            let _ = std::fs::set_permissions(&p, perms);
        }
    }
}

#[cfg(not(unix))]
fn make_executable(_dir: &Path) {}

fn require_tool(name: &str) -> Result<(), String> {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|_| ())
        .map_err(|_| format!("`{name}` not found on PATH — install it and retry"))
}

// --- small fs helpers (std-only) ---

fn show(p: &Path) -> String {
    p.strip_prefix(workspace_root())
        .unwrap_or(p)
        .display()
        .to_string()
}

/// Workspace root = the dir holding this xtask crate's parent. `CARGO_MANIFEST_DIR`
/// is `<workspace>/xtask`, so its parent is the workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
