//! Window, event loop and per-frame orchestration: scene preparation threads,
//! director events, rendering, the settings overlay and Chromecast output.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::cast::capture::FrameCapture;
use crate::cast::discovery::Discovery;
use crate::cast::{CastConfig, CastDeviceInfo, CastSession, CastStatus};
use crate::director::{Director, DirectorEvent};
use crate::gpu::{CompositeUniforms, Output, Renderer};
use crate::palette::Palette;
use crate::perf::{self, AutoQuality, ProcessMonitor};
use crate::rng::Rng;
use crate::scene::{pick_kind, Scene, SceneClock, SceneKind};
use crate::settings::{Settings, VideoModeSpec};
use crate::ui::{self, UiActions, UiInfo, UiState};

/// Open the window and run until quit.
pub fn run(settings: Settings, config_path: Option<PathBuf>, warning: Option<String>) -> anyhow::Result<()> {
    let event_loop = EventLoop::new().context("could not create event loop")?;
    let mut app = App::new(settings, config_path, warning);
    event_loop.run_app(&mut app).context("event loop failed")?;
    match app.fatal.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// A scene on screen with its motion clock and the render slot it draws into.
struct ActiveScene {
    scene: Scene,
    clock: SceneClock,
    slot: usize,
}

/// Window plus everything that needs it to exist.
struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

/// An active Chromecast stream and its capture target.
struct CastRuntime {
    session: CastSession,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    capture: FrameCapture,
    next_capture: Instant,
    period: Duration,
}

/// Rolling frame-rate measurement.
struct FrameStats {
    frames: u32,
    window_start: Instant,
    fps: f32,
    frame_ms: f32,
}

struct App {
    settings: Settings,
    config_path: Option<PathBuf>,
    warning: Option<String>,
    fatal: Option<anyhow::Error>,
    gfx: Option<Gfx>,
    egui_ctx: egui::Context,
    ui: UiState,

    director: Director,
    seq_rng: Rng,
    last_kind: Option<SceneKind>,
    current: Option<ActiveScene>,
    incoming: Option<ActiveScene>,
    pending: Option<Receiver<Scene>>,
    prepared: Option<Scene>,
    forced_kind: Option<SceneKind>,

    cast: Option<CastRuntime>,
    last_cast_status: CastStatus,
    discovery: Option<Discovery>,
    cast_devices: Vec<CastDeviceInfo>,
    ffmpeg: Result<(), String>,

    monitors: Vec<String>,
    video_modes: Vec<VideoModeSpec>,
    last_frame: Instant,
    next_frame_at: Instant,
    stats: FrameStats,
    last_title: Instant,
    auto_quality: AutoQuality,
    process: ProcessMonitor,
}

impl App {
    /// Initial state; the window is created on `resumed`.
    fn new(mut settings: Settings, config_path: Option<PathBuf>, warning: Option<String>) -> Self {
        if !settings.scenes.lock_seed {
            settings.scenes.seed = ui::random_seed();
        }
        let now = Instant::now();
        Self {
            director: Director::new(settings.scenes.duration_secs, settings.scenes.crossfade_secs),
            seq_rng: Rng::new(settings.scenes.seed),
            settings,
            config_path,
            warning,
            fatal: None,
            gfx: None,
            egui_ctx: egui::Context::default(),
            ui: UiState::default(),
            last_kind: None,
            current: None,
            incoming: None,
            pending: None,
            prepared: None,
            forced_kind: None,
            cast: None,
            last_cast_status: CastStatus::Idle,
            discovery: None,
            cast_devices: Vec::new(),
            ffmpeg: Ok(()),
            monitors: Vec::new(),
            video_modes: Vec::new(),
            last_frame: now,
            next_frame_at: now,
            stats: FrameStats { frames: 0, window_start: now, fps: 0.0, frame_ms: 0.0 },
            last_title: now,
            auto_quality: AutoQuality::default(),
            process: ProcessMonitor::new(Duration::from_millis(500)),
        }
    }

    /// Create the window, GPU device, surface and egui renderer.
    fn init_gfx(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let d = &self.settings.display;
        let attrs = Window::default_attributes().with_title("fractal-flow").with_inner_size(PhysicalSize::new(d.window_width, d.window_height));
        let window = Arc::new(event_loop.create_window(attrs).context("could not create window")?);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(window.clone()).context("could not create surface")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("no compatible GPU adapter")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).context("could not open GPU device")?;

        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB format: shaders output display-ready colors and egui expects it.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: present_mode(self.settings.display.vsync),
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let egui_renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let egui_state = egui_winit::State::new(self.egui_ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, Some(device.limits().max_texture_dimension_2d as usize));
        let renderer = Renderer::new(device, queue, self.scene_size_for(size, self.settings.display.render_scale));

        self.monitors = window.available_monitors().filter_map(|m| m.name()).collect();
        self.video_modes = window
            .current_monitor()
            .map(|m| {
                let mut modes: Vec<VideoModeSpec> = m.video_modes().map(|v| VideoModeSpec { width: v.size().width, height: v.size().height, refresh_millihertz: v.refresh_rate_millihertz() }).collect();
                modes.sort_by_key(|m| std::cmp::Reverse((m.width, m.height, m.refresh_millihertz)));
                modes.dedup();
                modes
            })
            .unwrap_or_default();

        self.gfx = Some(Gfx { window, surface, config, renderer, egui_state, egui_renderer });
        self.apply_display();
        Ok(())
    }

    /// Scene target size: the cast resolution while casting, otherwise the window, times `render_scale`.
    fn scene_size_for(&self, window: PhysicalSize<u32>, render_scale: f32) -> (u32, u32) {
        let base = match &self.cast {
            Some(c) => (c.texture.width(), c.texture.height()),
            None => (window.width, window.height),
        };
        let s = render_scale;
        (((base.0 as f32 * s) as u32).max(1), ((base.1 as f32 * s) as u32).max(1))
    }

    /// Apply fullscreen/monitor/window size/vsync settings to the window and surface.
    fn apply_display(&mut self) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let d = &self.settings.display;
        let monitor = match &d.monitor {
            Some(name) => gfx.window.available_monitors().find(|m| m.name().as_deref() == Some(name)),
            None => gfx.window.current_monitor(),
        };
        if d.fullscreen {
            let exclusive = d.video_mode.and_then(|spec| {
                monitor.as_ref()?.video_modes().find(|v| v.size().width == spec.width && v.size().height == spec.height && v.refresh_rate_millihertz() == spec.refresh_millihertz)
            });
            match exclusive {
                Some(mode) => gfx.window.set_fullscreen(Some(Fullscreen::Exclusive(mode))),
                None => gfx.window.set_fullscreen(Some(Fullscreen::Borderless(monitor))),
            }
            gfx.window.set_cursor_visible(self.ui.open);
        } else {
            gfx.window.set_fullscreen(None);
            gfx.window.set_cursor_visible(true);
            if let Some(size) = gfx.window.request_inner_size(PhysicalSize::new(d.window_width, d.window_height)) {
                gfx.config.width = size.width.max(1);
                gfx.config.height = size.height.max(1);
            }
        }
        gfx.config.present_mode = present_mode(d.vsync);
        gfx.surface.configure(&gfx.renderer.device, &gfx.config);
    }

    /// Start preparing the next scene on a background thread.
    fn request_next(&mut self) {
        if self.pending.is_some() || self.prepared.is_some() {
            return;
        }
        let kind = self.forced_kind.take().unwrap_or_else(|| pick_kind(&mut self.seq_rng, &self.settings, self.last_kind));
        let seed = self.seq_rng.next_u64();
        let settings = self.settings.clone();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("scene-prep".into()).spawn(move || {
            let scene = Scene::generate(kind, seed, &settings);
            if tx.send(scene).is_err() {
                log::debug!("prepared scene discarded (superseded)");
            }
        });
        match spawned {
            Ok(_) => self.pending = Some(rx),
            Err(e) => log::error!("could not start scene preparation thread: {e}"),
        }
    }

    /// Jump to a scene of a specific kind as soon as it is prepared.
    fn force_scene(&mut self, kind: SceneKind) {
        self.pending = None;
        self.prepared = None;
        self.forced_kind = Some(kind);
        self.request_next();
        self.director.skip();
    }

    /// Pick up a finished background scene, if any.
    fn poll_pending(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(scene) => {
                self.prepared = Some(scene);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                log::error!("scene preparation thread died; requesting another scene");
                self.pending = None;
                self.request_next();
            }
        }
    }

    /// React to a director event.
    fn handle_director(&mut self, event: DirectorEvent) {
        match event {
            DirectorEvent::None => {}
            DirectorEvent::RequestNext => self.request_next(),
            DirectorEvent::BeginTransition => {
                let Some(mut scene) = self.prepared.take() else { return };
                if let (true, Some(cur)) = (self.settings.color.lock_palette, &self.current) {
                    scene.palette = cur.scene.palette;
                }
                let slot = self.current.as_ref().map(|c| 1 - c.slot).unwrap_or(0);
                if let (Some(gfx), Some(orbit)) = (self.gfx.as_mut(), scene.orbit()) {
                    gfx.renderer.upload_orbit(slot, orbit);
                }
                log::info!("scene: {} (seed {})", scene.kind.label(), scene.seed);
                self.last_kind = Some(scene.kind);
                self.incoming = Some(ActiveScene { scene, clock: SceneClock::default(), slot });
            }
            DirectorEvent::FinishTransition => {
                if let Some(next) = self.incoming.take() {
                    self.current = Some(next);
                }
            }
        }
    }

    /// Give the visible scenes a new random palette.
    fn new_palette(&mut self) {
        let p = Palette::random(&mut self.seq_rng);
        for s in [self.current.as_mut(), self.incoming.as_mut()].into_iter().flatten() {
            s.scene.palette = p;
        }
    }

    /// Save settings to the config file, surfacing failures in the panel.
    fn save_settings(&mut self) {
        let Some(path) = &self.config_path else { return };
        match self.settings.save(path) {
            Ok(()) => log::info!("saved settings to {}", path.display()),
            Err(e) => {
                let msg = format!("Could not save settings to {}: {e}", path.display());
                log::error!("{msg}");
                self.warning = Some(msg);
            }
        }
    }

    /// Toggle the settings panel (saving on close). Starts Chromecast discovery the first time.
    fn toggle_ui(&mut self) {
        self.ui.open = !self.ui.open;
        if self.ui.open {
            self.ensure_discovery();
        } else {
            self.save_settings();
        }
        if let Some(gfx) = &self.gfx {
            gfx.window.set_cursor_visible(self.ui.open || !self.settings.display.fullscreen);
        }
    }

    /// Start mDNS discovery and check for ffmpeg (lazily, so launch never prompts for network access).
    fn ensure_discovery(&mut self) {
        if self.discovery.is_some() {
            return;
        }
        self.ffmpeg = crate::cast::ffmpeg_available();
        match Discovery::start() {
            Ok(d) => self.discovery = Some(d),
            Err(e) => self.warning = Some(format!("Chromecast discovery failed: {e}")),
        }
    }

    /// Start streaming to a device.
    fn start_cast(&mut self, device: CastDeviceInfo) {
        self.stop_cast(false);
        let Some(gfx) = &self.gfx else { return };
        let (w, h) = self.settings.cast.resolution.size();
        let config = CastConfig { width: w, height: h, fps: self.settings.cast.fps, bitrate_kbps: self.settings.cast.bitrate_kbps };
        let device_gpu = &gfx.renderer.device;
        let texture = device_gpu.create_texture(&wgpu::TextureDescriptor {
            label: Some("cast target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let capture = FrameCapture::new(device_gpu, w, h);
        log::info!("casting to {} at {w}x{h}@{}", device.name, config.fps);
        let session = CastSession::start(device, config);
        self.cast = Some(CastRuntime { session, texture, view, capture, next_capture: Instant::now(), period: Duration::from_secs_f64(1.0 / config.fps as f64) });
    }

    /// Stop streaming. With `wait` it blocks (up to ~6s) until the TV session is
    /// closed, which is what quitting needs; otherwise it closes in the background
    /// so the visuals keep running.
    fn stop_cast(&mut self, wait: bool) {
        if let Some(c) = self.cast.take() {
            if wait {
                c.session.stop();
            } else if let Err(e) = std::thread::Builder::new().name("cast-stop".into()).spawn(move || c.session.stop()) {
                log::error!("could not start cast stop thread: {e}");
            }
        }
        self.last_cast_status = CastStatus::Idle;
    }

    /// Act on panel requests.
    fn handle_ui_actions(&mut self, a: UiActions) {
        if a.display_changed {
            self.apply_display();
        }
        if a.next_scene {
            self.director.skip();
        }
        if let Some(k) = a.force_kind {
            self.force_scene(k);
        }
        if a.new_palette {
            self.new_palette();
        }
        if a.reset_defaults {
            let seed = self.settings.scenes.seed;
            self.settings = Settings::default();
            self.settings.scenes.seed = seed;
            self.apply_display();
        }
        if a.refresh_devices {
            if let Some(d) = self.discovery.as_mut() {
                if let Err(e) = d.refresh() {
                    self.warning = Some(format!("Chromecast discovery refresh failed: {e}"));
                }
            }
        }
        if let Some(dev) = a.cast_start {
            self.start_cast(dev);
        }
        if a.cast_stop {
            self.stop_cast(false);
        }
        if a.closed {
            self.save_settings();
            if let Some(gfx) = &self.gfx {
                gfx.window.set_cursor_visible(!self.settings.display.fullscreen);
            }
        }
    }

    /// Keyboard shortcuts.
    fn handle_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.ui.help {
                    self.ui.help = false;
                } else if self.ui.open {
                    self.toggle_ui();
                } else {
                    self.shutdown(event_loop);
                }
            }
            Key::Named(NamedKey::Tab) if !event.repeat => self.toggle_ui(),
            Key::Named(NamedKey::Space) if !event.repeat => self.director.skip(),
            Key::Character(c) => match c.as_str() {
                "s" | "S" if !event.repeat => self.toggle_ui(),
                "h" | "H" if !event.repeat => self.ui.help = !self.ui.help,
                "d" | "D" if !event.repeat => self.settings.display.debug_overlay = !self.settings.display.debug_overlay,
                "1" => self.force_scene(SceneKind::DeepZoom),
                "2" => self.force_scene(SceneKind::Julia),
                "3" => self.force_scene(SceneKind::Fly),
                "p" | "P" => self.new_palette(),
                "+" | "=" => self.settings.motion.speed = (self.settings.motion.speed * 1.25).min(10.0),
                "-" | "_" => self.settings.motion.speed = (self.settings.motion.speed / 1.25).max(0.01),
                "[" => self.settings.set_quality_preset(self.settings.quality.preset.step(false)),
                "]" => self.settings.set_quality_preset(self.settings.quality.preset.step(true)),
                "f" | "F" if !event.repeat => {
                    self.settings.display.fullscreen = !self.settings.display.fullscreen;
                    self.apply_display();
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Save, stop casting and exit.
    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.stop_cast(true);
        if self.ui.open {
            self.save_settings();
        }
        event_loop.exit();
    }

    /// Advance time, render the scenes, composite to the window (and the TV), draw the panel.
    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.update_stats(now);

        self.poll_pending();
        self.director.set_timing(self.settings.scenes.duration_secs, self.settings.scenes.crossfade_secs);
        let event = self.director.update(dt, self.prepared.is_some());
        self.handle_director(event);
        for s in [self.current.as_mut(), self.incoming.as_mut()].into_iter().flatten() {
            s.clock.advance(dt, &self.settings);
        }
        if let Some(d) = self.discovery.as_mut() {
            self.cast_devices = d.devices();
        }
        if let Some(c) = &self.cast {
            let status = c.session.status();
            if matches!(status, CastStatus::Error(_)) && !matches!(self.last_cast_status, CastStatus::Error(_)) {
                log::error!("cast failed: {status:?}");
            }
            self.last_cast_status = status;
        }

        let Some(window_size) = self.gfx.as_ref().map(|g| g.window.inner_size()) else { return };
        // Auto quality renders from a cheaper copy of the settings; the saved ones stay untouched.
        let render = perf::degrade(&self.settings, self.auto_quality.level());
        let scene_size = self.scene_size_for(window_size, render.display.render_scale);
        let blend = self.director.blend();
        let ui_actions = self.draw_ui(now, render.display.render_scale);
        let Some(gfx) = self.gfx.as_mut() else { return };
        gfx.renderer.resize_scenes(scene_size);
        // Upload egui textures before the surface can bail, or a dropped font atlas panics later.
        if let Some((_, textures, _)) = &ui_actions.1 {
            for (id, delta) in &textures.set {
                gfx.egui_renderer.update_texture(&gfx.renderer.device, &gfx.renderer.queue, *id, delta);
            }
        }

        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error; skipping frame");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = gfx.renderer.device.create_command_encoder(&Default::default());

        let mut weights = [0.0f32; 4];
        if let Some(cur) = &self.current {
            gfx.renderer.render_scene(&mut encoder, cur.slot, cur.scene.kind, &cur.scene.uniforms(&cur.clock, &render, scene_size));
            weights[cur.slot] = 1.0 - blend;
        }
        if let Some(inc) = &self.incoming {
            gfx.renderer.render_scene(&mut encoder, inc.slot, inc.scene.kind, &inc.scene.uniforms(&inc.clock, &render, scene_size));
            weights[inc.slot] = blend;
        }
        let c = &self.settings.color;
        let composite = CompositeUniforms { weights, grade: [c.saturation, c.brightness, c.contrast, 0.0], flags: [0; 4] };
        gfx.renderer.composite(&mut encoder, Output::Window, &view, gfx.config.format, &composite);

        let mut captured = false;
        if let Some(cast) = self.cast.as_mut() {
            if now >= cast.next_capture {
                gfx.renderer.composite(&mut encoder, Output::Cast, &cast.view, wgpu::TextureFormat::Rgba8Unorm, &composite);
                captured = cast.capture.copy(&mut encoder, &cast.texture);
                cast.next_capture = (cast.next_capture + cast.period).max(now);
            }
        }

        let mut command_buffers = Vec::new();
        if let Some((paint_jobs, textures, screen)) = ui_actions.1 {
            command_buffers = gfx.egui_renderer.update_buffers(&gfx.renderer.device, &gfx.renderer.queue, &mut encoder, &paint_jobs, &screen);
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                gfx.egui_renderer.render(&mut pass.forget_lifetime(), &paint_jobs, &screen);
            }
            for id in &textures.free {
                gfx.egui_renderer.free_texture(id);
            }
        }
        command_buffers.push(encoder.finish());
        gfx.renderer.queue.submit(command_buffers);
        gfx.window.pre_present_notify();
        frame.present();

        if let Some(cast) = self.cast.as_mut() {
            if captured {
                cast.capture.after_submit();
            }
            if let Some(rgba) = cast.capture.poll(&gfx.renderer.device) {
                cast.session.push_frame(rgba);
            }
        }
        self.handle_ui_actions(ui_actions.0);
        self.update_title(now);
    }

    /// Run the egui pass. Returns panel actions plus tessellated output to paint (if open).
    #[allow(clippy::type_complexity)]
    fn draw_ui(&mut self, now: Instant, effective_render_scale: f32) -> (UiActions, Option<(Vec<egui::ClippedPrimitive>, egui::TexturesDelta, egui_wgpu::ScreenDescriptor)>) {
        let Some(gfx) = self.gfx.as_mut() else { return (UiActions::default(), None) };
        let raw = gfx.egui_state.take_egui_input(&gfx.window);
        let shown = self.current.as_ref().or(self.incoming.as_ref());
        // Only pay for process sampling while someone is looking at it.
        let process = if self.settings.display.debug_overlay { self.process.sample(now) } else { Default::default() };
        let cast_status = if self.cast.is_some() { self.last_cast_status.clone() } else { CastStatus::Idle };
        let info = UiInfo {
            fps: self.stats.fps,
            frame_ms: self.stats.frame_ms,
            scene: shown.map(|s| (s.scene.kind, s.scene.seed)),
            zoom_decades: shown.and_then(|s| s.scene.zoom_decades(&s.clock)),
            preparing: self.pending.is_some(),
            process,
            auto_level: self.auto_quality.level(),
            effective_render_scale,
            monitors: &self.monitors,
            video_modes: &self.video_modes,
            cast_devices: &self.cast_devices,
            cast_status,
            ffmpeg: &self.ffmpeg,
            warning: self.warning.as_deref(),
        };
        let mut actions = UiActions::default();
        let output = self.egui_ctx.run_ui(raw, |ui| actions = ui::draw(ui.ctx(), &mut self.ui, &mut self.settings, &info));
        gfx.egui_state.handle_platform_output(&gfx.window, output.platform_output);
        let paint = if output.shapes.is_empty() && output.textures_delta.is_empty() {
            None
        } else {
            let jobs = self.egui_ctx.tessellate(output.shapes, output.pixels_per_point);
            let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [gfx.config.width, gfx.config.height], pixels_per_point: output.pixels_per_point };
            Some((jobs, output.textures_delta, screen))
        };
        (actions, paint)
    }

    /// Update the FPS counter, and feed each finished measurement to auto quality.
    fn update_stats(&mut self, now: Instant) {
        self.stats.frames += 1;
        let elapsed = (now - self.stats.window_start).as_secs_f32();
        if elapsed >= 0.5 {
            self.stats.fps = self.stats.frames as f32 / elapsed;
            self.stats.frame_ms = 1000.0 * elapsed / self.stats.frames as f32;
            self.stats.frames = 0;
            self.stats.window_start = now;
            if self.settings.quality.auto {
                if self.auto_quality.update(self.stats.fps, perf::target_fps(&self.settings), elapsed) {
                    log::info!("auto quality level {}", self.auto_quality.level());
                }
            } else if self.auto_quality.level() > 0 {
                self.auto_quality.reset();
            }
        }
    }

    /// Window title with scene, zoom depth and FPS (visible when windowed).
    fn update_title(&mut self, now: Instant) {
        if now - self.last_title < Duration::from_millis(500) {
            return;
        }
        self.last_title = now;
        let Some(gfx) = &self.gfx else { return };
        let mut title = format!("fractal-flow | {:.0} fps", self.stats.fps);
        if let Some(s) = self.current.as_ref().or(self.incoming.as_ref()) {
            title.push_str(&format!(" | {}", s.scene.kind.label()));
            if let Some(d) = s.scene.zoom_decades(&s.clock) {
                title.push_str(&format!(" {}", ui::format_depth(d)));
            }
        } else {
            title.push_str(" | preparing…");
        }
        if let CastStatus::Streaming { device } = &self.last_cast_status {
            title.push_str(&format!(" | casting to {device}"));
        }
        gfx.window.set_title(&title);
    }
}

