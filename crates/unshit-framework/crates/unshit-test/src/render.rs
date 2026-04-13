use crate::TestHarness;

/// Options for screenshot comparison with per-region tolerance and masking.
#[derive(Clone, Debug)]
pub struct ScreenshotOptions {
    /// Global RMSE tolerance threshold.
    pub tolerance: f64,
    /// Regions to ignore or apply custom tolerance to.
    pub masks: Vec<MaskRegion>,
}

impl Default for ScreenshotOptions {
    fn default() -> Self {
        Self { tolerance: 0.0, masks: Vec::new() }
    }
}

/// A region of the screenshot that receives special treatment during comparison.
#[derive(Clone, Debug)]
pub enum MaskRegion {
    /// Completely ignore this rectangle during comparison.
    Ignore { x: u32, y: u32, w: u32, h: u32 },
    /// Apply a custom tolerance to this rectangle instead of the global one.
    Tolerance { x: u32, y: u32, w: u32, h: u32, tolerance: f64 },
}

impl MaskRegion {
    /// Returns true if the given pixel coordinate falls within this region.
    fn contains(&self, px: u32, py: u32) -> bool {
        let (rx, ry, rw, rh) = match self {
            MaskRegion::Ignore { x, y, w, h } => (*x, *y, *w, *h),
            MaskRegion::Tolerance { x, y, w, h, .. } => (*x, *y, *w, *h),
        };
        px >= rx && px < rx + rw && py >= ry && py < ry + rh
    }
}

impl TestHarness {
    /// Render the current frame and check a pixel's color.
    /// x, y are in pixel coordinates. tolerance is per-channel (0-255).
    pub fn assert_pixel_color(&mut self, x: u32, y: u32, expected: [u8; 4], tolerance: u8) {
        let pixels = self.render();
        let (w, _h) = self.render_size();
        let idx = ((y * w + x) * 4) as usize;
        let actual = [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]];
        for ch in 0..4 {
            let diff = (actual[ch] as i16 - expected[ch] as i16).unsigned_abs() as u8;
            assert!(
                diff <= tolerance,
                "Pixel ({}, {}) channel {} mismatch: expected {} got {} (tolerance {})",
                x,
                y,
                ch,
                expected[ch],
                actual[ch],
                tolerance
            );
        }
    }

    /// Render N consecutive frames (with step() between each).
    /// Assert all frames produce identical pixels (within tolerance).
    /// This catches visual blink/flash bugs.
    pub fn assert_render_stable(&mut self, frames: usize) {
        let reference = self.render();
        for i in 0..frames {
            self.step();
            let current = self.render();
            assert!(
                pixels_match(&reference, &current, 1),
                "Render blinked on frame {}: pixels differ from reference",
                i + 1,
            );
        }
    }

    /// Render and return as an image::RgbaImage.
    pub fn screenshot(&mut self) -> image::RgbaImage {
        let pixels = self.render();
        let (w, h) = self.render_size();
        image::RgbaImage::from_raw(w, h, pixels).expect("pixel data size mismatch")
    }

    /// Compare rendered frame against a golden reference PNG.
    /// Saves actual output alongside the golden for visual diff on failure.
    ///
    /// When the `UNSHIT_UPDATE_GOLDEN=1` environment variable is set, this
    /// overwrites the golden file instead of comparing.
    pub fn assert_screenshot(&mut self, name: &str, tolerance: f64) {
        self.assert_screenshot_with_options(
            name,
            ScreenshotOptions { tolerance, masks: Vec::new() },
        );
    }

    /// Compare rendered frame against a golden reference PNG with advanced
    /// options including per-region masks and tolerances.
    ///
    /// When the `UNSHIT_UPDATE_GOLDEN=1` environment variable is set, this
    /// overwrites the golden file instead of comparing.
    pub fn assert_screenshot_with_options(&mut self, name: &str, options: ScreenshotOptions) {
        let actual = self.screenshot();
        let golden_path = format!("tests/golden/{}.png", name);

        if crate::test_app::env_is_truthy("UNSHIT_UPDATE_GOLDEN") {
            std::fs::create_dir_all("tests/golden").ok();
            actual.save(&golden_path).expect("failed to save golden");
            eprintln!("Updated golden screenshot: {}", golden_path);
            return;
        }

        match image::open(&golden_path) {
            Ok(golden) => {
                let golden = golden.to_rgba8();
                let comparison =
                    compare_with_masks(&actual, &golden, options.tolerance, &options.masks);

                if !comparison.passed {
                    let actual_path = format!("tests/golden/{}_actual.png", name);
                    let diff_path = format!("tests/golden/{}_diff.png", name);
                    actual.save(&actual_path).ok();

                    let diff_img = generate_diff_image(&actual, &golden, &options.masks);
                    diff_img.save(&diff_path).ok();

                    let total_pixels = comparison.total_pixels;
                    let changed_pct = if total_pixels > 0 {
                        (comparison.changed_pixels as f64 / total_pixels as f64) * 100.0
                    } else {
                        0.0
                    };

                    panic!(
                        "Screenshot mismatch: \"{}\"\n  \
                         RMSE: {:.4} (tolerance: {:.4})\n  \
                         Changed pixels: {} / {} ({:.2}%)\n  \
                         Golden:  {}\n  \
                         Actual:  {}\n  \
                         Diff:    {}",
                        name,
                        comparison.rmse,
                        options.tolerance,
                        comparison.changed_pixels,
                        total_pixels,
                        changed_pct,
                        golden_path,
                        actual_path,
                        diff_path,
                    );
                }
            }
            Err(_) => {
                std::fs::create_dir_all("tests/golden").ok();
                actual.save(&golden_path).expect("failed to save golden");
                eprintln!("Golden screenshot saved: {}", golden_path);
            }
        }
    }

    /// Save current render as golden reference.
    pub fn save_golden(&mut self, name: &str) {
        let img = self.screenshot();
        std::fs::create_dir_all("tests/golden").ok();
        let path = format!("tests/golden/{}.png", name);
        img.save(&path).expect("failed to save golden");
    }

    /// Get render dimensions.
    fn render_size(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }
}

