//! DirectWrite-based glyph rasterizer for Windows.
//!
//! Uses the native Windows DirectWrite API (via `dwrote`) for glyph rasterization,
//! producing ClearType subpixel coverage data that matches Windows Terminal quality.
//! On non-Windows platforms this module is not compiled.

use dwrote::{
    FontCollection, FontFace, FontMetrics, FontStretch, FontStyle, FontWeight, GdiInterop,
    GlyphOffset, RenderingParams,
};

/// A rasterized glyph with RGBA subpixel coverage data.
pub struct RasterizedGlyph {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data (4 bytes per pixel). RGB channels contain ClearType
    /// subpixel coverage; A = max(R, G, B) for compositing.
    pub data: Vec<u8>,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
}

/// DirectWrite glyph rasterizer. Created once at app startup and reused
/// for all glyph rasterization during the lifetime of the application.
pub struct DwRasterizer {
    font_face: FontFace,
    gdi_interop: GdiInterop,
    rendering_params: RenderingParams,
    design_units_per_em: u16,
    /// The resolved font family name, for use in cosmic-text shaping so
    /// both systems agree on glyph metrics.
    pub font_family: String,
}

impl DwRasterizer {
    /// Create a new rasterizer for the given font family name.
    /// Falls back to Consolas if the requested font is not found.
    pub fn new(font_name: &str) -> Self {
        let collection = FontCollection::system();
        let (family, resolved_name) = collection
            .font_family_by_name(font_name)
            .ok()
            .flatten()
            .map(|f| (f, font_name.to_string()))
            .or_else(|| {
                collection
                    .font_family_by_name("Consolas")
                    .ok()
                    .flatten()
                    .map(|f| (f, "Consolas".to_string()))
            })
            .expect("Neither requested font nor Consolas found");

        let font = family
            .first_matching_font(FontWeight::Regular, FontStretch::Normal, FontStyle::Normal)
            .expect("No matching font variant");
        let font_face = font.create_font_face();

        let gdi_interop = GdiInterop::create();
        let rendering_params = RenderingParams::create_for_primary_monitor();

        let metrics = font_face.metrics();
        let design_units_per_em = match metrics {
            FontMetrics::Metrics0(ref m) => m.designUnitsPerEm,
            FontMetrics::Metrics1(ref m) => m.designUnitsPerEm,
        };

        log::info!("DwRasterizer: resolved font family {:?}", resolved_name);
        Self {
            font_face,
            gdi_interop,
            rendering_params,
            design_units_per_em,
            font_family: resolved_name,
        }
    }

    /// Measure the advance width of a character at the given pixel size.
    /// Used to compute cell_w for grid rendering so that the measurement
    /// comes from the same font as the DirectWrite rasterized glyphs.
    pub fn measure_advance_width(&self, ch: char, pixel_size: f32) -> f32 {
        let scale = pixel_size / self.design_units_per_em as f32;
        if let Ok(indices) = self.font_face.glyph_indices(&[ch as u32]) {
            if let Ok(metrics) = self.font_face.design_glyph_metrics(&[indices[0]], false) {
                return metrics[0].advanceWidth as f32 * scale;
            }
        }
        pixel_size * 0.6
    }

