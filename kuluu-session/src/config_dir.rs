use std::path::PathBuf;

use anyhow::{anyhow, Result};

pub use ffxi_dat::install::APP_DIR;

pub fn config_file(name: &str) -> Result<PathBuf> {
    let base =
        dirs::config_dir().ok_or_else(|| anyhow!("could not resolve a user config directory"))?;
    Ok(base.join(APP_DIR).join(name))
}

pub fn data_dir(name: &str) -> Result<PathBuf> {
    let base =
        dirs::data_dir().ok_or_else(|| anyhow!("could not resolve a user data directory"))?;
    Ok(base.join(APP_DIR).join(name))
}

pub fn cache_dir(name: &str) -> Result<PathBuf> {
    let base =
        dirs::cache_dir().ok_or_else(|| anyhow!("could not resolve a user cache directory"))?;
    Ok(base.join(APP_DIR).join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_uses_player_facing_dir() {
        let p = config_file("launcher.json").unwrap();
        assert!(p.ends_with("kuluu/launcher.json"), "got {}", p.display());
    }
}
