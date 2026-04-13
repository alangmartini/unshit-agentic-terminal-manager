// Two pass separable Gaussian blur.
//
// One fragment shader runs twice per boundary: once horizontally and once
// vertically. Each invocation reads the source texture, samples
// `(2 * radius + 1)` taps along the direction axis weighted by a precomputed
// Gaussian kernel, and writes the blurred value to the destination.
//
// Total cost per pixel is `2 * (2 * radius + 1)` taps for the whole blur,
// which for the design target of radius 6 is 26 taps.

struct BlurUniforms {
    // `(1, 0)` for horizontal pass, `(0, 1)` for vertical pass. Scaled by the
    // pixel size in fragment shader so the same shader works for any texture
    // dimensions.
    direction: vec2<f32>,
    // Kernel radius in pixels. Clamped to 64 at parse time so the weights
    // array stays bounded.
    radius: f32,
    _pad0: f32,
    // Inverse texture size in normalized coordinates. Lets us convert
    // `direction` into a per tap offset.
    texel_size: vec2<f32>,
    _pad1: vec2<f32>,
    // Precomputed Gaussian weights. `weights[0]` is the center weight, then
    // `weights[i]` is the weight at offset `i` pixels out (symmetric on both
    // sides). Only the first `radius + 1` entries are read.
    // 17 vec4 slots cover up to radius 67, which is the first multiple of 4
    // at or past the clamp.
    weights: array<vec4<f32>, 17>,
};

@group(0) @binding(0) var<uniform> u: BlurUniforms;
@group(0) @binding(1) var src_texture: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    // Full screen triangle strip covering the [-1, 1] NDC rect.
    var corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
        vec2(-1.0, 1.0),
    );
    var uvs = array<vec2<f32>, 6>(
        vec2(0.0, 1.0),
        vec2(1.0, 1.0),
        vec2(0.0, 0.0),
        vec2(1.0, 1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 0.0),
    );
    var out: VsOut;
    out.position = vec4(corners[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}

fn fetch_weight(i: i32) -> f32 {
    let block = i / 4;
    let lane = i % 4;
    let v = u.weights[block];
    var w: f32 = 0.0;
    if (lane == 0) {
        w = v.x;
    } else if (lane == 1) {
        w = v.y;
    } else if (lane == 2) {
        w = v.z;
    } else {
        w = v.w;
    }
    return w;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let radius_i = i32(u.radius);
    let step = u.direction * u.texel_size;

    // Start with the center tap at weight 0.
    var accum: vec4<f32> = textureSample(src_texture, src_sampler, in.uv) * fetch_weight(0);

    // Symmetric side taps.
    for (var i: i32 = 1; i <= radius_i; i = i + 1) {
        let offset = step * f32(i);
        let w = fetch_weight(i);
        accum = accum + textureSample(src_texture, src_sampler, in.uv + offset) * w;
        accum = accum + textureSample(src_texture, src_sampler, in.uv - offset) * w;
    }

    return accum;
}
