//! Scenes: what is on screen, how it was randomly generated, and how it evolves.
//!
//! A `Scene` is generated once from a seed (slow for deep zooms, so off-thread).
//! A `SceneClock` integrates motion every frame; integrating rates (rather than
//! multiplying by elapsed time) keeps motion smooth when sliders change live.
//! `Scene::uniforms` turns both into the GPU uniform block.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

use crate::deep::{self, DeepTarget};
use crate::palette::Palette;
use crate::rng::Rng;
use crate::settings::Settings;

/// The three visual styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum SceneKind {
    /// Mandelbrot deep zoom with perturbation (image 1 style spirals).
    #[value(name = "deep")]
    DeepZoom,
    /// Morphing Julia set swirls.
    Julia,
    /// Raymarched 3D fractal tunnel flythrough (image 2 style).
    Fly,
}

impl SceneKind {
    /// Label for the UI and window title.
    pub fn label(self) -> &'static str {
        match self {
            Self::DeepZoom => "Deep zoom",
            Self::Julia => "Julia morph",
            Self::Fly => "3D flythrough",
        }
    }
}

/// Parameters for a Julia morph scene.
#[derive(Clone, Debug, PartialEq)]
pub struct JuliaParams {
    /// Radius of the circle `c` travels on (≈0.7885 gives connected, spiral-rich sets).
    pub c_radius: f32,
    /// Starting angle of `c` on that circle.
    pub c_angle: f32,
    /// Breathing zoom range (view heights).
    pub zoom_far: f32,
    pub zoom_near: f32,
    /// Center drift radius and direction.
    pub drift: f32,
    pub stripe_density: f32,
}

/// 3D fractal family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyVariant {
    Mandelbox = 0,
    Kifs = 1,
}

/// Parameters for a flythrough scene.
#[derive(Clone, Debug, PartialEq)]
pub struct FlyParams {
    pub variant: FlyVariant,
    /// Fractal scale (Mandelbox: negative; KIFS: 2–3).
    pub scale: f32,
    pub scale_wobble: f32,
    pub min_r2: f32,
    pub fixed_r2: f32,
    pub offset: [f32; 3],
    pub fold_angle: f32,
    pub tunnel_radius: f32,
    /// Lissajous tunnel path: x = a1·sin(f1·z) + a2·sin(f2·z), y likewise.
    pub path_x: [f32; 4],
    pub path_y: [f32; 4],
    pub roll: f32,
    pub fov_deg: f32,
}

/// Per-kind parameters.
#[derive(Clone, Debug)]
pub enum SceneParams {
    Deep(Arc<DeepTarget>),
    Julia(JuliaParams),
    Fly(FlyParams),
}

/// A fully generated scene.
#[derive(Clone, Debug)]
pub struct Scene {
    pub kind: SceneKind,
    pub seed: u64,
    pub palette: Palette,
    /// Rotation direction (±1) for 2D scenes.
    pub spin: f32,
    pub params: SceneParams,
}

/// Accumulated motion for one scene.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneClock {
    /// Seconds since the scene started (unscaled).
    pub time: f32,
    /// Zoom depth in decades.
    pub zoom: f64,
    pub rotation: f32,
    /// Distance travelled along the 3D path.
    pub camera: f32,
    pub morph: f32,
    /// Palette offset in cycles.
    pub color: f32,
}

/// GPU uniform block shared by all scene shaders. Must mirror `struct Scene` in
/// `shaders/common.wgsl` field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    pub resolution: [f32; 2],
    pub supersample: u32,
    pub max_iter: u32,
    pub pal_a: [f32; 4],
    pub pal_b: [f32; 4],
    pub pal_c: [f32; 4],
    pub pal_d: [f32; 4],
    /// x: palette offset, y: color frequency, z: stripe density, w: time.
    pub color: [f32; 4],
    /// x: cos(rotation), y: sin(rotation), z: pixel size (deep: mantissa), w: unused.
    pub view: [f32; 4],
    /// x: pixel size exponent (deep), y: orbit length, z: flags (1 AO, 2 shadows), w: fractal variant.
    pub view_i: [i32; 4],
    /// xy: Julia c, zw: view center (Julia).
    pub julia: [f32; 4],
    /// xyz: camera position, w: tan(fov/2).
    pub cam_pos: [f32; 4],
    pub cam_fwd: [f32; 4],
    pub cam_up: [f32; 4],
    /// x: scale, y: min_r2, z: fixed_r2, w: tunnel radius.
    pub fractal: [f32; 4],
    /// xyz: offset, w: fold angle.
    pub fractal2: [f32; 4],
    pub path_x: [f32; 4],
    pub path_y: [f32; 4],
    /// x: march steps, y: fold iterations.
    pub fly_i: [u32; 4],
}

