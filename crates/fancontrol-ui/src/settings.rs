//! UI preferences persisted next to channel-map.

use crate::shaders::GraphStyle;
use fancontrol_core::config::{config_dir, ensure_config_dirs};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const FILE: &str = "ui-settings.json";

const WINDOW_ALLOWED: [u16; 4] = [10, 20, 30, 60];
const SAMPLE_ALLOWED: [u16; 4] = [1, 2, 5, 10];
pub const SHADER_FPS_ALLOWED: [u16; 4] = [30, 60, 90, 120];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default = "default_true")]
    pub hide_zero_rpm: bool,
    /// Hide controls currently reporting 0% duty. Opt-in (default off): unlike
    /// hide_zero_rpm's passive fan readouts, controls are interactive/actionable,
    /// so hiding them by default risks hiding something the user meant to act on.
    #[serde(default)]
    pub hide_zero_duty_controls: bool,
    #[serde(default = "default_true", alias = "show_cpu_graph")]
    pub show_graph_panel: bool,
    #[serde(default = "default_true")]
    pub show_host_sensors: bool,
    /// When true and hardware writes allowed, apply profile curves each poll tick.
    #[serde(default = "default_true")]
    pub auto_apply_curves: bool,
    /// Visible history window for the graph panel (minutes).
    #[serde(default = "default_graph_window_minutes")]
    pub graph_window_minutes: u16,
    /// Minimum interval between graph samples (seconds).
    #[serde(default = "default_graph_sample_secs")]
    pub graph_sample_secs: u16,
    /// One-shot migration marker: product defaults (curve control on, etc.).
    #[serde(default)]
    pub product_defaults_applied: bool,
    /// Last profile switched to / saved in the UI — auto-loaded on next startup.
    #[serde(default)]
    pub last_profile_id: Option<String>,
    /// UI language code (e.g. "en", "fr"). `None` = not yet chosen → OS-locale detection.
    #[serde(default)]
    pub language: Option<String>,
    /// Visual style for the graph panel: the classic line graph, or one of the
    /// "fun" shader-based visualizations. Shader styles are opt-in (default Classic).
    #[serde(default)]
    pub graph_style: GraphStyle,
    #[serde(default = "default_shader_speed", alias = "fractal_speed")]
    pub shader_speed: f32,
    #[serde(default = "default_shader_color_a", alias = "fractal_color_a")]
    pub shader_color_a: [f32; 3],
    #[serde(default = "default_shader_color_b", alias = "fractal_color_b")]
    pub shader_color_b: [f32; 3],
    /// Shader animation frame rate. Higher values look smoother but use more
    /// GPU/CPU while a shader style is active.
    #[serde(default = "default_shader_fps")]
    pub shader_fps: u16,
    /// Deprecated: superseded by `graph_style`. Kept only so old
    /// `ui-settings.json` files deserialize; read once by the one-shot
    /// migration below, then ignored. `pub(crate)` (not fully private) only
    /// so other modules' tests can still use `..UiSettings::default()`.
    #[serde(default)]
    pub(crate) show_fractal: bool,
    /// One-shot migration marker: `show_fractal` -> `graph_style`.
    #[serde(default)]
    pub(crate) graph_style_migrated: bool,
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

fn default_shader_speed() -> f32 {
    1.0
}

fn default_shader_color_a() -> [f32; 3] {
    [0.2, 0.7, 0.9]
}

fn default_shader_color_b() -> [f32; 3] {
    [1.0, 0.0, 1.0]
}

fn default_shader_fps() -> u16 {
    60
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            hide_zero_rpm: true,
            hide_zero_duty_controls: false,
            show_graph_panel: true,
            show_host_sensors: true,
            auto_apply_curves: true,
            graph_window_minutes: default_graph_window_minutes(),
            graph_sample_secs: default_graph_sample_secs(),
            product_defaults_applied: true,
            last_profile_id: None,
            language: None,
            graph_style: GraphStyle::default(),
            shader_speed: default_shader_speed(),
            shader_color_a: default_shader_color_a(),
            shader_color_b: default_shader_color_b(),
            shader_fps: default_shader_fps(),
            show_fractal: false,
            graph_style_migrated: true,
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
        // Upgrade older installs: carry the old `show_fractal` toggle over to
        // the new graph-style picker, once.
        if !s.graph_style_migrated {
            if s.show_fractal {
                s.graph_style = GraphStyle::FractalPyramid;
            }
            s.graph_style_migrated = true;
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

    /// Clamp graph/shader options to allowed discrete values.
    pub fn clamp_graph_options(&mut self) {
        if !WINDOW_ALLOWED.contains(&self.graph_window_minutes) {
            self.graph_window_minutes = default_graph_window_minutes();
        }
        if !SAMPLE_ALLOWED.contains(&self.graph_sample_secs) {
            self.graph_sample_secs = default_graph_sample_secs();
        }
        if !SHADER_FPS_ALLOWED.contains(&self.shader_fps) {
            self.shader_fps = default_shader_fps();
        }
    }
}
