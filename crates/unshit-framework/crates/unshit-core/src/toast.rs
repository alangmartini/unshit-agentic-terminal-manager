//! Ephemeral, auto-dismissing notification primitive.
//!
//! `ToastStore` holds a bounded queue of `Toast` records. Each toast
//! has a numeric id, a kind, a message, and a tick-based lifetime.
//! Lifetimes are driven by `advance_ticks`, not a real clock, so tests
//! and consumers stay deterministic. The store evicts the oldest entry
//! when `push` is called at capacity, and removes any toast whose
//! `remaining_ticks` reaches zero.
//!
//! # Examples
//!
//! ```
//! use unshit_core::toast::ToastStore;
//!
//! let mut store = ToastStore::with_capacity(3, 8);
//! let id = store.push("rename failed: not connected");
//! assert_eq!(store.len(), 1);
//!
//! let dismissed = store.advance_ticks(8);
//! assert_eq!(dismissed, vec![id]);
//! assert!(store.is_empty());
//! ```

/// Numeric handle for a toast inside a `ToastStore`. Stable for the
/// lifetime of the toast; reused after a toast is removed because the
/// store assigns ids monotonically from a per-store counter that does
/// not reset on eviction.
pub type ToastId = u64;

/// Severity level of a toast. Only `Error` is defined today; future
/// slices can add `Info` / `Warn` / `Success` without changing the
/// public surface of `ToastStore`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Error,
}

/// A single notification record. Cloned into UI snapshots; the live
/// copy lives inside `ToastStore`.
#[derive(Clone, Debug)]
pub struct Toast {
    pub id: ToastId,
    pub kind: ToastKind,
    pub message: String,
    pub remaining_ticks: u32,
}

/// Bounded, deterministic notification queue. The store does not own a
/// clock; consumers call `advance_ticks` from whatever cadence they
/// already have (the app drives this from its 500 ms cursor blink).
///
/// # Examples
///
/// ```
/// use unshit_core::toast::ToastStore;
///
/// let mut store = ToastStore::with_capacity(2, 4);
/// store.push("first");
/// store.push("second");
/// store.push("third"); // evicts "first"
/// let messages: Vec<&str> = store.iter().map(|t| t.message.as_str()).collect();
/// assert_eq!(messages, vec!["second", "third"]);
/// ```
pub struct ToastStore {
    next_id: ToastId,
    items: Vec<Toast>,
    cap: usize,
    default_lifetime: u32,
}

impl ToastStore {
    /// Build a store that holds at most `cap` toasts, each with a
    /// default lifetime of `default_lifetime` ticks.
    pub fn with_capacity(cap: usize, default_lifetime: u32) -> Self {
        Self { next_id: 1, items: Vec::with_capacity(cap), cap, default_lifetime }
    }

    /// Push an error toast carrying `message`. Evicts the oldest entry
    /// if the store is at capacity. Returns the new toast's id.
    pub fn push(&mut self, message: impl Into<String>) -> ToastId {
        if self.items.len() >= self.cap && !self.items.is_empty() {
            self.items.remove(0);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Toast {
            id,
            kind: ToastKind::Error,
            message: message.into(),
            remaining_ticks: self.default_lifetime,
        });
        id
    }

    /// Remove the toast with the given id. Returns `true` if a toast
    /// was removed, `false` if no toast with that id exists. Idempotent
    /// across repeated calls.
    pub fn dismiss(&mut self, id: ToastId) -> bool {
        let len_before = self.items.len();
        self.items.retain(|t| t.id != id);
        self.items.len() != len_before
    }

    /// Decrement every toast's `remaining_ticks` by `n` (saturating at
    /// zero) and remove any that hit zero. Returns the ids of the
    /// removed toasts in push order so the caller can react if needed.
    pub fn advance_ticks(&mut self, n: u32) -> Vec<ToastId> {
        for t in self.items.iter_mut() {
            t.remaining_ticks = t.remaining_ticks.saturating_sub(n);
        }
        let mut dismissed = Vec::new();
        self.items.retain(|t| {
            if t.remaining_ticks == 0 {
                dismissed.push(t.id);
                false
            } else {
                true
            }
        });
        dismissed
    }

    /// Iterate over the live toasts in push order.
    pub fn iter(&self) -> impl Iterator<Item = &Toast> {
        self.items.iter()
    }

    /// Number of live toasts.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when there are no live toasts.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_under_cap_appends() {
        let mut store = ToastStore::with_capacity(3, 4);
        let a = store.push("a");
        let b = store.push("b");
        assert_eq!(store.len(), 2);
        let ids: Vec<ToastId> = store.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![a, b]);
    }

    #[test]
    fn push_at_cap_evicts_oldest() {
        let mut store = ToastStore::with_capacity(2, 4);
        store.push("first");
        store.push("second");
        store.push("third");
        assert_eq!(store.len(), 2);
        let messages: Vec<&str> = store.iter().map(|t| t.message.as_str()).collect();
        assert_eq!(messages, vec!["second", "third"]);
    }

    #[test]
    fn dismiss_removes_by_id_and_is_idempotent() {
        let mut store = ToastStore::with_capacity(3, 4);
        let a = store.push("a");
        store.push("b");
        assert!(store.dismiss(a));
        assert_eq!(store.len(), 1);
        assert!(!store.dismiss(a));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn advance_ticks_decrements_and_evicts_at_zero() {
        let mut store = ToastStore::with_capacity(3, 2);
        store.push("a");
        let dismissed = store.advance_ticks(1);
        assert!(dismissed.is_empty());
        assert_eq!(store.len(), 1);
        let dismissed = store.advance_ticks(1);
        assert_eq!(dismissed.len(), 1);
        assert!(store.is_empty());
    }

    #[test]
    fn advance_ticks_returns_dismissed_ids() {
        let mut store = ToastStore::with_capacity(3, 1);
        let a = store.push("a");
        let b = store.push("b");
        let dismissed = store.advance_ticks(1);
        assert_eq!(dismissed, vec![a, b]);
        assert!(store.is_empty());
    }

    #[test]
    fn iter_yields_in_push_order() {
        let mut store = ToastStore::with_capacity(3, 4);
        store.push("a");
        store.push("b");
        store.push("c");
        let messages: Vec<&str> = store.iter().map(|t| t.message.as_str()).collect();
        assert_eq!(messages, vec!["a", "b", "c"]);
    }

    #[test]
    fn advance_by_zero_is_a_noop() {
        let mut store = ToastStore::with_capacity(3, 4);
        let a = store.push("a");
        let dismissed = store.advance_ticks(0);
        assert!(dismissed.is_empty());
        assert_eq!(store.len(), 1);
        let ids: Vec<ToastId> = store.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![a]);
    }

    #[test]
    fn advance_overshoot_does_not_underflow() {
        let mut store = ToastStore::with_capacity(3, 2);
        store.push("a");
        let dismissed = store.advance_ticks(100);
        assert_eq!(dismissed.len(), 1);
        assert!(store.is_empty());
    }
}
