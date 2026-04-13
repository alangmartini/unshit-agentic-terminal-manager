// SVG pipeline shader.
//
// Draws pre tessellated geometry as an indexed triangle list. Each vertex
// carries a position in local SVG coordinates plus a flat color from the
// fill or stroke pass. A single uniform block holds the viewport size; per
// draw state (translate, scale, clip, tint, opacity) comes via uniform
// buffer at bind group 1 because wgpu does not support push constants on
// the default feature set.

struct GlobalUniforms {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

struct InstanceUniforms {
    translate: vec2<f32>,
    scale: vec2<f32>,
    clip_rect: vec4<f32>,
    color_tint: vec4<f32>,
    opacity: f32,
    _pad: vec3<f32>,
};

@group(0) @binding(0) var<uniform> globals: GlobalUniforms;
@group(1) @binding(0) var<uniform> instance: InstanceUniforms;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) coverage: f32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) pixel_pos: vec2<f32>,
    @location(2) coverage: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let local = in.position * instance.scale + instance.translate;
    let ndc = vec2(
        (local.x / globals.viewport.x) * 2.0 - 1.0,
        1.0 - (local.y / globals.viewport.y) * 2.0,
    );
    var out: VertexOutput;
    out.position = vec4(ndc, 0.0, 1.0);
    out.color = in.color * instance.color_tint;
    out.pixel_pos = local;
    out.coverage = in.coverage;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Smooth clip rect edges (half-pixel fade instead of hard discard).
    let clip_min = instance.clip_rect.xy;
    let clip_max = instance.clip_rect.xy + instance.clip_rect.zw;
    let dx = min(in.pixel_pos.x - clip_min.x, clip_max.x - in.pixel_pos.x);
    let dy = min(in.pixel_pos.y - clip_min.y, clip_max.y - in.pixel_pos.y);
    let clip_dist = min(dx, dy);
    let clip_alpha = smoothstep(0.0, 0.5, clip_dist);
    if clip_alpha < 0.001 { discard; }

    // Analytical edge AA using interpolated signed distance (Skia-style).
    // Fill vertices have coverage=0 everywhere so fwidth=0 -> fully opaque.
    // Stroke vertices carry a -1..+1 gradient across the stroke width;
    // fwidth tells us how many coverage units span one pixel.
    let d = abs(in.coverage);
    let w = fwidth(in.coverage);
    var edge_alpha = 1.0;
    if w > 0.001 {
        let fade_start = max(1.0 - w * 0.5, 0.0);
        edge_alpha = 1.0 - smoothstep(fade_start, 1.0, d);
    }

    let rgb = in.color.rgb;
    let a = in.color.a * instance.opacity * edge_alpha * clip_alpha;
    // Premultiplied alpha output to match the rest of the pipelines.
    return vec4(rgb * a, a);
}
