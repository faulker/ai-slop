use super::open;
use crate::backend::Backend;
use crate::state::*;
use crate::{sys, theme, widgets};
use eframe::egui::{self, RichText, ScrollArea, Ui};
use hullr_core::container::human_bytes;
use hullr_core::{ContainerInfo, ContainerState};

pub fn show(ui: &mut Ui, st: &mut AppState, be: &Backend) {
    ui.horizontal(|ui| {
        ui.heading("Containers");
        ui.add_space(12.0);
        ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("Filter by name, image or id").desired_width(240.0));
        if ui.checkbox(&mut st.show_all, "Show stopped").changed() {
            st.refresh_now = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("Run container...").strong()).fill(theme::ACCENT.linear_multiply(0.6))).clicked() {
                st.run = Some(RunDialog::default());
            }
        });
    });
    ui.separator();

    if st.selected_container().is_some() {
        egui::TopBottomPanel::bottom("container_detail")
            .resizable(true)
            .default_height(ui.available_height() * 0.5)
            .min_height(160.0)
            .show_inside(ui, |ui| detail(ui, st, be));
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| table(ui, st, be));
}

fn table(ui: &mut Ui, st: &mut AppState, be: &Backend) {
    let list: Vec<ContainerInfo> = st.visible_containers().into_iter().cloned().collect();
    if list.is_empty() {
        let (title, hint, button) = if let Some(e) = &st.data.error {
            ("Can't reach the engine".to_string(), e.clone(), None)
        } else if !st.data.loaded[0] {
            ("Loading containers...".into(), String::new(), None)
        } else if !st.filter.is_empty() {
            (format!("No containers match '{}'", st.filter), "Clear the filter to see everything.".into(), None)
        } else if !st.show_all && !st.data.containers.is_empty() {
            ("No running containers".into(), "Tick 'Show stopped' to see the others, or run a new one.".into(), Some("Run container..."))
        } else {
            ("No containers yet".into(), "Run one from an image. Press the button to get started.".into(), Some("Run your first container"))
        };
        if widgets::empty_state(ui, &title, &hint, button) {
            st.run = Some(RunDialog::default());
        }
        return;
    }
    let dark = ui.visuals().dark_mode;
    ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("containers_grid").striped(true).num_columns(6).spacing([18.0, 8.0]).show(ui, |ui| {
            for h in ["", "Name", "Image", "Status", "Ports", "Actions"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for c in &list {
                widgets::dot(ui, theme::state_color(c, dark));
                let selected = st.selected.as_deref() == Some(c.id.as_str());
                if ui.selectable_label(selected, RichText::new(&c.name).strong()).clicked() {
                    st.selected = if selected { None } else { Some(c.id.clone()) };
                }
                ui.label(short_ref(&c.image));
                ui.label(c.status_text(hullr_core::now_unix()));
                port_links_row(ui, st, c);
                actions(ui, st, be, c);
                ui.end_row();
            }
        });
    });
}

fn port_links_row(ui: &mut Ui, st: &mut AppState, c: &ContainerInfo) {
    ui.horizontal(|ui| {
        let links = port_links(&c.spec.ports);
        if links.is_empty() && c.spec.ports.is_empty() {
            widgets::muted_label(ui, "none");
        }
        for (port, url) in links {
            if ui.link(format!(":{port}")).on_hover_text(format!("Open {url}")).clicked() {
                open(st, &url);
            }
        }
        for p in c.spec.ports.iter().filter(|p| p.protocol == hullr_core::Protocol::Udp) {
            widgets::muted_label(ui, format!(":{}/udp", p.host_port));
        }
    });
}

fn actions(ui: &mut Ui, st: &mut AppState, be: &Backend, c: &ContainerInfo) {
    ui.horizontal(|ui| {
        let running = c.state == ContainerState::Running;
        if running {
            if ui.button("Stop").clicked() {
                be.stop(c.id.clone(), c.name.clone());
            }
            if ui.button("Restart").clicked() {
                be.restart(c.id.clone(), c.name.clone());
            }
        } else if ui.button("Start").clicked() {
            be.start(c.id.clone(), c.name.clone());
        }
        if widgets::danger_button(ui, "Delete").clicked() {
            st.confirm = Some(Confirm::RemoveContainer { id: c.id.clone(), name: c.name.clone(), running: running || c.state == ContainerState::Paused });
        }
    });
}

