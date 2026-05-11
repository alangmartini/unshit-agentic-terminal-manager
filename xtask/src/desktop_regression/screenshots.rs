use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelSampleRatios {
    pub bottom_lit_ratio: f64,
    pub mid_max_lit_ratio: f64,
}

#[cfg(target_os = "windows")]
pub fn capture_screen(path: &Path) -> Result<(), String> {
    use std::mem;
    use std::ptr::null_mut;

    use image::{ColorType, ImageFormat};
    use winapi::shared::windef::{HBITMAP, HGDIOBJ};
    use winapi::um::wingdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, RGBQUAD,
        SRCCOPY,
    };
    use winapi::um::winuser::{GetDC, GetSystemMetrics, ReleaseDC, SM_CXSCREEN, SM_CYSCREEN};

    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if width <= 0 || height <= 0 {
        return Err(format!("invalid screen size {width}x{height}"));
    }

    unsafe {
        let screen_dc = GetDC(null_mut());
        if screen_dc.is_null() {
            return Err("GetDC failed while capturing screenshot".to_owned());
        }

        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc.is_null() {
            ReleaseDC(null_mut(), screen_dc);
            return Err("CreateCompatibleDC failed while capturing screenshot".to_owned());
        }

        let bitmap: HBITMAP = CreateCompatibleBitmap(screen_dc, width, height);
        if bitmap.is_null() {
            DeleteDC(memory_dc);
            ReleaseDC(null_mut(), screen_dc);
            return Err("CreateCompatibleBitmap failed while capturing screenshot".to_owned());
        }

        let old = SelectObject(memory_dc, bitmap as HGDIOBJ);
        let copied = BitBlt(
            memory_dc,
            0,
            0,
            width,
            height,
            screen_dc,
            0,
            0,
            SRCCOPY | CAPTUREBLT,
        );
        if copied == 0 {
            SelectObject(memory_dc, old);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(memory_dc);
            ReleaseDC(null_mut(), screen_dc);
            return Err("BitBlt failed while capturing screenshot".to_owned());
        }

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbReserved: 0,
            }],
        };
        let mut bgra = vec![0_u8; (width as usize) * (height as usize) * 4];
        let rows = GetDIBits(
            memory_dc,
            bitmap,
            0,
            height as u32,
            bgra.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );

        SelectObject(memory_dc, old);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(memory_dc);
        ReleaseDC(null_mut(), screen_dc);

        if rows == 0 {
            return Err("GetDIBits failed while capturing screenshot".to_owned());
        }

        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }

        image::save_buffer_with_format(
            path,
            &bgra,
            width as u32,
            height as u32,
            ColorType::Rgba8,
            ImageFormat::Png,
        )
        .map_err(|e| format!("failed to write screenshot {}: {e}", path.display()))
    }
}

#[cfg(not(target_os = "windows"))]
pub fn capture_screen(_path: &Path) -> Result<(), String> {
    Err("desktop screenshot capture is only supported on Windows".to_owned())
}

pub fn stripe_lit_ratio_rgba(
    rgba: &[u8],
    bitmap_width: u32,
    bitmap_height: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    lit_sum_threshold: u16,
) -> f64 {
    if width <= 0 || height <= 0 || bitmap_width == 0 || bitmap_height == 0 {
        return 0.0;
    }
    let min_x = x.max(0) as u32;
    let min_y = y.max(0) as u32;
    let max_x = (x + width).min(bitmap_width as i32).max(0) as u32;
    let max_y = (y + height).min(bitmap_height as i32).max(0) as u32;
    if min_x >= max_x || min_y >= max_y {
        return 0.0;
    }

    let expected_len = bitmap_width as usize * bitmap_height as usize * 4;
    if rgba.len() < expected_len {
        return 0.0;
    }

    let mut lit = 0_u64;
    let mut total = 0_u64;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let offset = ((py * bitmap_width + px) as usize) * 4;
            let sum =
                u16::from(rgba[offset]) + u16::from(rgba[offset + 1]) + u16::from(rgba[offset + 2]);
            if sum >= lit_sum_threshold {
                lit += 1;
            }
            total += 1;
        }
    }

    if total == 0 {
        0.0
    } else {
        lit as f64 / total as f64
    }
}

pub fn max_stripe_lit_ratio_rgba(
    rgba: &[u8],
    bitmap_width: u32,
    bitmap_height: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    stripe_height: i32,
    step_px: i32,
    lit_sum_threshold: u16,
) -> f64 {
    if width <= 0 || height <= 0 || stripe_height <= 0 {
        return 0.0;
    }

    let mut max_lit = 0.0_f64;
    let end_y = y + height - stripe_height;
    let step = step_px.max(1) as usize;
    for stripe_y in (y..=end_y).step_by(step) {
        let lit = stripe_lit_ratio_rgba(
            rgba,
            bitmap_width,
            bitmap_height,
            x,
            stripe_y,
            width,
            stripe_height,
            lit_sum_threshold,
        );
        max_lit = max_lit.max(lit);
    }
    max_lit
}

#[cfg(target_os = "windows")]
pub fn sample_png_lit_ratios(
    path: &Path,
    bottom_sample: SampleRect,
    mid_sample: SampleRect,
    mid_stripe_height: i32,
    mid_step_px: i32,
) -> Result<PixelSampleRatios, String> {
    let image = image::ImageReader::open(path)
        .map_err(|e| format!("failed to open screenshot {}: {e}", path.display()))?
        .decode()
        .map_err(|e| format!("failed to decode screenshot {}: {e}", path.display()))?
        .to_rgba8();
    let (bitmap_width, bitmap_height) = image.dimensions();
    let rgba = image.as_raw();
    Ok(PixelSampleRatios {
        bottom_lit_ratio: stripe_lit_ratio_rgba(
            rgba,
            bitmap_width,
            bitmap_height,
            bottom_sample.x,
            bottom_sample.y,
            bottom_sample.width,
            bottom_sample.height,
            240,
        ),
        mid_max_lit_ratio: max_stripe_lit_ratio_rgba(
            rgba,
            bitmap_width,
            bitmap_height,
            mid_sample.x,
            mid_sample.y,
            mid_sample.width,
            mid_sample.height,
            mid_stripe_height,
            mid_step_px,
            240,
        ),
    })
}

#[cfg(not(target_os = "windows"))]
pub fn sample_png_lit_ratios(
    _path: &Path,
    _bottom_sample: SampleRect,
    _mid_sample: SampleRect,
    _mid_stripe_height: i32,
    _mid_step_px: i32,
) -> Result<PixelSampleRatios, String> {
    Err("desktop screenshot pixel sampling is only supported on Windows".to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripe_lit_ratio_counts_pixels_inside_clamped_region() {
        let rgba = [
            255, 255, 255, 255, 10, 10, 10, 255, 255, 0, 0, 255, 0, 0, 0, 255,
        ];

        let ratio = stripe_lit_ratio_rgba(&rgba, 2, 2, -1, 0, 3, 1, 240);

        assert_eq!(ratio, 0.5);
    }

    #[test]
    fn max_stripe_lit_ratio_scans_vertical_stripes() {
        let rgba = [
            255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0,
            0, 255,
        ];

        let ratio = max_stripe_lit_ratio_rgba(&rgba, 2, 3, 0, 0, 2, 3, 1, 1, 240);

        assert_eq!(ratio, 1.0);
    }
}
