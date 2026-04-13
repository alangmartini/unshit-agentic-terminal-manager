use std::time::Duration;

use unshit_core::element::ElementTree;
use unshit_core::id::NodeId;
use unshit_core::tree::NodeArena;

use crate::query::ElementSnapshot;
use crate::TestHarness;
use crate::WindowedTest;

/// Unified test harness that transparently delegates to either a headless
/// `TestHarness` or a windowed `WindowedTest` backend based on environment
/// variables.
///
/// Environment variables:
/// - `UNSHIT_TEST_HEADED=1`: open a real window instead of running headless
/// - `UNSHIT_TEST_SLOW_MO=<ms>`: wait this many milliseconds between actions
/// - `UNSHIT_TEST_HIGHLIGHT=1`: (reserved) flash target element before actions
pub struct TestApp {
    backend: TestBackend,
    slow_mo: Duration,
}

enum TestBackend {
    Headless(TestHarness),
    Headed(WindowedTest),
}

pub(crate) fn env_is_truthy(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
}

impl TestApp {
    /// Create a new test application.
    ///
    /// Reads environment variables to determine the backend and configuration:
    /// - `UNSHIT_TEST_HEADED=1` selects the windowed backend
    /// - `UNSHIT_TEST_SLOW_MO=<ms>` sets the delay between actions
    /// - `UNSHIT_TEST_HIGHLIGHT=1` enables element highlighting before actions
    pub fn new(
        css: &str,
        tree_fn: impl Fn() -> ElementTree + 'static,
        width: f32,
        height: f32,
    ) -> Self {
        let headed = env_is_truthy("UNSHIT_TEST_HEADED");

        let slow_mo = std::env::var("UNSHIT_TEST_SLOW_MO")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::ZERO);

        let backend = if headed {
            TestBackend::Headed(WindowedTest::new(css, tree_fn, width as u32, height as u32))
        } else {
            TestBackend::Headless(TestHarness::new(css, &tree_fn, width, height))
        };

