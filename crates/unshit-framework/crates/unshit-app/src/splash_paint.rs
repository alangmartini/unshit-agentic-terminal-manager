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

#[cfg(windows)]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use winapi::shared::windef::{COLORREF, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, HWND, RECT};
    use winapi::um::wingdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
        DeleteDC, DeleteObject, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY,
        DEFAULT_CHARSET, FF_DONTCARE, FW_NORMAL, OUT_TT_PRECIS, SRCCOPY, TRANSPARENT,
    };
    use winapi::um::winuser::{
        DrawTextW, FillRect, GetDC, ReleaseDC, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
        DT_VCENTER,
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
    /// Brushes and fonts are cached because a real frame is hundreds of
    /// rectangles over a handful of distinct colors, and `CreateSolidBrush` per
    /// rectangle is the kind of thing that turns a 2ms paint into a 20ms one.
    pub struct SplashSurface {
        hwnd: HWND,
        /// Back buffer, sized to the client area. Rebuilt on resize.
        back: Option<BackBuffer>,
        brushes: HashMap<COLORREF, HBRUSH>,
        fonts: HashMap<i32, HFONT>,
        face: Vec<u16>,
    }

    struct BackBuffer {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        width: i32,
        height: i32,
    }

    impl BackBuffer {
        fn new(window_dc: HDC, width: i32, height: i32) -> Option<BackBuffer> {
            // SAFETY: `window_dc` is a live DC obtained from `GetDC` on a window
            // that is still alive (the caller holds it for the whole call), and
            // the compatible DC/bitmap pair is released together in `destroy`.
            unsafe {
                let dc = CreateCompatibleDC(window_dc);
                if dc.is_null() {
                    return None;
                }
                let bitmap = CreateCompatibleBitmap(window_dc, width, height);
                if bitmap.is_null() {
                    DeleteDC(dc);
                    return None;
                }
                let previous = SelectObject(dc, bitmap as HGDIOBJ);
                Some(BackBuffer { dc, bitmap, previous, width, height })
            }
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
                brushes: HashMap::new(),
                fonts: HashMap::new(),
                face: {
                    let mut w = wide(face);
                    w.push(0);
                    w
                },
            })
        }

        fn brush(&mut self, color: COLORREF) -> HBRUSH {
            *self.brushes.entry(color).or_insert_with(||
                // SAFETY: `CreateSolidBrush` cannot fail for a valid COLORREF;
                // a null return is handled by `FillRect` becoming a no-op.
                unsafe { CreateSolidBrush(color) })
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
        pub fn paint(&mut self, commands: &[SplashCommand], size: (u32, u32), backdrop: Color) {
            let (w, h) = (size.0 as i32, size.1 as i32);
            if w <= 0 || h <= 0 {
                return;
            }

            // SAFETY: the window is alive for the duration of this call (it is
            // owned by the caller's event loop), and every DC acquired here is
            // released before returning.
            unsafe {
                let window_dc = GetDC(self.hwnd);
                if window_dc.is_null() {
                    return;
                }

                let stale = self.back.as_ref().is_none_or(|b| b.width != w || b.height != h);
                if stale {
                    if let Some(mut old) = self.back.take() {
                        old.destroy();
                    }
                    self.back = BackBuffer::new(window_dc, w, h);
                }
                let Some(back) = self.back.as_ref() else {
                    ReleaseDC(self.hwnd, window_dc);
                    return;
                };
                let dc = back.dc;

                let full = RECT { left: 0, top: 0, right: w, bottom: h };
                let clear = self.brush(colorref(backdrop));
                FillRect(dc, &full, clear);

                SetBkMode(dc, TRANSPARENT as i32);
                for command in commands {
                    match command {
                        SplashCommand::Fill { rect: r, color } => {
                            let brush = self.brush(colorref(*color));
                            FillRect(dc, &rect(*r), brush);
                        }
                        SplashCommand::Text { rect: r, color, font_size, text } => {
                            let px = font_size.round().clamp(1.0, 400.0) as i32;
                            let font = self.font(px);
                            let previous = SelectObject(dc, font as HGDIOBJ);
                            SetTextColor(dc, colorref(*color));
                            let mut buffer = wide(text);
                            let mut bounds = rect(*r);
                            DrawTextW(
                                dc,
                                buffer.as_mut_ptr(),
                                buffer.len() as i32,
                                &mut bounds,
                                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
                            );
                            SelectObject(dc, previous);
                        }
                    }
                }

                BitBlt(window_dc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
                ReleaseDC(self.hwnd, window_dc);
            }
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
                for (_, brush) in self.brushes.drain() {
                    DeleteObject(brush as HGDIOBJ);
                }
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

        pub fn paint(&mut self, _commands: &[SplashCommand], _size: (u32, u32), _backdrop: Color) {}
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
