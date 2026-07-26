//! Language resolution: persisted choice, OS-locale detection, and display names.

use crate::settings::UiSettings;

pub const SUPPORTED: [&str; 8] = ["en", "fr", "de", "es", "it", "zh", "ja", "lb"];

/// Native self-name for each supported language, for the Options-panel picker.
pub fn display_name_for(code: &str) -> &'static str {
    match code {
        "en" => "English",
        "fr" => "Français",
        "de" => "Deutsch",
        "es" => "Español",
        "it" => "Italiano",
        "zh" => "中文",
        "ja" => "日本語",
        "lb" => "Lëtzebuergesch",
        _ => "English",
    }
}

/// Pick the startup locale: persisted choice if valid, else OS locale, else English.
pub fn resolve_startup_locale(settings: &UiSettings) -> String {
    if let Some(lang) = &settings.language {
        if SUPPORTED.contains(&lang.as_str()) {
            return lang.clone();
        }
    }
    sys_locale::get_locale()
        .and_then(|os_locale| {
            let primary = os_locale.split(['-', '_']).next()?.to_lowercase();
            SUPPORTED.contains(&primary.as_str()).then_some(primary)
        })
        .unwrap_or_else(|| "en".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_language_wins() {
        let settings = UiSettings {
            language: Some("de".to_string()),
            ..UiSettings::default()
        };
        assert_eq!(resolve_startup_locale(&settings), "de");
    }

    #[test]
    fn unsupported_persisted_language_falls_back() {
        let settings = UiSettings {
            language: Some("pt".to_string()),
            ..UiSettings::default()
        };
        // Falls through to OS-locale detection (or "en" if that's unsupported too),
        // but must never return the unsupported persisted code itself.
        assert_ne!(resolve_startup_locale(&settings), "pt");
    }

    #[test]
    fn no_persisted_language_never_panics() {
        let settings = UiSettings::default();
        let locale = resolve_startup_locale(&settings);
        assert!(SUPPORTED.contains(&locale.as_str()));
    }
}
