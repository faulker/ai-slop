use crate::backend::Backend;
use crate::state::*;
use crate::widgets;
use eframe::egui::{self, RichText, ScrollArea, Ui};
use hullr_core::container::human_duration;

pub fn show(ui: &mut Ui, st: &mut AppState, be: &Backend) {
    ui.horizontal(|ui| {
        ui.heading("Volumes");
        ui.add_space(12.0);
        let name = st.new_volume.get_or_insert_with(String::new);
        let resp = ui.add(egui::TextEdit::singleline(name).hint_text("New volume name").desired_width(220.0));
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ok = !name.trim().is_empty();
        if (ui.add_enabled(ok, egui::Button::new(RichText::new("Create").strong())).clicked() || (enter && ok)) && ok {
            let n = name.trim().to_string();
            name.clear();
            be.create_volume(n);
        }
    });
    ui.separator();
    if st.data.volumes.is_empty() {
        let (title, hint) = if let Some(e) = &st.data.error {
            ("Can't reach the engine".to_string(), e.clone())
        } else if !st.data.loaded[2] {
            ("Loading volumes...".into(), String::new())
        } else {
            ("No volumes yet".into(), "Type a name above and press Create. Volumes keep data after a container is deleted.".into())
        };
        widgets::empty_state(ui, &title, &hint, None);
        return;
    }
    let now = hullr_core::now_unix();
    let vols = st.data.volumes.clone();
    ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("volumes_grid").striped(true).num_columns(4).spacing([18.0, 8.0]).show(ui, |ui| {
            for h in ["Name", "Location", "Created", "Actions"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for v in &vols {
                ui.label(RichText::new(&v.name).strong());
                ui.label(RichText::new(v.path.display().to_string()).monospace());
                ui.label(format!("{} ago", human_duration(now - v.created)));
                if widgets::danger_button(ui, "Delete").clicked() {
                    st.confirm = Some(Confirm::RemoveVolume { name: v.name.clone() });
                }
                ui.end_row();
            }
        });
    });
}
