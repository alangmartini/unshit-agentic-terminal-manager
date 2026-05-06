#[cfg(test)]
extern crate self as unshit_test;

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
pub mod __private {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    pub fn env_lock() -> MutexGuard<'static, ()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
