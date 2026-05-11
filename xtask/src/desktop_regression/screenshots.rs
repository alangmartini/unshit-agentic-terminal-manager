use std::path::Path;

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
