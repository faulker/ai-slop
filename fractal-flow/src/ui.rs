//! The settings panel (egui overlay). Edits `Settings` in place and reports
//! one-off requests (next scene, start casting, ...) back to the app as `UiActions`.

use egui::{Color32, ComboBox, RichText, Slider};

use crate::cast::{CastDeviceInfo, CastStatus};
use crate::perf::{ProcessSample, MAX_LEVEL};
use crate::quality::QualityPreset;
use crate::scene::SceneKind;
use crate::settings::{CastResolution, Settings, VideoModeSpec};

/// Window size presets offered in the Display section.
pub const WINDOW_PRESETS: [(&str, u32, u32); 4] = [("720p", 1280, 720), ("1080p", 1920, 1080), ("1440p", 2560, 1440), ("4K", 3840, 2160)];

/// Every keyboard shortcut as (key, action), shown by the H overlay.
pub const SHORTCUTS: [(&str, &str); 10] = [
    ("H", "Show/hide this shortcut list"),
    ("D", "Show/hide the debug overlay"),
    ("Tab or S", "Open/close the settings panel"),
    ("Space", "Next scene now"),
    ("1 / 2 / 3", "Jump to deep zoom / Julia / 3D flythrough"),
    ("P", "New palette"),
    ("+ / -", "Faster / slower"),
    ("[ / ]", "Lower / raise quality preset"),
    ("F", "Toggle fullscreen"),
    ("Esc", "Close help or panel, or quit"),
];

/// Panel state that is not part of the saved settings.
#[derive(Debug, Default)]
pub struct UiState {
    pub open: bool,
    /// Keyboard shortcut overlay is showing.
    pub help: bool,
    /// Index into the cast device list the user selected.
    pub selected_device: Option<usize>,
}

/// Live information shown in the panel.
pub struct UiInfo<'a> {
    pub fps: f32,
    pub frame_ms: f32,
    pub scene: Option<(SceneKind, u64)>,
    pub zoom_decades: Option<f64>,
    pub preparing: bool,
    /// CPU and memory of this process (only refreshed while the debug overlay shows).
    pub process: ProcessSample,
    /// Current auto quality level (0 = full quality).
    pub auto_level: u8,
    /// Render scale actually in use after auto quality.
    pub effective_render_scale: f32,
    pub monitors: &'a [String],
    pub video_modes: &'a [VideoModeSpec],
    pub cast_devices: &'a [CastDeviceInfo],
    pub cast_status: CastStatus,
    pub ffmpeg: &'a Result<(), String>,
    pub warning: Option<&'a str>,
}

/// Requests from the panel for the app to act on this frame.
#[derive(Debug, Default, PartialEq)]
pub struct UiActions {
    /// Fullscreen, monitor, window size or vsync changed.
    pub display_changed: bool,
    pub next_scene: bool,
    pub force_kind: Option<SceneKind>,
    pub new_palette: bool,
    pub reset_defaults: bool,
    pub cast_start: Option<CastDeviceInfo>,
    pub cast_stop: bool,
    pub refresh_devices: bool,
    /// Panel was closed (the app saves settings).
    pub closed: bool,
}

/// Format a zoom depth like "1e-23.4".
pub fn format_depth(decades: f64) -> String {
    format!("1e-{decades:.1}")
}

/// Label for a video mode.
pub fn format_mode(m: &VideoModeSpec) -> String {
    format!("{}×{} @ {:.0} Hz", m.width, m.height, m.refresh_millihertz as f64 / 1000.0)
}

/// Draw the panel if open. Returns what the app should do.
pub fn draw(ctx: &egui::Context, state: &mut UiState, settings: &mut Settings, info: &UiInfo) -> UiActions {
    let mut actions = UiActions::default();
    if state.help {
        help_overlay(ctx);
    }
    if settings.display.debug_overlay {
        debug_overlay(ctx, settings, info);
    }
    if !state.open {
        return actions;
    }
    let mut open = true;
    egui::Window::new("fractal-flow settings")
        .open(&mut open)
        .default_width(380.0)
        .default_pos([24.0, 24.0])
        .resizable(true)
        .show(ctx, |ui| {
            if let Some(w) = info.warning {
                ui.label(RichText::new(w).color(Color32::from_rgb(255, 180, 80)));
            }
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 120.0).show(ui, |ui| {
                display_section(ui, settings, info, &mut actions);
                quality_section(ui, settings, info);
                scenes_section(ui, settings, &mut actions);
                motion_section(ui, settings);
                color_section(ui, settings, &mut actions);
                cast_section(ui, state, settings, info, &mut actions);
                info_section(ui, info);
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reset to defaults").clicked() {
                    actions.reset_defaults = true;
                }
                if ui.button("Close (saves)").clicked() {
                    state.open = false;
                    actions.closed = true;
                }
            });
        });
    if !open {
        state.open = false;
        actions.closed = true;
    }
    actions
}

