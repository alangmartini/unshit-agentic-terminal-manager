struct Uniforms {
    viewport: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(0) var img_texture: texture_2d<f32>;
@group(1) @binding(1) var img_sampler: sampler;

struct ImageInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) border_radius: vec4<f32>,
    @location(3) opacity: f32,
    @location(4) _pad: vec3<f32>,
    @location(5) clip_rect: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local_pos: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) border_radius: vec4<f32>,
    @location(4) opacity: f32,
    @location(5) clip_rect: vec4<f32>,
    @location(6) pixel_pos: vec2<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: ImageInstance,
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
    out.uv = corner;
    out.local_pos = corner * instance.size;
    out.size = instance.size;
    out.border_radius = instance.border_radius;
    out.opacity = instance.opacity;
    out.clip_rect = instance.clip_rect;
    out.pixel_pos = pixel_pos;
    return out;
}

fn sdf_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2(radius);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn select_radius(local_pos: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half = size * 0.5;
    let is_right = step(half.x, local_pos.x);
    let is_bottom = step(half.y, local_pos.y);
    let top = mix(radii.x, radii.y, is_right);
    let bottom = mix(radii.w, radii.z, is_right);
    return mix(top, bottom, is_bottom);
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

    let half = in.size * 0.5;
    let p = in.local_pos - half;

    let r = select_radius(in.local_pos, in.size, in.border_radius);
    let safe_r = min(r, min(half.x, half.y));

    let d = sdf_rounded_rect(p, half, safe_r);
    let alpha = 1.0 - smoothstep(-0.5, 0.5, d);

    if alpha < 0.001 {
        discard;
    }

    let tex_color = textureSample(img_texture, img_sampler, in.uv);
    return vec4(tex_color.rgb, tex_color.a * alpha * in.opacity);
}
