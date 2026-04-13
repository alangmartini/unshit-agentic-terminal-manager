//! Optional async runtime support (behind the `async` feature flag).
//!
//! Provides a background tokio runtime that integrates with the framework's
//! event loop via [`EventSink`](crate::event_sink::EventSink).

use std::future::Future;

/// Handle to the background tokio runtime.
///
/// Created automatically when `App::new()` is called with the `async` feature
/// enabled. The runtime lives on dedicated threads and is independent of the
/// UI event loop.
pub struct AsyncRuntime {
    runtime: tokio::runtime::Runtime,
}

impl AsyncRuntime {
    pub(crate) fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("unshit-async")
            .build()
            .expect("failed to create tokio runtime");
        Self { runtime }
    }

    /// Spawn a future on the background runtime.
    ///
    /// The future runs on a tokio worker thread, not the UI thread.
    /// Use [`EventSink::send`] or [`EventSink::send_async`] from inside
    /// the future to push results back to the UI.
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(future)
    }

    /// Returns a [`tokio::runtime::Handle`] that can be cloned and moved
    /// into other contexts (e.g. non-async code that needs to block on a
    /// future).
    pub fn handle(&self) -> &tokio::runtime::Handle {
        self.runtime.handle()
    }
}
