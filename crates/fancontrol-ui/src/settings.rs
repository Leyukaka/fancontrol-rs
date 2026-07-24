//! UI preferences persisted next to channel-map.

use fancontrol_core::config::{config_dir, ensure_config_dirs};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const FILE: &str = "ui-settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default = "default_true")]
    pub hide_zero_rpm: bool,
    #[serde(default = "default_true")]
    pub show_cpu_graph: bool,
    #[serde(default = "default_true")]
    pub show_host_sensors: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            hide_zero_rpm: true,
            show_cpu_graph: true,
            show_host_sensors: true,
        }
    }
}

impl UiSettings {
    pub fn path() -> Option<PathBuf> {
        config_dir().ok().map(|d| d.join(FILE))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(data) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&data).unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = ensure_config_dirs();
        let Some(path) = Self::path() else {
            return;
        };
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}
