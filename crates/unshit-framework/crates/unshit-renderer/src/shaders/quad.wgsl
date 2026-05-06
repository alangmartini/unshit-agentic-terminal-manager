struct Uniforms {
    viewport: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct QuadInstance {
    pos: array<f32, 2>,
    size: array<f32, 2>,
    color: array<f32, 4>,
    border_color: array<f32, 4>,
    border_width: array<f32, 4>,
    border_radius: array<f32, 4>,
    clip_rect: array<f32, 4>,
    shadow_color: array<f32, 4>,
    shadow_offset: array<f32, 2>,
    shadow_params: array<f32, 2>,
    shadow_spread: array<f32, 2>,
    gradient_stop_color_0: array<f32, 4>,
    gradient_stop_color_1: array<f32, 4>,
    gradient_stop_color_2: array<f32, 4>,
    gradient_stop_color_3: array<f32, 4>,
    gradient_stop_color_4: array<f32, 4>,
    gradient_stop_color_5: array<f32, 4>,
    gradient_stop_color_6: array<f32, 4>,
    gradient_stop_color_7: array<f32, 4>,
    gradient_stop_positions_lo: array<f32, 4>,
    gradient_stop_positions_hi: array<f32, 4>,
    gradient_params: array<f32, 4>,
    gradient_extra: array<f32, 4>,
    mask_stops_01: array<f32, 4>,
    mask_stops_23: array<f32, 4>,
    mask_params: array<f32, 4>,
};

struct QuadInstances {
    data: array<QuadInstance>,
};

@group(1) @binding(0) var<storage, read> quad_instances: QuadInstances;

fn arr2(a: array<f32, 2>) -> vec2<f32> {
    return vec2<f32>(a[0], a[1]);
}

fn arr4(a: array<f32, 4>) -> vec4<f32> {
    return vec4<f32>(a[0], a[1], a[2], a[3]);
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) pixel_pos: vec2<f32>,
    @location(2) @interpolate(flat) instance_index: u32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let instance = quad_instances.data[instance_index];
    let inst_pos = arr2(instance.pos);
    let inst_size = arr2(instance.size);
    let inst_shadow_params = arr2(instance.shadow_params);
    let inst_shadow_spread = arr2(instance.shadow_spread);
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let corner = corners[vertex_index];

    // Expand quad to accommodate shadow blur and outer spread. Inset
    // shadows paint inside the padding box so they do not contribute to
    // the expand, but outer shadows grow outward by both blur and spread.
    let blur = inst_shadow_params.x;
    let inset_flag = inst_shadow_params.y;
    let spread = inst_shadow_spread.x;
    let spread_expand = select(max(spread, 0.0), 0.0, inset_flag > 0.5);
    let expand = blur * 3.0 + spread_expand;
    let expanded_pos = inst_pos - vec2(expand);
    let expanded_size = inst_size + vec2(expand * 2.0);

    let pixel_pos = expanded_pos + corner * expanded_size;
    let ndc = vec2(
        (pixel_pos.x / uniforms.viewport.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / uniforms.viewport.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4(ndc, 0.0, 1.0);
    out.local_pos = corner * expanded_size;
    out.pixel_pos = pixel_pos;
    out.instance_index = instance_index;
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
    let inst = quad_instances.data[in.instance_index];
    let inst_size = arr2(inst.size);
    let inst_color = arr4(inst.color);
    let inst_border_color = arr4(inst.border_color);
    let inst_border_width = arr4(inst.border_width);
    let inst_border_radius = arr4(inst.border_radius);
    let inst_clip_rect = arr4(inst.clip_rect);
    let inst_shadow_color = arr4(inst.shadow_color);
    let inst_shadow_offset = arr2(inst.shadow_offset);
    let inst_shadow_params = arr2(inst.shadow_params);
    let inst_shadow_spread = arr2(inst.shadow_spread);
    let inst_gradient_stop_color_0 = arr4(inst.gradient_stop_color_0);
    let inst_gradient_stop_color_1 = arr4(inst.gradient_stop_color_1);
    let inst_gradient_stop_color_2 = arr4(inst.gradient_stop_color_2);
    let inst_gradient_stop_color_3 = arr4(inst.gradient_stop_color_3);
    let inst_gradient_stop_color_4 = arr4(inst.gradient_stop_color_4);
    let inst_gradient_stop_color_5 = arr4(inst.gradient_stop_color_5);
    let inst_gradient_stop_color_6 = arr4(inst.gradient_stop_color_6);
    let inst_gradient_stop_color_7 = arr4(inst.gradient_stop_color_7);
    let inst_gradient_stop_positions_lo = arr4(inst.gradient_stop_positions_lo);
    let inst_gradient_stop_positions_hi = arr4(inst.gradient_stop_positions_hi);
    let inst_gradient_params = arr4(inst.gradient_params);
    let inst_gradient_extra = arr4(inst.gradient_extra);
    let inst_mask_stops_01 = arr4(inst.mask_stops_01);
    let inst_mask_stops_23 = arr4(inst.mask_stops_23);
    let inst_mask_params = arr4(inst.mask_params);

    // Clip rect discard: clip_rect = [x, y, width, height]
    let clip_min = inst_clip_rect.xy;
    let clip_max = inst_clip_rect.xy + inst_clip_rect.zw;
    if in.pixel_pos.x < clip_min.x || in.pixel_pos.x > clip_max.x ||
       in.pixel_pos.y < clip_min.y || in.pixel_pos.y > clip_max.y {
        discard;
    }

    let blur = inst_shadow_params.x;
    let inset = inst_shadow_params.y > 0.5;
    let spread = inst_shadow_spread.x;
    let spread_expand = select(max(spread, 0.0), 0.0, inset);
    let expand = blur * 3.0 + spread_expand;

    // Position relative to the original (unexpanded) rect.
    let half = inst_size * 0.5;
    let rect_origin = vec2(expand);
    let rect_local = in.local_pos - rect_origin;
    let p = rect_local - half;

    let r = select_radius(rect_local, inst_size, inst_border_radius);
    let safe_r = min(r, min(half.x, half.y));

    // Inset shadow path: sample the rounded rect SDF relative to the
    // shadow offset and use the inside of the rect as the "lit" side.
    // Fragments outside the padding box are discarded so the inset shadow
    // never paints beyond the element.
    if inset {
        // Discard fragments outside the padding box (respecting the
        // rounded corners).
        let d_outer = sdf_rounded_rect(p, half, safe_r);
        if d_outer > 0.5 {
            discard;
        }
        // Shrink the inset sample box by `spread` so positive spread pulls
        // the shadow boundary inward.
        let inset_half = max(half - vec2(max(spread, 0.0)), vec2(0.0));
        let inset_r = max(safe_r - max(spread, 0.0), 0.0);
        let shadow_p = p - inst_shadow_offset;
        let shadow_d = sdf_rounded_rect(shadow_p, inset_half, inset_r);
        // Inset: shadow is strongest at d = 0 and fades into the interior.
        // Approximate a Gaussian erf with tanh so the falloff width matches
        // CSS box-shadow conventions (a simple smoothstep over [-blur, blur]
        // is too narrow and makes stacked shadows invisible past a few px).
        let sigma = max(blur, 0.5);
        let shadow_alpha = 0.5 + 0.5 * tanh(shadow_d / sigma * 0.75);
        // Clip softly to the outer rounded rect so the shadow does not
        // bleed past the visible edge.
        let edge_clip = 1.0 - smoothstep(-0.5, 0.5, d_outer);
        let final_a = inst_shadow_color.a * shadow_alpha * edge_clip;
        if final_a < 0.001 {
            discard;
        }
        return vec4(inst_shadow_color.rgb * final_a, final_a);
    }

    // Outer shadow pass (and the main rect's background pass when no
    // shadow is present on this instance).
    var shadow = vec4(0.0);
    if inst_shadow_color.a > 0.0 {
        // Outer spread grows the SDF sample box by `spread`, matching CSS
        // semantics where positive spread makes the shadow bigger.
        let outer_half = half + vec2(max(spread, 0.0));
        let outer_r = safe_r + max(spread, 0.0);
        let shadow_p = p - inst_shadow_offset;
        let shadow_d = sdf_rounded_rect(shadow_p, outer_half, outer_r);
        // See the inset path for why this uses tanh instead of smoothstep.
        let sigma = max(blur, 0.5);
        let shadow_alpha = 0.5 - 0.5 * tanh(shadow_d / sigma * 0.75);
        shadow = vec4(inst_shadow_color.rgb, inst_shadow_color.a * shadow_alpha);
    }

    // Main rect pass
    let d = sdf_rounded_rect(p, half, safe_r);
    let outer_alpha = 1.0 - smoothstep(-0.5, 0.5, d);

    if outer_alpha < 0.001 && shadow.a < 0.001 {
        discard;
    }

    // Gradient or solid color.
    //
    // `gradient_params.w` is a tagged stop count:
    //   * `0` means solid color (use `inst_color`).
    //   * positive (>= 2) means an N stop linear gradient. For linear,
    //     `gradient_params.y` is the repeating flag (issue #128): 0.0 for
    //     a non repeating gradient, 1.0 for a repeating one.
    //   * negative means an N stop radial gradient with `|w|` stops. For
    //     radial, `gradient_params.y` is the shape tag: -1.0 for a circle,
    //     1.0 for an ellipse. `gradient_extra.xy` carries the resolved
    //     center in element local pixels and `gradient_extra.zw` carries
    //     the resolved `(rx, ry)`.
    //
    // The same stop list (positions in `_lo`/`_hi`, colors in the eight
    // per stop attribute slots) is reused for both gradient kinds. The
    // only difference between the two paths is how the per pixel `t`
    // value is derived: linear projects onto a direction vector and
    // optionally wraps via `fract` for repeating; radial computes a
    // normalized distance from the explicit center.
    var base_color: vec4<f32>;
    let raw_tag = inst_gradient_params.w;
    let stop_count = i32(abs(raw_tag) + 0.5);
    if (stop_count >= 2) {
        var raw_t: f32 = 0.0;
        var is_linear: bool = raw_tag > 0.0;
        if (is_linear) {
            // Linear gradient: project rect local point onto direction.
            let angle = inst_gradient_params.x;
            let dir = vec2<f32>(sin(angle), -cos(angle));
            let normalized = rect_local / inst_size;
            raw_t = dot(normalized - vec2(0.5), dir) + 0.5;
        } else {
            // Radial gradient: normalized distance from the resolved
            // center to the current pixel, scaled by the resolved radii.
            // `gradient_extra.xy` is the center in element local pixels;
            // `gradient_extra.zw` is `(rx, ry)`. A degenerate radius
            // (`rx <= 0` or `ry <= 0`) collapses the gradient: we set
            // `raw_t = 1` so the shader falls through to the last stop color.
            let center = inst_gradient_extra.xy;
            let rx = inst_gradient_extra.z;
            let ry = inst_gradient_extra.w;
            let local = rect_local - center;
            let shape_is_circle = inst_gradient_params.y < 0.0;
            if (rx <= 0.0 || ry <= 0.0) {
                raw_t = 1.0;
            } else if (shape_is_circle) {
                // True circle: isotropic distance scaled by the single
                // radius. `rx == ry` here, so picking either is fine.
                raw_t = length(local) / rx;
            } else {
                let nx = local.x / rx;
                let ny = local.y / ry;
                raw_t = sqrt(nx * nx + ny * ny);
            }
        }

        var stop_positions = array<f32, 8>(
            inst_gradient_stop_positions_lo.x,
            inst_gradient_stop_positions_lo.y,
            inst_gradient_stop_positions_lo.z,
            inst_gradient_stop_positions_lo.w,
            inst_gradient_stop_positions_hi.x,
            inst_gradient_stop_positions_hi.y,
            inst_gradient_stop_positions_hi.z,
            inst_gradient_stop_positions_hi.w,
        );
        var stop_colors = array<vec4<f32>, 8>(
            inst_gradient_stop_color_0,
            inst_gradient_stop_color_1,
            inst_gradient_stop_color_2,
            inst_gradient_stop_color_3,
            inst_gradient_stop_color_4,
            inst_gradient_stop_color_5,
            inst_gradient_stop_color_6,
            inst_gradient_stop_color_7,
        );

        let last_idx = stop_count - 1;
        let stop_first = stop_positions[0];
        let stop_last = stop_positions[last_idx];

        // Wrap the projected coordinate into the stop list range when the
        // gradient is linear and marked repeating (issue #128). `fract` is
        // the GLSL / WGSL native modulo for floats and handles wraparound
        // at distance 0 and exactly the tile length cleanly so the seam
        // between tiles always uses the first stop's color, matching the
        // CSS spec requirement that the gradient's color at the start and
        // the end of a tile are the same. Radial gradients ignore the
        // repeating flag (they currently never repeat) and clamp instead.
        let repeating = is_linear && (inst_gradient_params.y >= 0.5);
        var t: f32;
        if (repeating) {
            let tile = max(stop_last - stop_first, 1e-6);
            t = stop_first + fract((raw_t - stop_first) / tile) * tile;
        } else {
            t = raw_t;
        }

        // Early out for positions outside the [first, last] range so a hard
        // stop at either boundary resolves cleanly. `clamp(t, 0, 1)` in each
        // per segment branch handles the same case for interior segments.
        // For a repeating gradient the wrap keeps `t` inside the range by
        // construction, so the out of range branches only fire in the non
        // repeating path.
        if (t <= stop_first) {
            base_color = stop_colors[0];
        } else if (t >= stop_last) {
            base_color = stop_colors[last_idx];
        } else {
            // Unrolled linear scan: find the first segment [i, i+1] whose
            // end position strictly exceeds `t`. Because positions are
            // monotonic and in [0, 1], this always converges.
            var segment_lo: vec4<f32> = stop_colors[0];
            var segment_hi: vec4<f32> = stop_colors[0];
            var pos_lo: f32 = stop_positions[0];
            var pos_hi: f32 = stop_positions[0];
            var found: bool = false;
            for (var i: i32 = 0; i < 7; i = i + 1) {
                if (!found && i + 1 < stop_count) {
                    let p0 = stop_positions[i];
                    let p1 = stop_positions[i + 1];
                    if (t >= p0 && t <= p1) {
                        segment_lo = stop_colors[i];
                        segment_hi = stop_colors[i + 1];
                        pos_lo = p0;
                        pos_hi = p1;
                        found = true;
                    }
                }
            }
            let range = max(pos_hi - pos_lo, 1e-6);
            let local_t = clamp((t - pos_lo) / range, 0.0, 1.0);
            base_color = mix(segment_lo, segment_hi, local_t);
        }
    } else {
        base_color = inst_color;
    }

    // Border
    //
    // `bw` packs the four side widths in (top, right, bottom, left)
    // order, matching `Edges::to_array()`. Two paths:
    //
    //  * All four sides equal: the previous uniform SDF path is exact
    //    and plays nicely with rounded corners.
    //  * Mismatched sides: walk axis aligned distances from each edge
    //    so e.g. `border-left-width: 1px` alone paints only a left
    //    stripe. This ignores rounded corners (CSS requires all
    //    corners be square for mismatched borders in practice), but
    //    lets the common left-only / right-only patterns render.
    let bw = inst_border_width;
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
            // Distances from the four edges. rect_local is in [0, size].
            let d_top = rect_local.y;
            let d_left = rect_local.x;
            let d_right = inst_size.x - rect_local.x;
            let d_bottom = inst_size.y - rect_local.y;
            // Each side contributes 1.0 when inside the stripe and
            // smoothly fades at the inner edge. Widths of 0 discard
            // their stripe entirely.
            let f_top = select(smoothstep(bw.x + 0.5, bw.x - 0.5, d_top), 0.0, bw.x <= 0.0);
            let f_right = select(smoothstep(bw.y + 0.5, bw.y - 0.5, d_right), 0.0, bw.y <= 0.0);
            let f_bottom = select(smoothstep(bw.z + 0.5, bw.z - 0.5, d_bottom), 0.0, bw.z <= 0.0);
            let f_left = select(smoothstep(bw.w + 0.5, bw.w - 0.5, d_left), 0.0, bw.w <= 0.0);
            // Union of the four stripes (max), then clamp.
            border_factor = clamp(max(max(f_top, f_right), max(f_bottom, f_left)), 0.0, 1.0);
        }
        // Composite border OVER background (CSS-like alpha blending)
        let ba = inst_border_color.a * border_factor;
        let one_minus_ba = 1.0 - ba;
        let result_a = ba + inst_color.a * one_minus_ba;
        let result_rgb = select(
            vec3(0.0),
            (inst_border_color.rgb * ba + inst_color.rgb * inst_color.a * one_minus_ba) / result_a,
            result_a > 0.001
        );
        rect_color = vec4(result_rgb, result_a);
    } else {
        rect_color = base_color;
    }
    rect_color = vec4(rect_color.rgb, rect_color.a * outer_alpha);

    // `mask-image: linear-gradient(...)`. When `mask_params.w >= 2` the
    // fragment samples the mask gradient's alpha at the current pixel and
    // multiplies the rect alpha by it, implementing the CSS alpha masking
    // semantics from the CSS Masking Module Level 1 spec. The mask is a
    // simple linear gradient; positions and alpha values are packed two
    // per stop in `mask_stops_01` / `mask_stops_23`.
    let mask_count = i32(inst_mask_params.w + 0.5);
    if (mask_count >= 2) {
        let mask_angle = inst_mask_params.x;
        let mask_dir = vec2<f32>(sin(mask_angle), -cos(mask_angle));
        let normalized = rect_local / inst_size;
        let mask_t = dot(normalized - vec2(0.5), mask_dir) + 0.5;

        var mask_alphas = array<f32, 4>(
            inst_mask_stops_01.x,
            inst_mask_stops_01.z,
            inst_mask_stops_23.x,
            inst_mask_stops_23.z,
        );
        var mask_positions = array<f32, 4>(
            inst_mask_stops_01.y,
            inst_mask_stops_01.w,
            inst_mask_stops_23.y,
            inst_mask_stops_23.w,
        );
        let m_last_idx = mask_count - 1;
        let m_first = mask_positions[0];
        let m_last = mask_positions[m_last_idx];
        var mask_alpha: f32;
        if (mask_t <= m_first) {
            mask_alpha = mask_alphas[0];
        } else if (mask_t >= m_last) {
            mask_alpha = mask_alphas[m_last_idx];
        } else {
            var seg_lo_a: f32 = mask_alphas[0];
            var seg_hi_a: f32 = mask_alphas[0];
            var seg_lo_p: f32 = mask_positions[0];
            var seg_hi_p: f32 = mask_positions[0];
            var m_found: bool = false;
            for (var i: i32 = 0; i < 3; i = i + 1) {
                if (!m_found && i + 1 < mask_count) {
                    let p0 = mask_positions[i];
                    let p1 = mask_positions[i + 1];
                    if (mask_t >= p0 && mask_t <= p1) {
                        seg_lo_a = mask_alphas[i];
                        seg_hi_a = mask_alphas[i + 1];
                        seg_lo_p = p0;
                        seg_hi_p = p1;
                        m_found = true;
                    }
                }
            }
            let m_range = max(seg_hi_p - seg_lo_p, 1e-6);
            let m_local = clamp((mask_t - seg_lo_p) / m_range, 0.0, 1.0);
            mask_alpha = mix(seg_lo_a, seg_hi_a, m_local);
        }
        rect_color = vec4(rect_color.rgb, rect_color.a * clamp(mask_alpha, 0.0, 1.0));
    }

    // Composite: shadow behind rect (over operator), premultiplied output.
    let result = vec4(
        rect_color.rgb * rect_color.a + shadow.rgb * shadow.a * (1.0 - rect_color.a),
        rect_color.a + shadow.a * (1.0 - rect_color.a),
    );

    return result;
}
