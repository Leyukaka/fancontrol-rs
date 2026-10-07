//! Top visualization row: equal-height Sensors graph / GPU / CPU slots.

use super::*;

impl FanApp {
    /// Fixed-height slot shared by Sensors / GPU / CPU columns so bottoms align.
    pub(super) fn domain_column_slot(
        ui: &mut egui::Ui,
        row_h: f32,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        ui.allocate_ui(egui::vec2(ui.available_width(), row_h), |ui| {
            ui.set_min_height(row_h);
            ui.set_max_height(row_h);
            egui::ScrollArea::vertical()
                .id_salt(ui.id().with("domain_slot_scroll"))
                .max_height(row_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_height(row_h);
                    add_contents(ui);
                });
        });
    }

    /// Thermal / multi-metric graph (or shader style) for the top visualization row.
    /// `slot_h` is the **total** column height (equal to GPU/CPU slots); the plot uses
    /// remaining space after the legend so the three domain cards share one surface.
    pub(super) fn ui_thermal_graph_block(
        &mut self,
        ui: &mut egui::Ui,
        labels: &HashMap<&str, &str>,
        units: &HashMap<&str, Option<&str>>,
        kinds: &HashMap<&str, SensorKind>,
        slot_h: f32,
        power_axis_ceiling: Option<f32>,
    ) {
        let (win, samp) = (
            self.settings.graph_window_minutes,
            self.settings.graph_sample_secs,
        );
        for id in &self.settings.graph_sensor_ids {
            self.histories.entry(id.clone()).or_insert_with(|| {
                let mut h = TempHistory::default();
                h.configure(win, samp);
                h
            });
        }
        // Sensors graph is temperature-only (see spec goal 2/8): GPU/CPU power ids some
        // users still have saved in `graph_sensor_ids` from before the CPU/GPU panels
        // existed are filtered out here (by live `SensorKind`, not stripped from
        // settings) rather than stripped from settings, so nothing is lost if a future
        // graph adds other units back. An id currently absent from the live snapshot
        // (e.g. a power sensor with PawnIO not elevated) is excluded too - its kind is
        // unknown, and showing an empty, uncategorized ghost series helps no one.
        let series: Vec<GraphSeries> = self
            .settings
            .graph_sensor_ids
            .iter()
            .filter(|id| kinds.get(id.as_str()) == Some(&SensorKind::Temperature))
            // Index the drawn series, so the first visible line gets color 0 and the fill.
            .enumerate()
            .filter_map(|(i, id)| {
                self.histories.get(id).map(|h| GraphSeries {
                    label: labels.get(id.as_str()).copied().unwrap_or(id.as_str()),
                    palette_index: i,
                    history: h,
                    unit: units.get(id.as_str()).copied().flatten(),
                })
            })
            .collect();
        let style = self.settings.graph_style;
        let only_temps = series
            .iter()
            .all(|s| s.unit.is_none() || s.unit == Some("°C") || s.unit == Some("C"));

        // Match GPU/CPU domain_card outer size: fill the slot, plot uses rest of height.
        let fill = egui::vec2(ui.available_width(), slot_h.max(40.0));
        ui.allocate_ui(fill, |ui| {
            ui.set_min_height(slot_h);
            ui.set_max_height(slot_h);
            // Reserve plot height from remaining space after group header (~legend).
            // header_budget: multi-sensor legend can wrap; keep plot usable.
            let header_budget = if series.len() > 1 { 56.0 } else { 36.0 };
            let plot_h = clamp_ui_height(ui.available_height() - header_budget, 70.0, slot_h);

            if style == GraphStyle::Classic || !self.shader_backend_available || !only_temps {
                show_metric_graph(
                    ui,
                    &series,
                    self.settings.graph_window_minutes,
                    &mut self.graph_axis_max,
                    &mut self.graph_axis_max_secondary,
                    plot_h,
                    power_axis_ceiling,
                );
                if style.is_shader() && !only_temps {
                    ui.small(t!("graph.shader_temps_only_note").to_string());
                } else if style.is_shader() && !self.shader_backend_available {
                    ui.small(t!("graph.shader_fallback_note").to_string());
                }
            } else {
                let t = self.shader_clock.elapsed().as_secs_f32() * self.settings.shader_speed;
                let readings: Vec<(String, f32)> = series
                    .iter()
                    .filter_map(|s| s.history.last().map(|v| (s.label.to_string(), v)))
                    .collect();
                let signal = ThermalSignal::from_readings(readings);
                ui.allocate_ui(egui::vec2(ui.available_width(), plot_h), |ui| {
                    show_shader_panel(
                        ui,
                        style,
                        t,
                        signal,
                        self.settings.shader_color_a,
                        self.settings.shader_color_b,
                    );
                });
            }
            let leftover = ui.available_height();
            if leftover > 1.0 {
                ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), leftover),
                    egui::Sense::hover(),
                );
            }
        });
    }
}
