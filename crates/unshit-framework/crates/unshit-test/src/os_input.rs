#![allow(non_snake_case, clippy::upper_case_acronyms)]

/// OS-level input injection for windowed integration tests.
/// Windows-only: uses SetCursorPos and SendInput FFI.
#[cfg(target_os = "windows")]
mod windows_impl {
    use std::mem;

    const INPUT_MOUSE: u32 = 0;
    const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
    const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
    const MOUSEEVENTF_WHEEL: u32 = 0x0800;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct MOUSEINPUT {
        dx: i32,
        dy: i32,
        mouse_data: u32,
        flags: u32,
        time: u32,
        extra_info: usize,
    }

    #[repr(C)]
    struct INPUT {
        input_type: u32,
        input: MOUSEINPUT,
    }

    extern "system" {
        fn SetCursorPos(X: i32, Y: i32) -> i32;
        fn SendInput(nInputs: u32, pInputs: *mut INPUT, cbSize: i32) -> u32;
        fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    }

    pub fn set_cursor_pos(x: i32, y: i32) {
        // SAFETY: SetCursorPos is a Windows API that moves the system cursor.
        // It takes plain i32 coordinates and has no memory safety preconditions.
        unsafe {
            SetCursorPos(x, y);
        }
    }

    pub fn get_cursor_pos() -> (i32, i32) {
        // SAFETY: POINT is a repr(C) struct of two i32 fields, so zeroed memory is valid.
        // GetCursorPos writes into the provided pointer, which is a valid mutable reference.
        unsafe {
            let mut pt: POINT = mem::zeroed();
            GetCursorPos(&mut pt);
            (pt.x, pt.y)
        }
    }

    pub fn send_mouse_down() {
        send_mouse_event(MOUSEEVENTF_LEFTDOWN, 0);
    }

    pub fn send_mouse_up() {
        send_mouse_event(MOUSEEVENTF_LEFTUP, 0);
    }

    pub fn send_mouse_wheel(delta: i32) {
        // delta is in multiples of WHEEL_DELTA (120)
        send_mouse_event(MOUSEEVENTF_WHEEL, delta as u32);
    }

    fn send_mouse_event(flags: u32, data: u32) {
        // SAFETY: INPUT is a repr(C) struct matching the Windows API layout. We pass a
        // valid pointer to a single INPUT struct with the correct cbSize. SendInput reads
        // exactly `nInputs` entries from the pointer, and we pass 1.
        unsafe {
            let mut input = INPUT {
                input_type: INPUT_MOUSE,
                input: MOUSEINPUT { dx: 0, dy: 0, mouse_data: data, flags, time: 0, extra_info: 0 },
            };
            SendInput(1, &mut input, mem::size_of::<INPUT>() as i32);
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::*;

#[cfg(not(target_os = "windows"))]
pub fn set_cursor_pos(_x: i32, _y: i32) {
    eprintln!("OS input injection not supported on this platform");
}

#[cfg(not(target_os = "windows"))]
pub fn send_mouse_down() {}

#[cfg(not(target_os = "windows"))]
pub fn send_mouse_up() {}

#[cfg(not(target_os = "windows"))]
pub fn send_mouse_wheel(_delta: i32) {}

#[cfg(not(target_os = "windows"))]
pub fn get_cursor_pos() -> (i32, i32) {
    (0, 0)
}
