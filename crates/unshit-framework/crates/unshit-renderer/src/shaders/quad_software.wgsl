// Software/CPU-renderer quad shader (AdapterTier::Software).
//
// A deliberately minimal twin of `quad.wgsl` used only when the renderer runs
// on a software adapter such as WARP, whose vertex->fragment varying budget
// is 60 components. The full `quad.wgsl` packs 96 components of varyings (the
// 8 gradient stop colors alone are 32) which WARP rejects at pipeline
// creation, so the full feature set physically cannot run there. This lite
// variant keeps the features a software fallback needs to be legible and
// correctly laid out -- solid color, per-side borders, rounded corners, the
// ancestor clip rect, and the CSS transform -- and drops the three varying
// hogs: gradients, box-shadows, and `mask-image`. The terminal text grid and
// all panel backgrounds/borders render identically; only gradient/shadow/mask
// flourishes fall back to flat colors. The GPU-accelerated path never loads
// this module: it keeps the full `quad.wgsl`, byte-identical.

struct Uniforms {
    viewport: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// Reads the SAME `QuadInstance` vertex buffer the full shader consumes (the
// instance buffer is unchanged), but declares only the attributes this lite
// variant touches. Unread gradient/shadow/mask slots are omitted so the
// pipeline's vertex attribute list stays short; the buffer stride is still
// `sizeof(QuadInstance)` (see `QuadPipeline::new_software`).
struct QuadInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) border_width: vec4<f32>,
    @location(5) border_radius: vec4<f32>,
    @location(6) clip_rect: vec4<f32>,
    @location(26) xform: vec4<f32>,
    @location(27) xform_translate: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) border_width: vec4<f32>,
    @location(5) border_radius: vec4<f32>,
    @location(6) clip_rect: vec4<f32>,
    @location(7) pixel_pos: vec2<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: QuadInstance,
) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let corner = corners[vertex_index];

    // Apply the CSS `transform` affine to the screen-space position. See
    // quad.wgsl for the delta-from-identity storage rationale; local
    // (in-quad) coordinates stay untransformed so the border-radius SDF
    // rotates/scales with the quad while the clip test sees the final
    // on-screen position.
    let xform_m = mat2x2<f32>(
        instance.xform.x + 1.0, instance.xform.y,
        instance.xform.z, instance.xform.w + 1.0,
    );
    let pixel_pos = xform_m * (instance.pos + corner * instance.size) + instance.xform_translate;
    let ndc = vec2(
        (pixel_pos.x / uniforms.viewport.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / uniforms.viewport.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4(ndc, 0.0, 1.0);
    out.local_pos = corner * instance.size;
    out.size = instance.size;
    out.color = instance.color;
    out.border_color = instance.border_color;
    out.border_width = instance.border_width;
    out.border_radius = instance.border_radius;
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
    // Clip rect discard: clip_rect = [x, y, width, height]
    let clip_min = in.clip_rect.xy;
    let clip_max = in.clip_rect.xy + in.clip_rect.zw;
    if in.pixel_pos.x < clip_min.x || in.pixel_pos.x > clip_max.x ||
       in.pixel_pos.y < clip_min.y || in.pixel_pos.y > clip_max.y {
        discard;
    }

    // Position relative to the rect center.
    let half = in.size * 0.5;
    let p = in.local_pos - half;
    let r = select_radius(in.local_pos, in.size, in.border_radius);
    let safe_r = min(r, min(half.x, half.y));

    let d = sdf_rounded_rect(p, half, safe_r);
    let outer_alpha = 1.0 - smoothstep(-0.5, 0.5, d);

    if outer_alpha < 0.001 {
        discard;
    }

    // No gradient/shadow/mask on the software path: the base color is the
    // solid instance color.
    var base_color = in.color;

    // Border -- same uniform vs mismatched-sides logic as quad.wgsl, so a
    // `border-left-width: 1px`-style stripe still paints. Rounded corners are
    // honored for uniform borders and ignored for mismatched ones (matches
    // the full shader and CSS practice).
    let bw = in.border_width;
    let max_border = max(max(bw.x, bw.y), max(bw.z, bw.w));
    let min_border = min(min(bw.x, bw.y), min(bw.z, bw.w));
    let uniform_border = (max_border - min_border) < 0.001;
    var rect_color: vec4<f32>;

    if max_border > 0.0 {
        var border_factor: f32;
        if uniform_border {
            let avg_border = (bw.x + bw.y + bw.z + bw.w) * 0.25;
            let inner_half = half - vec2(avg_border);
            let inner_r = max(safe_r - avg_border, 0.0);
            let inner_d = sdf_rounded_rect(p, inner_half, inner_r);
            border_factor = smoothstep(-0.5, 0.5, inner_d);
        } else {
            let d_top = in.local_pos.y;
            let d_left = in.local_pos.x;
            let d_right = in.size.x - in.local_pos.x;
            let d_bottom = in.size.y - in.local_pos.y;
            let f_top = select(smoothstep(bw.x + 0.5, bw.x - 0.5, d_top), 0.0, bw.x <= 0.0);
            let f_right = select(smoothstep(bw.y + 0.5, bw.y - 0.5, d_right), 0.0, bw.y <= 0.0);
            let f_bottom = select(smoothstep(bw.z + 0.5, bw.z - 0.5, d_bottom), 0.0, bw.z <= 0.0);
            let f_left = select(smoothstep(bw.w + 0.5, bw.w - 0.5, d_left), 0.0, bw.w <= 0.0);
            border_factor = clamp(max(max(f_top, f_right), max(f_bottom, f_left)), 0.0, 1.0);
        }
        let ba = in.border_color.a * border_factor;
        let one_minus_ba = 1.0 - ba;
        let result_a = ba + base_color.a * one_minus_ba;
        let result_rgb = select(
            vec3(0.0),
            (in.border_color.rgb * ba + base_color.rgb * base_color.a * one_minus_ba) / result_a,
            result_a > 0.001
        );
        rect_color = vec4(result_rgb, result_a);
    } else {
        rect_color = base_color;
    }
    rect_color = vec4(rect_color.rgb, rect_color.a * outer_alpha);

    // Premultiplied output (no shadow to composite behind).
    return vec4(rect_color.rgb * rect_color.a, rect_color.a);
}
