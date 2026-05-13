use std::time::Duration;

const MIN_OCCLUDER_WIDTH_PX: i32 = 2;
const MIN_OCCLUDER_HEIGHT_PX: i32 = 2;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowOcclusionCandidate {
    pub handle: WindowHandle,
    pub rect: DesktopRect,
    pub visible: bool,
    pub owned: bool,
}

impl WindowOcclusionCandidate {
    fn occludes(self, target_rect: DesktopRect) -> bool {
        self.visible
            && !self.owned
            && self.rect.width() >= MIN_OCCLUDER_WIDTH_PX
            && self.rect.height() >= MIN_OCCLUDER_HEIGHT_PX
            && rects_overlap(target_rect, self.rect)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapCaptureReadinessError {
    ForegroundStolen {
        foreground: Option<WindowHandle>,
    },
    StuckModifier {
        modifier: &'static str,
    },
    WindowOccluded {
        occluder: WindowOcclusionCandidate,
    },
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    Unsupported,
}

impl SnapCaptureReadinessError {
    pub fn first_bad_signal(self) -> &'static str {
        match self {
            Self::ForegroundStolen { .. } => "snap-foreground-stolen",
            Self::StuckModifier { .. } => "snap-stuck-modifier",
            Self::WindowOccluded { .. } => "snap-window-occluded",
            Self::Unsupported => "snap-composition-unsupported",
        }
    }

    pub fn message(self) -> String {
        match self {
            Self::ForegroundStolen { foreground } => {
                format!("foreground window changed before post-snap capture: {foreground:?}")
            }
            Self::StuckModifier { modifier } => {
                format!("modifier key remained pressed after Win+Left: {modifier}")
            }
            Self::WindowOccluded { occluder } => {
                format!("post-snap window is obscured by {occluder:?}")
            }
            Self::Unsupported => "snap composition checks are only supported on Windows".to_owned(),
        }
    }
}

pub fn rects_overlap(a: DesktopRect, b: DesktopRect) -> bool {
    a.width() > 0
        && a.height() > 0
        && b.width() > 0
        && b.height() > 0
        && a.left < b.right
        && a.right > b.left
        && a.top < b.bottom
        && a.bottom > b.top
}

#[cfg(test)]
fn first_occluding_window_before_target(
    target: WindowHandle,
    target_rect: DesktopRect,
    z_order_top_to_bottom: &[WindowOcclusionCandidate],
) -> Option<WindowOcclusionCandidate> {
    z_order_top_to_bottom
        .iter()
        .copied()
        .take_while(|candidate| candidate.handle != target)
        .find(|candidate| candidate.occludes(target_rect))
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::thread;
    use std::time::{Duration, Instant};

    use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, TRUE};
    use winapi::shared::windef::{HWND, POINT, RECT};
    use winapi::um::winuser::{
        keybd_event, mouse_event, ClientToScreen, CloseWindow, EnumWindows, GetClassNameW,
        GetDpiForWindow, GetForegroundWindow, GetKeyState, GetSystemMetrics, GetWindow,
        GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, SendInput,
        SetCursorPos, SetForegroundWindow, SetProcessDPIAware, SetProcessDpiAwarenessContext,
        SetWindowPos, ShowWindow, GW_HWNDFIRST, GW_HWNDNEXT, GW_OWNER, INPUT, INPUT_KEYBOARD,
        KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_RESTORE,
        VK_CONTROL, VK_LEFT, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT, WM_CLOSE,
    };
    use winapi::um::winuser::{GetClientRect, IsZoomed, PostMessageW, HWND_TOP};

    use super::{
        DesktopRect, DesktopSize, SnapCaptureReadinessError, WindowHandle, WindowOcclusionCandidate,
    };

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

    pub fn get_client_rect(handle: WindowHandle) -> Result<DesktopRect, String> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe { GetClientRect(hwnd(handle), &mut rect) };
        if ok == 0 {
            return Err("GetClientRect failed".to_owned());
        }

