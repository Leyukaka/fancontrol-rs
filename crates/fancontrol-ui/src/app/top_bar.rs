//! Top bar: graph window / sampling controls and the panel toggles.

use super::*;

impl FanApp {
    pub(super) fn ui_graph_controls(&mut self, ui: &mut egui::Ui) {
        let mut dirty = false;
        ui.horizontal(|ui| {
            ui.label(t!("graph_controls.window_label").to_string());
            for m in GRAPH_WINDOWS {
                let selected = self.settings.graph_window_minutes == m;
                if ui.selectable_label(selected, format!("{m}m")).clicked() && !selected {
                    self.settings.graph_window_minutes = m;
                    dirty = true;
                }
            }
            ui.separator();
            ui.label(t!("graph_controls.sample_label").to_string());
            for s in GRAPH_SAMPLES {
                let selected = self.settings.graph_sample_secs == s;
                if ui.selectable_label(selected, format!("{s}s")).clicked() && !selected {
                    self.settings.graph_sample_secs = s;
                    dirty = true;
                }
            }
        });
        if dirty {
            self.settings.clamp_graph_options();
            self.settings.save();
            for h in self.histories.values_mut() {
                h.configure(
                    self.settings.graph_window_minutes,
                    self.settings.graph_sample_secs,
                );
            }
            self.cpu_power_history.configure(
                self.settings.graph_window_minutes,
                self.settings.graph_sample_secs,
            );
            self.gpu_power_history.configure(
                self.settings.graph_window_minutes,
                self.settings.graph_sample_secs,
            );
            self.load_history.configure(
                self.settings.activity_window_minutes,
                1, // activity worker ~1 Hz
            );
        }
    }

    /// Top-bar view toggles, Curve control and Options, laid out right to left.
    /// Records their width so the next frame knows whether they fit next to the
    /// left-hand controls or need a row of their own.
    pub(super) fn ui_top_toggles(&mut self, ui: &mut egui::Ui) {
        // Laid out right to left, and the order follows where each panel sits:
        // Sensors / GPU / CPU (top row) on the left, then Activity, the
        // Temperatures / Fans / Controls lists, Curves (bottom), Options (right panel).
        if ui
            .selectable_label(
                self.show_settings,
                format!("⚙ {}", t!("top_bar.options_button")),
            )
            .clicked()
        {
            self.show_settings = !self.show_settings;
        }
        if ui
            .selectable_label(self.show_curves, t!("top_bar.curves_toggle").to_string())
            .on_hover_text(t!("top_bar.curves_toggle_tooltip").to_string())
            .clicked()
        {
            self.show_curves = !self.show_curves;
        }
        if ui
            .selectable_label(
                self.show_controls,
                t!("top_bar.controls_toggle").to_string(),
            )
            .on_hover_text(t!("top_bar.controls_toggle_tooltip").to_string())
            .clicked()
        {
            self.show_controls = !self.show_controls;
        }
        if ui
            .selectable_label(self.show_fans, t!("top_bar.fans_toggle").to_string())
            .on_hover_text(t!("top_bar.fans_toggle_tooltip").to_string())
            .clicked()
        {
            self.show_fans = !self.show_fans;
        }
        if ui
            .selectable_label(self.show_temps, t!("top_bar.temps_toggle").to_string())
            .on_hover_text(t!("top_bar.temps_toggle_tooltip").to_string())
            .clicked()
        {
            self.show_temps = !self.show_temps;
        }
        if ui
            .selectable_label(
                self.settings.show_activity_deck,
                t!("top_bar.activity_toggle").to_string(),
            )
            .on_hover_text(t!("top_bar.activity_toggle_tooltip").to_string())
            .clicked()
        {
            self.settings.show_activity_deck = !self.settings.show_activity_deck;
            self.apply_activity_deck_gate();
            self.settings.save();
        }
        if ui
            .selectable_label(
                self.settings.show_cpu_panel,
                t!("top_bar.cpu_toggle").to_string(),
            )
            .on_hover_text(t!("top_bar.cpu_toggle_tooltip").to_string())
            .clicked()
        {
            self.settings.show_cpu_panel = !self.settings.show_cpu_panel;
            self.settings.save();
        }
        if ui
            .selectable_label(
                self.settings.show_gpu_panel,
                t!("top_bar.gpu_toggle").to_string(),
            )
            .on_hover_text(t!("top_bar.gpu_toggle_tooltip").to_string())
            .clicked()
        {
            self.settings.show_gpu_panel = !self.settings.show_gpu_panel;
            self.settings.save();
        }
        if ui
            .selectable_label(
                self.settings.show_graph_panel,
                t!("top_bar.sensors_toggle").to_string(),
            )
            .on_hover_text(t!("top_bar.sensors_toggle_tooltip").to_string())
            .clicked()
        {
            self.settings.show_graph_panel = !self.settings.show_graph_panel;
            self.settings.save();
        }
        // Prominent Curve control toggle (auto-apply to hardware)
        let curve_on = self.settings.auto_apply_curves;
        let rgb = egui::Color32::from_rgb;
        let (fill, text_color) = match (curve_on, ui.visuals().dark_mode) {
            (true, true) => (rgb(30, 90, 50), rgb(140, 255, 170)),
            (true, false) => (rgb(200, 235, 205), rgb(20, 100, 40)),
            (false, true) => (rgb(70, 40, 40), rgb(220, 160, 160)),
            (false, false) => (rgb(245, 215, 215), rgb(140, 40, 40)),
        };
        let label = if curve_on {
            t!("top_bar.curve_control_on").to_string()
        } else {
            t!("top_bar.curve_control_off").to_string()
        };
        let btn =
            egui::Button::new(egui::RichText::new(label).color(text_color).strong()).fill(fill);
        if ui
            .add(btn)
            .on_hover_text(t!("top_bar.curve_control_tooltip").to_string())
            .clicked()
        {
            self.settings.auto_apply_curves = !self.settings.auto_apply_curves;
            self.settings.save();
        }
        self.top_toggles_w = ui.min_rect().width() + ui.spacing().item_spacing.x;
    }
}
