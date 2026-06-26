// Software/CPU-renderer quad shader (AdapterTier::Software).
//
// A deliberately minimal twin of `quad.wgsl` used only when the renderer runs
// on a software adapter such as WARP, whose vertex->fragment varying budget
// is 60 components. The full `quad.wgsl` packs 96 components of varyings (the
// 8 gradient stop colors alone are 32) which WARP rejects at pipeline
// creation, so the full feature set physically cannot run there. This variant
// keeps the features a software fallback needs to look close to the GPU path
// -- solid color, per-side borders, rounded corners, the ancestor clip rect,
// the CSS transform, and box-shadows (outer + inset) -- and drops only the
// three biggest varying hogs: gradients, `mask-image`. (The full set is ~36
// varying components here vs WARP's 60 budget, so shadows fit; gradients and
// masks do not.) The terminal text grid and all panel backgrounds, borders,
// and drop shadows render identically to the GPU path; only gradient and
// mask flourishes fall back. The GPU-accelerated path never loads this module:
// it keeps the full `quad.wgsl`, byte-identical.

struct Uniforms {
    viewport: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// Reads the SAME `QuadInstance` vertex buffer the full shader consumes (the
// instance buffer is unchanged), but declares only the attributes this variant
// touches. Unread gradient/mask slots are omitted so the pipeline's vertex
// attribute list stays short; the buffer stride is still `sizeof(QuadInstance)`
// (see `QuadPipeline::new_software`).
struct QuadInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) border_width: vec4<f32>,
    @location(5) border_radius: vec4<f32>,
    @location(6) clip_rect: vec4<f32>,
    @location(7) shadow_color: vec4<f32>,
    @location(8) shadow_offset: vec2<f32>,
    @location(9) shadow_params: vec2<f32>,
    @location(10) shadow_spread: vec2<f32>,
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
    @location(7) shadow_color: vec4<f32>,
    @location(8) shadow_offset: vec2<f32>,
    @location(9) shadow_params: vec2<f32>,
    @location(10) shadow_spread: vec2<f32>,
    @location(11) pixel_pos: vec2<f32>,
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

    // Expand the quad to accommodate shadow blur and outer spread. Inset
    // shadows paint inside the padding box so they do not contribute to the
    // expand, but outer shadows grow outward by both blur and spread.
    let blur = instance.shadow_params.x;
    let inset_flag = instance.shadow_params.y;
    let spread = instance.shadow_spread.x;
    let spread_expand = select(max(spread, 0.0), 0.0, inset_flag > 0.5);
    let expand = blur * 3.0 + spread_expand;
    let expanded_pos = instance.pos - vec2(expand);
    let expanded_size = instance.size + vec2(expand * 2.0);

    let local_pixel_pos = expanded_pos + corner * expanded_size;
    // Apply the CSS `transform` affine to the screen-space position. See
    // quad.wgsl for the delta-from-identity storage rationale; local
    // (in-quad) coordinates stay untransformed so the border-radius SDF and
    // shadow sample box rotate/scale with the quad while the clip test sees
    // the final on-screen position.
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
    out.local_pos = corner * expanded_size;
    out.size = instance.size;
    out.color = instance.color;
    out.border_color = instance.border_color;
    out.border_width = instance.border_width;
    out.border_radius = instance.border_radius;
    out.clip_rect = instance.clip_rect;
    out.shadow_color = instance.shadow_color;
    out.shadow_offset = instance.shadow_offset;
    out.shadow_params = instance.shadow_params;
    out.shadow_spread = instance.shadow_spread;
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

    let blur = in.shadow_params.x;
    let inset = in.shadow_params.y > 0.5;
    let spread = in.shadow_spread.x;
    let spread_expand = select(max(spread, 0.0), 0.0, inset);
    let expand = blur * 3.0 + spread_expand;

    // Position relative to the original (unexpanded) rect.
    let half = in.size * 0.5;
    let rect_origin = vec2(expand);
    let rect_local = in.local_pos - rect_origin;
    let p = rect_local - half;

    let r = select_radius(rect_local, in.size, in.border_radius);
    let safe_r = min(r, min(half.x, half.y));

    // Inset shadow path: sample the rounded rect SDF relative to the shadow
    // offset and use the inside of the rect as the "lit" side. Fragments
    // outside the padding box are discarded so the inset shadow never paints
    // beyond the element. (Mirrors quad.wgsl exactly.)
    if inset {
        let d_outer = sdf_rounded_rect(p, half, safe_r);
        if d_outer > 0.5 {
            discard;
        }
        let inset_half = max(half - vec2(max(spread, 0.0)), vec2(0.0));
        let inset_r = max(safe_r - max(spread, 0.0), 0.0);
        let shadow_p = p - in.shadow_offset;
        let shadow_d = sdf_rounded_rect(shadow_p, inset_half, inset_r);
        // Approximate a Gaussian erf with tanh so the falloff width matches
        // CSS box-shadow conventions.
        let sigma = max(blur, 0.5);
        let shadow_alpha = 0.5 + 0.5 * tanh(shadow_d / sigma * 0.75);
        let edge_clip = 1.0 - smoothstep(-0.5, 0.5, d_outer);
        let final_a = in.shadow_color.a * shadow_alpha * edge_clip;
        if final_a < 0.001 {
            discard;
        }
        return vec4(in.shadow_color.rgb * final_a, final_a);
    }

    // Outer shadow pass (and the main rect's background pass when no shadow is
    // present on this instance).
    var shadow = vec4(0.0);
    if in.shadow_color.a > 0.0 {
        let outer_half = half + vec2(max(spread, 0.0));
        let outer_r = safe_r + max(spread, 0.0);
        let shadow_p = p - in.shadow_offset;
        let shadow_d = sdf_rounded_rect(shadow_p, outer_half, outer_r);
        let sigma = max(blur, 0.5);
        let shadow_alpha = 0.5 - 0.5 * tanh(shadow_d / sigma * 0.75);
        shadow = vec4(in.shadow_color.rgb, in.shadow_color.a * shadow_alpha);
    }

    // Main rect pass
    let d = sdf_rounded_rect(p, half, safe_r);
    let outer_alpha = 1.0 - smoothstep(-0.5, 0.5, d);

    if outer_alpha < 0.001 && shadow.a < 0.001 {
        discard;
    }

    // No gradient/mask on the software path: the base color is the solid
    // instance color.
    var base_color = in.color;

    // Border -- same uniform vs mismatched-sides logic as quad.wgsl (using the
    // unexpanded rect_local), so a `border-left-width: 1px`-style stripe still
    // paints. Rounded corners are honored for uniform borders and ignored for
    // mismatched ones (matches the full shader and CSS practice).
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
            let d_top = rect_local.y;
            let d_left = rect_local.x;
            let d_right = in.size.x - rect_local.x;
            let d_bottom = in.size.y - rect_local.y;
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

    // Composite: shadow behind rect (over operator), premultiplied output.
    return vec4(
        rect_color.rgb * rect_color.a + shadow.rgb * shadow.a * (1.0 - rect_color.a),
        rect_color.a + shadow.a * (1.0 - rect_color.a),
    );
}