        let mut top_left = POINT {
            x: rect.left,
            y: rect.top,
        };
        let mut bottom_right = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        let top_left_ok = unsafe { ClientToScreen(hwnd(handle), &mut top_left) };
        let bottom_right_ok = unsafe { ClientToScreen(hwnd(handle), &mut bottom_right) };
        if top_left_ok == 0 || bottom_right_ok == 0 {
            return Err("ClientToScreen failed".to_owned());
        }

        Ok(DesktopRect {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        })
    }

    pub fn window_scale_factor(handle: WindowHandle) -> Result<f64, String> {
        let dpi = unsafe { GetDpiForWindow(hwnd(handle)) };
        if dpi == 0 {
            return Err("GetDpiForWindow returned 0".to_owned());
        }

        Ok(dpi as f64 / 96.0)
    }

    pub fn is_window_maximized(handle: WindowHandle) -> Result<bool, String> {
        Ok(unsafe { IsZoomed(hwnd(handle)) != 0 })
    }

    pub fn focus_window(handle: WindowHandle) -> Result<(), String> {
        let rect = get_window_rect(handle)?;
        let click_x = (rect.left + rect.right) / 2;
        let click_y = rect.top + 8;
        show_and_click(handle, click_x, click_y)
    }

    pub fn mouse_click(x: i32, y: i32, button: Option<&str>) -> Result<(), String> {
        if !button
            .map(|value| value.eq_ignore_ascii_case("left"))
            .unwrap_or(true)
        {
            return Err(format!(
                "unsupported mouse button '{}'",
                button.unwrap_or_default()
            ));
        }
        unsafe {
            SetCursorPos(x, y);
            thread::sleep(Duration::from_millis(50));
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
            thread::sleep(Duration::from_millis(30));
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
        }
        thread::sleep(Duration::from_millis(250));

        Ok(())
    }

    fn show_and_click(handle: WindowHandle, click_x: i32, click_y: i32) -> Result<(), String> {
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

    pub fn send_win_left() -> Result<(), String> {
        unsafe {
            keybd_event(VK_LWIN as u8, 0, 0, 0);
            thread::sleep(Duration::from_millis(30));
            keybd_event(VK_LEFT as u8, 0, 0, 0);
            thread::sleep(Duration::from_millis(30));
            keybd_event(VK_LEFT as u8, 0, KEYEVENTF_KEYUP, 0);
            thread::sleep(Duration::from_millis(30));
            keybd_event(VK_LWIN as u8, 0, KEYEVENTF_KEYUP, 0);
        }
        Ok(())
    }

    pub fn verify_snap_capture_ready(
        handle: WindowHandle,
        post_rect: DesktopRect,
    ) -> Result<(), SnapCaptureReadinessError> {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != hwnd(handle) {
            return Err(SnapCaptureReadinessError::ForegroundStolen {
                foreground: non_null_window_handle(foreground),
            });
        }

        if let Some(modifier) = stuck_modifier() {
            return Err(SnapCaptureReadinessError::StuckModifier { modifier });
        }

        if let Some(occluder) = first_occluding_window(handle, post_rect) {
            return Err(SnapCaptureReadinessError::WindowOccluded { occluder });
        }

        Ok(())
    }

    pub fn send_text_enter(text: &str) -> Result<(), String> {
        for unit in text.encode_utf16() {
            send_keyboard_input(0, unit, KEYEVENTF_UNICODE)?;
            send_keyboard_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)?;
            thread::sleep(Duration::from_millis(2));
        }
        send_keyboard_input(VK_RETURN as u16, 0, 0)?;
        send_keyboard_input(VK_RETURN as u16, 0, KEYEVENTF_KEYUP)?;
        Ok(())
    }

    fn send_keyboard_input(vk: u16, scan: u16, flags: u32) -> Result<(), String> {
        let mut input = unsafe { std::mem::zeroed::<INPUT>() };
        input.type_ = INPUT_KEYBOARD;
        unsafe {
            *input.u.ki_mut() = KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            };
            let sent = SendInput(1, &mut input, std::mem::size_of::<INPUT>() as i32);
            if sent != 1 {
                return Err("SendInput failed".to_owned());
            }
        }
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

    fn first_occluding_window(
        target: WindowHandle,
        target_rect: DesktopRect,
    ) -> Option<WindowOcclusionCandidate> {
        let mut current = unsafe { GetWindow(hwnd(target), GW_HWNDFIRST) };
        for _ in 0..1024 {
            if current.is_null() || current == hwnd(target) {
                return None;
            }
            if let Some(candidate) = window_occlusion_candidate(current) {
                if candidate.occludes(target_rect) {
                    return Some(candidate);
                }
            }
            current = unsafe { GetWindow(current, GW_HWNDNEXT) };
        }

        None
    }

    fn window_occlusion_candidate(window: HWND) -> Option<WindowOcclusionCandidate> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe { GetWindowRect(window, &mut rect) };
        if ok == 0 {
            return None;
        }

        Some(WindowOcclusionCandidate {
            handle: WindowHandle(window as isize),
            rect: DesktopRect {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            visible: unsafe { IsWindowVisible(window) != 0 },
            owned: unsafe { !GetWindow(window, GW_OWNER).is_null() },
        })
    }

    fn stuck_modifier() -> Option<&'static str> {
        const MODIFIERS: &[(i32, &str)] = &[
            (VK_LWIN, "win"),
            (VK_RWIN, "win"),
            (VK_CONTROL, "ctrl"),
            (VK_SHIFT, "shift"),
            (VK_MENU, "alt"),
        ];

        MODIFIERS
            .iter()
            .copied()
            .find_map(|(vk, name)| modifier_is_down(vk).then_some(name))
    }

    fn modifier_is_down(vk: i32) -> bool {
        unsafe { (GetKeyState(vk) as u16 & 0x8000) != 0 }
    }

    fn non_null_window_handle(window: HWND) -> Option<WindowHandle> {
        (!window.is_null()).then_some(WindowHandle(window as isize))
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

    use super::{DesktopRect, DesktopSize, SnapCaptureReadinessError, WindowHandle};

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

    pub fn get_client_rect(_handle: WindowHandle) -> Result<DesktopRect, String> {
        Err(unsupported())
    }

    pub fn window_scale_factor(_handle: WindowHandle) -> Result<f64, String> {
        Err(unsupported())
    }

    pub fn is_window_maximized(_handle: WindowHandle) -> Result<bool, String> {
        Err(unsupported())
    }

    pub fn focus_window(_handle: WindowHandle) -> Result<(), String> {
        Err(unsupported())
    }

    pub fn mouse_click(_x: i32, _y: i32, _button: Option<&str>) -> Result<(), String> {
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

    pub fn send_win_left() -> Result<(), String> {
        Err(unsupported())
    }

    pub fn verify_snap_capture_ready(
        _handle: WindowHandle,
        _post_rect: DesktopRect,
    ) -> Result<(), SnapCaptureReadinessError> {
        Err(SnapCaptureReadinessError::Unsupported)
    }

    pub fn send_text_enter(_text: &str) -> Result<(), String> {
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

pub fn get_client_rect(handle: WindowHandle) -> Result<DesktopRect, String> {
    imp::get_client_rect(handle)
}

pub fn window_scale_factor(handle: WindowHandle) -> Result<f64, String> {
    imp::window_scale_factor(handle)
}

pub fn is_window_maximized(handle: WindowHandle) -> Result<bool, String> {
    imp::is_window_maximized(handle)
}

pub fn focus_window(handle: WindowHandle) -> Result<(), String> {
    imp::focus_window(handle)
}

pub fn mouse_click(x: i32, y: i32, button: Option<&str>) -> Result<(), String> {
    imp::mouse_click(x, y, button)
}

pub fn left_edge_drag(
    handle: WindowHandle,
    from_x: i32,
    from_y: i32,
    to_x: i32,
) -> Result<(), String> {
    imp::left_edge_drag(handle, from_x, from_y, to_x)
}

pub fn send_win_left() -> Result<(), String> {
    imp::send_win_left()
}

pub fn verify_snap_capture_ready(
    handle: WindowHandle,
    post_rect: DesktopRect,
) -> Result<(), SnapCaptureReadinessError> {
    imp::verify_snap_capture_ready(handle, post_rect)
}

pub fn send_text_enter(text: &str) -> Result<(), String> {
    imp::send_text_enter(text)
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

    #[test]
    fn desktop_rect_overlap_requires_positive_area() {
        let target = DesktopRect {
            left: 10,
            top: 10,
            right: 50,
            bottom: 50,
        };

        assert!(rects_overlap(
            target,
            DesktopRect {
                left: 49,
                top: 49,
                right: 80,
                bottom: 80,
            }
        ));
        assert!(!rects_overlap(
            target,
            DesktopRect {
                left: 50,
                top: 10,
                right: 80,
                bottom: 50,
            }
        ));
    }

    #[test]
    fn occlusion_decision_uses_first_visible_non_owned_overlap_above_target() {
        let target = WindowHandle(7);
        let target_rect = DesktopRect {
            left: 100,
            top: 100,
            right: 300,
            bottom: 300,
        };
        let hidden_overlap = WindowOcclusionCandidate {
            handle: WindowHandle(1),
            rect: target_rect,
            visible: false,
            owned: false,
        };
        let owned_overlap = WindowOcclusionCandidate {
            handle: WindowHandle(2),
            rect: target_rect,
            visible: true,
            owned: true,
        };
        let visible_overlap = WindowOcclusionCandidate {
            handle: WindowHandle(3),
            rect: DesktopRect {
                left: 120,
                top: 120,
                right: 180,
                bottom: 180,
            },
            visible: true,
            owned: false,
        };

        let occluder = first_occluding_window_before_target(
            target,
            target_rect,
            &[hidden_overlap, owned_overlap, visible_overlap],
        );

        assert_eq!(occluder, Some(visible_overlap));
    }

    #[test]
    fn occlusion_decision_ignores_tiny_sentinel_windows() {
        let target = WindowHandle(7);
        let target_rect = DesktopRect {
            left: 0,
            top: 0,
            right: 300,
            bottom: 300,
        };
        let sentinel_overlap = WindowOcclusionCandidate {
            handle: WindowHandle(1),
            rect: DesktopRect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            },
            visible: true,
            owned: false,
        };
        let visible_overlap = WindowOcclusionCandidate {
            handle: WindowHandle(2),
            rect: DesktopRect {
                left: 120,
                top: 120,
                right: 180,
                bottom: 180,
            },
            visible: true,
            owned: false,
        };

        let occluder = first_occluding_window_before_target(
            target,
            target_rect,
            &[sentinel_overlap, visible_overlap],
        );

        assert_eq!(occluder, Some(visible_overlap));
    }

    #[test]
    fn occlusion_decision_ignores_windows_below_target() {
        let target = WindowHandle(7);
        let target_rect = DesktopRect {
            left: 100,
            top: 100,
            right: 300,
            bottom: 300,
        };
        let below_target_overlap = WindowOcclusionCandidate {
            handle: WindowHandle(9),
            rect: target_rect,
            visible: true,
            owned: false,
        };

        let occluder =
            first_occluding_window_before_target(target, target_rect, &[below_target_overlap]);
        assert_eq!(occluder, Some(below_target_overlap));

        let occluder = first_occluding_window_before_target(
            target,
            target_rect,
            &[
                WindowOcclusionCandidate {
                    handle: target,
                    rect: target_rect,
                    visible: true,
                    owned: false,
                },
                below_target_overlap,
            ],
        );
        assert_eq!(occluder, None);
    }
}
