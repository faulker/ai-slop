//! User settings: persisted to `config.toml`, edited live in the settings panel,
//! and overridable per run from the command line.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::quality::{QualityParams, QualityPreset};

/// Everything the user can configure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub display: DisplaySettings,
    pub quality: QualitySettings,
    pub scenes: SceneSettings,
    pub motion: MotionSettings,
    pub color: ColorSettings,
    pub cast: CastSettings,
}

/// Window, resolution and frame pacing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    pub fullscreen: bool,
    /// Monitor name to go fullscreen on; `None` = the window's current monitor.
    pub monitor: Option<String>,
    /// Exclusive fullscreen video mode; `None` = borderless at native resolution.
    pub video_mode: Option<VideoModeSpec>,
    /// Window inner size in physical pixels when windowed.
    pub window_width: u32,
    pub window_height: u32,
    /// Scene render resolution relative to the output (above 1 supersamples).
    pub render_scale: f32,
    pub vsync: bool,
    /// Frame rate cap; 0 = uncapped.
    pub fps_cap: u32,
    /// Show the FPS / CPU / memory overlay.
    pub debug_overlay: bool,
}

/// An exclusive-fullscreen video mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoModeSpec {
    pub width: u32,
    pub height: u32,
    pub refresh_millihertz: u32,
}

/// Quality preset plus the concrete values it maps to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QualitySettings {
    pub preset: QualityPreset,
    pub params: QualityParams,
    /// Lower render cost automatically when FPS falls below `target_fps`.
    pub auto: bool,
    /// Frame rate auto quality tries to hold.
    pub target_fps: u32,
}

/// Which scenes play and how long each lasts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSettings {
    pub deep_zoom: bool,
    pub julia: bool,
    pub fly: bool,
    pub duration_secs: f32,
    pub crossfade_secs: f32,
    /// How deep (in decades) a zoom may go before the scene's target runs out.
    pub max_zoom_decades: f32,
    pub seed: u64,
    /// Reuse `seed` on every launch instead of a fresh random one.
    pub lock_seed: bool,
}

/// Speeds of the different motions. All are multipliers or rates per second.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionSettings {
    /// Global speed multiplier applied to every motion below.
    pub speed: f32,
    /// Zoom rate in decades per second.
    pub zoom_speed: f32,
    pub rotation_speed: f32,
    pub camera_speed: f32,
    pub morph_speed: f32,
}

/// Color cycling and grading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorSettings {
    /// Palette cycles per second.
    pub cycle_speed: f32,
    /// Keep the current palette when scenes change.
    pub lock_palette: bool,
    pub saturation: f32,
    pub brightness: f32,
    pub contrast: f32,
    /// How many color bands span the structure (higher = finer color detail).
    pub frequency: f32,
}

/// Chromecast output resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CastResolution {
    P720,
    P1080,
    P2160,
}

impl CastResolution {
    /// Pixel dimensions.
    pub fn size(self) -> (u32, u32) {
        match self {
            Self::P720 => (1280, 720),
            Self::P1080 => (1920, 1080),
            Self::P2160 => (3840, 2160),
        }
    }

    /// Label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::P720 => "720p",
            Self::P1080 => "1080p",
            Self::P2160 => "4K (needs 4K Chromecast/Google TV)",
        }
    }
}

/// Chromecast streaming options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CastSettings {
    pub resolution: CastResolution,
    pub fps: u32,
    pub bitrate_kbps: u32,
    /// Friendly name of the last device cast to, preselected in the list.
    pub last_device: Option<String>,
}

impl Default for DisplaySettings {
    /// Borderless fullscreen (1280×720 when windowed) at full render scale with vsync.
    fn default() -> Self {
        Self { fullscreen: true, monitor: None, video_mode: None, window_width: 1280, window_height: 720, render_scale: 1.0, vsync: true, fps_cap: 0, debug_overlay: false }
    }
}

impl Default for QualitySettings {
    /// The High preset, auto quality off, 60 fps target.
    fn default() -> Self {
        Self { preset: QualityPreset::High, params: QualityParams::default(), auto: false, target_fps: 60 }
    }
}

impl Default for SceneSettings {
    /// All scenes, 45 s each, 4 s crossfades, zooms up to 30 decades.
    fn default() -> Self {
        Self { deep_zoom: true, julia: true, fly: true, duration_secs: 45.0, crossfade_secs: 4.0, max_zoom_decades: 30.0, seed: 1, lock_seed: false }
    }
}

impl Default for MotionSettings {
    /// Gentle, ambient speeds.
    fn default() -> Self {
        Self { speed: 1.0, zoom_speed: 0.4, rotation_speed: 0.05, camera_speed: 0.6, morph_speed: 0.05 }
    }
}

