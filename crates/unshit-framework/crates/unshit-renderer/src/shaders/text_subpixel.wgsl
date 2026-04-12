struct Uniforms {
    viewport: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(0) var atlas_texture: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct GlyphInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_min: vec2<f32>,
    @location(3) uv_max: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) clip_rect: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip_rect: vec4<f32>,
    @location(3) pixel_pos: vec2<f32>,
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
    let pixel_pos = instance.pos + corner * instance.size;
    let ndc = vec2(
        (pixel_pos.x / uniforms.viewport.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / uniforms.viewport.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4(ndc, 0.0, 1.0);
    out.uv = mix(instance.uv_min, instance.uv_max, corner);
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.pixel_pos = pixel_pos;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Clip rect discard
    let clip_min = in.clip_rect.xy;
    let clip_max = in.clip_rect.xy + in.clip_rect.zw;
    if in.pixel_pos.x < clip_min.x || in.pixel_pos.x > clip_max.x ||
       in.pixel_pos.y < clip_min.y || in.pixel_pos.y > clip_max.y {
        discard;
    }

    let tex = textureSample(atlas_texture, atlas_sampler, in.uv);

    // Gamma correction: boost coverage to match Windows Terminal weight.
    // DirectWrite rasterizes for sRGB compositing (gamma ~1.8). Applying
    // inverse gamma thickens stems so light-on-dark text looks solid.
    let gamma = 1.0 / 1.8;
    let cr = pow(tex.r, gamma);
    let cg = pow(tex.g, gamma);
    let cb = pow(tex.b, gamma);

    // Per-channel subpixel blending (ClearType-style).
    let r = in.color.r * cr;
    let g = in.color.g * cg;
    let b = in.color.b * cb;
    let a = in.color.a * max(cr, max(cg, cb));
    return vec4(r, g, b, a);
}
