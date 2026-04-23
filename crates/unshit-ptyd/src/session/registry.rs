//! Per-daemon session registry.
//!
//! Central table of live [`Session`] objects keyed by their monotonic
//! id. The registry hands out ids and exposes enough state to service
//! `ListSessions` without leaking mutex guards across await points.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{mpsc, Mutex};

use super::Session;
use crate::protocol::message::SessionInfo;

/// Thread-safe, mutex-guarded map of live sessions.
#[derive(Default)]
pub struct SessionRegistry {
    sessions: Mutex<HashMap<u64, Session>>,
    next_id: AtomicU64,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            // Ids start at 1. Zero is reserved as a sentinel for "no
            // session" in future slices.
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocates the next monotonic id. Saturating; never wraps to zero.
    pub fn next_id(&self) -> u64 {
        // fetch_add wraps on overflow which would hand out zero; saturate
        // instead so clients never see a sentinel id.
        loop {
            let current = self.next_id.load(Ordering::Relaxed);
            if current == u64::MAX {
                return u64::MAX;
            }
            let next = current + 1;
            if self
                .next_id
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return current;
            }
        }
    }

    /// Spawns a new session and inserts it into the registry.
    ///
    /// Returns the assigned id and the matching output receiver so the
    /// handler can forward bytes to its connection.
    pub async fn spawn(
        &self,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
    ) -> std::io::Result<(u64, mpsc::Receiver<Vec<u8>>)> {
        let id = self.next_id();
        let (session, rx) = Session::spawn(id, cols, rows, cwd, shell)?;
        let mut guard = self.sessions.lock().await;
        guard.insert(id, session);
        Ok((id, rx))
    }

    /// Writes `bytes` to the session with the given id.
    pub async fn write(&self, id: u64, bytes: &[u8]) -> std::io::Result<()> {
        let mut guard = self.sessions.lock().await;
        let session = guard.get_mut(&id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no session for id {id}"),
            )
        })?;
        session.write(bytes).await
    }

    /// Resizes the session with the given id. No-op if missing.
    pub async fn resize(&self, id: u64, cols: u16, rows: u16) -> std::io::Result<()> {
        let mut guard = self.sessions.lock().await;
        let session = guard.get_mut(&id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no session for id {id}"),
            )
        })?;
        session.resize(cols, rows);
        Ok(())
    }

    /// Removes a session from the registry and kills its child.
    pub async fn remove(&self, id: u64) -> bool {
        let mut guard = self.sessions.lock().await;
        match guard.remove(&id) {
            Some(mut session) => {
                session.kill();
                true
            }
            None => false,
        }
    }

    /// Returns a snapshot describing every currently live session.
    pub async fn list(&self) -> Vec<SessionInfo> {
        let guard = self.sessions.lock().await;
        let mut out: Vec<SessionInfo> = guard
            .iter()
            .map(|(id, s)| SessionInfo {
                id: *id,
                cols: s.cols(),
                rows: s.rows(),
                alive: s.alive(),
                pid: s.pid(),
            })
            .collect();
        out.sort_by_key(|info| info.id);
        out
    }

    /// Returns how many sessions the registry currently holds.
    pub async fn len(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Convenience: true when the registry is empty. Satisfies the
    /// clippy `len_without_is_empty` pair.
    pub async fn is_empty(&self) -> bool {
        self.sessions.lock().await.is_empty()
    }

    /// Drops every session, killing their children.
    pub async fn kill_all(&self) -> Vec<u64> {
        let mut guard = self.sessions.lock().await;
        let ids: Vec<u64> = guard.keys().copied().collect();
        for id in &ids {
            if let Some(mut s) = guard.remove(id) {
                s.kill();
            }
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn next_id_is_monotonic_starting_at_one() {
        let reg = SessionRegistry::new();
        assert_eq!(reg.next_id(), 1);
        assert_eq!(reg.next_id(), 2);
        assert_eq!(reg.next_id(), 3);
    }

    #[tokio::test]
    async fn next_id_saturates_at_max_rather_than_wrapping() {
        let reg = SessionRegistry::new();
        reg.next_id.store(u64::MAX, Ordering::Relaxed);
        assert_eq!(reg.next_id(), u64::MAX);
        assert_eq!(reg.next_id(), u64::MAX, "must not wrap past u64::MAX to 0");
    }

    #[tokio::test]
    async fn len_starts_at_zero_on_new_registry() {
        let reg = SessionRegistry::new();
        assert_eq!(reg.len().await, 0);
    }

    #[tokio::test]
    async fn list_is_empty_on_new_registry() {
        let reg = SessionRegistry::new();
        assert!(reg.list().await.is_empty());
    }

    #[tokio::test]
    async fn remove_unknown_id_returns_false() {
        let reg = SessionRegistry::new();
        assert!(!reg.remove(99).await);
    }

    #[tokio::test]
    async fn write_to_unknown_id_is_not_found() {
        let reg = SessionRegistry::new();
        let err = reg.write(99, b"x").await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn resize_unknown_id_is_not_found() {
        let reg = SessionRegistry::new();
        let err = reg.resize(99, 80, 24).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
