//! Floating UI: modal dialogs, the activity window and toasts.

use crate::backend::Backend;
use crate::state::*;
use crate::{theme, widgets};
use eframe::egui::{self, Align2, Context, RichText, Ui};
use hullr_core::container::human_bytes;

pub fn show_all(ctx: &Context, st: &mut AppState, be: &Backend) {
    run_dialog(ctx, st, be);
    pull_dialog(ctx, st, be);
    compose_dialog(ctx, st, be);
    confirm_dialog(ctx, st, be);
    activity(ctx, st);
    toasts(ctx, st);
}

fn platform_combo(ui: &mut Ui, id: &str, value: &mut PlatformChoice) {
    egui::ComboBox::from_id_salt(id).selected_text(value.label()).show_ui(ui, |ui| {
        for p in PlatformChoice::ALL {
            ui.selectable_value(value, p, p.label());
        }
    });
}

fn run_dialog(ctx: &Context, st: &mut AppState, be: &Backend) {
    let Some(mut d) = st.run.take() else { return };
    let mut submit = false;
    let mut cancel = false;
    let resp = egui::Modal::new(egui::Id::new("run_dialog")).show(ctx, |ui| {
        ui.set_width(560.0);
        ui.heading("Run a container");
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() * 0.7).show(ui, |ui| {
            egui::Grid::new("run_basic").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label("Image");
                ui.add(egui::TextEdit::singleline(&mut d.image).hint_text("nginx:latest").desired_width(f32::INFINITY));
                ui.end_row();
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("optional").desired_width(f32::INFINITY));
                ui.end_row();
                ui.label("Command");
                ui.add(egui::TextEdit::singleline(&mut d.command).hint_text("optional, replaces the image default").desired_width(f32::INFINITY));
                ui.end_row();
                ui.label("Platform");
                platform_combo(ui, "run_platform", &mut d.platform);
                ui.end_row();
                ui.label("Restart");
                egui::ComboBox::from_id_salt("run_restart").selected_text(d.restart.label()).show_ui(ui, |ui| {
                    for r in RestartChoice::ALL {
                        ui.selectable_value(&mut d.restart, r, r.label());
                    }
                });
                ui.end_row();
            });
            ui.add_space(8.0);

            ui.label(RichText::new("Ports").strong());
            let mut remove = None;
            for (i, r) in d.ports.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut r.host).hint_text("host").desired_width(80.0));
                    ui.label("->");
                    ui.add(egui::TextEdit::singleline(&mut r.container).hint_text("container").desired_width(80.0));
                    ui.checkbox(&mut r.udp, "UDP");
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                d.ports.remove(i);
            }
            if ui.small_button("+ Add port").clicked() {
                d.ports.push(PortRow::default());
            }
            ui.add_space(8.0);

            ui.label(RichText::new("Environment").strong());
            let mut remove = None;
            for (i, r) in d.env.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut r.key).hint_text("NAME").desired_width(150.0));
                    ui.label("=");
                    ui.add(egui::TextEdit::singleline(&mut r.value).hint_text("value").desired_width(250.0));
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                d.env.remove(i);
            }
            if ui.small_button("+ Add variable").clicked() {
                d.env.push(EnvRow::default());
            }
            ui.add_space(8.0);

            ui.label(RichText::new("Volumes").strong());
            let mut remove = None;
            for (i, r) in d.volumes.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut r.source).hint_text("folder or volume name").desired_width(190.0));
                    ui.label(":");
                    ui.add(egui::TextEdit::singleline(&mut r.target).hint_text("/path/in/container").desired_width(170.0));
                    ui.checkbox(&mut r.read_only, "Read-only");
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                d.volumes.remove(i);
            }
            if ui.small_button("+ Add volume").clicked() {
                d.volumes.push(VolumeRow::default());
            }
            ui.add_space(8.0);
            ui.checkbox(&mut d.detach, "Run in the background (detach)");
            if !d.detach {
                widgets::muted_label(ui, "The container's logs open once it starts.");
            }
            ui.checkbox(&mut d.auto_remove, "Delete the container when it exits");
        });
        if let Some(e) = &d.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(theme::danger(ui.visuals().dark_mode)));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Run").strong()).fill(theme::ACCENT.linear_multiply(0.7))).clicked() {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if resp.should_close() {
        cancel = true;
    }
    if cancel {
        return;
    }
    if submit {
        match d.to_spec(&st.cwd) {
            Ok(spec) => {
                let show = !d.detach;
                let op = st.start_op(format!("Starting {}", spec.name.clone().unwrap_or_else(|| spec.image.clone())));
                be.run_container(op, spec, show);
                return;
            }
            Err(e) => d.error = Some(e),
        }
    }
    st.run = Some(d);
}

