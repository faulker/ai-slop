// Mandelbrot deep zoom by perturbation (GPU twin of deep::perturb_escape).
//
// Each pixel tracks dz = z - Z, its difference from a high-precision reference
// orbit Z (storage buffer `orbit`). Tiny quantities are stored as a mantissa
// vec2 plus a shared base-2 exponent ("floatexp") so zooms far below f32's
// 1e-38 floor still work. Rebasing (Zhuoran) avoids perturbation glitches.

const BAILOUT2: f32 = 1.0e6;
// dz leaves extended range once its exponent reaches this, and re-enters below PLAIN_LO.
const PLAIN_EXP: i32 = -60;
const PLAIN_LO: f32 = 1.0e-24;
// The derivative mantissa is renormalized only outside [DER_LO, DER_HI]. One
// iteration multiplies it by at most 2·|z| ≤ 2e3, so it can't overflow before that.
const DER_HI: f32 = 1.0e15;
const DER_LO: f32 = 1.0e-15;

// ldexp on both components, with the exponent kept in the range where ldexp is
// well defined (large negative exponents simply underflow to zero).
fn ldx(v: vec2<f32>, e: i32) -> vec2<f32> {
    let k = clamp(e, -150, 126);
    return vec2<f32>(ldexp(v.x, k), ldexp(v.y, k));
}

// Exponent that brings max(|v.x|, |v.y|) into [0.5, 1).
fn norm_exp(v: vec2<f32>) -> i32 {
    let m = max(abs(v.x), abs(v.y));
    if (m == 0.0) {
        return 0;
    }
    return frexp(m).exp;
}

fn deep_sample(pix: vec2<f32>) -> vec3<f32> {
    let off = rotate_view((pix - 0.5 * u.resolution) * vec2<f32>(1.0, -1.0));
    // dc = off * pixel_size, as mantissa dcm with exponent dce.
    var dcm = off * u.view.z;
    var dce = u.view_i.x;
    let e0 = norm_exp(dcm);
    dcm = ldx(dcm, -e0);
    dce += e0;

    let len = u.view_i.y;
    // z1 = C + dc, so dz starts as dc at orbit index 1.
    // dz = w·2^we. While dz is a normal-range f32 we keep we = 0 and skip the
    // floatexp bookkeeping; it only kicks in when dz is tiny (start of a deep
    // zoom, or shrinking toward underflow). Power-of-two scaling is exact, so
    // both paths give the same result.
    var w = dcm;
    var we = dce;
    // dc in units of 2^we, refreshed whenever we changes.
    var dcw = ldx(dcm, dce - we);
    var m = 1;
    // dz/dc as floatexp (der = du * 2^de), z1' = 1. Renormalized only when it drifts far from 1.
    var du = vec2<f32>(1.0, 0.0);
    var de = 0;
    var one_d = 1.0;

    var escaped = false;
    var iter = 0u;
    var z = vec2<f32>(0.0);
    var r2 = 0.0;
    var stripe_sum = 0.0;
    var stripe_last = 0.0;
    var stripe_n = 0.0;
    var trap = 1.0e9;
    let density = u.color.z;

    for (var i = 1u; i < u.max_iter; i++) {
        let zr = orbit[m];
        var dzf = w;
        if (we != 0) {
            dzf = ldx(w, we);
        }
        z = zr + dzf;
        r2 = dot(z, z);
        if (r2 > BAILOUT2) {
            escaped = true;
            iter = i;
            break;
        }
        stripe_last = 0.5 + 0.5 * sin(density * atan2(z.y, z.x));
        stripe_sum += stripe_last;
        stripe_n += 1.0;
        trap = min(trap, abs(r2 - 0.25));

        // der' = 2·z·der + 1
        du = 2.0 * cmul(z, du) + vec2<f32>(one_d, 0.0);
        let dmag = max(abs(du.x), abs(du.y));
        if (dmag > DER_HI || dmag < DER_LO) {
            let ne = norm_exp(du);
            du = ldx(du, -ne);
            de += ne;
            one_d = ldexp(1.0, clamp(-de, -150, 126));
        }

        if (r2 < dot(dzf, dzf) || m + 1 >= len) {
            w = z;
            we = 0;
            m = 0;
            dcw = ldx(dcm, dce);
        }
        let zm = orbit[m];
        // dz' = 2·Z·dz + dz² + dc, all in units of 2^we.
        var sq = cmul(w, w);
        if (we != 0) {
            sq = ldx(sq, we);
        }
        w = 2.0 * cmul(zm, w) + sq + dcw;
        m += 1;
        let wmag = max(abs(w.x), abs(w.y));
        if (we != 0) {
            // Extended range: renormalize, and fold back to plain f32 once dz is big enough.
            let k = norm_exp(w);
            w = ldx(w, -k);
            we += k;
            if (we >= PLAIN_EXP) {
                w = ldx(w, we);
                we = 0;
            }
            dcw = ldx(dcm, dce - we);
        } else if (wmag < PLAIN_LO && wmag != 0.0) {
            // dz shrinking toward underflow: switch to extended range.
            let k = norm_exp(w);
            w = ldx(w, -k);
            we = k;
            dcw = ldx(dcm, dce - we);
        }
    }

    if (!escaped) {
        return color_2d(0.0, 0.0, 0.0, trap, false);
    }
    let log_zn = 0.5 * log(r2);
    let smooth_n = f32(iter) + 1.0 - log2(log_zn / log(2.0));
    // Stripe average, interpolated between the last two iterations by the smooth fraction.
    let frac = fract(smooth_n);
    let avg = stripe_sum / max(stripe_n, 1.0);
    let prev = (stripe_sum - stripe_last) / max(stripe_n - 1.0, 1.0);
    let stripe = mix(prev, avg, frac);
    // Distance estimate |z|·ln|z| / |dz/dc|, converted to pixels.
    let zmag = sqrt(r2);
    let de_mant = zmag * log_zn / max(length(du) * u.view.z, 1.0e-30);
    let de_px = ldexp(de_mant, clamp(-de - u.view_i.x, -150, 126));
    return color_2d(smooth_n, stripe, de_px, trap, true);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = max(u.supersample, 1u);
    var acc = vec3<f32>(0.0);
    for (var sy = 0u; sy < n; sy++) {
        for (var sx = 0u; sx < n; sx++) {
            acc += deep_sample(in.pos.xy + ss_offset(sx, sy, n));
        }
    }
    return vec4<f32>(acc / f32(n * n), 1.0);
}
