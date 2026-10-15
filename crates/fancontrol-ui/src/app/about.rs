//! Options > About: version, project links, log folder.

use super::*;

const REPO_URL: &str = "https://github.com/Leyukaka/fancontrol-rs";
const SITE_URL: &str = "https://leyukaka.github.io/fancontrol-rs/";

impl FanApp {
    pub(super) fn ui_about_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(t!("options.section_about").to_string())
            .default_open(false)
            .show(ui, |ui| {
                let title = format!("Fancontrol-RS {}", env!("CARGO_PKG_VERSION"));
                ui.label(egui::RichText::new(title).strong());
                ui.small(t!("about.tagline").to_string());
                ui.add_space(4.0);
                let releases = format!("{REPO_URL}/releases");
                let issues = format!("{REPO_URL}/issues/new");
                ui.hyperlink_to(t!("about.github").to_string(), REPO_URL);
                ui.hyperlink_to(t!("about.releases").to_string(), releases);
                ui.hyperlink_to(t!("about.website").to_string(), SITE_URL);
                ui.hyperlink_to(t!("about.report_bug").to_string(), issues);
                ui.add_space(4.0);
                // The log (fancontrol-rs.log) and settings live in the config dir.
                if ui.button(t!("about.open_logs").to_string()).clicked()
                    && let Ok(dir) = fancontrol_core::config_dir()
                {
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
                ui.small(t!("about.license").to_string());
            });
    }
}