        Self { backend, slow_mo }
    }

    /// Returns `true` when running in headed (windowed) mode.
    pub fn is_headed(&self) -> bool {
        matches!(self.backend, TestBackend::Headed(_))
    }

    /// Returns the configured slow-mo duration.
    pub fn slow_mo(&self) -> Duration {
        self.slow_mo
    }

    // -- query delegation ---------------------------------------------------

    /// Find the first element matching a simple selector.
    pub fn query(&self, selector: &str) -> Option<ElementSnapshot> {
        match &self.backend {
            TestBackend::Headless(h) => h.query(selector),
            TestBackend::Headed(w) => w.query(selector),
        }
    }

    /// Find all elements matching a simple selector.
    pub fn query_all(&self, selector: &str) -> Vec<ElementSnapshot> {
        match &self.backend {
            TestBackend::Headless(h) => h.query_all(selector),
            TestBackend::Headed(w) => w.query_all(selector),
        }
    }

    // -- locator API --------------------------------------------------------

    /// Create a locator for elements matching `selector`.
    ///
    /// Only supported in headless mode. Panics in headed mode.
    pub fn locator(&mut self, selector: &str) -> crate::Locator<'_> {
        self.as_harness_mut()
            .expect("locator() is only supported in headless mode")
            .locator(selector)
    }

    /// Create a locator matching elements whose text equals `text`.
    ///
    /// Only supported in headless mode. Panics in headed mode.
    pub fn locator_by_text(&mut self, text: &str) -> crate::Locator<'_> {
        self.as_harness_mut()
            .expect("locator_by_text() is only supported in headless mode")
            .locator_by_text(text)
    }

    /// Create a locator matching elements whose text contains `text`.
    ///
    /// Only supported in headless mode. Panics in headed mode.
    pub fn locator_by_text_contains(&mut self, text: &str) -> crate::Locator<'_> {
        self.as_harness_mut()
            .expect("locator_by_text_contains() is only supported in headless mode")
            .locator_by_text_contains(text)
    }

    // -- state inspection ---------------------------------------------------

    /// Returns the root node ID.
    pub fn root(&self) -> NodeId {
        match &self.backend {
            TestBackend::Headless(h) => h.root(),
            TestBackend::Headed(w) => w.root(),
        }
    }

    /// Returns a reference to the node arena.
    pub fn arena(&self) -> &NodeArena {
        match &self.backend {
            TestBackend::Headless(h) => h.arena(),
            TestBackend::Headed(w) => w.arena(),
        }
    }

    /// Returns the currently hovered element.
    pub fn hovered(&self) -> NodeId {
        match &self.backend {
            TestBackend::Headless(h) => h.hovered(),
            TestBackend::Headed(w) => w.hovered(),
        }
    }

    /// Returns the currently active (pressed) element, if any.
    pub fn active(&self) -> Option<NodeId> {
        match &self.backend {
            TestBackend::Headless(h) => h.active(),
            TestBackend::Headed(w) => w.active(),
        }
    }

    // -- frame advancement --------------------------------------------------

    /// Advance one frame. In headless mode this calls `step()`, in headed
    /// mode it calls `pump(1)`.
    pub fn step(&mut self) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.step(),
            TestBackend::Headed(w) => w.pump(1),
        }
    }

    /// Pump multiple frames.
    pub fn pump(&mut self, frames: usize) {
        match &mut self.backend {
            TestBackend::Headless(h) => {
                for _ in 0..frames {
                    h.step();
                }
            }
            TestBackend::Headed(w) => w.pump(frames),
        }
    }

    // -- input simulation ---------------------------------------------------

    /// Simulate mouse movement to (x, y).
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.mouse_move(x, y),
            TestBackend::Headed(w) => {
                w.inject_mouse_move(x, y);
                w.pump(1);
            }
        }
        self.apply_slow_mo();
    }

    /// Simulate a full click at (x, y).
    pub fn click(&mut self, x: f32, y: f32) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.click(x, y),
            TestBackend::Headed(w) => w.inject_click(x, y),
        }
        self.apply_slow_mo();
    }

    /// Simulate mouse button press at (x, y).
    pub fn mouse_down(&mut self, x: f32, y: f32) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.mouse_down(x, y),
            TestBackend::Headed(w) => {
                w.inject_mouse_move(x, y);
                w.pump(1);
                w.inject_mouse_down();
                w.pump(1);
            }
        }
        self.apply_slow_mo();
    }

    /// Simulate mouse button release at (x, y).
    pub fn mouse_up(&mut self, x: f32, y: f32) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.mouse_up(x, y),
            TestBackend::Headed(w) => {
                w.inject_mouse_move(x, y);
                w.pump(1);
                w.inject_mouse_up();
                w.pump(1);
            }
        }
        self.apply_slow_mo();
    }

    /// Simulate a mouse wheel event at (x, y).
    pub fn mouse_wheel(&mut self, x: f32, y: f32, delta_x: f32, delta_y: f32) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.mouse_wheel(x, y, delta_x, delta_y),
            TestBackend::Headed(w) => {
                w.inject_mouse_move(x, y);
                w.pump(1);
                w.inject_mouse_wheel((delta_y * 120.0) as i32);
                w.pump(1);
            }
        }
        self.apply_slow_mo();
    }

    /// Type text into the focused input. Not yet supported in headed mode.
    pub fn type_text(&mut self, text: &str) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.type_text(text),
            TestBackend::Headed(_) => {
                eprintln!("TestApp::type_text in headed mode is not yet supported");
            }
        }
        self.apply_slow_mo();
    }

    /// Type a single character into the focused input.
    pub fn type_char(&mut self, ch: char) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.type_char(ch),
            TestBackend::Headed(_) => {
                eprintln!("TestApp::type_char in headed mode is not yet supported");
            }
        }
        self.apply_slow_mo();
    }

    /// Press a special key on the focused input element.
    pub fn press_key(&mut self, key: unshit_core::event::Key) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.press_key(key),
            TestBackend::Headed(_) => {
                eprintln!("TestApp::press_key in headed mode is not yet supported");
            }
        }
        self.apply_slow_mo();
    }

    /// Set focus to a specific node. Not yet supported in headed mode.
    pub fn focus(&mut self, node_id: NodeId) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.focus(node_id),
            TestBackend::Headed(_) => {
                eprintln!("TestApp::focus in headed mode is not yet supported");
            }
        }
    }

    /// Simulate Tab key press.
    pub fn tab(&mut self) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.tab(),
            TestBackend::Headed(_) => {
                eprintln!("TestApp::tab in headed mode is not yet supported");
            }
        }
        self.apply_slow_mo();
    }

    /// Simulate Shift+Tab key press.
    pub fn shift_tab(&mut self) {
        match &mut self.backend {
            TestBackend::Headless(h) => h.shift_tab(),
            TestBackend::Headed(_) => {
                eprintln!("TestApp::shift_tab in headed mode is not yet supported");
            }
        }
        self.apply_slow_mo();
    }

    /// Get the currently focused element.
    pub fn focused(&self) -> NodeId {
        match &self.backend {
            TestBackend::Headless(h) => h.focused(),
            TestBackend::Headed(_) => NodeId::DANGLING,
        }
    }

    /// Get the current input value of the focused element.
    pub fn input_value(&self) -> Option<String> {
        match &self.backend {
            TestBackend::Headless(h) => h.input_value(),
            TestBackend::Headed(_) => None,
        }
    }

    // -- headed-only features -----------------------------------------------

    /// Pause execution and keep the window open, pumping frames in a loop
    /// until Enter is pressed in the terminal.
    ///
    /// In headless mode this is a no-op (returns immediately).
    pub fn pause(&mut self) {
        let TestBackend::Headed(w) = &mut self.backend else {
            return;
        };

        eprintln!("TestApp paused. Press Enter to continue...");

        loop {
            w.pump(1);

            if stdin_has_input() {
                // Consume the newline
                let mut buf = String::new();
                let _ = std::io::stdin().read_line(&mut buf);
                break;
            }

            std::thread::sleep(Duration::from_millis(16));
        }
    }

    // -- slow-mo helpers ----------------------------------------------------

    /// Apply the slow-mo delay by pumping frames for the configured duration.
    fn apply_slow_mo(&mut self) {
        if self.slow_mo.is_zero() {
            return;
        }

        let iterations = (self.slow_mo.as_millis() / 16).max(1) as usize;
        for _ in 0..iterations {
            self.step();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    // -- direct backend access ----------------------------------------------

    /// Get a reference to the underlying `TestHarness` (headless mode only).
    /// Returns `None` in headed mode.
    pub fn as_harness(&self) -> Option<&TestHarness> {
        match &self.backend {
            TestBackend::Headless(h) => Some(h),
            TestBackend::Headed(_) => None,
        }
    }

    /// Get a mutable reference to the underlying `TestHarness`.
    /// Returns `None` in headed mode.
    pub fn as_harness_mut(&mut self) -> Option<&mut TestHarness> {
        match &mut self.backend {
            TestBackend::Headless(h) => Some(h),
            TestBackend::Headed(_) => None,
        }
    }

    /// Get a reference to the underlying `WindowedTest` (headed mode only).
    /// Returns `None` in headless mode.
    pub fn as_windowed(&self) -> Option<&WindowedTest> {
        match &self.backend {
            TestBackend::Headless(_) => None,
            TestBackend::Headed(w) => Some(w),
        }
    }

    /// Get a mutable reference to the underlying `WindowedTest`.
    /// Returns `None` in headless mode.
    pub fn as_windowed_mut(&mut self) -> Option<&mut WindowedTest> {
        match &mut self.backend {
            TestBackend::Headless(_) => None,
            TestBackend::Headed(w) => Some(w),
        }
    }
}

/// Check if stdin has pending input without blocking.
#[cfg(target_os = "windows")]
fn stdin_has_input() -> bool {
    use std::os::windows::io::AsRawHandle;
    let handle = std::io::stdin().as_raw_handle();
    unsafe {
        let mut events_available: u32 = 0;
        let result = windows_sys_peek_named_pipe(
            handle as isize,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut events_available,
            std::ptr::null_mut(),
        );
        result != 0 && events_available > 0
    }
}

#[cfg(target_os = "windows")]
extern "system" {
    #[link_name = "PeekNamedPipe"]
    fn windows_sys_peek_named_pipe(
        h_named_pipe: isize,
        lp_buffer: *mut u8,
        n_buffer_size: u32,
        lp_bytes_read: *mut u32,
        lp_total_bytes_avail: *mut u32,
        lp_bytes_left_this_message: *mut u32,
    ) -> i32;
}

#[cfg(not(target_os = "windows"))]
fn stdin_has_input() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use unshit_core::element::{ElementDef, ElementTree, Tag};

    fn simple_tree() -> ElementTree {
        ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("container")
                .with_child(ElementDef::new(Tag::Span).with_class("label"))
                .with_child(ElementDef::new(Tag::Div).with_class("button")),
        }
    }

    #[test]
    fn test_app_defaults_to_headless() {
        // Ensure env vars are not set for this test
        std::env::remove_var("UNSHIT_TEST_HEADED");
        std::env::remove_var("UNSHIT_TEST_SLOW_MO");

        let app = TestApp::new("", simple_tree, 800.0, 600.0);
        assert!(!app.is_headed());
        assert!(app.slow_mo().is_zero());
    }

    #[test]
    fn test_app_query_works_headless() {
        std::env::remove_var("UNSHIT_TEST_HEADED");

        let app = TestApp::new(".container { width: 100%; }", simple_tree, 800.0, 600.0);

        let result = app.query(".container");
        assert!(result.is_some(), "should find .container");

        let result = app.query(".label");
        assert!(result.is_some(), "should find .label");

        let result = app.query(".nonexistent");
        assert!(result.is_none(), "should not find .nonexistent");
    }

    #[test]
    fn test_app_query_all_works_headless() {
        std::env::remove_var("UNSHIT_TEST_HEADED");

        let app = TestApp::new("", simple_tree, 800.0, 600.0);

        let results = app.query_all("div");
        assert!(results.len() >= 2, "should find at least 2 divs");
    }

    #[test]
    fn test_app_step_works_headless() {
        std::env::remove_var("UNSHIT_TEST_HEADED");

        let mut app = TestApp::new("", simple_tree, 800.0, 600.0);

        // step should not panic
        app.step();
        app.pump(3);
    }

    #[test]
    fn test_app_click_works_headless() {
        std::env::remove_var("UNSHIT_TEST_HEADED");

        let mut app =
            TestApp::new(".button { width: 100px; height: 40px; }", simple_tree, 800.0, 600.0);

        // click should not panic even if no element is at coordinates
        app.click(50.0, 20.0);
    }

    #[test]
    fn test_slow_mo_env_parsing() {
        std::env::set_var("UNSHIT_TEST_HEADED", "0");
        std::env::set_var("UNSHIT_TEST_SLOW_MO", "200");

        let app = TestApp::new("", simple_tree, 800.0, 600.0);
        assert_eq!(app.slow_mo(), Duration::from_millis(200));

        // Cleanup
        std::env::remove_var("UNSHIT_TEST_SLOW_MO");
        std::env::remove_var("UNSHIT_TEST_HEADED");
    }

    #[test]
    fn test_slow_mo_adds_delay() {
        std::env::set_var("UNSHIT_TEST_HEADED", "0");
        std::env::set_var("UNSHIT_TEST_SLOW_MO", "100");

        let mut app = TestApp::new("", simple_tree, 800.0, 600.0);

        let start = Instant::now();
        app.click(10.0, 10.0);
        let elapsed = start.elapsed();

        // With 100ms slow_mo, we expect at least ~80ms of delay
        // (100ms / 16ms = 6 iterations, 6 * 16ms = 96ms of sleep)
        assert!(
            elapsed.as_millis() >= 50,
            "slow_mo should add measurable delay, got {}ms",
            elapsed.as_millis()
        );

        // Cleanup
        std::env::remove_var("UNSHIT_TEST_SLOW_MO");
        std::env::remove_var("UNSHIT_TEST_HEADED");
    }

    #[test]
    fn test_pause_is_noop_in_headless() {
        std::env::remove_var("UNSHIT_TEST_HEADED");

        let mut app = TestApp::new("", simple_tree, 800.0, 600.0);

        // pause() should return immediately in headless mode
        let start = Instant::now();
        app.pause();
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 100, "pause() in headless mode should be a no-op");
    }

    #[test]
    fn test_backend_access() {
        std::env::remove_var("UNSHIT_TEST_HEADED");

        let app = TestApp::new("", simple_tree, 800.0, 600.0);

        assert!(app.as_harness().is_some());
        assert!(app.as_windowed().is_none());
    }

    // -- ui_test macro tests ---------------------------------------------------

    #[unshit_macros::ui_test]
    fn ui_test_macro_basic() {
        std::env::remove_var("UNSHIT_TEST_HEADED");
        let app = TestApp::new("", simple_tree, 800.0, 600.0);
        assert!(!app.is_headed());
    }

    #[unshit_macros::ui_test(headed = false, slow_mo = 0)]
    fn ui_test_macro_with_config() {
        let app = TestApp::new("", simple_tree, 800.0, 600.0);
        assert!(!app.is_headed());
        assert!(app.slow_mo().is_zero());
    }

    #[unshit_macros::ui_test(slow_mo = 50)]
    fn ui_test_macro_slow_mo() {
        let val = std::env::var("UNSHIT_TEST_SLOW_MO").unwrap();
        assert_eq!(val, "50");
    }

    #[unshit_macros::ui_test(width = 1024, height = 768)]
    fn ui_test_macro_dimensions() {
        assert_eq!(std::env::var("UNSHIT_TEST_WIDTH").unwrap(), "1024");
        assert_eq!(std::env::var("UNSHIT_TEST_HEIGHT").unwrap(), "768");
    }

    #[unshit_macros::ui_test(gpu = true)]
    fn ui_test_macro_gpu_flag() {
        assert_eq!(std::env::var("UNSHIT_TEST_GPU").unwrap(), "1");
    }

    #[unshit_macros::ui_test(timeout = 5000)]
    fn ui_test_macro_timeout() {
        assert_eq!(std::env::var("UNSHIT_TEST_TIMEOUT").unwrap(), "5000");
    }

    #[unshit_macros::ui_test(gpu)]
    fn ui_test_macro_bare_flag() {
        assert_eq!(std::env::var("UNSHIT_TEST_GPU").unwrap(), "1");
    }

    #[unshit_macros::ui_test(slow_mo = 100)]
    fn ui_test_macro_cleanup_on_success() {
        assert_eq!(std::env::var("UNSHIT_TEST_SLOW_MO").unwrap(), "100");
    }
}
