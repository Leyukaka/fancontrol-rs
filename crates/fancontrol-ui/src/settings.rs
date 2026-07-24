//! UI preferences persisted next to channel-map.

use fancontrol_core::config::{config_dir, ensure_config_dirs};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const FILE: &str = "ui-settings.json";

const WINDOW_ALLOWED: [u16; 4] = [10, 20, 30, 60];
const SAMPLE_ALLOWED: [u16; 4] = [1, 2, 5, 10];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default = "default_true")]
    pub hide_zero_rpm: bool,
    #[serde(default = "default_true")]
    pub show_cpu_graph: bool,
    #[serde(default = "default_true")]
    pub show_host_sensors: bool,
    /// When true and hardware writes allowed, apply profile curves each poll tick.
    #[serde(default = "default_true")]
    pub auto_apply_curves: bool,
    /// Visible history window for the CPU graph (minutes).
    #[serde(default = "default_graph_window_minutes")]
    pub graph_window_minutes: u16,
    /// Minimum interval between graph samples (seconds).
    #[serde(default = "default_graph_sample_secs")]
    pub graph_sample_secs: u16,
    /// One-shot migration marker: product defaults (curve control on, etc.).
    #[serde(default)]
    pub product_defaults_applied: bool,
}

fn default_true() -> bool {
    true
}

fn default_graph_window_minutes() -> u16 {
    10
}

fn default_graph_sample_secs() -> u16 {
    2
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            hide_zero_rpm: true,
            show_cpu_graph: true,
            show_host_sensors: true,
            auto_apply_curves: true,
            graph_window_minutes: default_graph_window_minutes(),
            graph_sample_secs: default_graph_sample_secs(),
            product_defaults_applied: true,
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
        let mut s: Self = serde_json::from_str(&data).unwrap_or_default();
        s.clamp_graph_options();
        // Upgrade older installs: enable curve control once (user can still turn it off).
        if !s.product_defaults_applied {
            s.auto_apply_curves = true;
            s.product_defaults_applied = true;
            s.save();
        }
        s
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

    /// Clamp graph options to allowed discrete values (invalid → 10m / 2s).
    pub fn clamp_graph_options(&mut self) {
        if !WINDOW_ALLOWED.contains(&self.graph_window_minutes) {
            self.graph_window_minutes = default_graph_window_minutes();
        }
        if !SAMPLE_ALLOWED.contains(&self.graph_sample_secs) {
            self.graph_sample_secs = default_graph_sample_secs();
        }
    }
}
