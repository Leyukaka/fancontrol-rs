//! Theme choice (system / light / dark / neon), fonts, and theme-aware colours.
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
    /// Experimental: dark neon palette with an animated RGB border.
    Neon,
}

impl ThemeChoice {
    pub const ALL: [Self; 4] = [Self::System, Self::Light, Self::Dark, Self::Neon];

    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark | Self::Neon => egui::ThemePreference::Dark,
        }
    }

    /// Whether this theme animates (the neon border) and needs a steady repaint.
    pub fn is_animated(self) -> bool {
        self == Self::Neon
    }

    pub fn label(self) -> String {
        match self {
            Self::System => t!("options.theme_system"),
            Self::Light => t!("options.theme_light"),
            Self::Dark => t!("options.theme_dark"),
            Self::Neon => t!("options.theme_neon"),
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

/// Apply a theme choice: light / dark preference plus, for Neon, its own dark
/// visuals (reset to the stock dark visuals otherwise).
pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(choice.preference());
    let dark = if choice == ThemeChoice::Neon {
        neon_visuals()
    } else {
        Visuals::dark()
    };
    ctx.set_visuals_of(egui::Theme::Dark, dark);
}

const NEON_CYAN: Color32 = Color32::from_rgb(0, 229, 255);
const NEON_MAGENTA: Color32 = Color32::from_rgb(255, 46, 196);

fn neon_visuals() -> Visuals {
    let mut v = Visuals::dark();
    let radius = egui::CornerRadius::same(6);
    v.panel_fill = Color32::from_rgb(9, 9, 16);
    v.window_fill = Color32::from_rgb(13, 13, 24);
    v.extreme_bg_color = Color32::from_rgb(4, 4, 9);
    v.faint_bg_color = Color32::from_rgb(16, 16, 30);
    v.window_stroke = egui::Stroke::new(1.0, NEON_CYAN.gamma_multiply(0.45));
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.hyperlink_color = NEON_CYAN;
    v.selection.bg_fill = NEON_MAGENTA.gamma_multiply(0.55);
    v.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, NEON_CYAN.gamma_multiply(0.25));
    for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = radius;
    }
    w.inactive.weak_bg_fill = Color32::from_rgb(22, 22, 40);
    w.inactive.bg_fill = Color32::from_rgb(22, 22, 40);
    w.inactive.bg_stroke = egui::Stroke::new(1.0, NEON_CYAN.gamma_multiply(0.35));
    w.hovered.weak_bg_fill = Color32::from_rgb(30, 30, 56);
    w.hovered.bg_stroke = egui::Stroke::new(1.0, NEON_CYAN);
    w.active.weak_bg_fill = Color32::from_rgb(40, 20, 60);
    w.active.bg_stroke = egui::Stroke::new(1.5, NEON_MAGENTA);
    v
}

/// Neon theme: a glowing border around the window whose hue runs around the
/// edges over time. `time_s` drives the animation.
pub fn paint_neon_border(ctx: &egui::Context, time_s: f64) {
    const SEGMENTS: usize = 160;
    // Wide faint passes first, sharp core last: a cheap glow.
    const PASSES: [(f32, f32); 3] = [(9.0, 0.10), (4.0, 0.30), (1.5, 1.0)];
    let rect = ctx.content_rect().shrink(1.0);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("neon_border"),
    ));
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let perimeter = 2.0 * (rect.width() + rect.height());
    // Point at distance `d` along the edges, clockwise from the top-left corner.
    let at = |mut d: f32| {
        for (i, &a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % 4];
            let len = a.distance(b);
            if d <= len || i == 3 {
                return a + (b - a) * (d / len.max(1.0)).min(1.0);
            }
            d -= len;
        }
        corners[0]
    };
    let shift = (time_s * 0.12).fract() as f32;
    for (width, alpha) in PASSES {
        for i in 0..SEGMENTS {
            let f0 = i as f32 / SEGMENTS as f32;
            let f1 = (i + 1) as f32 / SEGMENTS as f32;
            let hue = (f0 + shift).fract();
            let color = Color32::from(egui::ecolor::Hsva::new(hue, 1.0, 1.0, alpha));
            painter.line_segment([at(f0 * perimeter), at(f1 * perimeter)], (width, color));
        }
    }
}

/// Use the Windows UI fonts (Segoe UI, Cascadia Mono) instead of egui's built-in
/// ones, falling back to those if a file is missing.
/// The CJK fallback is always appended (egui's fonts have no CJK glyphs).
pub fn install_fonts(ctx: &egui::Context, system_fonts: bool) {
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, data: egui::FontData, family: egui::FontFamily, first: bool| {
        fonts
            .font_data
            .insert(name.to_owned(), std::sync::Arc::new(data));
        let list = fonts.families.entry(family).or_default();
        if first {
            list.insert(0, name.to_owned());
        } else {
            list.push(name.to_owned());
        }
    };
    if system_fonts {
        let dir = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        if let Some(data) = read_font(&dir, "segoeui.ttf") {
            add("segoe_ui", data, egui::FontFamily::Proportional, true);
        }
        if let Some(data) = read_font(&dir, "CascadiaMono.ttf") {
            add("cascadia_mono", data, egui::FontFamily::Monospace, true);
        }
    }
    let cjk =
        egui::FontData::from_static(include_bytes!("../assets/fonts/NotoSansCJK-Regular.ttc"));
    add(
        "noto_sans_cjk",
        cjk.clone(),
        egui::FontFamily::Proportional,
        false,
    );
    add("noto_sans_cjk", cjk, egui::FontFamily::Monospace, false);
    ctx.set_fonts(fonts);
}

fn read_font(dir: &std::path::Path, file: &str) -> Option<egui::FontData> {
    let bytes = std::fs::read(dir.join(file)).ok()?;
    Some(egui::FontData::from_owned(bytes))
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
