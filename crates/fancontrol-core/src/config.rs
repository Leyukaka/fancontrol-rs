//! Config directory helpers (`%APPDATA%/fancontrol-rs` on Windows).

use crate::error::{CoreError, Result};
use std::path::{Path, PathBuf};

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

/// Replace `path` with `contents` without ever leaving a truncated file behind:
/// write a sibling temp file, then rename it over the target (atomic on the same
/// volume; `fs::rename` replaces an existing file on Windows too). A crash mid-write
/// leaves the previous file intact instead of an empty or half-written one.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        write_atomic(&path, "first").expect("write 1");
        write_atomic(&path, "second").expect("write 2");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "second");
        assert!(!dir.path().join("settings.json.tmp").exists());
    }

    #[test]
    fn config_dir_resolves() {
        let dir = config_dir().expect("config dir");
        assert!(dir.to_string_lossy().contains("fancontrol-rs"));
    }
}
