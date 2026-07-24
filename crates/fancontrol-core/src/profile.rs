//! Profile load / save as JSON under the app config directory.

use crate::config::{ensure_config_dirs, profiles_dir};
use crate::error::{CoreError, Result};
use crate::models::Profile;
use std::fs;
use std::path::{Path, PathBuf};

fn profile_path(id: &str) -> Result<PathBuf> {
    // Sanitize id to a safe file name (alphanumeric, dash, underscore).
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        return Err(CoreError::InvalidCurve("empty profile id".into()));
    }
    Ok(profiles_dir()?.join(format!("{safe}.json")))
}

/// Save a profile to disk (creates config dirs if needed).
pub fn save_profile(profile: &Profile) -> Result<PathBuf> {
    profile.validate().map_err(CoreError::InvalidCurve)?;
    ensure_config_dirs()?;
    let path = profile_path(profile.id.as_str())?;
    let json = serde_json::to_string_pretty(profile)?;
    fs::write(&path, json)?;
    tracing::info!(path = %path.display(), id = %profile.id.as_str(), "profile saved");
    Ok(path)
}

/// Load a profile by id from the profiles directory.
pub fn load_profile(id: &str) -> Result<Profile> {
    let path = profile_path(id)?;
    if !path.exists() {
        return Err(CoreError::ProfileNotFound(id.to_string()));
    }
    let data = fs::read_to_string(&path)?;
    let profile: Profile = serde_json::from_str(&data)?;
    profile.validate().map_err(CoreError::InvalidCurve)?;
    Ok(profile)
}

/// Load a profile from an arbitrary path (useful for tests / import).
pub fn load_profile_from_path(path: &Path) -> Result<Profile> {
    let data = fs::read_to_string(path)?;
    let profile: Profile = serde_json::from_str(&data)?;
    profile.validate().map_err(CoreError::InvalidCurve)?;
    Ok(profile)
}

/// List profile ids found in the profiles directory (file stems).
pub fn list_profiles() -> Result<Vec<String>> {
    let dir = profiles_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                ids.push(stem.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

/// Delete a profile by id.
pub fn delete_profile(id: &str) -> Result<()> {
    let path = profile_path(id)?;
    if !path.exists() {
        return Err(CoreError::ProfileNotFound(id.to_string()));
    }
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CurvePoint, FanCurve, Profile};
    use std::collections::HashMap;

    fn sample_profile() -> Profile {
        let mut p = Profile::new("default", "Default");
        p.curves.push(FanCurve {
            id: crate::models::CurveId::new("quiet"),
            name: "Quiet".into(),
            points: vec![CurvePoint::new(30.0, 20), CurvePoint::new(70.0, 100)],
            hysteresis_c: 2.0,
            response_time_s: 0.0,
        });
        p.assignments = HashMap::from([("mock.cpu_fan".into(), "quiet".into())]);
        p
    }

    #[test]
    fn roundtrip_json_in_temp_dir() {
        let profile = sample_profile();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("default.json");
        let json = serde_json::to_string_pretty(&profile).unwrap();
        fs::write(&path, json).unwrap();

        let loaded = load_profile_from_path(&path).unwrap();
        assert_eq!(loaded.id.as_str(), "default");
        assert_eq!(loaded.curves.len(), 1);
        assert_eq!(loaded.assignment_for("mock.cpu_fan").unwrap().name, "Quiet");
    }

    #[test]
    fn validate_rejects_bad_assignment() {
        let mut p = sample_profile();
        p.assignments.insert("fan1".into(), "missing_curve".into());
        assert!(p.validate().is_err());
    }
}
