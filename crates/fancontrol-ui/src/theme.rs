//! Light / dark theme choice and theme-aware colour helpers.
//!
//! The UI palette was designed on a dark background: bright accents (cyan, lime,
//! amber) that become hard to read on white. Rather than keeping two hand-made
//! palettes, accents are darkened in light mode and neutral surfaces come from
//! the active `egui::Visuals`.

use eframe::egui::{self, Color32, Visuals};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    /// Follow the Windows light / dark app mode.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::System => t!("options.theme_system"),
            Self::Light => t!("options.theme_light"),
            Self::Dark => t!("options.theme_dark"),
        }
        .to_string()
    }
}

/// Darken a bright accent so it keeps contrast on a light background.
/// Returned unchanged in dark mode.
pub fn accent(v: &Visuals, c: Color32) -> Color32 {
    if v.dark_mode {
        c
    } else {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let d = |x: u8| (f32::from(x) * 0.62).round() as u8;
        Color32::from_rgba_unmultiplied(d(r), d(g), d(b), a)
    }
}

/// "Good" status text (was `LIGHT_GREEN`).
pub fn ok(v: &Visuals) -> Color32 {
    if v.dark_mode {
        Color32::LIGHT_GREEN
    } else {
        Color32::from_rgb(20, 120, 40)
    }
}

/// Warning text (was `YELLOW`).
pub fn warn(v: &Visuals) -> Color32 {
    if v.dark_mode {
        Color32::YELLOW
    } else {
        v.warn_fg_color
    }
}

/// Error text (was `LIGHT_RED`).
pub fn error(v: &Visuals) -> Color32 {
    if v.dark_mode {
        Color32::LIGHT_RED
    } else {
        v.error_fg_color
    }
}

/// Empty track behind a bar / gauge.
pub fn track(v: &Visuals) -> Color32 {
    if v.dark_mode {
        Color32::from_gray(40)
    } else {
        Color32::from_gray(220)
    }
}

/// Outline drawn around light markers (was `WHITE`).
pub fn marker_outline(v: &Visuals) -> Color32 {
    if v.dark_mode {
        Color32::WHITE
    } else {
        Color32::from_gray(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_is_identity_in_dark_mode() {
        let c = Color32::from_rgb(120, 220, 255);
        assert_eq!(accent(&Visuals::dark(), c), c);
    }

    #[test]
    fn accent_darkens_in_light_mode() {
        let c = Color32::from_rgb(120, 220, 255);
        let d = accent(&Visuals::light(), c);
        assert!(d.r() < c.r() && d.g() < c.g() && d.b() < c.b());
    }

    #[test]
    fn theme_choice_roundtrips_snake_case() {
        let json = serde_json::to_string(&ThemeChoice::System).unwrap();
        assert_eq!(json, "\"system\"");
        let back: ThemeChoice = serde_json::from_str("\"light\"").unwrap();
        assert_eq!(back, ThemeChoice::Light);
    }
}
