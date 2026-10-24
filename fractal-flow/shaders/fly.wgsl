// Raymarched 3D fractal flythrough. Space is tiled so the structure never ends,
// and a tunnel is carved along the camera path so the camera never collides.
// Detail comes from high fold counts and a hit threshold that scales with each
// pixel's footprint: sharp up close, shimmer-free far away.

const TMAX: f32 = 30.0;
// Mandelbox world scale: one tile (4 units) spans a whole box (≈ ±3.5 in fractal units).
const MB_K: f32 = 1.8;

fn tunnel_path(z: f32) -> vec2<f32> {
    let px = u.path_x;
    let py = u.path_y;
    return vec2<f32>(px.x * sin(px.y * z) + px.z * sin(px.w * z), py.x * sin(py.y * z + 1.3) + py.z * cos(py.w * z));
}

fn rot2(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

// Mandelbox: box fold, sphere fold, scale. Returns (distance, orbit trap).
fn de_mandelbox(p0: vec3<f32>) -> vec2<f32> {
    let tile = 4.0;
    let p = (p0 - tile * round(p0 / tile)) * MB_K;
    var z = p;
    var dr = 1.0;
    var trap = 1.0e9;
    let scale = u.fractal.x;
    let min_r2 = u.fractal.y;
    let fixed_r2 = u.fractal.z;
    for (var i = 0u; i < u.fly_i.y; i++) {
        z = clamp(z, vec3<f32>(-1.0), vec3<f32>(1.0)) * 2.0 - z;
        let r2 = dot(z, z);
        if (r2 < min_r2) {
            let k = fixed_r2 / min_r2;
            z *= k;
            dr *= k;
        } else if (r2 < fixed_r2) {
            let k = fixed_r2 / r2;
            z *= k;
            dr *= k;
        }
        z = z * scale + p;
        dr = dr * abs(scale) + 1.0;
        trap = min(trap, r2);
    }
    return vec2<f32>(length(z) / abs(dr) / MB_K, trap);
}

// Kaleidoscopic IFS (rotated Menger-style folds). Returns (distance, orbit trap).
fn de_kifs(p0: vec3<f32>) -> vec2<f32> {
    let tile = 2.0;
    var p = p0 - tile * round(p0 / tile);
    let scale = u.fractal.x;
    let off = u.fractal2.xyz;
    let ang = u.fractal2.w;
    var s = 1.0;
    var trap = 1.0e9;
    for (var i = 0u; i < u.fly_i.y; i++) {
        p = abs(p);
        if (p.x < p.y) { p = p.yxz; }
        if (p.x < p.z) { p = p.zyx; }
        if (p.y < p.z) { p = p.xzy; }
        let xy = rot2(p.xy, ang);
        p = vec3<f32>(xy, p.z);
        p = p * scale - off * (scale - 1.0);
        if (p.z < -0.5 * off.z * (scale - 1.0)) {
            p.z += off.z * (scale - 1.0);
        }
        s *= scale;
        trap = min(trap, dot(p, p) / (s * s) * 40.0);
    }
    let d = (max(max(abs(p.x), abs(p.y)), abs(p.z)) - 1.0) / s;
    return vec2<f32>(d, trap);
}

fn map(p: vec3<f32>) -> vec2<f32> {
    var r: vec2<f32>;
    if (u.view_i.w == 0) {
        r = de_mandelbox(p);
    } else {
        r = de_kifs(p);
    }
    let tunnel = u.fractal.w - length(p.xy - tunnel_path(p.z));
    r.x = max(r.x, tunnel);
    return r;
}

fn calc_normal(p: vec3<f32>, e: f32) -> vec3<f32> {
    let k = vec2<f32>(1.0, -1.0);
    return normalize(
        k.xyy * map(p + k.xyy * e).x +
        k.yyx * map(p + k.yyx * e).x +
        k.yxy * map(p + k.yxy * e).x +
        k.xxx * map(p + k.xxx * e).x
    );
}

fn calc_ao(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var occ = 0.0;
    var sca = 1.0;
    for (var i = 0; i < 5; i++) {
        let h = 0.01 + 0.04 * f32(i);
        occ += (h - map(p + n * h).x) * sca;
        sca *= 0.7;
    }
    return clamp(1.0 - 2.5 * occ, 0.0, 1.0);
}

fn soft_shadow(ro: vec3<f32>, rd: vec3<f32>) -> f32 {
    var res = 1.0;
    var t = 0.02;
    for (var i = 0; i < 48; i++) {
        let h = map(ro + rd * t).x;
        res = min(res, 10.0 * h / t);
        t += clamp(h, 0.01, 0.2);
        if (res < 0.005 || t > 3.0) {
            break;
        }
    }
    return clamp(res, 0.0, 1.0);
}

fn fly_sample(pix: vec2<f32>) -> vec3<f32> {
    let ndc = (pix - 0.5 * u.resolution) / (0.5 * u.resolution.y) * vec2<f32>(1.0, -1.0);
    let fwd = u.cam_fwd.xyz;
    let up = u.cam_up.xyz;
    let right = cross(up, fwd);
    let tan_half = u.cam_pos.w;
    let rd = normalize(fwd + (ndc.x * right + ndc.y * up) * tan_half);
    let ro = u.cam_pos.xyz;
    let pix_angle = 2.0 * tan_half / u.resolution.y;
    let offset = u.color.x;
    let freq = u.color.y;

    var t = 0.02;
    var hit = false;
    var trap = 0.0;
    var glow = 0.0;
    var eps = 1.0e-4;
    for (var i = 0u; i < u.fly_i.x; i++) {
        let p = ro + rd * t;
        let r = map(p);
        eps = max(t * pix_angle * 0.6, 2.0e-5);
        if (r.x < eps) {
            hit = true;
            trap = r.y;
            break;
        }
        glow += exp(-r.x * 45.0) * 0.012;
        t += r.x * 0.85;
        if (t > TMAX) {
            break;
        }
    }

    let fog_col = palette(offset + 0.75) * 0.06;
    let glow_col = palette(offset + 0.55);
    var col = fog_col;
    if (hit) {
        let p = ro + rd * t;
        let n = calc_normal(p, eps);
        let l = normalize(-rd + up * 0.5 + right * 0.2);
        let tc = log(max(trap, 1.0e-6)) * 0.18 * freq + offset + p.z * 0.015;
        let base = palette(tc);
        let dif = clamp(dot(n, l), 0.0, 1.0);
        let spec = pow(clamp(dot(reflect(rd, n), l), 0.0, 1.0), 24.0);
        var ao = 1.0;
        if ((u.view_i.z & 1) != 0) {
            ao = calc_ao(p, n);
        }
        var sha = 1.0;
        if ((u.view_i.z & 2) != 0) {
            sha = soft_shadow(p + n * eps * 4.0, l);
        }
        col = base * (0.12 + 0.88 * dif * sha) * ao + vec3<f32>(spec * 0.45 * ao * sha);
        // Emissive "windows" picked out of the orbit trap bands.
        let win = smoothstep(0.96, 0.99, fract(tc * 9.0)) * smoothstep(0.3, 0.7, ao);
        col += palette(offset + 0.15) * win * 0.9;
        col = mix(col, fog_col, 1.0 - exp(-t * 0.07));
    }
    col += glow_col * glow;
    return col;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = max(u.supersample, 1u);
    var acc = vec3<f32>(0.0);
    for (var sy = 0u; sy < n; sy++) {
        for (var sx = 0u; sx < n; sx++) {
            acc += fly_sample(in.pos.xy + ss_offset(sx, sy, n));
        }
    }
    return vec4<f32>(acc / f32(n * n), 1.0);
}