impl SceneClock {
    /// Integrate one frame of motion using the current settings.
    pub fn advance(&mut self, dt: f32, settings: &Settings) {
        let m = &settings.motion;
        let s = m.speed;
        self.time += dt;
        self.zoom += (dt * m.zoom_speed * s) as f64;
        self.rotation += dt * m.rotation_speed * s;
        self.camera += dt * m.camera_speed * s;
        self.morph += dt * m.morph_speed * s;
        self.color += dt * settings.color.cycle_speed * s;
    }
}

/// Pick the next scene kind among the enabled ones, avoiding an immediate repeat
/// when there is a choice.
pub fn pick_kind(rng: &mut Rng, settings: &Settings, previous: Option<SceneKind>) -> SceneKind {
    let s = &settings.scenes;
    let enabled: Vec<SceneKind> = [(s.deep_zoom, SceneKind::DeepZoom), (s.julia, SceneKind::Julia), (s.fly, SceneKind::Fly)]
        .into_iter()
        .filter_map(|(on, k)| on.then_some(k))
        .collect();
    let choices: Vec<SceneKind> = if enabled.len() > 1 { enabled.iter().copied().filter(|k| Some(*k) != previous).collect() } else { enabled };
    choices.get(rng.index(choices.len().max(1))).copied().unwrap_or(SceneKind::Julia)
}

impl Scene {
    /// Generate a scene of `kind` from `seed`. Deep zooms run a target search
    /// that can take seconds, so call this off the render thread.
    pub fn generate(kind: SceneKind, seed: u64, settings: &Settings) -> Scene {
        let mut rng = Rng::new(seed);
        let palette = Palette::random(&mut rng);
        let spin = if rng.bool() { 1.0 } else { -1.0 };
        let params = match kind {
            SceneKind::DeepZoom => SceneParams::Deep(Arc::new(deep::find_target(rng.next_u64(), settings.scenes.max_zoom_decades as f64, &settings.quality.params))),
            SceneKind::Julia => SceneParams::Julia(JuliaParams {
                c_radius: rng.range(0.775, 0.795),
                c_angle: rng.range(0.0, std::f32::consts::TAU),
                zoom_far: rng.range(1.4, 2.0),
                zoom_near: rng.range(0.06, 0.15),
                drift: rng.range(0.35, 0.6),
                stripe_density: rng.range(3.0, 7.0).round(),
            }),
            SceneKind::Fly => SceneParams::Fly(random_fly(&mut rng)),
        };
        Scene { kind, seed, palette, spin, params }
    }

    /// Build this frame's uniforms. `size` is the render target in pixels.
    pub fn uniforms(&self, clock: &SceneClock, settings: &Settings, size: (u32, u32)) -> SceneUniforms {
        let q = &settings.quality.params;
        let pal = self.palette.to_gpu();
        let rot = clock.rotation * self.spin;
        let mut u = SceneUniforms {
            resolution: [size.0 as f32, size.1 as f32],
            supersample: q.supersample,
            max_iter: q.base_iterations,
            pal_a: pal[0],
            pal_b: pal[1],
            pal_c: pal[2],
            pal_d: pal[3],
            color: [clock.color, settings.color.frequency, 5.0, clock.time],
            view: [rot.cos(), rot.sin(), 0.0, 0.0],
            view_i: [0, 0, (q.ao as i32) | ((q.shadows as i32) << 1), 0],
            ..Zeroable::zeroed()
        };
        match &self.params {
            SceneParams::Deep(target) => {
                let decades = clock.zoom.min(target.decades);
                let (mant, exp) = deep_pixel_size(decades, size.1);
                u.max_iter = q.iterations_at_depth(decades);
                u.view[2] = mant;
                u.view_i[0] = exp;
                u.view_i[1] = target.orbit.len() as i32;
            }
            SceneParams::Julia(j) => {
                let a = j.c_angle + clock.morph;
                // A slow second harmonic on the radius keeps it from looking like a plain loop.
                let r = j.c_radius + 0.012 * (clock.morph * 2.3).sin();
                let breathe = 0.5 - 0.5 * (clock.time * 0.07).cos();
                let height = j.zoom_far * (j.zoom_near / j.zoom_far).powf(breathe);
                u.max_iter = q.base_iterations.min(4_000);
                u.julia = [r * a.cos(), r * a.sin(), j.drift * (clock.morph * 0.7).cos(), j.drift * (clock.morph * 0.9).sin()];
                u.view[2] = height / size.1 as f32;
                u.color[2] = j.stripe_density;
            }
            SceneParams::Fly(f) => {
                let (pos, fwd, up) = fly_camera(f, clock.camera);
                let t = clock.morph;
                u.cam_pos = [pos[0], pos[1], pos[2], (f.fov_deg.to_radians() * 0.5).tan()];
                u.cam_fwd = [fwd[0], fwd[1], fwd[2], 0.0];
                u.cam_up = [up[0], up[1], up[2], 0.0];
                u.fractal = [f.scale + f.scale_wobble * (t * 0.9).sin(), f.min_r2, f.fixed_r2, f.tunnel_radius];
                u.fractal2 = [f.offset[0], f.offset[1], f.offset[2], f.fold_angle + 0.15 * (t * 0.6).sin()];
                u.path_x = f.path_x;
                u.path_y = f.path_y;
                u.view_i[3] = f.variant as i32;
                u.fly_i = [q.march_steps, q.fold_iterations, 0, 0];
            }
        }
        u
    }