/// Centered list of keyboard shortcuts.
fn help_overlay(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("shortcut_help")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).interactable(false).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.heading("Keyboard shortcuts");
            ui.separator();
            egui::Grid::new("shortcut_grid").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                for (key, action) in SHORTCUTS {
                    ui.label(RichText::new(key).strong().monospace());
                    ui.label(action);
                    ui.end_row();
                }
            });
        });
    });
}

/// Top-right readout of frame rate, CPU, memory and auto quality.
fn debug_overlay(ctx: &egui::Context, settings: &Settings, info: &UiInfo) {
    egui::Area::new(egui::Id::new("debug_overlay")).anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0]).interactable(false).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let line = |ui: &mut egui::Ui, text: String| ui.label(RichText::new(text).monospace());
            line(ui, format!("{:>5.1} fps  {:>5.1} ms", info.fps, info.frame_ms));
            line(ui, format!("CPU {:.0}% ({} cores)", info.process.cpu_percent, info.process.cores));
            line(ui, format!("Mem {}", format_bytes(info.process.memory_bytes)));
            line(ui, format!("Quality {}", settings.quality.preset.label()));
            if settings.quality.auto {
                line(ui, format!("Auto level {}/{MAX_LEVEL}, target {:.0} fps", info.auto_level, crate::perf::target_fps(settings)));
            }
            line(ui, format!("Render scale {:.2}×", info.effective_render_scale));
        });
    });
}

/// Window mode, monitor, resolution, render scale, vsync, FPS cap.
fn display_section(ui: &mut egui::Ui, settings: &mut Settings, info: &UiInfo, actions: &mut UiActions) {
    egui::CollapsingHeader::new("Display").default_open(true).show(ui, |ui| {
        let d = &mut settings.display;
        actions.display_changed |= ui.checkbox(&mut d.fullscreen, "Fullscreen (F)").changed();

        let current = d.monitor.clone().unwrap_or_else(|| "Current monitor".into());
        ComboBox::from_label("Monitor").selected_text(current).show_ui(ui, |ui| {
            actions.display_changed |= ui.selectable_value(&mut d.monitor, None, "Current monitor").changed();
            for name in info.monitors {
                actions.display_changed |= ui.selectable_value(&mut d.monitor, Some(name.clone()), name).changed();
            }
        });

        let mode_text = d.video_mode.as_ref().map(format_mode).unwrap_or_else(|| "Borderless (native)".into());
        ComboBox::from_label("Fullscreen mode").selected_text(mode_text).show_ui(ui, |ui| {
            actions.display_changed |= ui.selectable_value(&mut d.video_mode, None, "Borderless (native)").changed();
            for m in info.video_modes {
                actions.display_changed |= ui.selectable_value(&mut d.video_mode, Some(*m), format_mode(m)).changed();
            }
        });

        ui.horizontal(|ui| {
            ui.label("Window size");
            let preset = WINDOW_PRESETS.iter().find(|p| p.1 == d.window_width && p.2 == d.window_height).map(|p| p.0).unwrap_or("Custom");
            ComboBox::from_id_salt("window_preset").selected_text(preset).show_ui(ui, |ui| {
                for (name, w, h) in WINDOW_PRESETS {
                    if ui.selectable_label(preset == name, name).clicked() {
                        d.window_width = w;
                        d.window_height = h;
                        actions.display_changed = true;
                    }
                }
            });
            actions.display_changed |= ui.add(egui::DragValue::new(&mut d.window_width).range(320..=7680).suffix(" w")).changed();
            actions.display_changed |= ui.add(egui::DragValue::new(&mut d.window_height).range(240..=4320).suffix(" h")).changed();
        });

        ui.add(Slider::new(&mut d.render_scale, 0.25..=2.0).text("Render scale")).on_hover_text("Scene resolution relative to the output. Above 1.0 supersamples (sharper, slower).");
        actions.display_changed |= ui.checkbox(&mut d.vsync, "VSync").changed();
        ui.add(Slider::new(&mut d.fps_cap, 0..=240).text("FPS cap (0 = off)"));
        ui.checkbox(&mut d.debug_overlay, "Debug overlay (D)").on_hover_text("FPS, CPU and memory in the top-right corner.");
    });
}

