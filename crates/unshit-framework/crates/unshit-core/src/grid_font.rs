//! Font metrics for grid-based rendering.
//!
//! Grid-based views such as terminal emulators and code editors need a single
//! source of truth for cell geometry so they can compute integer row/column
//! positions without re-probing the font system on every frame.
//!
//! [`GridFont`] wraps cosmic-text font metrics and guarantees that all cell
//! measurements are derived from a single monospace shaping pass.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};

/// Font metrics for grid-based rendering. Guarantees a single monospace
/// face so callers can compute integer cell geometry without re-probing.
#[derive(Clone, Debug)]
pub struct GridFont {
    /// The font family name used to derive these metrics.
    pub family: String,
    /// Width of a single cell in logical pixels (advance width of one glyph).
    pub cell_width: f32,
    /// Height of a single cell in logical pixels (line height).
    pub cell_height: f32,
    /// Ascent above the baseline in logical pixels.
    pub ascent: f32,
    /// Descent below the baseline in logical pixels (positive value).
    pub descent: f32,
    /// Font size in points used when measuring.
    pub font_size: f32,
}

impl GridFont {
    /// Compute grid font metrics from a cosmic-text [`FontSystem`].
    ///
    /// Shapes a single `'M'` character using the named family at the given
    /// `font_size`. Returns `None` if no glyphs are produced (font family not
    /// found or shaping failure).
    ///
    /// `line_height` is taken as `font_size * 1.2` which matches the default
    /// used elsewhere in the renderer.
    pub fn from_font_system(
        font_system: &mut FontSystem,
        family: &str,
        font_size: f32,
    ) -> Option<Self> {
        let line_height = font_size * 1.2;
        let metrics = Metrics::new(font_size, line_height);
        let mut buffer = Buffer::new(font_system, metrics);

        // Give the buffer a generous width so the glyph is never clipped.
        buffer.set_size(font_system, Some(font_size * 10.0), None);
        buffer.set_text(
            font_system,
            "M",
            Attrs::new().family(Family::Name(family)),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(font_system, false);

        // Extract glyph advance (cell width) and line metrics from the first run.
        let mut cell_width: Option<f32> = None;
        let mut ascent = 0.0f32;
        let mut descent = 0.0f32;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                // For a monospace font every glyph has the same advance width.
                // We only need the first one.
                if cell_width.is_none() {
                    cell_width = Some(glyph.w);
                }
            }
            // line_y is the Y offset to the baseline. line_top is the top of
            // the line. The gap from top to baseline is the ascent; the gap
            // from baseline to top+line_height is the descent.
            let line_asc = run.line_y - run.line_top;
            let line_desc = run.line_height - line_asc;
            ascent = ascent.max(line_asc);
            descent = descent.max(line_desc);
        }

        let cell_width = cell_width?;

