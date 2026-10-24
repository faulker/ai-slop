// Morphing Julia set swirls. Plain f32 is enough here: the zoom only breathes
// between a few view heights, while `c` travels near the connectedness boundary
// so the spirals constantly unwind and re-form.

const J_BAILOUT2: f32 = 1.0e6;

fn julia_sample(pix: vec2<f32>) -> vec3<f32> {
    let off = rotate_view((pix - 0.5 * u.resolution) * vec2<f32>(1.0, -1.0));
    var z = u.julia.zw + off * u.view.z;
    let c = u.julia.xy;
    var der = vec2<f32>(1.0, 0.0);
    var escaped = false;
    var iter = 0u;
    var r2 = 0.0;
    var stripe_sum = 0.0;
    var stripe_last = 0.0;
    var stripe_n = 0.0;
    var trap = 1.0e9;
    let density = u.color.z;

    for (var i = 0u; i < u.max_iter; i++) {
        r2 = dot(z, z);
        if (r2 > J_BAILOUT2) {
            escaped = true;
            iter = i;
            break;
        }
        stripe_last = 0.5 + 0.5 * sin(density * atan2(z.y, z.x));
        stripe_sum += stripe_last;
        stripe_n += 1.0;
        trap = min(trap, abs(r2 - 0.25));
        der = 2.0 * cmul(z, der);
        z = cmul(z, z) + c;
    }

    if (!escaped) {
        return color_2d(0.0, 0.0, 0.0, trap, false);
    }
    let log_zn = 0.5 * log(r2);
    let smooth_n = f32(iter) + 1.0 - log2(log_zn / log(2.0));
    let frac = fract(smooth_n);
    let avg = stripe_sum / max(stripe_n, 1.0);
    let prev = (stripe_sum - stripe_last) / max(stripe_n - 1.0, 1.0);
    let stripe = mix(prev, avg, frac);
    let de = sqrt(r2) * log_zn / max(length(der), 1.0e-30);
    return color_2d(smooth_n, stripe, de / u.view.z, trap, true);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = max(u.supersample, 1u);
    var acc = vec3<f32>(0.0);
    for (var sy = 0u; sy < n; sy++) {
        for (var sx = 0u; sx < n; sx++) {
            acc += julia_sample(in.pos.xy + ss_offset(sx, sy, n));
        }
    }
    return vec4<f32>(acc / f32(n * n), 1.0);
}