/// Preset plus every individual cost knob.
fn quality_section(ui: &mut egui::Ui, settings: &mut Settings, info: &UiInfo) {
    egui::CollapsingHeader::new("Quality").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut settings.quality.auto, "Auto quality").on_hover_text("Lowers samples, iterations and render scale while FPS is below the target, and raises them again when it recovers. The settings below stay the ceiling.");
            if settings.quality.auto {
                ui.label(format!("level {}/{MAX_LEVEL}", info.auto_level));
            }
        });
        ui.add_enabled(settings.quality.auto, Slider::new(&mut settings.quality.target_fps, 15..=240).suffix(" fps").text("Target FPS"));
        let mut preset = settings.quality.preset;
        ComboBox::from_label("Preset ([ / ])").selected_text(preset.label()).show_ui(ui, |ui| {
            for p in [QualityPreset::Low, QualityPreset::Medium, QualityPreset::High, QualityPreset::Ultra] {
                ui.selectable_value(&mut preset, p, p.label());
            }
        });
        if preset != settings.quality.preset {
            settings.set_quality_preset(preset);
        }
        let q = &mut settings.quality.params;
        let before = *q;
        ui.add(Slider::new(&mut q.supersample, 1..=4).text("Supersampling (N×N)"));
        ui.add(Slider::new(&mut q.base_iterations, 64..=20_000).logarithmic(true).text("Base iterations"));
        ui.add(Slider::new(&mut q.iteration_growth, 0.0..=2.0).text("Iteration growth / decade"));
        ui.add(Slider::new(&mut q.max_iterations, 1_000..=100_000).logarithmic(true).text("Max iterations"));
        ui.add(Slider::new(&mut q.march_steps, 32..=1_000).text("3D march steps"));
        ui.add(Slider::new(&mut q.fold_iterations, 4..=30).text("3D fold iterations"));
        ui.checkbox(&mut q.ao, "3D ambient occlusion");
        ui.checkbox(&mut q.shadows, "3D soft shadows (expensive)");
        if *q != before {
            q.clamp();
            settings.quality.preset = QualityPreset::Custom;
        }
    });
}

/// Enabled scenes, timing, zoom depth, seed and manual scene control.
fn scenes_section(ui: &mut egui::Ui, settings: &mut Settings, actions: &mut UiActions) {
    egui::CollapsingHeader::new("Scenes").show(ui, |ui| {
        let s = &mut settings.scenes;
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.deep_zoom, "Deep zoom");
            ui.checkbox(&mut s.julia, "Julia morph");
            ui.checkbox(&mut s.fly, "3D flythrough");
        });
        if !(s.deep_zoom || s.julia || s.fly) {
            s.julia = true;
        }
        ui.add(Slider::new(&mut s.duration_secs, 5.0..=600.0).logarithmic(true).suffix(" s").text("Scene length"));
        let max_fade = s.duration_secs / 2.0;
        ui.add(Slider::new(&mut s.crossfade_secs, 0.0..=max_fade.min(30.0)).suffix(" s").text("Crossfade"));
        ui.add(Slider::new(&mut s.max_zoom_decades, 2.0..=250.0).text("Max zoom depth (decades)")).on_hover_text("Deeper zooms take longer to prepare in the background.");
        ui.horizontal(|ui| {
            ui.label("Seed");
            ui.add(egui::DragValue::new(&mut s.seed));
            if ui.button("Randomize").clicked() {
                s.seed = random_seed();
            }
            ui.checkbox(&mut s.lock_seed, "Use on launch");
        });
        ui.horizontal(|ui| {
            if ui.button("Next scene (Space)").clicked() {
                actions.next_scene = true;
            }
            for (k, label) in [(SceneKind::DeepZoom, "Deep (1)"), (SceneKind::Julia, "Julia (2)"), (SceneKind::Fly, "Fly (3)")] {
                if ui.button(label).clicked() {
                    actions.force_kind = Some(k);
                }
            }
        });
    });
}