impl Default for ColorSettings {
    /// Neutral grading, slow cycling.
    fn default() -> Self {
        Self { cycle_speed: 0.04, lock_palette: false, saturation: 1.1, brightness: 1.0, contrast: 1.05, frequency: 1.0 }
    }
}

impl Default for CastSettings {
    /// 1080p30 at 12 Mbps, which every Chromecast handles.
    fn default() -> Self {
        Self { resolution: CastResolution::P1080, fps: 30, bitrate_kbps: 12_000, last_device: None }
    }
}

/// Per-run command line overrides. Anything left `None` keeps the saved value.
#[derive(Clone, Debug, clap::Parser)]
#[command(name = "fractal-flow", about = "Endless high-detail animated fractal visuals")]
pub struct Cli {
    /// Fixed seed for a reproducible scene sequence.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Seconds per scene.
    #[arg(long)]
    pub interval: Option<f32>,
    /// Quality preset.
    #[arg(long, value_enum)]
    pub quality: Option<QualityPreset>,
    /// Start in a window instead of fullscreen.
    #[arg(long)]
    pub windowed: bool,
    /// Start fullscreen.
    #[arg(long, conflicts_with = "windowed")]
    pub fullscreen: bool,
    /// Show the FPS / CPU / memory debug overlay.
    #[arg(long)]
    pub debug: bool,
    /// Use this config file instead of the default location.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Render one frame to this PNG file and exit (no window).
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
    /// Scene for --snapshot.
    #[arg(long, value_enum, default_value = "deep")]
    pub snapshot_scene: crate::scene::SceneKind,
    /// Seconds into the scene for --snapshot.
    #[arg(long, default_value_t = 20.0)]
    pub snapshot_time: f32,
    /// Size for --snapshot, e.g. 1920x1080.
    #[arg(long, default_value = "1280x720", value_parser = parse_size)]
    pub snapshot_size: (u32, u32),
}

/// Parse "WIDTHxHEIGHT".
fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT, e.g. 1920x1080")?;
    let w: u32 = w.trim().parse().map_err(|_| format!("bad width in {s}"))?;
    let h: u32 = h.trim().parse().map_err(|_| format!("bad height in {s}"))?;
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err(format!("size {s} out of range"));
    }
    Ok((w, h))
}

impl Settings {
    /// Default config file location (e.g. `~/Library/Application Support/fractal-flow/config.toml`).
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "fractal-flow").map(|d| d.config_dir().join("config.toml"))
    }

    /// Load settings from `path`. A missing file gives defaults silently; an
    /// unreadable or corrupt one gives defaults plus a warning to show the user.
    pub fn load(path: &Path) -> (Settings, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
            Err(e) => return (Settings::default(), Some(format!("Could not read {}: {e}. Using defaults.", path.display()))),
        };
        match toml::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.clamp();
                (s, None)
            }
            Err(e) => (Settings::default(), Some(format!("Config {} is invalid ({e}). Using defaults.", path.display()))),
        }
    }

    /// Write settings to `path`, creating parent directories.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Clamp every value into its valid range so bad input can't break rendering.
    pub fn clamp(&mut self) {
        let d = &mut self.display;
        d.window_width = d.window_width.clamp(320, 7680);
        d.window_height = d.window_height.clamp(240, 4320);
        d.render_scale = finite_or(d.render_scale, 1.0).clamp(0.25, 2.0);
        d.fps_cap = d.fps_cap.min(480);

        self.quality.params.clamp();
        self.quality.target_fps = self.quality.target_fps.clamp(15, 240);
        if let Some(p) = self.quality.preset.params() {
            if p != self.quality.params {
                self.quality.preset = QualityPreset::Custom;
            }
        }

        let s = &mut self.scenes;
        s.duration_secs = finite_or(s.duration_secs, 45.0).clamp(5.0, 3600.0);
        s.crossfade_secs = finite_or(s.crossfade_secs, 4.0).clamp(0.0, s.duration_secs / 2.0);
        s.max_zoom_decades = finite_or(s.max_zoom_decades, 30.0).clamp(2.0, 250.0);
        if !(s.deep_zoom || s.julia || s.fly) {
            s.deep_zoom = true;
            s.julia = true;
            s.fly = true;
        }

        let m = &mut self.motion;
        m.speed = finite_or(m.speed, 1.0).clamp(0.0, 10.0);
        m.zoom_speed = finite_or(m.zoom_speed, 0.4).clamp(0.0, 5.0);
        m.rotation_speed = finite_or(m.rotation_speed, 0.05).clamp(-2.0, 2.0);
        m.camera_speed = finite_or(m.camera_speed, 0.6).clamp(0.0, 10.0);
        m.morph_speed = finite_or(m.morph_speed, 0.05).clamp(0.0, 2.0);

        let c = &mut self.color;
        c.cycle_speed = finite_or(c.cycle_speed, 0.04).clamp(-2.0, 2.0);
        c.saturation = finite_or(c.saturation, 1.0).clamp(0.0, 3.0);
        c.brightness = finite_or(c.brightness, 1.0).clamp(0.1, 3.0);
        c.contrast = finite_or(c.contrast, 1.0).clamp(0.2, 3.0);
        c.frequency = finite_or(c.frequency, 1.0).clamp(0.05, 20.0);

        let k = &mut self.cast;
        k.fps = if k.fps >= 45 { 60 } else { 30 };
        k.bitrate_kbps = k.bitrate_kbps.clamp(1_000, 80_000);
    }

    /// Select a quality preset, copying its concrete values in.
    pub fn set_quality_preset(&mut self, preset: QualityPreset) {
        self.quality.preset = preset;
        if let Some(p) = preset.params() {
            self.quality.params = p;
        }
    }

    /// Apply command line overrides on top of the loaded settings.
    pub fn apply_cli(&mut self, cli: &Cli) {
        if let Some(seed) = cli.seed {
            self.scenes.seed = seed;
            self.scenes.lock_seed = true;
        }
        if let Some(secs) = cli.interval {
            self.scenes.duration_secs = secs;
        }
        if let Some(q) = cli.quality {
            self.set_quality_preset(q);
        }
        if cli.windowed {
            self.display.fullscreen = false;
        }
        if cli.fullscreen {
            self.display.fullscreen = true;
        }
        if cli.debug {
            self.display.debug_overlay = true;
        }
        self.clamp();
    }
}

