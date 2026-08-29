//! Putting [`crate::splash`] commands on screen without a GPU.
//!
//! On Windows this is plain GDI into the window's own device context. That
//! sounds retrograde, and it is, but it is also the only rasterizer guaranteed
//! to be available before D3D12 has finished enumerating adapters -- which is
//! precisely the interval this exists to cover.
//!
//! Two Win32 details make it work at all:
//!
//! * winit registers its window class with a null `hbrBackground`
//!   (`winit-win32/src/window.rs`), so nothing erases the client area behind
//!   our back. Whatever we draw stays drawn.
//! * winit's `WM_PAINT` handler dispatches `RedrawRequested` and *then* calls
//!   `DefWindowProcW`, whose `BeginPaint`/`EndPaint` validates the update
//!   region. Painting from the `RedrawRequested` handler therefore neither
//!   loops nor leaves the window perpetually dirty.
//!
//! Everything is double-buffered through a memory DC: the command list is a
//! back-to-front overdraw, and blitting it a rectangle at a time straight to
//! the window is visibly a redraw rather than a frame.
//!
//! On every other platform this is a stub that reports "no surface", and
//! callers fall back to the previous behaviour of waiting for the GPU.

use crate::splash::{SplashCommand, SplashRect};
use unshit_core::style::types::Color;

/// What actually made it onto the screen.
///
/// A placeholder that silently paints nothing looks exactly like a placeholder
/// that was never asked to paint, and both look like a slow startup. GDI
/// reports failure per call and never explains itself, so the counts are the
/// only way to tell a broken painter from an empty command list -- and the
/// only signal a future session gets without reproducing the launch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintReport {
    pub fills: u32,
    pub fills_failed: u32,
    pub texts: u32,
    pub texts_failed: u32,
    pub backdrop_failed: bool,
    pub blit_failed: bool,
    /// Set when the paint returned before drawing anything, naming the reason.
    pub gave_up: &'static str,
}

impl PaintReport {
    /// True when nothing about this paint needs looking into.
    pub fn is_clean(&self) -> bool {
        self.gave_up.is_empty()
            && !self.backdrop_failed
            && !self.blit_failed
            && self.fills_failed == 0
            && self.texts_failed == 0
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use winapi::ctypes::c_void;
    use winapi::shared::minwindef::DWORD;
    use winapi::shared::windef::{COLORREF, HBITMAP, HDC, HFONT, HGDIOBJ, HWND, RECT};
    use winapi::um::wingdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
        GdiFlush, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        CLEARTYPE_QUALITY, DEFAULT_CHARSET, DIB_RGB_COLORS, FF_DONTCARE, FW_NORMAL, OUT_TT_PRECIS,
        SRCCOPY, TRANSPARENT,
    };
    use winapi::um::winuser::{
        DrawTextW, GetDC, ReleaseDC, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    };

    /// GDI wants 0x00BBGGRR, which is the reverse of how everyone writes colors.
    fn colorref(c: Color) -> COLORREF {
        (c.r as u32) | ((c.g as u32) << 8) | ((c.b as u32) << 16)
    }

    fn rect(r: SplashRect) -> RECT {
        RECT { left: r.x, top: r.y, right: r.x + r.width, bottom: r.y + r.height }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// A window we can paint into, plus the GDI objects worth keeping.
    ///
    /// Fonts are cached because a real frame is dozens of text runs over two
    /// or three sizes, and `CreateFontW` per run is the kind of thing that
    /// turns a 2ms paint into a 20ms one. Brushes are not cached because
    /// there are none: fills are blended by hand, which GDI cannot do.
    pub struct SplashSurface {
        hwnd: HWND,
        /// Back buffer, sized to the client area. Rebuilt on resize.
        back: Option<BackBuffer>,
        fonts: HashMap<i32, HFONT>,
        face: Vec<u16>,
    }

    /// A bitmap two things can draw into.
    ///
    /// It is a DIB section rather than a compatible bitmap so that the pixels
    /// are addressable: fills need real alpha compositing, which GDI has no
    /// primitive for at this granularity. Text still goes through GDI, into
    /// the same pixels, via the DC the bitmap is selected into.
    struct BackBuffer {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        /// `width * height` BGRA pixels, top row first.
        bits: *mut u32,
        width: i32,
        height: i32,
    }

    impl BackBuffer {
        fn new(window_dc: HDC, width: i32, height: i32) -> Option<BackBuffer> {
            // SAFETY: `window_dc` is a live DC from `GetDC` on a window the
            // caller holds for the whole call. The DC and the DIB section are
            // released together in `destroy`, and `biHeight` is negated to get
            // a top-down bitmap so `bits` can be indexed as `y * width + x`
            // rather than upside down.
            unsafe {
                let dc = CreateCompatibleDC(window_dc);
                if dc.is_null() {
                    return None;
                }
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as DWORD,
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
                };
                let mut bits: *mut c_void = std::ptr::null_mut();
                let bitmap =
                    CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
                if bitmap.is_null() || bits.is_null() {
                    if !bitmap.is_null() {
                        DeleteObject(bitmap as HGDIOBJ);
                    }
                    DeleteDC(dc);
                    return None;
                }
                let previous = SelectObject(dc, bitmap as HGDIOBJ);
                Some(BackBuffer { dc, bitmap, previous, bits: bits as *mut u32, width, height })
            }
        }

        /// Paint every pixel the given color, ignoring what was there.
        fn clear(&mut self, color: Color) {
            let packed = ((color.r as u32) << 16) | ((color.g as u32) << 8) | (color.b as u32);
            // SAFETY: `bits` addresses exactly `width * height` u32s, which is
            // the length of this slice.
            let pixels = unsafe {
                std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize)
            };
            pixels.fill(packed);
        }

