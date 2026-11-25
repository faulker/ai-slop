use crate::backend::Backend;
use crate::state::*;
use crate::{widgets};
use eframe::egui::{self, RichText, ScrollArea, Ui};
use hullr_core::container::{human_bytes, human_duration};

pub fn show(ui: &mut Ui, st: &mut AppState, _be: &Backend) {
    ui.horizontal(|ui| {
        ui.heading("Images");
        ui.add_space(12.0);
        ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("Filter images").desired_width(240.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("Pull image...").strong()).fill(crate::theme::ACCENT.linear_multiply(0.6))).clicked() {
                st.pull = Some(PullDialog::default());
            }
        });
    });
    ui.separator();
    let rows = st.image_rows();
    if rows.is_empty() {
        let (title, hint, button) = if let Some(e) = &st.data.error {
            ("Can't reach the engine".to_string(), e.clone(), None)
        } else if !st.data.loaded[1] {
            ("Loading images...".into(), String::new(), None)
        } else if !st.filter.is_empty() {
            (format!("No images match '{}'", st.filter), "Clear the filter to see everything.".into(), None)
        } else {
            ("No images yet".into(), "Pull one, for example nginx:latest or alpine.".into(), Some("Pull an image"))
        };
        if widgets::empty_state(ui, &title, &hint, button) {
            st.pull = Some(PullDialog::default());
        }
        return;
    }
    let now = hullr_core::now_unix();
    ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("images_grid").striped(true).num_columns(6).spacing([18.0, 8.0]).show(ui, |ui| {
            for h in ["Image", "Size", "Created", "Platform", "ID", "Actions"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for r in &rows {
                ui.label(RichText::new(&r.label).strong());
                ui.label(human_bytes(r.info.size));
                ui.label(format!("{} ago", human_duration(now - r.info.created)));
                ui.label(r.info.platform.to_string());
                widgets::muted_label(ui, r.info.short_id());
                ui.horizontal(|ui| {
                    if ui.button("Run").clicked() {
                        st.run = Some(RunDialog::for_image(&r.label));
                    }
                    if widgets::danger_button(ui, "Delete").clicked() {
                        st.confirm = Some(Confirm::RemoveImage { reference: r.reference.clone(), label: r.label.clone() });
                    }
                });
                ui.end_row();
            }
        });
    });
}