    /// Current zoom depth in decades (deep zoom only).
    pub fn zoom_decades(&self, clock: &SceneClock) -> Option<f64> {
        match &self.params {
            SceneParams::Deep(t) => Some(clock.zoom.min(t.decades)),
            _ => None,
        }
    }

    /// The GPU reference orbit, if this is a deep zoom.
    pub fn orbit(&self) -> Option<&[[f32; 2]]> {
        match &self.params {
            SceneParams::Deep(t) => Some(&t.orbit),
            _ => None,
        }
    }
}

/// Pixel size at a zoom depth as (mantissa in [1, 2), base-2 exponent), so depths
/// far beyond f32 range are representable.
pub fn deep_pixel_size(decades: f64, height_px: u32) -> (f32, i32) {
    let log2px = deep::target::START_HEIGHT.log2() - decades * std::f64::consts::LOG2_10 - (height_px.max(1) as f64).log2();
    let exp = log2px.floor();
    ((log2px - exp).exp2() as f32, exp as i32)
}

/// Random flythrough parameters.
fn random_fly(rng: &mut Rng) -> FlyParams {
    let variant = if rng.bool() { FlyVariant::Mandelbox } else { FlyVariant::Kifs };
    let (scale, wobble) = match variant {
        FlyVariant::Mandelbox => (rng.range(-2.1, -1.6), rng.range(0.03, 0.08)),
        FlyVariant::Kifs => (rng.range(2.0, 2.6), rng.range(0.02, 0.06)),
    };
    FlyParams {
        variant,
        scale,
        scale_wobble: wobble,
        min_r2: rng.range(0.15, 0.35),
        fixed_r2: rng.range(0.9, 1.1),
        offset: [rng.range(0.8, 1.2), rng.range(0.8, 1.2), rng.range(0.6, 1.2)],
        fold_angle: rng.range(-0.4, 0.4),
        tunnel_radius: rng.range(0.35, 0.55),
        path_x: [rng.range(0.4, 0.9), rng.range(0.08, 0.16), rng.range(0.1, 0.3), rng.range(0.25, 0.4)],
        path_y: [rng.range(0.3, 0.7), rng.range(0.06, 0.13), rng.range(0.1, 0.25), rng.range(0.2, 0.35)],
        roll: rng.range(0.2, 0.6),
        fov_deg: rng.range(70.0, 90.0),
    }
}

/// Tunnel path center at depth `z` (mirrors `tunnel_path` in fly.wgsl).
pub fn tunnel_path(f: &FlyParams, z: f32) -> [f32; 2] {
    let (px, py) = (f.path_x, f.path_y);
    [px[0] * (px[1] * z).sin() + px[2] * (px[3] * z).sin(), py[0] * (py[1] * z + 1.3).sin() + py[2] * (py[3] * z).cos()]
}

/// Camera position, forward and up vectors at path distance `s`.
pub fn fly_camera(f: &FlyParams, s: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let at = |z: f32| {
        let p = tunnel_path(f, z);
        [p[0], p[1], z]
    };
    let pos = at(s);
    let ahead = at(s + 0.6);
    let fwd = normalize(sub(ahead, pos));
    let roll = f.roll * (s * 0.11).sin();
    let world_up = [roll.sin(), roll.cos(), 0.0];
    let right = normalize(cross(world_up, fwd));
    let up = cross(fwd, right);
    (pos, fwd, up)
}