        /// Composite a color over a rectangle.
        ///
        /// Returns false when the rectangle lands entirely outside the buffer,
        /// which is a caller bug worth reporting rather than a no-op worth
        /// hiding -- every command is supposed to arrive pre-clipped.
        fn blend(&mut self, r: SplashRect, color: Color) -> bool {
            let x0 = r.x.max(0);
            let y0 = r.y.max(0);
            let x1 = (r.x + r.width).min(self.width);
            let y1 = (r.y + r.height).min(self.height);
            if x0 >= x1 || y0 >= y1 {
                return false;
            }
            let a = color.a as u32;
            if a == 0 {
                return true;
            }
            // SAFETY: as in `clear`.
            let pixels = unsafe {
                std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize)
            };
            let stride = self.width as usize;

            if a == 255 {
                let packed = ((color.r as u32) << 16) | ((color.g as u32) << 8) | (color.b as u32);
                for y in y0..y1 {
                    let row = y as usize * stride;
                    pixels[row + x0 as usize..row + x1 as usize].fill(packed);
                }
                return true;
            }

            let inv = 255 - a;
            // Round-to-nearest rather than truncate: a 3%-alpha wash applied
            // over a dark background truncates to no change at all, which is
            // how a deliberate tint becomes invisible.
            let mix = |src: u32, dst: u32| -> u32 { (src * a + dst * inv + 127) / 255 };
            let (sr, sg, sb) = (color.r as u32, color.g as u32, color.b as u32);
            for y in y0..y1 {
                let row = y as usize * stride;
                for px in &mut pixels[row + x0 as usize..row + x1 as usize] {
                    let dr = (*px >> 16) & 0xFF;
                    let dg = (*px >> 8) & 0xFF;
                    let db = *px & 0xFF;
                    *px = (mix(sr, dr) << 16) | (mix(sg, dg) << 8) | mix(sb, db);
                }
            }
            true
        }

        fn destroy(&mut self) {
            // SAFETY: every handle here was created by `new` and is deleted
            // exactly once; the bitmap is deselected first because a bitmap
            // still selected into a DC cannot be deleted.
            unsafe {
                SelectObject(self.dc, self.previous);
                DeleteObject(self.bitmap as HGDIOBJ);
                DeleteDC(self.dc);
            }
        }
    }

    impl SplashSurface {
        /// Wrap a live window. `face` is the font family to approximate text
        /// with; GDI substitutes if it is not installed, which is fine because
        /// this is a placeholder and not the frame.
        pub fn new(hwnd: isize, face: &str) -> Option<SplashSurface> {
            let hwnd = hwnd as HWND;
            if hwnd.is_null() {
                return None;
            }
            Some(SplashSurface {
                hwnd,
                back: None,
                fonts: HashMap::new(),
                face: {
                    let mut w = wide(face);
                    w.push(0);
                    w
                },
            })
        }

        fn font(&mut self, px: i32) -> HFONT {
            let face = self.face.clone();
            *self.fonts.entry(px).or_insert_with(|| {
                // SAFETY: `face` is NUL-terminated UTF-16 that outlives the
                // call; `CreateFontW` copies the name.
                unsafe {
                    CreateFontW(
                        // Negative height asks for a character height (the em
                        // size CSS means) rather than a cell height.
                        -px,
                        0,
                        0,
                        0,
                        FW_NORMAL,
                        0,
                        0,
                        0,
                        DEFAULT_CHARSET,
                        OUT_TT_PRECIS,
                        0,
                        CLEARTYPE_QUALITY,
                        FF_DONTCARE,
                        face.as_ptr(),
                    )
                }
            })
        }

        /// Draw one frame of the placeholder.
        ///
        /// `size` is the client area in physical pixels; commands outside it
        /// were already clipped by the collector, and the backdrop covers
        /// whatever the commands do not.
        ///
        /// Fills happen first, in order, then text -- rather than strictly
        /// interleaved. Two different rasterizers are writing to one buffer
        /// and GDI batches its work, so interleaving would mean flushing
        /// between every command. Text almost always sits on top of its own
        /// element's background anyway, so the visible difference is nil.
        pub fn paint(
            &mut self,
            commands: &[SplashCommand],
            size: (u32, u32),
            backdrop: Color,
        ) -> PaintReport {
            let mut report = PaintReport::default();
            let (w, h) = (size.0 as i32, size.1 as i32);
            if w <= 0 || h <= 0 {
                report.gave_up = "empty_size";
                return report;
            }

            // SAFETY: the window is alive for the duration of this call (it is
            // owned by the caller's event loop), and every DC acquired here is
            // released before returning.
            unsafe {
                let window_dc = GetDC(self.hwnd);
                if window_dc.is_null() {
                    report.gave_up = "no_window_dc";
                    return report;
                }

                let stale = self.back.as_ref().is_none_or(|b| b.width != w || b.height != h);
                if stale {
                    if let Some(mut old) = self.back.take() {
                        old.destroy();
                    }
                    self.back = BackBuffer::new(window_dc, w, h);
                }
                if self.back.is_none() {
                    ReleaseDC(self.hwnd, window_dc);
                    report.gave_up = "no_back_buffer";
                    return report;
                }

                // GDI batches; the bits must not be touched while it still has
                // queued work against them.
                GdiFlush();

                {
                    let back = self.back.as_mut().expect("checked just above");
                    back.clear(backdrop);
                    for command in commands {
                        if let SplashCommand::Fill { rect: r, color } = command {
                            report.fills += 1;
                            if !back.blend(*r, *color) {
                                report.fills_failed += 1;
                            }
                        }
                    }
                }

                let dc = self.back.as_ref().expect("checked just above").dc;
                SetBkMode(dc, TRANSPARENT as i32);
                for command in commands {
                    if let SplashCommand::Text { rect: r, color, font_size, text } = command {
                        report.texts += 1;
                        // Text is drawn by GDI, which has no alpha. A run
                        // faint enough to be invisible is skipped rather than
                        // drawn at full strength, which would be louder than
                        // the real frame rather than merely different.
                        if color.a < 24 {
                            continue;
                        }
                        let px = font_size.round().clamp(1.0, 400.0) as i32;
                        let font = self.font(px);
                        if font.is_null() {
                            report.texts_failed += 1;
                            continue;
                        }
                        let previous = SelectObject(dc, font as HGDIOBJ);
                        SetTextColor(dc, colorref(*color));
                        let mut buffer = wide(text);
                        let mut bounds = rect(*r);
                        if DrawTextW(
                            dc,
                            buffer.as_mut_ptr(),
                            buffer.len() as i32,
                            &mut bounds,
                            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
                        ) == 0
                        {
                            report.texts_failed += 1;
                        }
                        SelectObject(dc, previous);
                    }
                }

                if BitBlt(window_dc, 0, 0, w, h, dc, 0, 0, SRCCOPY) == 0 {
                    report.blit_failed = true;
                }
                ReleaseDC(self.hwnd, window_dc);
            }
            report
        }
    }

    impl Drop for SplashSurface {
        fn drop(&mut self) {
            if let Some(mut back) = self.back.take() {
                back.destroy();
            }
            // SAFETY: every cached handle was created by this type and is
            // deleted exactly once, here, after the back buffer that could
            // have them selected is gone.
            unsafe {
                for (_, font) in self.fonts.drain() {
                    DeleteObject(font as HGDIOBJ);
                }
            }
        }
    }
}
#[cfg(not(windows))]
mod imp {
    use super::*;

    /// Placeholder painting is Windows-only for now: it exists to cover D3D12
    /// adapter enumeration, which is a Windows cost. Elsewhere the app keeps
    /// its previous behaviour of showing the window once it can draw.
    pub struct SplashSurface {
        _private: (),
    }

    impl SplashSurface {
        pub fn new(_hwnd: isize, _face: &str) -> Option<SplashSurface> {
            None
        }

        pub fn paint(
            &mut self,
            _commands: &[SplashCommand],
            _size: (u32, u32),
            _backdrop: Color,
        ) -> PaintReport {
            PaintReport { gave_up: "unsupported_platform", ..PaintReport::default() }
        }
    }
}

pub use imp::SplashSurface;

/// The window handle a [`SplashSurface`] needs, or `None` when the platform
/// does not have one we can paint into.
pub fn window_handle(window: &dyn winit::window::Window) -> Option<isize> {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}
