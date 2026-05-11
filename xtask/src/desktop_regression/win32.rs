use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl DesktopRect {
    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopSize {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowHandle(pub isize);

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::thread;
    use std::time::{Duration, Instant};

    use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, TRUE};
    use winapi::shared::windef::{HWND, RECT};
    use winapi::um::winuser::{
        mouse_event, CloseWindow, EnumWindows, GetClassNameW, GetSystemMetrics, GetWindowRect,
        GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, SetCursorPos,
        SetForegroundWindow, SetProcessDPIAware, SetProcessDpiAwarenessContext, SetWindowPos,
        ShowWindow, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, SM_CXSCREEN, SM_CYSCREEN,
        SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_RESTORE, WM_CLOSE,
    };
    use winapi::um::winuser::{PostMessageW, HWND_TOP};

    use super::{DesktopRect, DesktopSize, WindowHandle};

    pub fn find_window_for_process(
        process_id: u32,
        timeout: Duration,
        title_substrings: &[&str],
        class_substrings: &[&str],
    ) -> Result<WindowHandle, String> {
        enable_dpi_awareness();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let windows = enumerate_process_windows(process_id);
            if let Some(window) = windows
                .iter()
                .copied()
                .find(|hwnd| matches_expected(*hwnd, title_substrings, class_substrings))
            {
                return Ok(WindowHandle(window as isize));
            }
            if let Some(window) = windows.first().copied() {
                return Ok(WindowHandle(window as isize));
            }
            thread::sleep(Duration::from_millis(100));
        }

        Err(format!("window did not appear for pid={process_id}"))
    }

    pub fn screen_size() -> Result<DesktopSize, String> {
        enable_dpi_awareness();
        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        if width <= 0 || height <= 0 {
            return Err(format!("invalid screen size {width}x{height}"));
        }

        Ok(DesktopSize { width, height })
    }

    pub fn set_window_rect(
        handle: WindowHandle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let ok = unsafe {
            SetWindowPos(
                hwnd(handle),
                HWND_TOP,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
            )
        };
        if ok == 0 {
            return Err("SetWindowPos failed".to_owned());
        }

        Ok(())
    }

    pub fn get_window_rect(handle: WindowHandle) -> Result<DesktopRect, String> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe { GetWindowRect(hwnd(handle), &mut rect) };
        if ok == 0 {
            return Err("GetWindowRect failed".to_owned());
        }

        Ok(DesktopRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    pub fn focus_window(handle: WindowHandle) -> Result<(), String> {
        let rect = get_window_rect(handle)?;
        let click_x = (rect.left + rect.right) / 2;
        let click_y = rect.top + 8;
        unsafe {
            ShowWindow(hwnd(handle), SW_RESTORE);
            SetCursorPos(click_x, click_y);
            thread::sleep(Duration::from_millis(50));
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
            thread::sleep(Duration::from_millis(30));
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
            thread::sleep(Duration::from_millis(200));
            SetForegroundWindow(hwnd(handle));
        }
        thread::sleep(Duration::from_millis(250));

        Ok(())
    }

    pub fn left_edge_drag(
        _handle: WindowHandle,
        from_x: i32,
        from_y: i32,
        to_x: i32,
    ) -> Result<(), String> {
        if from_x == to_x {
            return Err("invalid drag distance".to_owned());
        }

        unsafe {
            SetCursorPos(from_x, from_y);
            thread::sleep(Duration::from_millis(40));
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        }

        let direction = if to_x > from_x { 1 } else { -1 };
        let mut x = from_x;
        loop {
            unsafe {
                SetCursorPos(x, from_y);
            }
            thread::sleep(Duration::from_millis(6));
            if (direction == 1 && x >= to_x) || (direction == -1 && x <= to_x) {
                break;
            }
            x += 5 * direction;
        }

        unsafe {
            SetCursorPos(to_x, from_y);
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
        }
        thread::sleep(Duration::from_millis(250));

        Ok(())
    }

    pub fn close_window(handle: WindowHandle) -> Result<(), String> {
        unsafe {
            PostMessageW(hwnd(handle), WM_CLOSE, 0, 0);
            CloseWindow(hwnd(handle));
        }
        Ok(())
    }

    fn enumerate_process_windows(process_id: u32) -> Vec<HWND> {
        struct State {
            process_id: u32,
            windows: Vec<HWND>,
        }

        unsafe extern "system" fn enum_proc(window: HWND, lparam: LPARAM) -> BOOL {
            let state = &mut *(lparam as *mut State);
            let mut owner_pid: DWORD = 0;
            GetWindowThreadProcessId(window, &mut owner_pid);
            if owner_pid == state.process_id && IsWindowVisible(window) != 0 {
                state.windows.push(window);
            }
            TRUE
        }

        let mut state = State {
            process_id,
            windows: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(enum_proc), &mut state as *mut State as LPARAM);
        }
        state.windows
    }

    fn matches_expected(
        window: HWND,
        title_substrings: &[&str],
        class_substrings: &[&str],
    ) -> bool {
        let title = window_text(window).to_ascii_lowercase();
        let class = window_class(window).to_ascii_lowercase();
        title_substrings
            .iter()
            .any(|expected| title.contains(&expected.to_ascii_lowercase()))
            || class_substrings
                .iter()
                .any(|expected| class.contains(&expected.to_ascii_lowercase()))
    }

    fn window_text(window: HWND) -> String {
        let mut buf = [0_u16; 512];
        let len = unsafe { GetWindowTextW(window, buf.as_mut_ptr(), buf.len() as i32) };
        OsString::from_wide(&buf[..len.max(0) as usize])
            .to_string_lossy()
            .into_owned()
    }

    fn window_class(window: HWND) -> String {
        let mut buf = [0_u16; 256];
        let len = unsafe { GetClassNameW(window, buf.as_mut_ptr(), buf.len() as i32) };
        OsString::from_wide(&buf[..len.max(0) as usize])
            .to_string_lossy()
            .into_owned()
    }

    fn hwnd(handle: WindowHandle) -> HWND {
        handle.0 as HWND
    }

    fn enable_dpi_awareness() {
        unsafe {
            let per_monitor_v2 = (-4_isize) as _;
            let per_monitor = (-3_isize) as _;
            if SetProcessDpiAwarenessContext(per_monitor_v2) != 0 {
                return;
            }
            if SetProcessDpiAwarenessContext(per_monitor) != 0 {
                return;
            }
            let _ = SetProcessDPIAware();
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use std::time::Duration;

    use super::{DesktopRect, DesktopSize, WindowHandle};

    pub fn find_window_for_process(
        _process_id: u32,
        _timeout: Duration,
        _title_substrings: &[&str],
        _class_substrings: &[&str],
    ) -> Result<WindowHandle, String> {
        Err(unsupported())
    }

    pub fn screen_size() -> Result<DesktopSize, String> {
        Err(unsupported())
    }

    pub fn set_window_rect(
        _handle: WindowHandle,
        _x: i32,
        _y: i32,
        _width: i32,
        _height: i32,
    ) -> Result<(), String> {
        Err(unsupported())
    }

    pub fn get_window_rect(_handle: WindowHandle) -> Result<DesktopRect, String> {
        Err(unsupported())
    }

    pub fn focus_window(_handle: WindowHandle) -> Result<(), String> {
        Err(unsupported())
    }

    pub fn left_edge_drag(
        _handle: WindowHandle,
        _from_x: i32,
        _from_y: i32,
        _to_x: i32,
    ) -> Result<(), String> {
        Err(unsupported())
    }

    pub fn close_window(_handle: WindowHandle) -> Result<(), String> {
        Err(unsupported())
    }

    fn unsupported() -> String {
        "desktop regression execution is only supported on Windows".to_owned()
    }
}

pub fn find_window_for_process(
    process_id: u32,
    timeout: Duration,
    title_substrings: &[&str],
    class_substrings: &[&str],
) -> Result<WindowHandle, String> {
    imp::find_window_for_process(process_id, timeout, title_substrings, class_substrings)
}

pub fn screen_size() -> Result<DesktopSize, String> {
    imp::screen_size()
}

pub fn set_window_rect(
    handle: WindowHandle,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    imp::set_window_rect(handle, x, y, width, height)
}

pub fn get_window_rect(handle: WindowHandle) -> Result<DesktopRect, String> {
    imp::get_window_rect(handle)
}

pub fn focus_window(handle: WindowHandle) -> Result<(), String> {
    imp::focus_window(handle)
}

pub fn left_edge_drag(
    handle: WindowHandle,
    from_x: i32,
    from_y: i32,
    to_x: i32,
) -> Result<(), String> {
    imp::left_edge_drag(handle, from_x, from_y, to_x)
}

pub fn close_window(handle: WindowHandle) -> Result<(), String> {
    imp::close_window(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_rect_reports_width_and_height() {
        let rect = DesktopRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 70,
        };

        assert_eq!(rect.width(), 100);
        assert_eq!(rect.height(), 50);
    }
}
