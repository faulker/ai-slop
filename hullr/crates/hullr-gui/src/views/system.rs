use crate::backend::Backend;
use crate::state::*;
use crate::{theme, widgets};
use eframe::egui::{self, RichText, Ui};

pub fn show(ui: &mut Ui, st: &mut AppState, _be: &Backend) {
    ui.heading("System");
    ui.separator();
    let dark = ui.visuals().dark_mode;
    let Some(info) = st.data.info.clone() else {
        match &st.data.error {
            Some(e) => widgets::empty_state(ui, "Can't reach the engine", e, None),
            None => widgets::empty_state(ui, "Loading system info...", "", None),
        };
        return;
    };
    if !info.warnings.is_empty() {
        egui::Frame::new()
            .fill(theme::warning(dark).linear_multiply(0.18))
            .stroke(egui::Stroke::new(1.0_f32, theme::warning(dark)))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Heads up").strong().color(theme::warning(dark)));
                for w in &info.warnings {
                    ui.label(format!("- {w}"));
                }
            });
        ui.add_space(10.0);
    }
    egui::Grid::new("sys_grid").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        let row = |ui: &mut Ui, k: &str, v: String| {
            widgets::muted_label(ui, k);
            ui.label(v);
            ui.end_row();
        };
        row(ui, "Version", info.version.clone());
        row(ui, "Host", format!("{} ({})", info.os, info.host_platform));
        row(ui, "Mode", info.mode.clone());
        row(ui, "Rootless", if info.rootless { "yes" } else { "no" }.into());
        row(ui, "Data folder", info.data_root.display().to_string());
        row(
            ui,
            "Can emulate",
            if info.emulated_platforms.is_empty() { "nothing (native platform only)".into() } else { info.emulated_platforms.join(", ") },
        );
        row(ui, "Containers", format!("{} ({} running)", info.containers, info.running));
        row(ui, "Images", info.images.to_string());
    });
    ui.add_space(14.0);
    ui.label(RichText::new("Clean up").strong());
    ui.horizontal(|ui| {
        if ui.button("Prune unused").on_hover_text("Remove stopped containers and unused images. Volumes are kept.").clicked() {
            st.confirm = Some(Confirm::Prune { deep: false });
        }
        if widgets::danger_button(ui, "Deep prune...").on_hover_text("Also removes all unused images and unused volumes").clicked() {
            st.confirm = Some(Confirm::Prune { deep: true });
        }
    });
    ui.add_space(14.0);
    ui.label(RichText::new("Appearance").strong());
    ui.horizontal(|ui| {
        ui.selectable_value(&mut st.theme, ThemeChoice::System, "Follow system");
        ui.selectable_value(&mut st.theme, ThemeChoice::Light, "Light");
        ui.selectable_value(&mut st.theme, ThemeChoice::Dark, "Dark");
    });
}
