//! Config directory helpers (`%APPDATA%/fancontrol-rs` on Windows).

use crate::error::{CoreError, Result};
use std::path::PathBuf;

const APP_QUALIFIER: &str = "eu";
const APP_ORGANIZATION: &str = "fancontrol-rs";
const APP_NAME: &str = "fancontrol-rs";

/// Return the application config directory.
///
/// On Windows this resolves to something like:
/// `C:\Users\<user>\AppData\Roaming\eu\fancontrol-rs\fancontrol-rs`
///
/// We intentionally use the `directories` crate so paths stay idiomatic per OS.
pub fn config_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
        .map(|d| d.config_dir().to_path_buf())
        .ok_or_else(|| CoreError::ConfigPath("could not resolve project dirs".into()))
}

/// Directory where profile JSON files are stored.
pub fn profiles_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("profiles"))
}

/// Ensure config and profiles directories exist.
pub fn ensure_config_dirs() -> Result<PathBuf> {
    let root = config_dir()?;
    std::fs::create_dir_all(&root)?;
    let profiles = root.join("profiles");
    std::fs::create_dir_all(&profiles)?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_resolves() {
        let dir = config_dir().expect("config dir");
        assert!(dir.to_string_lossy().contains("fancontrol-rs"));
    }
}