/// a - b.
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Cross product.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Unit vector.
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let mut s = Settings::default();
        s.set_quality_preset(crate::quality::QualityPreset::Low);
        s.scenes.max_zoom_decades = 6.0;
        s
    }

    #[test]
    fn uniform_block_is_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<SceneUniforms>() % 16, 0);
        assert_eq!(std::mem::size_of::<SceneUniforms>(), 272);
    }

    #[test]
    fn same_seed_same_scene() {
        let s = settings();
        for kind in [SceneKind::Julia, SceneKind::Fly, SceneKind::DeepZoom] {
            let a = Scene::generate(kind, 99, &s);
            let b = Scene::generate(kind, 99, &s);
            assert_eq!(a.palette, b.palette);
            let (ua, ub) = (a.uniforms(&SceneClock::default(), &s, (640, 360)), b.uniforms(&SceneClock::default(), &s, (640, 360)));
            assert_eq!(bytemuck::bytes_of(&ua), bytemuck::bytes_of(&ub), "{kind:?}");
        }
    }

    #[test]
    fn julia_c_stays_near_the_connectedness_boundary() {
        let s = settings();
        for seed in 0..50 {
            let scene = Scene::generate(SceneKind::Julia, seed, &s);
            let mut clock = SceneClock::default();
            for _ in 0..200 {
                clock.advance(0.5, &s);
                let u = scene.uniforms(&clock, &s, (640, 360));
                let r = (u.julia[0].powi(2) + u.julia[1].powi(2)).sqrt();
                assert!((0.75..0.81).contains(&r), "|c| = {r}");
            }
        }
    }

    #[test]
    fn fly_params_in_range_and_camera_is_continuous() {
        let s = settings();
        for seed in 0..30 {
            let scene = Scene::generate(SceneKind::Fly, seed, &s);
            let SceneParams::Fly(f) = &scene.params else { panic!("not a fly scene") };
            match f.variant {
                FlyVariant::Mandelbox => assert!((-2.2..-1.5).contains(&f.scale)),
                FlyVariant::Kifs => assert!((1.9..2.7).contains(&f.scale)),
            }
            // The camera is always on the tunnel axis, inside the carved-out radius.
            let mut clock = SceneClock::default();
            let mut last = scene.uniforms(&clock, &s, (64, 64));
            for _ in 0..500 {
                clock.advance(1.0 / 60.0, &s);
                let u = scene.uniforms(&clock, &s, (64, 64));
                let step: f32 = (0..3).map(|i| (u.cam_pos[i] - last.cam_pos[i]).powi(2)).sum::<f32>().sqrt();
                assert!(step < 0.05, "camera jumped {step}");
                let turn: f32 = (0..3).map(|i| u.cam_fwd[i] * last.cam_fwd[i]).sum();
                assert!(turn > 0.995, "camera snapped direction");
                let axis = tunnel_path(f, u.cam_pos[2]);
                assert!((axis[0] - u.cam_pos[0]).abs() < 1e-5 && (axis[1] - u.cam_pos[1]).abs() < 1e-5);
                last = u;
            }
        }
    }

    #[test]
    fn deep_zoom_depth_is_capped_and_pixel_size_tracks_depth() {
        let s = settings();
        let scene = Scene::generate(SceneKind::DeepZoom, 3, &s);
        let mut clock = SceneClock { zoom: 100.0, ..Default::default() };
        assert_eq!(scene.zoom_decades(&clock), Some(6.0));
        clock.zoom = 2.0;
        let u = scene.uniforms(&clock, &s, (800, 600));
        let px = u.view[2] as f64 * (u.view_i[0] as f64).exp2();
        let expected = 3.0 * 1e-2 / 600.0;
        assert!((px / expected - 1.0).abs() < 1e-5, "pixel size {px} vs {expected}");
        assert_eq!(u.view_i[1] as usize, scene.orbit().unwrap().len());
    }

    #[test]
    fn pixel_size_survives_depths_beyond_f32() {
        let (mant, exp) = deep_pixel_size(200.0, 1080);
        assert!((1.0..2.0).contains(&mant));
        assert!(exp < -600);
    }

    #[test]
    fn clock_motion_is_continuous_when_speed_changes() {
        let mut s = settings();
        let mut c = SceneClock::default();
        c.advance(1.0, &s);
        let before = c;
        s.motion.speed = 5.0;
        c.advance(0.01, &s);
        assert!((c.rotation - before.rotation).abs() < 0.01);
        assert!((c.zoom - before.zoom).abs() < 0.05);
    }

    #[test]
    fn pick_kind_respects_enabled_and_avoids_repeats() {
        let mut s = settings();
        let mut rng = Rng::new(1);
        for _ in 0..100 {
            assert_ne!(pick_kind(&mut rng, &s, Some(SceneKind::Fly)), SceneKind::Fly);
        }
        s.scenes.deep_zoom = false;
        s.scenes.julia = false;
        assert_eq!(pick_kind(&mut rng, &s, Some(SceneKind::Fly)), SceneKind::Fly);
    }
}