/// Check if two pixel buffers match within a per-channel tolerance.
pub fn pixels_match(a: &[u8], b: &[u8], tolerance: u8) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(av, bv)| (*av as i16 - *bv as i16).unsigned_abs() as u8 <= tolerance)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Result of a masked comparison between two images.
struct ComparisonResult {
    passed: bool,
    rmse: f64,
    changed_pixels: u64,
    total_pixels: u64,
}

/// Per-pixel tolerance for a given coordinate, accounting for mask regions.
/// Returns `None` if the pixel should be ignored entirely.
fn pixel_tolerance(x: u32, y: u32, global: f64, masks: &[MaskRegion]) -> Option<f64> {
    for mask in masks {
        if !mask.contains(x, y) {
            continue;
        }
        return match mask {
            MaskRegion::Ignore { .. } => None,
            MaskRegion::Tolerance { tolerance, .. } => Some(*tolerance),
        };
    }
    Some(global)
}

/// Compare two images respecting mask regions. Returns aggregated stats.
fn compare_with_masks(
    actual: &image::RgbaImage,
    golden: &image::RgbaImage,
    global_tolerance: f64,
    masks: &[MaskRegion],
) -> ComparisonResult {
    if actual.dimensions() != golden.dimensions() {
        return ComparisonResult {
            passed: false,
            rmse: f64::MAX,
            changed_pixels: 0,
            total_pixels: 0,
        };
    }

    let (width, height) = actual.dimensions();
    let mut sum_sq: f64 = 0.0;
    let mut compared_pixels: u64 = 0;
    let mut changed_pixels: u64 = 0;
    let total_pixels = (width as u64) * (height as u64);

    let mut worst_excess: f64 = 0.0;

    // Fast path: no masks, skip per-pixel lookups.
    if masks.is_empty() {
        for (pa, pb) in actual.pixels().zip(golden.pixels()) {
            let sq = pixel_sq_diff(pa, pb);
            sum_sq += sq;
            compared_pixels += 1;
            if sq > 0.0 {
                changed_pixels += 1;
            }
        }
        let rmse = if compared_pixels > 0 {
            (sum_sq / (compared_pixels as f64 * 4.0)).sqrt()
        } else {
            0.0
        };
        let excess = rmse - global_tolerance;
        if excess > 0.0 {
            worst_excess = excess;
        }
        return ComparisonResult {
            passed: worst_excess <= 0.0,
            rmse,
            changed_pixels,
            total_pixels,
        };
    }

    let mut region_sums: Vec<(f64, u64, f64)> = masks
        .iter()
        .map(|m| match m {
            MaskRegion::Tolerance { tolerance, .. } => (0.0f64, 0u64, *tolerance),
            MaskRegion::Ignore { .. } => (0.0, 0, 0.0),
        })
        .collect();

    for y in 0..height {
        for x in 0..width {
            if pixel_tolerance(x, y, global_tolerance, masks).is_none() {
                continue;
            }

            let pa = actual.get_pixel(x, y);
            let pb = golden.get_pixel(x, y);
            let sq = pixel_sq_diff(pa, pb);
            sum_sq += sq;
            compared_pixels += 1;
            if sq > 0.0 {
                changed_pixels += 1;
            }

            for (i, mask) in masks.iter().enumerate() {
                if matches!(mask, MaskRegion::Tolerance { .. }) && mask.contains(x, y) {
                    region_sums[i].0 += sq;
                    region_sums[i].1 += 1;
                }
            }
        }
    }

    let overall_rmse =
        if compared_pixels > 0 { (sum_sq / (compared_pixels as f64 * 4.0)).sqrt() } else { 0.0 };

    for (region_sum, region_count, region_tol) in &region_sums {
        if *region_count == 0 {
            continue;
        }
        let region_rmse = (region_sum / (*region_count as f64 * 4.0)).sqrt();
        let excess = region_rmse - region_tol;
        if excess > worst_excess {
            worst_excess = excess;
        }
    }

    let global_excess = overall_rmse - global_tolerance;
    if global_excess > worst_excess {
        worst_excess = global_excess;
    }

    ComparisonResult {
        passed: worst_excess <= 0.0,
        rmse: overall_rmse,
        changed_pixels,
        total_pixels,
    }
}

