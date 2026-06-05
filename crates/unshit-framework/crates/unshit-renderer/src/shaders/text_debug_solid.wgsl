struct Uniforms {
    viewport: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct GlyphInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_min: vec2<f32>,
    @location(3) uv_max: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) clip_rect: vec4<f32>,
    @location(6) xform: vec4<f32>,
    @location(7) xform_translate: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) clip_rect: vec4<f32>,
    @location(2) pixel_pos: vec2<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: GlyphInstance,
) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let corner = corners[vertex_index];
    let local_pixel_pos = instance.pos + corner * instance.size;
    let xform_m = mat2x2<f32>(
        instance.xform.x + 1.0, instance.xform.y,
        instance.xform.z, instance.xform.w + 1.0,
    );
    let pixel_pos = xform_m * local_pixel_pos + instance.xform_translate;
    let ndc = vec2(
        (pixel_pos.x / uniforms.viewport.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / uniforms.viewport.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4(ndc, 0.0, 1.0);
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.pixel_pos = pixel_pos;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let clip_min = in.clip_rect.xy;
    let clip_max = in.clip_rect.xy + in.clip_rect.zw;
    if in.pixel_pos.x < clip_min.x || in.pixel_pos.x > clip_max.x ||
       in.pixel_pos.y < clip_min.y || in.pixel_pos.y > clip_max.y {
        discard;
    }

    return in.color;
}