    /// Rasterize a single glyph using DirectWrite.
    ///
    /// `pixel_size` is the final pixel size (already scaled by DPI).
    /// Returns RGBA data with ClearType subpixel coverage in the RGB channels.
    pub fn rasterize_glyph(&self, ch: char, pixel_size: f32) -> Option<RasterizedGlyph> {
        let glyph_indices = self.font_face.glyph_indices(&[ch as u32]).ok()?;
        let glyph_index = glyph_indices[0];
        if glyph_index == 0 && ch != '\0' {
            return None;
        }

        let metrics = self.font_face.design_glyph_metrics(&[glyph_index], false).ok()?;
        let gm = &metrics[0];

        let scale = pixel_size / self.design_units_per_em as f32;
        let advance_w = gm.advanceWidth as f32 * scale;

        let pad = 4u32;
        let rt_width = (advance_w as u32 + pad * 2).max(pad * 2 + 2);
        let rt_height = (pixel_size * 2.0) as u32 + pad * 2;

        let baseline_x = pad as f32;
        let baseline_y = (pixel_size * 1.3).round();

        let rt = self.gdi_interop.create_bitmap_render_target(rt_width, rt_height);
        rt.set_pixels_per_dip(1.0);

        let rect = rt.draw_glyph_run(
            baseline_x,
            baseline_y,
            dwrote::DWRITE_MEASURING_MODE_NATURAL,
            &self.font_face,
            pixel_size,
            &[glyph_index],
            &[0.0_f32],
            &[GlyphOffset { advanceOffset: 0.0, ascenderOffset: 0.0 }],
            &self.rendering_params,
            &(255.0, 255.0, 255.0),
        );

        let glyph_left = rect.left.max(0) as u32;
        let glyph_top = rect.top.max(0) as u32;
        let glyph_right = (rect.right as u32).min(rt_width);
        let glyph_bottom = (rect.bottom as u32).min(rt_height);

        if glyph_right <= glyph_left || glyph_bottom <= glyph_top {
            return Some(RasterizedGlyph {
                width: 0,
                height: 0,
                data: vec![],
                bearing_x: 0.0,
                bearing_y: 0.0,
                advance: advance_w,
            });
        }

        let glyph_w = glyph_right - glyph_left;
        let glyph_h = glyph_bottom - glyph_top;

        let raw_bgra = read_raw_bitmap(&rt, rt_width, rt_height);

        let mut rgba = Vec::with_capacity((glyph_w * glyph_h * 4) as usize);
        for row in glyph_top..glyph_bottom {
            for col in glyph_left..glyph_right {
                let idx = (row * rt_width + col) as usize * 4;
                let b = raw_bgra[idx];
                let g = raw_bgra[idx + 1];
                let r = raw_bgra[idx + 2];
                let alpha = r.max(g).max(b);
                rgba.push(r);
                rgba.push(g);
                rgba.push(b);
                rgba.push(alpha);
            }
        }

        let bearing_x = glyph_left as f32 - baseline_x;
        let bearing_y = glyph_top as f32 - baseline_y;

        Some(RasterizedGlyph {
            width: glyph_w,
            height: glyph_h,
            data: rgba,
            bearing_x,
            bearing_y,
            advance: advance_w,
        })
    }
}

/// Read raw BGRA pixel data from the GDI bitmap behind a BitmapRenderTarget.
fn read_raw_bitmap(rt: &dwrote::BitmapRenderTarget, width: u32, height: u32) -> Vec<u8> {
    use winapi::um::wingdi::{GetCurrentObject, GetObjectW, BITMAP, OBJ_BITMAP};
    unsafe {
        let memory_dc = rt.get_memory_dc();
        let mut bitmap: BITMAP = std::mem::zeroed();
        let ret = GetObjectW(
            GetCurrentObject(memory_dc, OBJ_BITMAP),
            std::mem::size_of::<BITMAP>() as i32,
            &mut bitmap as *mut _ as *mut std::ffi::c_void,
        );
        assert!(ret == std::mem::size_of::<BITMAP>() as i32);
        assert!(bitmap.bmBitsPixel == 32);

        let stride = bitmap.bmWidthBytes as usize;
        let w = width as usize;
        let h = height as usize;

        let mut out = vec![0u8; w * h * 4];
        for row in 0..h {
            let src =
                std::slice::from_raw_parts((bitmap.bmBits as *const u8).add(row * stride), w * 4);
            let dst_offset = row * w * 4;
            out[dst_offset..dst_offset + w * 4].copy_from_slice(src);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: issue #17. The font_family field must be stored so
    // cosmic-text can use the same font as DirectWrite for shaping.
    #[test]
    fn dw_rasterizer_stores_font_family() {
        let dw = DwRasterizer::new("Consolas");
        assert_eq!(
            dw.font_family, "Consolas",
            "DwRasterizer must store the resolved font family name"
        );
    }

    // Regression: issue #17. When the requested font is not installed,
    // the fallback name (Consolas) must be stored, not the missing name.
    #[test]
    fn dw_rasterizer_fallback_stores_consolas() {
        let dw = DwRasterizer::new("NonExistentFont12345");
        assert_eq!(
            dw.font_family, "Consolas",
            "fallback font family must be Consolas, not the missing font name"
        );
    }

    #[test]
    fn dw_advance_width_is_positive() {
        let dw = DwRasterizer::new("Consolas");
        let advance = dw.measure_advance_width('M', 14.0);
        assert!(advance > 0.0, "advance width for 'M' at 14px must be positive, got {}", advance);
    }

    // Regression: issue #17. In a monospace font every character must
    // have the same advance width. A mismatch here caused the 'I' gap.
    #[test]
    fn dw_advance_width_is_monospace() {
        let dw = DwRasterizer::new("Consolas");
        let reference = dw.measure_advance_width('M', 14.0);
        let epsilon = 0.01;
        for ch in ['I', 'i', 'W', 'n', '.', ' '] {
            let advance = dw.measure_advance_width(ch, 14.0);
            assert!(
                (advance - reference).abs() < epsilon,
                "monospace invariant: advance for '{}' ({:.4}) must equal 'M' ({:.4})",
                ch,
                advance,
                reference
            );
        }
    }
}