/// Replace NaN/inf with a fallback (clamp alone would keep NaN).
fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn toml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let mut s = Settings::default();
        s.scenes.seed = 1234;
        s.color.saturation = 1.7;
        s.display.video_mode = Some(VideoModeSpec { width: 1920, height: 1080, refresh_millihertz: 60_000 });
        s.save(&path).unwrap();
        let (loaded, warn) = Settings::load(&path);
        assert!(warn.is_none());
        assert_eq!(loaded, s);
    }

    #[test]
    fn missing_file_is_defaults_without_warning() {
        let (s, warn) = Settings::load(Path::new("/definitely/not/here/config.toml"));
        assert_eq!(s, Settings::default());
        assert!(warn.is_none());
    }

    #[test]
    fn corrupt_file_is_defaults_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is = = not toml [[[").unwrap();
        let (s, warn) = Settings::load(&path);
        assert_eq!(s, Settings::default());
        assert!(warn.unwrap().contains("invalid"));
    }

    #[test]
    fn partial_file_fills_defaults_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[display]\nrender_scale = 50.0\n[scenes]\ndeep_zoom = false\njulia = false\nfly = false\n").unwrap();
        let (s, warn) = Settings::load(&path);
        assert!(warn.is_none());
        assert_eq!(s.display.render_scale, 2.0);
        assert!(s.scenes.deep_zoom && s.scenes.julia && s.scenes.fly, "no scenes enabled re-enables all");
        assert_eq!(s.motion, MotionSettings::default());
    }

    #[test]
    fn clamp_handles_nan_and_ranges() {
        let mut s = Settings::default();
        s.motion.speed = f32::NAN;
        s.color.contrast = 100.0;
        s.cast.fps = 47;
        s.scenes.crossfade_secs = 1000.0;
        s.quality.target_fps = 1000;
        s.clamp();
        assert_eq!(s.quality.target_fps, 240);
        assert_eq!(s.motion.speed, 1.0);
        assert_eq!(s.color.contrast, 3.0);
        assert_eq!(s.cast.fps, 60);
        assert!(s.scenes.crossfade_secs <= s.scenes.duration_secs / 2.0);
    }

    #[test]
    fn cli_overrides_saved_values() {
        let mut s = Settings::default();
        s.display.fullscreen = true;
        let cli = Cli::parse_from(["fractal-flow", "--seed", "42", "--interval", "120", "--quality", "ultra", "--windowed", "--debug"]);
        s.apply_cli(&cli);
        assert_eq!(s.scenes.seed, 42);
        assert!(s.scenes.lock_seed);
        assert_eq!(s.scenes.duration_secs, 120.0);
        assert_eq!(s.quality.preset, QualityPreset::Ultra);
        assert_eq!(s.quality.params, QualityPreset::Ultra.params().unwrap());
        assert!(!s.display.fullscreen);
        assert!(s.display.debug_overlay);
    }

    #[test]
    fn preset_sets_values_and_edits_become_custom() {
        let mut s = Settings::default();
        s.set_quality_preset(QualityPreset::Low);
        assert_eq!(s.quality.params, QualityPreset::Low.params().unwrap());
        s.quality.params.march_steps += 7;
        s.clamp();
        assert_eq!(s.quality.preset, QualityPreset::Custom);
    }
}
