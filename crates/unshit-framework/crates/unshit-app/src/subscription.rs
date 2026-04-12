//! Declarative subscription system for async event sources.
//!
//! Subscriptions are identity-tracked async streams that the framework manages.
//! Each frame after a rebuild, the framework diffs the current set of
//! subscriptions against the previous set: new subscriptions are started,
//! removed ones are cancelled.
//!
//! Requires the `async` feature.

use crate::event_sink::{EventSink, ExternalEvent};
use futures_core::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use tokio::task::JoinHandle;

/// Identity for a subscription. Subscriptions with the same id are considered
/// the same source; the framework will not restart a subscription whose id
/// persists across rebuilds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(String);

impl SubscriptionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A declarative async event source.
///
/// Each subscription has an identity and a factory that builds a `Stream` of
/// [`ExternalEvent`]s. The framework calls the factory when the subscription
/// first appears and polls the resulting stream on the background tokio
/// runtime, forwarding each item through the [`EventSink`].
///
/// # Example
///
/// ```ignore
/// use std::time::Duration;
/// use unshit::app::{ExternalEvent, Subscription};
///
/// Subscription::new("tick", |_sink| {
///     Box::pin(async_stream::stream! {
///         loop {
///             tokio::time::sleep(Duration::from_secs(1)).await;
///             yield ExternalEvent::RequestRebuild;
///         }
///     })
/// })
/// ```
pub struct Subscription {
    id: SubscriptionId,
    #[allow(clippy::type_complexity)]
    factory:
        Box<dyn Fn(EventSink) -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>> + Send + Sync>,
}

impl Subscription {
    /// Create a new subscription with the given identity and stream factory.
    ///
    /// The `factory` receives an [`EventSink`] for convenience (though the
    /// stream items are already forwarded automatically). The returned stream
    /// is polled on the background tokio runtime.
    pub fn new(
        id: impl Into<String>,
        factory: impl Fn(EventSink) -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self { id: SubscriptionId::new(id), factory: Box::new(factory) }
    }

    /// Returns this subscription's identity.
    pub fn id(&self) -> &SubscriptionId {
        &self.id
    }
}

/// Internal manager that tracks active subscriptions and reconciles them
/// after each tree rebuild.
pub(crate) struct SubscriptionManager {
    /// User-provided function returning the currently desired subscriptions.
    subscriptions_fn: Box<dyn Fn() -> Vec<Subscription> + Send>,
    /// Currently running subscription tasks, keyed by id.
    active: HashMap<SubscriptionId, JoinHandle<()>>,
}

impl SubscriptionManager {
    pub(crate) fn new(subscriptions_fn: Box<dyn Fn() -> Vec<Subscription> + Send>) -> Self {
        Self { subscriptions_fn, active: HashMap::default() }
    }

    /// Reconcile: call the subscriptions function, diff against active set,
    /// start new subscriptions, cancel removed ones.
    ///
    /// Must be called on the UI thread (it invokes `subscriptions_fn`).
    /// Spawns stream-polling tasks on the provided tokio handle.
    pub(crate) fn reconcile(&mut self, runtime: &tokio::runtime::Handle, sink: &EventSink) {
        use tokio_stream::StreamExt;

        let desired: Vec<Subscription> = (self.subscriptions_fn)();

        // Collect desired ids for fast lookup.
        let mut desired_ids: HashMap<SubscriptionId, ()> = HashMap::new();
        for sub in &desired {
            desired_ids.insert(sub.id.clone(), ());
        }

        // Cancel subscriptions that are no longer desired.
        self.active.retain(|id, handle: &mut JoinHandle<()>| {
            if desired_ids.contains_key(id) {
                true
            } else {
                log::debug!("Subscription cancelled: {}", id);
                handle.abort();
                false
            }
        });

        // Start subscriptions that are new.
        for sub in desired {
            if self.active.contains_key(&sub.id) {
                continue; // Already running.
            }
            let id = sub.id.clone();
            let sink = sink.clone();
            let stream = (sub.factory)(sink.clone());
            let handle = runtime.spawn(async move {
                let mut stream = std::pin::pin!(stream);
                while let Some(event) = stream.next().await {
                    if sink.send(event).is_err() {
                        break; // Event loop shut down.
                    }
                }
            });
            log::debug!("Subscription started: {}", id);
            self.active.insert(id, handle);
        }
    }

    /// Cancel all active subscriptions. Called on shutdown.
    pub(crate) fn cancel_all(&mut self) {
        for (id, handle) in &self.active {
            log::debug!("Subscription cancelled (shutdown): {}", id);
            handle.abort();
        }
        self.active.clear();
    }
}

impl Drop for SubscriptionManager {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn subscription_id_equality() {
        let a = SubscriptionId::new("timer");
        let b = SubscriptionId::new("timer");
        let c = SubscriptionId::new("pty");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn manager_starts_and_cancels_subscriptions() {
        let counter = Arc::new(AtomicU64::new(0));
        let counter_clone = Arc::clone(&counter);

        let (tx, rx) = flume::unbounded();
        let proxy_cell = Arc::new(std::sync::OnceLock::new());
        let sink = EventSink::new(tx, proxy_cell);

        let rt = tokio::runtime::Handle::current();

        // Phase 1: one subscription active.
        let c = Arc::clone(&counter_clone);
        let mut manager = SubscriptionManager::new(Box::new(move || {
            let c = Arc::clone(&c);
            vec![Subscription::new("counter", move |_sink| {
                let c = Arc::clone(&c);
                Box::pin(async_stream::stream! {
                    loop {
                        c.fetch_add(1, Ordering::Relaxed);
                        yield ExternalEvent::RequestRebuild;
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
            })]
        }));

        manager.reconcile(&rt, &sink);
        assert_eq!(manager.active.len(), 1);

        // Let it tick a few times.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(counter.load(Ordering::Relaxed) > 0);

        // Drain events from the channel.
        let event_count = rx.try_iter().count();
        assert!(event_count > 0);

        // Phase 2: cancel all.
        manager.cancel_all();
        assert_eq!(manager.active.len(), 0);
    }

    #[tokio::test]
    async fn manager_does_not_restart_existing_subscription() {
        let start_count = Arc::new(AtomicU64::new(0));
        let sc = Arc::clone(&start_count);

        let (tx, _rx) = flume::unbounded();
        let proxy_cell = Arc::new(std::sync::OnceLock::new());
        let sink = EventSink::new(tx, proxy_cell);
        let rt = tokio::runtime::Handle::current();

        let mut manager = SubscriptionManager::new(Box::new(move || {
            let sc = Arc::clone(&sc);
            vec![Subscription::new("stable", move |_sink| {
                sc.fetch_add(1, Ordering::Relaxed);
                Box::pin(tokio_stream::pending())
            })]
        }));

        manager.reconcile(&rt, &sink);
        manager.reconcile(&rt, &sink);
        manager.reconcile(&rt, &sink);

        // Factory should only have been called once (subscription persisted).
        assert_eq!(start_count.load(Ordering::Relaxed), 1);
        assert_eq!(manager.active.len(), 1);
    }
}