/// Speed sliders.
fn motion_section(ui: &mut egui::Ui, settings: &mut Settings) {
    egui::CollapsingHeader::new("Motion").show(ui, |ui| {
        let m = &mut settings.motion;
        ui.add(Slider::new(&mut m.speed, 0.0..=10.0).logarithmic(true).text("Overall speed (+ / -)"));
        ui.add(Slider::new(&mut m.zoom_speed, 0.0..=5.0).text("Zoom (decades/s)"));
        ui.add(Slider::new(&mut m.rotation_speed, -2.0..=2.0).text("Rotation"));
        ui.add(Slider::new(&mut m.camera_speed, 0.0..=10.0).text("3D camera"));
        ui.add(Slider::new(&mut m.morph_speed, 0.0..=2.0).text("Morph"));
    });
}

/// Color cycling and grading.
fn color_section(ui: &mut egui::Ui, settings: &mut Settings, actions: &mut UiActions) {
    egui::CollapsingHeader::new("Color").show(ui, |ui| {
        let c = &mut settings.color;
        ui.add(Slider::new(&mut c.cycle_speed, -2.0..=2.0).text("Cycle speed"));
        ui.add(Slider::new(&mut c.frequency, 0.05..=20.0).logarithmic(true).text("Color bands"));
        ui.add(Slider::new(&mut c.saturation, 0.0..=3.0).text("Saturation"));
        ui.add(Slider::new(&mut c.brightness, 0.1..=3.0).text("Brightness"));
        ui.add(Slider::new(&mut c.contrast, 0.2..=3.0).text("Contrast"));
        ui.horizontal(|ui| {
            ui.checkbox(&mut c.lock_palette, "Keep palette across scenes");
            if ui.button("New palette (P)").clicked() {
                actions.new_palette = true;
            }
        });
    });
}

/// Chromecast device list, controls and stream options.
fn cast_section(ui: &mut egui::Ui, state: &mut UiState, settings: &mut Settings, info: &UiInfo, actions: &mut UiActions) {
    egui::CollapsingHeader::new("Cast to TV").show(ui, |ui| {
        if let Err(e) = info.ffmpeg {
            ui.label(RichText::new(e).color(Color32::from_rgb(255, 120, 120)));
            return;
        }
        let streaming = matches!(info.cast_status, CastStatus::Starting | CastStatus::Streaming { .. });
        ui.horizontal(|ui| {
            ui.label("Devices");
            if ui.button("Refresh").clicked() {
                actions.refresh_devices = true;
            }
        });
        if info.cast_devices.is_empty() {
            ui.label("Searching the network for Chromecasts…");
        }
        if state.selected_device.is_none() {
            state.selected_device = info.cast_devices.iter().position(|d| Some(&d.name) == settings.cast.last_device.as_ref());
        }
        for (i, dev) in info.cast_devices.iter().enumerate() {
            if ui.selectable_label(state.selected_device == Some(i), format!("{}  ({})", dev.name, dev.host)).clicked() {
                state.selected_device = Some(i);
            }
        }
        ui.add_enabled_ui(!streaming, |ui| {
            ComboBox::from_label("Resolution").selected_text(settings.cast.resolution.label()).show_ui(ui, |ui| {
                for r in [CastResolution::P720, CastResolution::P1080, CastResolution::P2160] {
                    ui.selectable_value(&mut settings.cast.resolution, r, r.label());
                }
            });
            ui.horizontal(|ui| {
                ui.label("Frame rate");
                ui.radio_value(&mut settings.cast.fps, 30, "30");
                ui.radio_value(&mut settings.cast.fps, 60, "60");
            });
            let mut mbps = settings.cast.bitrate_kbps as f32 / 1000.0;
            if ui.add(Slider::new(&mut mbps, 2.0..=40.0).suffix(" Mbps").text("Bitrate")).changed() {
                settings.cast.bitrate_kbps = (mbps * 1000.0) as u32;
            }
        });
        ui.horizontal(|ui| {
            let selected = state.selected_device.and_then(|i| info.cast_devices.get(i));
            if streaming {
                if ui.button("Stop casting").clicked() {
                    actions.cast_stop = true;
                }
            } else if ui.add_enabled(selected.is_some(), egui::Button::new("Cast")).clicked() {
                if let Some(dev) = selected {
                    settings.cast.last_device = Some(dev.name.clone());
                    actions.cast_start = Some(dev.clone());
                }
            }
            let (text, color) = match &info.cast_status {
                CastStatus::Idle => ("Not casting".to_string(), Color32::GRAY),
                CastStatus::Starting => ("Starting stream…".to_string(), Color32::from_rgb(255, 210, 90)),
                CastStatus::Streaming { device } => (format!("Streaming to {device}"), Color32::from_rgb(120, 230, 120)),
                CastStatus::Error(e) => (format!("Error: {e}"), Color32::from_rgb(255, 120, 120)),
            };
            ui.label(RichText::new(text).color(color));
        });
        ui.label(RichText::new("The TV shows the stream a few seconds behind. The settings panel is never sent to the TV.").small());
    });
}