fn pull_dialog(ctx: &Context, st: &mut AppState, be: &Backend) {
    let Some(mut d) = st.pull.take() else { return };
    let (mut submit, mut cancel) = (false, false);
    let resp = egui::Modal::new(egui::Id::new("pull_dialog")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading("Pull an image");
        ui.add_space(6.0);
        let r = ui.add(egui::TextEdit::singleline(&mut d.reference).hint_text("nginx:latest or ghcr.io/org/app:1.2").desired_width(f32::INFINITY));
        r.request_focus();
        if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        ui.horizontal(|ui| {
            ui.label("Platform");
            platform_combo(ui, "pull_platform", &mut d.platform);
        });
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).color(theme::danger(ui.visuals().dark_mode)));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Pull").strong()).fill(theme::ACCENT.linear_multiply(0.7))).clicked() {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if cancel || resp.should_close() {
        return;
    }
    if submit {
        let r = d.reference.trim().to_string();
        if r.is_empty() {
            d.error = Some("Enter an image name, for example nginx:latest".into());
        } else {
            let op = st.start_op(format!("Pulling {r}"));
            be.pull(op, r, d.platform.platform());
            return;
        }
    }
    st.pull = Some(d);
}

fn compose_dialog(ctx: &Context, st: &mut AppState, be: &Backend) {
    let Some(mut path) = st.compose_path.take() else { return };
    let (mut submit, mut cancel) = (false, false);
    let mut error = None;
    let resp = egui::Modal::new(egui::Id::new("compose_dialog")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.heading("Open compose file");
        ui.add_space(4.0);
        widgets::muted_label(ui, "Type the path of a compose.yaml, or of the folder that contains one.");
        let r = ui.add(egui::TextEdit::singleline(&mut path).hint_text("~/projects/shop/compose.yaml").desired_width(f32::INFINITY));
        r.request_focus();
        if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Bring up").strong()).fill(theme::ACCENT.linear_multiply(0.7))).clicked() {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if cancel || resp.should_close() {
        return;
    }
    if submit {
        match compose_opts_from_path(&path) {
            Ok(opts) => {
                let op = st.start_op(format!("Compose up: {}", opts.project_dir.display()));
                be.compose_up(op, opts);
                return;
            }
            Err(e) => error = Some(e),
        }
    }
    if let Some(e) = error {
        st.toasts.push(Level::Error, e, st.now);
    }
    st.compose_path = Some(path);
}

fn confirm_dialog(ctx: &Context, st: &mut AppState, be: &Backend) {
    let Some(c) = st.confirm.clone() else { return };
    let (mut yes, mut no) = (false, false);
    let resp = egui::Modal::new(egui::Id::new("confirm_dialog")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading("Are you sure?");
        ui.add_space(6.0);
        ui.label(c.message());
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::danger_button(ui, c.button()).clicked() {
                yes = true;
            }
            if ui.button("Cancel").clicked() {
                no = true;
            }
        });
    });
    if no || resp.should_close() {
        st.confirm = None;
    }
    if yes {
        st.confirm = None;
        match c {
            Confirm::RemoveContainer { id, name, running } => be.remove_container(id, name, running),
            Confirm::RemoveImage { reference, label } => be.remove_image(reference, label),
            Confirm::RemoveVolume { name } => be.remove_volume(name),
            Confirm::Prune { deep } => be.prune(deep),
            Confirm::ComposeDown { project } => {
                let opts = st.known.projects.get(&project).cloned().unwrap_or_else(|| hullr_core::ComposeOptions {
                    project_dir: st.cwd.clone(),
                    ..Default::default()
                });
                let op = st.start_op(format!("Compose down: {project}"));
                be.compose_down(op, project, opts);
            }
        }
    }
}

