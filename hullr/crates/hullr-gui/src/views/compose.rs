use super::open;
use crate::backend::Backend;
use crate::state::*;
use crate::{theme, widgets};
use eframe::egui::{self, RichText, ScrollArea, Ui};
use hullr_core::{ComposeStatus, ContainerState};

pub fn running_summary(p: &ComposeStatus) -> String {
    let running = p
        .services
        .iter()
        .filter(|s| s.container.as_ref().is_some_and(|c| c.state == ContainerState::Running))
        .count();
    format!("{running} of {} services running", p.services.len())
}

pub fn show(ui: &mut Ui, st: &mut AppState, be: &Backend) {
    ui.horizontal(|ui| {
        ui.heading("Compose");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("Open compose file...").strong()).fill(theme::ACCENT.linear_multiply(0.6))).clicked() {
                st.compose_path = Some(String::new());
            }
        });
    });
    ui.separator();
    let projects = st.data.projects.clone();
    if projects.is_empty() {
        let (title, hint) = if let Some(e) = &st.data.error {
            ("Can't reach the engine".to_string(), e.clone())
        } else if !st.data.loaded[3] {
            ("Loading projects...".into(), String::new())
        } else {
            ("No compose projects running".into(), "Open a compose.yaml (or the folder that holds it) to bring a project up.".into())
        };
        if widgets::empty_state(ui, &title, &hint, if st.data.loaded[3] && st.data.error.is_none() { Some("Open compose file...") } else { None }) {
            st.compose_path = Some(String::new());
        }
        return;
    }
    let dark = ui.visuals().dark_mode;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for p in &projects {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let known = st.known.projects.get(&p.project).cloned();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&p.project).heading());
                    widgets::muted_label(ui, running_summary(p));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::danger_button(ui, "Down").on_hover_text("Stop and remove the project's containers").clicked() {
                            st.confirm = Some(Confirm::ComposeDown { project: p.project.clone() });
                        }
                        let opts = known.clone();
                        if ui
                            .add_enabled(opts.is_some(), egui::Button::new("Up"))
                            .on_hover_text(if opts.is_some() { "Bring the project up again" } else { "Open its compose file once so hullr remembers where it lives" })
                            .clicked()
                        {
                            if let Some(o) = opts {
                                let op = st.start_op(format!("Compose up: {}", p.project));
                                be.compose_up(op, o);
                            }
                        }
                        if ui
                            .add_enabled(known.is_some(), egui::Button::new("Open folder"))
                            .on_hover_text(if known.is_some() { "Show the project folder" } else { "Folder unknown until you open its compose file" })
                            .clicked()
                        {
                            if let Some(o) = &known {
                                open(st, &o.project_dir.display().to_string());
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                for s in &p.services {
                    ui.horizontal(|ui| match &s.container {
                        Some(c) => {
                            widgets::dot(ui, theme::state_color(c, dark));
                            ui.label(RichText::new(&s.service).strong());
                            widgets::muted_label(ui, c.status_text(hullr_core::now_unix()));
                            if ui.small_button("Show").clicked() {
                                st.page = Page::Containers;
                                st.selected = Some(c.id.clone());
                                st.detail_tab = DetailTab::Overview;
                            }
                        }
                        None => {
                            widgets::dot(ui, theme::muted(dark));
                            ui.label(RichText::new(&s.service).strong());
                            widgets::muted_label(ui, "not created");
                        }
                    });
                }
            });
            ui.add_space(6.0);
        }
    });
}