/// Live stats and keyboard help.
fn info_section(ui: &mut egui::Ui, info: &UiInfo) {
    egui::CollapsingHeader::new("Info").show(ui, |ui| {
        ui.label(format!("{:.0} fps ({:.1} ms/frame)", info.fps, info.frame_ms));
        if let Some((kind, seed)) = info.scene {
            ui.label(format!("{}  (seed {seed})", kind.label()));
        }
        if let Some(d) = info.zoom_decades {
            ui.label(format!("Zoom depth {}", format_depth(d)));
        }
        if info.preparing {
            ui.label("Preparing next scene…");
        }
        ui.separator();
        ui.label("Press H for keyboard shortcuts");
    });
}

/// Format a byte count like "312 MB" or "1.2 GB".
pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

/// A fresh random seed from the clock (not used for anything security related).
pub fn random_seed() -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x5eed);
    crate::rng::Rng::new(nanos).next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info<'a>(ffmpeg: &'a Result<(), String>) -> UiInfo<'a> {
        UiInfo {
            fps: 60.0,
            frame_ms: 16.6,
            scene: Some((SceneKind::DeepZoom, 7)),
            zoom_decades: Some(12.34),
            preparing: true,
            process: ProcessSample { cpu_percent: 42.0, memory_bytes: 300 << 20, cores: 8 },
            auto_level: 2,
            effective_render_scale: 0.8,
            monitors: &[],
            video_modes: &[],
            cast_devices: &[],
            cast_status: CastStatus::Idle,
            ffmpeg,
            warning: Some("test warning"),
        }
    }

    #[test]
    fn closed_panel_draws_nothing_and_requests_nothing() {
        let ctx = egui::Context::default();
        let mut state = UiState::default();
        let mut s = Settings::default();
        let ok = Ok(());
        let mut actions = None;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| actions = Some(draw(ui.ctx(), &mut state, &mut s, &info(&ok))));
        assert_eq!(actions.unwrap(), UiActions::default());
    }

    #[test]
    fn open_panel_renders_without_changing_settings() {
        let ctx = egui::Context::default();
        let mut state = UiState { open: true, ..Default::default() };
        let mut s = Settings::default();
        let err = Err("ffmpeg not found".to_string());
        for _ in 0..3 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let a = draw(ui.ctx(), &mut state, &mut s, &info(&err));
                assert!(!a.display_changed && a.cast_start.is_none());
            });
        }
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn help_overlay_draws_without_panel() {
        let ctx = egui::Context::default();
        let mut state = UiState { help: true, ..Default::default() };
        let mut s = Settings::default();
        let ok = Ok(());
        let mut shapes = 0;
        for _ in 0..2 {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| assert_eq!(draw(ui.ctx(), &mut state, &mut s, &info(&ok)), UiActions::default()));
            shapes = out.shapes.len();
        }
        assert!(shapes > 0);
        assert!(SHORTCUTS.iter().any(|(k, _)| *k == "H"));
    }

    #[test]
    fn debug_overlay_draws_without_panel() {
        let ctx = egui::Context::default();
        let mut state = UiState::default();
        let mut s = Settings::default();
        s.display.debug_overlay = true;
        s.quality.auto = true;
        let ok = Ok(());
        let mut shapes = 0;
        for _ in 0..2 {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| assert_eq!(draw(ui.ctx(), &mut state, &mut s, &info(&ok)), UiActions::default()));
            shapes = out.shapes.len();
        }
        assert!(shapes > 0);
    }

    #[test]
    fn formatting() {
        assert_eq!(format_bytes(300 << 20), "300 MB");
        assert_eq!(format_bytes(3 << 29), "1.5 GB");
        assert_eq!(format_depth(23.44), "1e-23.4");
        assert_eq!(format_mode(&VideoModeSpec { width: 1920, height: 1080, refresh_millihertz: 59_940 }), "1920×1080 @ 60 Hz");
    }
}
