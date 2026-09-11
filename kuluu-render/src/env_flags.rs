//! Read-once env switches for gated diagnostics.
//!
//! Kept outside `particle_sim` because that module is cfg-gated out on wasm32 while the
//! KULUU_* probes live in modules that compile there too; a gate must not reach into a
//! module its own target excludes.

use std::sync::OnceLock;

/// Any value enables, unset disables; the answer is memoized in `cell`.
pub(crate) fn env_flag(cell: &'static OnceLock<bool>, name: &str) -> bool {
    *cell.get_or_init(|| std::env::var_os(name).is_some())
}
