// Single pass fragment shader grid renderer.
//
// The vertex stage emits one full screen triangle with no vertex buffer
// bound; the fragment stage looks up a `GpuCell` from the storage buffer
// using `frag_pos` and samples the glyph atlas directly.
//
// Feature gated behind `grid-fragment-shader` on `unshit-renderer`. See
// `grid_fragment_upload.rs` for the CPU side buffer layout and
// `pipeline::grid_fragment` for the pipeline wiring.

struct Uniforms {
    viewport: vec2<f32>,
    cell_size: vec2<f32>,
    grid_origin: vec2<f32>,
    cols: u32,
    rows: u32,
    cursor_col: u32,
    cursor_row: u32,
    cursor_style: u32,
    scroll_origin_row: u32,
    // Padding so the selection range lands at a 16 byte aligned offset.
    _pad_sel: u32,
    selection_start: u32,
    selection_end: u32,
    atlas_generation: u32,
    _pad_tail: u32,
    // Tail padding to round the struct size up to a 16 byte boundary. The
    // Rust side carries the same three u32 filler so `bytemuck::bytes_of`
    // copies the exact byte layout the shader expects.
    _pad_extra_0: u32,
    _pad_extra_1: u32,
    _pad_extra_2: u32,
};

struct GpuCell {
    glyph_id: u32,
    fg_rgba: u32,
    bg_rgba: u32,
    flags: u32,
};

struct GpuGlyphMeta {
    atlas_uv_min: vec2<f32>,
    atlas_uv_max: vec2<f32>,
    pixel_offset: vec2<f32>,
    pixel_size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> cells: array<GpuCell>;
@group(0) @binding(2) var<storage, read> glyph_meta: array<GpuGlyphMeta>;
@group(1) @binding(0) var atlas_mono: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

const EMPTY_GLYPH_ID: u32 = 0xFFFFFFFFu;
const FLAG_INVERSE: u32 = 16u; // bit 4
const FLAG_DIM: u32 = 32u; // bit 5
const FLAG_CURSOR: u32 = 256u; // bit 8
const DIM_INTENSITY: f32 = 0.5;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    // Full screen triangle. Three vertices, no vertex buffer.
    //   (-1, -3), (-1, 1), (3, 1) covers [-1, 1]^2 without
    // overdraw beyond the viewport.
    var pts = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(pts[vi], 0.0, 1.0);
    return out;
}

fn is_cursor_cell(col: u32, row: u32) -> bool {
    return col == uniforms.cursor_col && row == uniforms.cursor_row;
}

fn compose_cursor(base: vec4<f32>, fg: vec4<f32>, coverage: f32) -> vec4<f32> {
    // Block cursor: invert base color so the text is readable over the
    // cursor rectangle. Underline/bar cursors leave the base unchanged
    // and draw a small filled rect via the `cursor_style` bits; for this
    // scaffold we ship the block style only and treat other values as
    // block. Follow-up work can add underline/bar paths without extra
    // draw calls.
    _ = uniforms.cursor_style;
    let inverted = vec4<f32>(1.0 - base.rgb, base.a);
    _ = fg;
    _ = coverage;
    return inverted;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let frag = in.position.xy;
    let local = frag - uniforms.grid_origin;
    if local.x < 0.0 || local.y < 0.0 {
        discard;
    }
    let col_f = local.x / uniforms.cell_size.x;
    let row_f = local.y / uniforms.cell_size.y;
    let col = u32(col_f);
    let row = u32(row_f);
    if col >= uniforms.cols || row >= uniforms.rows {
        discard;
    }

    let linear = (row + uniforms.scroll_origin_row) * uniforms.cols + col;
    let cell = cells[linear];

    var fg = unpack4x8unorm(cell.fg_rgba);
    var bg = unpack4x8unorm(cell.bg_rgba);
    if (cell.flags & FLAG_INVERSE) != 0u {
        let tmp = fg;
        fg = bg;
        bg = tmp;
    }
    if (cell.flags & FLAG_DIM) != 0u {
        fg = vec4<f32>(fg.rgb * DIM_INTENSITY, fg.a);
    }

    let cursor_here = is_cursor_cell(col, row);

    // Selection overlay: linear index inclusive on both ends. A start of
    // `u32::MAX` means no selection.
    var out_color = bg;
    if uniforms.selection_start != 0xFFFFFFFFu
        && linear >= uniforms.selection_start
        && linear <= uniforms.selection_end {
        out_color = mix(bg, vec4<f32>(0.2, 0.4, 0.8, 1.0), 0.5);
    }

    if cell.glyph_id != EMPTY_GLYPH_ID {
        let gmeta = glyph_meta[cell.glyph_id];
        let cell_origin = vec2<f32>(
            f32(col) * uniforms.cell_size.x,
            f32(row) * uniforms.cell_size.y,
        );
        let glyph_px = local - cell_origin - gmeta.pixel_offset;
        if glyph_px.x >= 0.0 && glyph_px.y >= 0.0
            && glyph_px.x < gmeta.pixel_size.x
            && glyph_px.y < gmeta.pixel_size.y {
            let uv = mix(gmeta.atlas_uv_min, gmeta.atlas_uv_max, glyph_px / gmeta.pixel_size);
            let coverage = textureSample(atlas_mono, atlas_sampler, uv).r;
            // Mild stem-contrast curve, identical to text.wgsl, so terminal-cell
            // text and UI text share the same grayscale stem weight.
            let cov = pow(coverage, 0.88);
            out_color = vec4<f32>(mix(out_color.rgb, fg.rgb, cov), 1.0);
        }
    }

    if cursor_here {
        out_color = compose_cursor(out_color, fg, 1.0);
    }
    return out_color;
}