fn detail(ui: &mut Ui, st: &mut AppState, be: &Backend) {
    let Some(c) = st.selected_container().cloned() else { return };
    let dark = ui.visuals().dark_mode;
    ui.horizontal(|ui| {
        widgets::dot(ui, theme::state_color(&c, dark));
        ui.label(RichText::new(&c.name).heading());
        widgets::muted_label(ui, c.status_text(hullr_core::now_unix()));
        ui.add_space(12.0);
        for (t, label) in [(DetailTab::Overview, "Overview"), (DetailTab::Logs, "Logs"), (DetailTab::Stats, "Stats")] {
            ui.selectable_value(&mut st.detail_tab, t, label);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                st.selected = None;
            }
            let running = c.state == ContainerState::Running;
            if ui
                .add_enabled(running, egui::Button::new("Open shell in terminal"))
                .on_hover_text(if running { "Opens your terminal app running: hullr exec -it <id> sh" } else { "Start the container first" })
                .clicked()
            {
                match sys::open_shell_in_terminal(&c.id) {
                    Ok(_) => {}
                    Err(e) => st.toasts.push(Level::Error, e, st.now),
                }
            }
        });
    });
    ui.separator();
    match st.detail_tab {
        DetailTab::Overview => overview(ui, st, &c),
        DetailTab::Logs => logs(ui, st),
        DetailTab::Stats => stats(ui, st, &c),
    }
    let _ = be;
}

