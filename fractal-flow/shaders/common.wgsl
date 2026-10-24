// Shared prelude for every scene shader. It is prepended to each scene file at
// pipeline creation (see gpu.rs), so scene files can use everything here.

// Mirrors `SceneUniforms` in src/scene.rs field for field.
struct Scene {
    resolution: vec2<f32>,
    supersample: u32,
    max_iter: u32,
    pal_a: vec4<f32>,
    pal_b: vec4<f32>,
    pal_c: vec4<f32>,
    pal_d: vec4<f32>,
    color: vec4<f32>,     // x palette offset, y frequency, z stripe density, w time
    view: vec4<f32>,      // x cos(rot), y sin(rot), z pixel size (deep: mantissa)
    view_i: vec4<i32>,    // x pixel exponent, y orbit length, z flags, w variant
    julia: vec4<f32>,     // xy c, zw center
    cam_pos: vec4<f32>,   // xyz, w tan(fov/2)
    cam_fwd: vec4<f32>,
    cam_up: vec4<f32>,
    fractal: vec4<f32>,   // scale, min_r2, fixed_r2, tunnel radius
    fractal2: vec4<f32>,  // offset xyz, fold angle
    path_x: vec4<f32>,
    path_y: vec4<f32>,
    fly_i: vec4<u32>,     // march steps, fold iterations
}

@group(0) @binding(0) var<uniform> u: Scene;
@group(0) @binding(1) var<storage, read> orbit: array<vec2<f32>>;

const TAU: f32 = 6.28318530718;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
}

// Fullscreen triangle; fragment shaders work from @builtin(position) pixel coords.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// Cosine palette (mirrors Palette::eval).
fn palette(t: f32) -> vec3<f32> {
    return u.pal_a.xyz + u.pal_b.xyz * cos(TAU * (u.pal_c.xyz * t + u.pal_d.xyz));
}

fn cmul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

// Rotate a screen-space offset by the view rotation.
fn rotate_view(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(v.x * u.view.x - v.y * u.view.y, v.x * u.view.y + v.y * u.view.x);
}

// Sub-pixel offset for supersample (sx, sy) of an n×n grid, slightly rotated so
// horizontal/vertical filaments don't alias along the grid.
fn ss_offset(sx: u32, sy: u32, n: u32) -> vec2<f32> {
    let g = (vec2<f32>(f32(sx), f32(sy)) + 0.5) / f32(n) - 0.5;
    return vec2<f32>(g.x * 0.966 - g.y * 0.259, g.x * 0.259 + g.y * 0.966);
}

// Shared 2D escape-time coloring: smooth iteration, stripe-average texture and a
// distance-estimate edge term combine into fine, structure-following color.
//   smooth: continuous iteration count, stripe: 0..1 stripe average,
//   de_px: distance to the set in pixels, escaped: false for interior points.
fn color_2d(smooth_n: f32, stripe: f32, de_px: f32, trap: f32, escaped: bool) -> vec3<f32> {
    let freq = u.color.y;
    if (!escaped) {
        // Dim but textured interior (orbit trap bands) instead of flat black.
        return palette(u.color.x + 0.5 + log(trap + 1.0e-4) * 0.12 * freq) * 0.16;
    }
    let t = log2(max(smooth_n, 1.0)) * 0.85 * freq + sqrt(smooth_n) * 0.05 * freq + stripe * 0.6 + u.color.x;
    var col = palette(t);
    // Soft seams right at the boundary, bright glow just outside it.
    let edge = clamp(de_px * 0.5, 0.0, 1.0);
    col *= 0.5 + 0.5 * pow(edge, 0.5);
    col += palette(t + 0.33) * 0.3 * exp(-de_px * 0.3) * edge;
    // Stripe shading adds the feathery texture.
    col *= 0.55 + 0.9 * stripe;
    // Fade the far exterior (few iterations) so the eye goes to the structure.
    col *= 0.35 + 0.65 * smoothstep(1.0, 8.0, smooth_n);
    return col;
}