/// Floating progress for pulls, runs and compose operations.
fn activity(ctx: &Context, st: &mut AppState) {
    if st.ops.is_empty() {
        return;
    }
    let dark = ctx.style().visuals.dark_mode;
    let mut dismiss = None;
    egui::Window::new("Activity")
        .anchor(Align2::LEFT_BOTTOM, [236.0, -12.0])
        .collapsible(true)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            for op in &st.ops {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&op.title).strong());
                    if !matches!(op.status, OpStatus::Running) && ui.small_button("Dismiss").clicked() {
                        dismiss = Some(op.id);
                    }
                });
                match &op.status {
                    OpStatus::Running => {
                        match (op.progress.fraction(), op.progress.bytes()) {
                            (Some(f), Some((c, t))) => {
                                ui.add(egui::ProgressBar::new(f).desired_width(400.0).text(format!("{} / {}", human_bytes(c), human_bytes(t))));
                            }
                            _ => {
                                ui.add(egui::ProgressBar::new(0.0).desired_width(400.0).animate(true).text("Working..."));
                            }
                        }
                        if let Some(l) = op.progress.lines.back() {
                            widgets::muted_label(ui, crate::state_text::clip(l, 70));
                        }
                        if !op.progress.layers.is_empty() {
                            egui::CollapsingHeader::new(format!("Layers ({} of {} done)", op.progress.layers_done(), op.progress.layers.len()))
                                .id_salt(("layers", op.id))
                                .show(ui, |ui| {
                                    for l in &op.progress.layers {
                                        let id: String = l.id.trim_start_matches("sha256:").chars().take(12).collect();
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(id).monospace());
                                            let text = if l.total > 0 { human_bytes(l.total) } else { "?".into() };
                                            ui.add(egui::ProgressBar::new(l.fraction()).desired_width(230.0).text(text));
                                        });
                                    }
                                });
                        }
                    }
                    OpStatus::Done(m) => {
                        ui.label(RichText::new(m).color(theme::success(dark)));
                    }
                    OpStatus::Failed(e) => {
                        ui.label(RichText::new(e).color(theme::danger(dark)));
                    }
                }
                ui.separator();
            }
        });
    if let Some(id) = dismiss {
        st.dismiss_op(id);
    }
    // Finished successes tidy themselves up after a while via toasts; keep
    // the rows until dismissed so errors stay readable.
}

fn toasts(ctx: &Context, st: &mut AppState) {
    st.toasts.expire(st.now);
    if st.toasts.items.is_empty() {
        return;
    }
    let dark = ctx.style().visuals.dark_mode;
    let mut dismiss = None;
    egui::Area::new(egui::Id::new("toasts")).anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.set_max_width(380.0);
        for t in &st.toasts.items {
            let color = match t.level {
                Level::Info => theme::ACCENT,
                Level::Success => theme::success(dark),
                Level::Error => theme::danger(dark),
            };
            egui::Frame::popup(ui.style()).stroke(egui::Stroke::new(1.5_f32, color)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(&t.text).wrap());
                    if ui.small_button("x").clicked() {
                        dismiss = Some(t.id);
                    }
                });
            });
            ui.add_space(4.0);
        }
    });
    if let Some(id) = dismiss {
        st.toasts.dismiss(id);
    }
}