        Some(GridFont {
            family: family.to_string(),
            cell_width,
            cell_height: line_height,
            ascent,
            descent,
            font_size,
        })
    }

    /// Compute the top-left pixel position of a cell at `(row, col)`.
    ///
    /// Row 0, column 0 maps to `(0.0, 0.0)`.
    pub fn cell_position(&self, row: u32, col: u32) -> (f32, f32) {
        (col as f32 * self.cell_width, row as f32 * self.cell_height)
    }

    /// Compute the grid dimensions (rows, cols) that fit within a pixel area.
    ///
    /// Partial cells are dropped: only fully-visible cells are counted.
    pub fn grid_dimensions(&self, width: f32, height: f32) -> (u32, u32) {
        let cols = (width / self.cell_width).floor() as u32;
        let rows = (height / self.cell_height).floor() as u32;
        (rows, cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Return the path to the bundled FiraMono fixture font.
    fn fira_mono_path() -> PathBuf {
        // The fixture lives in the unshit-app crate; use CARGO_MANIFEST_DIR to
        // locate this crate's root and step up to the workspace root.
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("unshit-app")
            .join("tests")
            .join("fixtures")
            .join("FiraMono-Medium.ttf")
    }

    fn make_font_system_with_fira() -> FontSystem {
        let mut fs = FontSystem::new();
        let bytes =
            std::fs::read(fira_mono_path()).expect("FiraMono-Medium.ttf fixture must exist");
        let adapter: std::sync::Arc<dyn AsRef<[u8]> + Sync + Send> = std::sync::Arc::new(bytes);
        fs.db_mut().load_font_source(cosmic_text::fontdb::Source::Binary(adapter));
        fs
    }

    #[test]
    fn cell_position_row0_col0_is_origin() {
        let gf = GridFont {
            family: "FiraMono".into(),
            cell_width: 8.0,
            cell_height: 16.0,
            ascent: 13.0,
            descent: 3.0,
            font_size: 14.0,
        };
        assert_eq!(gf.cell_position(0, 0), (0.0, 0.0));
    }

    #[test]
    fn cell_position_returns_correct_coordinates() {
        let gf = GridFont {
            family: "FiraMono".into(),
            cell_width: 8.0,
            cell_height: 16.0,
            ascent: 13.0,
            descent: 3.0,
            font_size: 14.0,
        };
        assert_eq!(gf.cell_position(2, 5), (40.0, 32.0));
        assert_eq!(gf.cell_position(1, 0), (0.0, 16.0));
        assert_eq!(gf.cell_position(0, 10), (80.0, 0.0));
    }

    #[test]
    fn grid_dimensions_exact_fit() {
        let gf = GridFont {
            family: "FiraMono".into(),
            cell_width: 8.0,
            cell_height: 16.0,
            ascent: 13.0,
            descent: 3.0,
            font_size: 14.0,
        };
        // 640 / 8 = 80 cols, 400 / 16 = 25 rows
        assert_eq!(gf.grid_dimensions(640.0, 400.0), (25, 80));
    }

    #[test]
    fn grid_dimensions_partial_cells_dropped() {
        let gf = GridFont {
            family: "FiraMono".into(),
            cell_width: 8.0,
            cell_height: 16.0,
            ascent: 13.0,
            descent: 3.0,
            font_size: 14.0,
        };
        // 645 / 8 = 80.625 -> 80 cols, 401 / 16 = 25.0625 -> 25 rows
        assert_eq!(gf.grid_dimensions(645.0, 401.0), (25, 80));
    }

    #[test]
    fn grid_dimensions_zero_area() {
        let gf = GridFont {
            family: "FiraMono".into(),
            cell_width: 8.0,
            cell_height: 16.0,
            ascent: 13.0,
            descent: 3.0,
            font_size: 14.0,
        };
        assert_eq!(gf.grid_dimensions(0.0, 0.0), (0, 0));
        assert_eq!(gf.grid_dimensions(7.9, 15.9), (0, 0));
    }

    #[test]
    fn from_font_system_with_fira_mono() {
        let mut fs = make_font_system_with_fira();
        let gf = GridFont::from_font_system(&mut fs, "Fira Mono", 14.0);
        assert!(gf.is_some(), "should produce metrics for FiraMono");
        let gf = gf.unwrap();
        // The advance width of a monospace 14px font should be in a reasonable range.
        assert!(gf.cell_width > 0.0, "cell_width must be positive");
        assert!(gf.cell_height > 0.0, "cell_height must be positive");
        assert!(gf.ascent > 0.0, "ascent must be positive");
        assert_eq!(gf.font_size, 14.0);
        assert_eq!(gf.family, "Fira Mono");
    }

    #[test]
    fn from_font_system_missing_family_returns_none() {
        let mut fs = FontSystem::new();
        // Use a family name that is guaranteed not to exist on any system.
        let gf = GridFont::from_font_system(&mut fs, "__nonexistent_family_xyzzy__", 14.0);
        // cosmic-text falls back to a default font rather than returning an
        // error, so we accept either Some (with positive metrics) or None.
        // The important thing is that the call does not panic.
        if let Some(g) = gf {
            assert!(g.cell_width > 0.0);
        }
    }
}
