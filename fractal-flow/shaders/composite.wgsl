// Blends the two scene targets (outgoing and incoming) and applies color grading.
// Standalone shader (no common.wgsl prelude).

struct Composite {
    weights: vec4<f32>,   // x weight of slot 0, y weight of slot 1
    grade: vec4<f32>,     // x saturation, y brightness, z contrast
    flags: vec4<u32>,     // x: 1 = output is sRGB (pre-linearize)
}

@group(0) @binding(0) var tex0: texture_2d<f32>;
@group(0) @binding(1) var tex1: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> c: Composite;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var col = textureSample(tex0, samp, in.uv).rgb * c.weights.x + textureSample(tex1, samp, in.uv).rgb * c.weights.y;
    col = (col - 0.5) * c.grade.z + 0.5;
    let luma = dot(col, vec3<f32>(0.299, 0.587, 0.114));
    col = mix(vec3<f32>(luma), col, c.grade.x) * c.grade.y;
    col = clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
    if (c.flags.x == 1u) {
        col = pow(col, vec3<f32>(2.2));
    }
    return vec4<f32>(col, 1.0);
}