/// VSync on = FIFO, off = lowest-latency mode available.
fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    }
}

impl ApplicationHandler for App {
    /// Create the window on first resume.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        if let Err(e) = self.init_gfx(event_loop) {
            log::error!("startup failed: {e:#}");
            self.fatal = Some(e);
            event_loop.exit();
        }
    }

    /// Route window events to egui first, then to the app.
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let consumed = match self.gfx.as_mut() {
            Some(gfx) if self.ui.open => gfx.egui_state.on_window_event(&gfx.window, &event).consumed,
            _ => false,
        };
        match event {
            WindowEvent::CloseRequested => self.shutdown(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.config.width = size.width.max(1);
                    gfx.config.height = size.height.max(1);
                    gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let typing = self.ui.open && (consumed || self.egui_ctx.egui_wants_keyboard_input());
                let is_escape = event.logical_key == Key::Named(NamedKey::Escape);
                if !typing || is_escape {
                    self.handle_key(event_loop, &event);
                }
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    /// Pace frames: uncapped redraws continuously (vsync throttles), capped waits.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(gfx) = &self.gfx else { return };
        let cap = self.settings.display.fps_cap;
        if cap == 0 {
            gfx.window.request_redraw();
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }
        let now = Instant::now();
        if now >= self.next_frame_at {
            gfx.window.request_redraw();
            self.next_frame_at = (self.next_frame_at + Duration::from_secs_f64(1.0 / cap as f64)).max(now);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame_at));
    }

    /// Stop casting if the loop exits some other way.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.stop_cast(true);
    }
}