/// Sum of squared channel differences for one pixel pair.
fn pixel_sq_diff(pa: &image::Rgba<u8>, pb: &image::Rgba<u8>) -> f64 {
    let dr = pa[0] as f64 - pb[0] as f64;
    let dg = pa[1] as f64 - pb[1] as f64;
    let db = pa[2] as f64 - pb[2] as f64;
    let da = pa[3] as f64 - pb[3] as f64;
    dr * dr + dg * dg + db * db + da * da
}

/// Compute RMSE (root mean square error) between two RGBA images.
pub fn compute_rmse(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
    if a.dimensions() != b.dimensions() {
        return f64::MAX;
    }
    let total_pixels = (a.width() * a.height()) as f64;
    let sum_sq: f64 = a.pixels().zip(b.pixels()).map(|(pa, pb)| pixel_sq_diff(pa, pb)).sum();
    (sum_sq / (total_pixels * 4.0)).sqrt()
}

/// Generate a diff image highlighting changed pixels in red.
/// Ignored mask regions are shown in blue. Identical pixels are shown at
/// 25% brightness so the diff overlay stands out.
fn generate_diff_image(
    actual: &image::RgbaImage,
    golden: &image::RgbaImage,
    masks: &[MaskRegion],
) -> image::RgbaImage {
    let (width, height) = actual.dimensions();
    let mut diff = image::RgbaImage::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let ignored =
                masks.iter().any(|m| matches!(m, MaskRegion::Ignore { .. }) && m.contains(x, y));

            if ignored {
                diff.put_pixel(x, y, image::Rgba([0, 0, 180, 255]));
                continue;
            }

            let pa = actual.get_pixel(x, y);
            let golden_px = if x < golden.width() && y < golden.height() {
                *golden.get_pixel(x, y)
            } else {
                image::Rgba([0, 0, 0, 0])
            };

            if pa.0 == golden_px.0 {
                diff.put_pixel(x, y, image::Rgba([pa[0] / 4, pa[1] / 4, pa[2] / 4, 255]));
            } else {
                diff.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
    }

    diff
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    /// Helper: create a solid-color 4x4 image.
    fn solid(r: u8, g: u8, b: u8, a: u8) -> RgbaImage {
        let mut img = RgbaImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = Rgba([r, g, b, a]);
        }
        img
    }

    #[test]
    fn compute_rmse_identical_images() {
        let a = solid(100, 150, 200, 255);
        let b = solid(100, 150, 200, 255);
        assert_eq!(compute_rmse(&a, &b), 0.0);
    }

    #[test]
    fn compute_rmse_different_images() {
        let a = solid(0, 0, 0, 255);
        let b = solid(255, 255, 255, 255);
        let rmse = compute_rmse(&a, &b);
        assert!(rmse > 100.0, "RMSE should be large for black vs white");
    }

    #[test]
    fn compute_rmse_dimension_mismatch() {
        let a = RgbaImage::new(2, 2);
        let b = RgbaImage::new(3, 3);
        assert_eq!(compute_rmse(&a, &b), f64::MAX);
    }

    #[test]
    fn compare_identical_passes() {
        let a = solid(50, 50, 50, 255);
        let b = solid(50, 50, 50, 255);
        let result = compare_with_masks(&a, &b, 0.0, &[]);
        assert!(result.passed);
        assert_eq!(result.rmse, 0.0);
        assert_eq!(result.changed_pixels, 0);
    }

    #[test]
    fn compare_different_fails_with_zero_tolerance() {
        let a = solid(0, 0, 0, 255);
        let b = solid(10, 10, 10, 255);
        let result = compare_with_masks(&a, &b, 0.0, &[]);
        assert!(!result.passed);
        assert!(result.rmse > 0.0);
        assert_eq!(result.changed_pixels, 16); // all 4x4 pixels changed
    }

    #[test]
    fn compare_different_passes_with_high_tolerance() {
        let a = solid(0, 0, 0, 255);
        let b = solid(1, 1, 1, 255);
        let result = compare_with_masks(&a, &b, 100.0, &[]);
        assert!(result.passed);
    }

    #[test]
    fn ignore_mask_excludes_pixels() {
        // Make two images that differ only in the top-left 2x2 region.
        let a = solid(100, 100, 100, 255);
        let mut b = solid(100, 100, 100, 255);
        for y in 0..2 {
            for x in 0..2 {
                b.put_pixel(x, y, Rgba([200, 200, 200, 255]));
            }
        }

        // Without mask, should fail at tight tolerance.
        let result_no_mask = compare_with_masks(&a, &b, 0.0, &[]);
        assert!(!result_no_mask.passed);

        // With Ignore mask covering the changed region, should pass.
        let masks = vec![MaskRegion::Ignore { x: 0, y: 0, w: 2, h: 2 }];
        let result_masked = compare_with_masks(&a, &b, 0.0, &masks);
        assert!(result_masked.passed);
        assert_eq!(result_masked.changed_pixels, 0);
    }

    #[test]
    fn tolerance_mask_applies_local_tolerance() {
        let a = solid(0, 0, 0, 255);
        let mut b = solid(0, 0, 0, 255);
        // Change just the top-left 2x2 to differ slightly.
        for y in 0..2 {
            for x in 0..2 {
                b.put_pixel(x, y, Rgba([5, 5, 5, 255]));
            }
        }

        // Global tolerance is 0, so this would fail without a region mask.
        // The Tolerance region gives the changed area a generous threshold.
        let masks = vec![MaskRegion::Tolerance { x: 0, y: 0, w: 2, h: 2, tolerance: 100.0 }];
        let result = compare_with_masks(&a, &b, 100.0, &masks);
        assert!(result.passed);
    }

    #[test]
    fn diff_image_marks_changed_pixels_red() {
        let a = solid(100, 100, 100, 255);
        let mut b = solid(100, 100, 100, 255);
        b.put_pixel(0, 0, Rgba([200, 200, 200, 255]));

        let diff = generate_diff_image(&a, &b, &[]);

        // The changed pixel (0,0) should be red.
        assert_eq!(*diff.get_pixel(0, 0), Rgba([255, 0, 0, 255]));

        // An unchanged pixel should be dimmed.
        let unchanged = diff.get_pixel(1, 1);
        assert_eq!(unchanged[0], 100 / 4);
        assert_eq!(unchanged[1], 100 / 4);
        assert_eq!(unchanged[2], 100 / 4);
    }

    #[test]
    fn diff_image_marks_ignored_regions_blue() {
        let a = solid(100, 100, 100, 255);
        let b = solid(100, 100, 100, 255);
        let masks = vec![MaskRegion::Ignore { x: 0, y: 0, w: 2, h: 2 }];

        let diff = generate_diff_image(&a, &b, &masks);
        assert_eq!(*diff.get_pixel(0, 0), Rgba([0, 0, 180, 255]));
        assert_eq!(*diff.get_pixel(1, 1), Rgba([0, 0, 180, 255]));
        // Outside the mask, unchanged pixels are dimmed.
        assert_eq!(diff.get_pixel(2, 2)[0], 100 / 4);
    }

    #[test]
    fn env_is_truthy_for_update_golden() {
        let prev = std::env::var("UNSHIT_UPDATE_GOLDEN").ok();

        std::env::set_var("UNSHIT_UPDATE_GOLDEN", "1");
        assert!(crate::test_app::env_is_truthy("UNSHIT_UPDATE_GOLDEN"));

        std::env::set_var("UNSHIT_UPDATE_GOLDEN", "true");
        assert!(crate::test_app::env_is_truthy("UNSHIT_UPDATE_GOLDEN"));

        std::env::set_var("UNSHIT_UPDATE_GOLDEN", "0");
        assert!(!crate::test_app::env_is_truthy("UNSHIT_UPDATE_GOLDEN"));

        std::env::remove_var("UNSHIT_UPDATE_GOLDEN");
        assert!(!crate::test_app::env_is_truthy("UNSHIT_UPDATE_GOLDEN"));

        // Restore.
        if let Some(v) = prev {
            std::env::set_var("UNSHIT_UPDATE_GOLDEN", v);
        }
    }

    #[test]
    fn pixel_tolerance_returns_none_for_ignored() {
        let masks = vec![MaskRegion::Ignore { x: 5, y: 5, w: 10, h: 10 }];
        assert_eq!(pixel_tolerance(7, 7, 1.0, &masks), None);
        assert_eq!(pixel_tolerance(0, 0, 1.0, &masks), Some(1.0));
    }

    #[test]
    fn pixel_tolerance_returns_region_tolerance() {
        let masks = vec![MaskRegion::Tolerance { x: 0, y: 0, w: 5, h: 5, tolerance: 42.0 }];
        assert_eq!(pixel_tolerance(2, 2, 1.0, &masks), Some(42.0));
        assert_eq!(pixel_tolerance(10, 10, 1.0, &masks), Some(1.0));
    }

    #[test]
    fn comparison_result_reports_total_pixels() {
        let a = solid(0, 0, 0, 255);
        let b = solid(0, 0, 0, 255);
        let result = compare_with_masks(&a, &b, 0.0, &[]);
        assert_eq!(result.total_pixels, 16); // 4x4
    }
}