fn overview(ui: &mut Ui, st: &mut AppState, c: &ContainerInfo) {
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let now = hullr_core::now_unix();
        egui::Grid::new("overview_grid").num_columns(2).spacing([20.0, 6.0]).show(ui, |ui| {
            let row = |ui: &mut Ui, k: &str, v: String| {
                widgets::muted_label(ui, k);
                ui.label(v);
                ui.end_row();
            };
            row(ui, "ID", c.id.clone());
            row(ui, "Image", c.image.clone());
            row(ui, "Command", c.command.join(" "));
            row(ui, "Created", format!("{} ({} ago)", fmt_datetime(c.created), hullr_core::container::human_duration(now - c.created)));
            row(ui, "State", c.status_text(now));
            if let Some(pid) = c.pid {
                row(ui, "PID", pid.to_string());
            }
            let native = if c.platform.is_native() { "native" } else { "emulated" };
            row(ui, "Platform", format!("{} ({native})", c.platform));
            row(ui, "Restart", format!("{:?}", c.spec.restart));
            row(ui, "Network", format!("{:?}", c.spec.network));
            if let Some(w) = &c.spec.working_dir {
                row(ui, "Workdir", w.clone());
            }
            if let Some(u) = &c.spec.user {
                row(ui, "User", u.clone());
            }
            widgets::muted_label(ui, "Ports");
            ui.horizontal_wrapped(|ui| {
                if c.spec.ports.is_empty() {
                    widgets::muted_label(ui, "none published");
                }
                for p in &c.spec.ports {
                    let label = format!("{}:{} -> {}/{}", p.host_ip.as_deref().unwrap_or("0.0.0.0"), p.host_port, p.container_port, if p.protocol == hullr_core::Protocol::Udp { "udp" } else { "tcp" });
                    if p.protocol == hullr_core::Protocol::Tcp {
                        if ui.link(label).on_hover_text("Open in browser").clicked() {
                            open(st, &format!("http://localhost:{}", p.host_port));
                        }
                    } else {
                        ui.label(label);
                    }
                }
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        egui::CollapsingHeader::new(format!("Environment ({})", c.spec.env.len())).show(ui, |ui| {
            if c.spec.env.is_empty() {
                widgets::muted_label(ui, "No variables set on the container (the image's own are not listed).");
            }
            for e in &c.spec.env {
                ui.label(RichText::new(e).monospace());
            }
        });
        egui::CollapsingHeader::new(format!("Mounts ({})", c.spec.mounts.len())).default_open(!c.spec.mounts.is_empty()).show(ui, |ui| {
            if c.spec.mounts.is_empty() {
                widgets::muted_label(ui, "No mounts.");
            }
            for m in &c.spec.mounts {
                let src = if m.source.is_empty() { "(anonymous)" } else { m.source.as_str() };
                ui.label(RichText::new(format!("{:?}: {} -> {}{}", m.kind, src, m.target, if m.read_only { " (read-only)" } else { "" })).monospace());
            }
        });
        if !c.spec.labels.is_empty() {
            egui::CollapsingHeader::new(format!("Labels ({})", c.spec.labels.len())).show(ui, |ui| {
                for (k, v) in &c.spec.labels {
                    ui.label(RichText::new(format!("{k}={v}")).monospace());
                }
            });
        }
    });
}

fn logs(ui: &mut Ui, st: &mut AppState) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.logs.search).hint_text("Search logs").desired_width(260.0));
        ui.checkbox(&mut st.logs.autoscroll, "Autoscroll");
        if ui.button("Clear").clicked() {
            st.logs.lines.clear();
        }
        let shown = filter_logs(&st.logs.lines, &st.logs.search).len();
        widgets::muted_label(ui, format!("{shown} of {} lines", st.logs.lines.len()));
    });
    let dark = ui.visuals().dark_mode;
    let lines = filter_logs(&st.logs.lines, &st.logs.search);
    if lines.is_empty() {
        let msg = if st.logs.lines.is_empty() { "No output yet. New lines appear here as they are written." } else { "No lines match your search." };
        widgets::muted_label(ui, msg);
        return;
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(st.logs.autoscroll).show_rows(ui, row_h, lines.len(), |ui, range| {
        for e in &lines[range] {
            let mut t = RichText::new(&e.text).monospace();
            if e.stderr {
                t = t.color(theme::danger(dark));
            }
            ui.add(egui::Label::new(t).wrap_mode(egui::TextWrapMode::Extend));
        }
    });
}

fn stats(ui: &mut Ui, st: &mut AppState, c: &ContainerInfo) {
    if c.state != ContainerState::Running {
        widgets::muted_label(ui, "Stats are shown while the container is running.");
        return;
    }
    let Some(last) = st.stats.last.clone() else {
        widgets::muted_label(ui, "Waiting for the first sample...");
        return;
    };
    let dark = ui.visuals().dark_mode;
    let width = ((ui.available_width() - 16.0) / 2.0).max(200.0);
    let cpu_max = widgets::nice_max(st.stats.cpu.iter().copied().fold(0.0, f32::max), 100.0);
    let mem_max = match last.memory_limit {
        Some(l) if l > 0 => l as f32,
        _ => widgets::nice_max(st.stats.mem.iter().copied().fold(0.0, f32::max) * 1.2, 1.0e6),
    };
    ui.horizontal(|ui| {
        widgets::line_plot(ui, &st.stats.cpu, egui::vec2(width, 130.0), theme::success(dark), cpu_max, "CPU %", &format!("{:.1}%", last.cpu_percent));
        let mem_now = match last.memory_limit {
            Some(l) if l > 0 => format!("{} / {}", human_bytes(last.memory_bytes), human_bytes(l)),
            _ => human_bytes(last.memory_bytes),
        };
        widgets::line_plot(ui, &st.stats.mem, egui::vec2(width, 130.0), theme::ACCENT, mem_max, "Memory", &mem_now);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(format!("Processes: {}", last.pids));
        ui.separator();
        ui.label(format!("Network in: {}", human_bytes(last.net_rx_bytes)));
        ui.label(format!("out: {}", human_bytes(last.net_tx_bytes)));
    });
}
