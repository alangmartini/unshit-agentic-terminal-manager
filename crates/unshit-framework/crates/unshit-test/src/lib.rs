mod actions;
mod assertions;
mod harness;
mod input;
mod locator;
pub mod os_input;
mod query;
mod render;
mod replay;
pub mod report;
mod select;
pub mod selector;
mod test_app;
pub mod trace;
mod windowed;

extern crate self as unshit_test;

pub use harness::TestHarness;
pub use locator::Locator;
pub use query::ElementSnapshot;
pub use render::{compute_rmse, pixels_match, MaskRegion, ScreenshotOptions};
pub use replay::TestEvent;
pub use report::{TestReport, TestReportEntry, TestStatus};
pub use test_app::TestApp;
pub use trace::{TraceAction, TraceRecorder, TraceStep};
pub use unshit_macros::ui_test;
pub use windowed::WindowedTest;

#[doc(hidden)]
pub fn __ui_test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(())).lock().unwrap()
}
