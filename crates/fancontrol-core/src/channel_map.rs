//! User-editable display names for sensors and controls.

use crate::config::{config_dir, ensure_config_dirs};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MAP_FILE: &str = "channel-map.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelMap {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub sensors: HashMap<String, String>,
    #[serde(default)]
    pub controls: HashMap<String, String>,
}

fn default_version() -> u32 {
    1
}

impl Default for ChannelMap {
    fn default() -> Self {
        Self {
            version: 1,
            sensors: HashMap::new(),
            controls: HashMap::new(),
        }
    }
}

impl ChannelMap {
    pub fn map_path() -> Result<PathBuf> {
        Ok(config_dir()?.join(MAP_FILE))
    }

    /// Seed labels for the validated owner NCT668x layout (+ mock ids).
    pub fn owner_nct668_seed() -> Self {
        let mut m = Self::default();
        let sensors = [
            ("pawnio.0.temp.CPU", "CPU"),
            ("pawnio.0.temp.System", "System"),
            ("pawnio.0.temp.MOS", "MOS"),
            ("pawnio.0.temp.PCH", "PCH"),
            ("pawnio.0.fan0", "Fan 0"),
            ("pawnio.0.fan1", "Fan 1"),
            ("pawnio.0.fan12", "Fan 12"),
            ("pawnio.0.fan13", "Fan 13"),
            ("pawnio.0.fan14", "Fan 14"),
            ("mock.cpu_temp", "CPU Package (mock)"),
            ("mock.gpu_temp", "GPU Core (mock)"),
            ("mock.cpu_fan_rpm", "CPU Fan (mock)"),
            ("mock.case_fan_rpm", "Case Fan (mock)"),
        ];
        for (id, name) in sensors {
            m.sensors.insert(id.into(), name.into());
        }
        let controls = [
            ("pawnio.0.ctrl0", "Control 0 / Fan 0"),
            ("pawnio.0.ctrl1", "Control 1 / Fan 1"),
            ("pawnio.0.ctrl2", "Control 2"),
            ("pawnio.0.ctrl3", "Control 3"),
            ("pawnio.0.ctrl9", "Control 9 (ext)"),
            ("pawnio.0.ctrl10", "Control 10 (ext)"),
            ("pawnio.0.ctrl11", "Control 11 (ext)"),
            ("pawnio.0.ctrl12", "Control 12 (ext)"),
            ("pawnio.0.ctrl13", "Control 13 (ext)"),
            ("pawnio.0.ctrl14", "Control 14 (ext)"),
            ("pawnio.0.ctrl15", "Control 15 (ext)"),
            ("mock.cpu_fan", "CPU Fan (mock)"),
            ("mock.case_fan", "Case Fan (mock)"),
        ];
        for (id, name) in controls {
            m.controls.insert(id.into(), name.into());
        }
        for i in 0..17 {
            m.sensors
                .entry(format!("pawnio.0.fan{i}"))
                .or_insert_with(|| format!("Fan {i}"));
        }
        m
    }

    pub fn sensor_name<'a>(&'a self, id: &str, fallback: &'a str) -> &'a str {
        self.sensors
            .get(id)
            .map(String::as_str)
            .unwrap_or(fallback)
    }

    pub fn control_name<'a>(&'a self, id: &str, fallback: &'a str) -> &'a str {
        self.controls
            .get(id)
            .map(String::as_str)
            .unwrap_or(fallback)
    }

    pub fn load() -> Result<Self> {
        let path = Self::map_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let data = fs::read_to_string(path)?;
        let map: Self = serde_json::from_str(&data)?;
        Ok(map)
    }

    /// Load map or owner seed if missing (does not write).
    pub fn load_or_seed() -> Result<Self> {
        let path = Self::map_path()?;
        if path.exists() {
            Self::load_from_path(&path)
        } else {
            Ok(Self::owner_nct668_seed())
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        ensure_config_dirs()?;
        let path = Self::map_path()?;
        self.save_to_path(&path)?;
        Ok(path)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Write seed file if absent; return path + whether created.
    pub fn init_seed_if_missing() -> Result<(PathBuf, bool)> {
        ensure_config_dirs()?;
        let path = Self::map_path()?;
        if path.exists() {
            return Ok((path, false));
        }
        let seed = Self::owner_nct668_seed();
        seed.save_to_path(&path)?;
        Ok((path, true))
    }

    /// Force overwrite with seed (explicit).
    pub fn write_seed() -> Result<PathBuf> {
        ensure_config_dirs()?;
        let seed = Self::owner_nct668_seed();
        let path = seed.save()?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_has_owner_channels() {
        let m = ChannelMap::owner_nct668_seed();
        assert_eq!(m.sensor_name("pawnio.0.temp.CPU", "x"), "CPU");
        assert_eq!(m.control_name("pawnio.0.ctrl1", "x"), "Control 1 / Fan 1");
    }

    #[test]
    fn roundtrip_tempfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel-map.json");
        let m = ChannelMap::owner_nct668_seed();
        m.save_to_path(&path).unwrap();
        let loaded = ChannelMap::load_from_path(&path).unwrap();
        assert_eq!(loaded.sensors.get("pawnio.0.fan0").unwrap(), "Fan 0");
    }

    #[test]
    fn missing_file_is_default() {
        // load_from_path on missing should error; load() uses default when missing via map_path
        let err = ChannelMap::load_from_path(Path::new("Z:/no/such/channel-map.json"));
        assert!(err.is_err());
    }

    #[test]
    fn fallback_when_unmapped() {
        let m = ChannelMap::default();
        assert_eq!(m.sensor_name("unknown", "fallback"), "fallback");
    }
}
