//! Per-daemon session registry.
//!
//! Central table of live [`Session`] objects keyed by their monotonic
//! id. The registry hands out ids and exposes enough state to service
//! `ListSessions` without leaking mutex guards across await points.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{mpsc, Mutex};

use unshit_terminal_core::Snapshot;

use super::{AttachmentToken, Session};
use crate::protocol::message::SessionInfo;
use crate::protocol::message::{EnsureDisposition, SNAPSHOT_MAX_SCROLLBACK_LINES};

pub struct EnsuredSession {
    pub session_id: u64,
    pub attachment_token: AttachmentToken,
    pub hook_capability: String,
    pub disposition: EnsureDisposition,
    pub snapshot: Snapshot,
    pub output: mpsc::Receiver<Vec<u8>>,
}

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
    /// Returns the assigned id, attachment token, and matching output
    /// receiver so the handler can forward bytes to its connection and
    /// later detach only the attachment it owns.
    pub async fn spawn(
        &self,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
        shell_args: &[String],
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
    ) -> std::io::Result<(u64, AttachmentToken, String, mpsc::Receiver<Vec<u8>>)> {
        self.spawn_with_context(
            cols,
            rows,
            cwd,
            shell,
            shell_args,
            workspace_id,
            pane_id,
            name,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_with_context(
        &self,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
        shell_args: &[String],
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
        restore_correlation_id: Option<&str>,
    ) -> std::io::Result<(u64, AttachmentToken, String, mpsc::Receiver<Vec<u8>>)> {
        let mut guard = self.sessions.lock().await;
        if guard.values().any(|session| {
            session.workspace_id() == workspace_id
                && session.pane_id() == pane_id
                && session.alive()
        }) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a live session already owns this workspace and pane key",
            ));
        }
        guard.retain(|_, session| {
            session.workspace_id() != workspace_id
                || session.pane_id() != pane_id
                || session.alive()
        });
        let id = self.next_id();
        let (session, attachment_token, rx) = Session::spawn_with_context(
            id,
            cols,
            rows,
            cwd,
            shell,
            shell_args,
            workspace_id,
            pane_id,
            name,
            restore_correlation_id,
        )?;
        let hook_capability = session.hook_capability().to_string();
        guard.insert(id, session);
        Ok((id, attachment_token, hook_capability, rx))
    }

    /// Atomically acquire the live session identified by
    /// `(workspace_id, pane_id)`, spawning the supplied command only
    /// when the key is positively absent. The registry mutex covers
    /// both lookup and insert, closing the list-to-spawn race across
    /// clients. A lost response is safe to retry: the next call sees
    /// the session created by the first.
    pub async fn ensure(
        &self,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
        shell_args: &[String],
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
        scrollback_lines: usize,
    ) -> std::io::Result<EnsuredSession> {
        self.ensure_with_context(
            cols,
            rows,
            cwd,
            shell,
            shell_args,
            workspace_id,
            pane_id,
            name,
            scrollback_lines,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn ensure_with_context(
        &self,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
        shell_args: &[String],
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
        scrollback_lines: usize,
        restore_correlation_id: Option<&str>,
    ) -> std::io::Result<EnsuredSession> {
        let mut guard = self.sessions.lock().await;
        let matching_live: Vec<u64> = guard
            .iter()
            .filter_map(|(&id, session)| {
                (session.workspace_id() == workspace_id
                    && session.pane_id() == pane_id
                    && session.alive())
                .then_some(id)
            })
            .collect();

        match matching_live.as_slice() {
            [session_id] => {
                let session_id = *session_id;
                // A surviving session keeps the geometry of the run that
                // created it. The client has just told us the pane's
                // current size, so honour it before handing the session
                // back: without this a reattached PTY stays at the
                // previous window's row count while the UI renders the
                // new one, and every absolute cursor move the client
                // application makes below the UI's last row collapses
                // onto it, leaving stale rows on screen.
                let resized = guard
                    .get_mut(&session_id)
                    .expect("matching live session disappeared under registry lock");
                if resized.cols() != cols || resized.rows() != rows {
                    resized.resize(cols, rows);
                }
                let session = guard
                    .get(&session_id)
                    .expect("matching live session disappeared under registry lock");
                if let Some((attachment_token, snapshot, output)) = session
                    .attach_with_snapshot(scrollback_lines.min(SNAPSHOT_MAX_SCROLLBACK_LINES))
                {
                    return Ok(EnsuredSession {
                        session_id,
                        attachment_token,
                        hook_capability: session.hook_capability().to_string(),
                        disposition: EnsureDisposition::Existing,
                        snapshot,
                        output,
                    });
                }
                // The reader can exit after `alive()` above but before the
                // atomic attach boundary. Remove that unusable session and
                // let this same Ensure transaction spawn the fallback.
                guard.remove(&session_id);
            }
            [] => {}
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "multiple live sessions share one workspace and pane key",
                ));
            }
        }

        // Dead sessions with this key cannot be attached and must not
        // make the key permanently ambiguous. Removing them under the
        // same lock leaves exactly one insertion point below.
        let dead_matching: Vec<u64> = guard
            .iter()
            .filter_map(|(&id, session)| {
                (session.workspace_id() == workspace_id
                    && session.pane_id() == pane_id
                    && !session.alive())
                .then_some(id)
            })
            .collect();
        for id in dead_matching {
            guard.remove(&id);
        }

        let session_id = self.next_id();
        let (session, attachment_token, output) = Session::spawn_with_context(
            session_id,
            cols,
            rows,
            cwd,
            shell,
            shell_args,
            workspace_id,
            pane_id,
            name,
            restore_correlation_id,
        )?;
        let snapshot = session.snapshot(scrollback_lines.min(SNAPSHOT_MAX_SCROLLBACK_LINES));
        let hook_capability = session.hook_capability().to_string();
        guard.insert(session_id, session);
        Ok(EnsuredSession {
            session_id,
            attachment_token,
            hook_capability,
            disposition: EnsureDisposition::Spawned,
            snapshot,
            output,
        })
    }

    /// Swaps the session's output sender for a fresh one and returns the
    /// matching receiver. Returns `None` if no session matches `id`.
    pub async fn attach(
        &self,
        id: u64,
    ) -> Option<(AttachmentToken, String, mpsc::Receiver<Vec<u8>>)> {
        let mut guard = self.sessions.lock().await;
        let attachment = guard.get(&id).and_then(|session| {
            session.attach().map(|(attachment_token, output)| {
                (
                    attachment_token,
                    session.hook_capability().to_string(),
                    output,
                )
            })
        });
        if attachment.is_none() {
            guard.remove(&id);
        }
        attachment
    }

    /// Attach and capture the replay snapshot at one stream boundary.
    pub async fn attach_with_snapshot(
        &self,
        id: u64,
        scrollback_lines: usize,
    ) -> Option<(AttachmentToken, String, Snapshot, mpsc::Receiver<Vec<u8>>)> {
        let mut guard = self.sessions.lock().await;
        let attachment = guard.get(&id).and_then(|session| {
            session
                .attach_with_snapshot(scrollback_lines.min(SNAPSHOT_MAX_SCROLLBACK_LINES))
                .map(|(attachment_token, snapshot, output)| {
                    (
                        attachment_token,
                        session.hook_capability().to_string(),
                        snapshot,
                        output,
                    )
                })
        });
        if attachment.is_none() {
            guard.remove(&id);
        }
        attachment
    }

    /// Clears the session's output sender only when `attachment_token`
    /// still owns it. Returns `false` for an unknown session or a stale
    /// attachment token.
    pub async fn detach(&self, id: u64, attachment_token: AttachmentToken) -> bool {
        let guard = self.sessions.lock().await;
        match guard.get(&id) {
            Some(s) => s.detach(attachment_token),
            None => false,
        }
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

    /// Set or clear the display name of a session. Returns `false` if
    /// no session matches `id`.
    pub async fn rename(&self, id: u64, name: Option<String>) -> bool {
        let mut guard = self.sessions.lock().await;
        match guard.get_mut(&id) {
            Some(session) => {
                session.set_name(name);
                true
            }
            None => false,
        }
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
                memory_rss_bytes: s.pid().and_then(crate::memory::resident_set_bytes),
                workspace_id: s.workspace_id(),
                pane_id: s.pane_id(),
                name: s.name().map(|n| n.to_string()),
            })
            .collect();
        out.sort_by_key(|info| info.id);
        out
    }

    /// Returns a snapshot of the session identified by `id`, or `None`
    /// if the id is unknown. `scrollback_lines` bounds how many
    /// most-recent scrollback rows ride along in the snapshot.
    pub async fn snapshot(&self, id: u64, scrollback_lines: usize) -> Option<Snapshot> {
        let guard = self.sessions.lock().await;
        guard.get(&id).map(|s| s.snapshot(scrollback_lines))
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

    #[tokio::test]
    async fn snapshot_returns_none_for_unknown_id() {
        let reg = SessionRegistry::new();
        assert!(reg.snapshot(99, 0).await.is_none());
    }

    fn test_shell() -> &'static str {
        #[cfg(windows)]
        {
            "cmd.exe"
        }
        #[cfg(unix)]
        {
            "/bin/sh"
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_round_trips_through_registry() {
        let reg = SessionRegistry::new();
        let (id, _token, _capability, mut rx) = reg
            .spawn(100, 30, None, Some(test_shell()), &[], 0, 0, None)
            .await
            .expect("spawn");

        #[cfg(windows)]
        let payload = b"echo regmarker\r\n";
        #[cfg(unix)]
        let payload = b"echo regmarker\n";
        reg.write(id, payload).await.expect("write");

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1500);
        while tokio::time::timeout_at(deadline, rx.recv()).await.is_ok() {}

        let snap = reg.snapshot(id, 0).await.expect("snapshot");
        assert_eq!(snap.grid.rows(), 30);
        assert_eq!(snap.grid.cols(), 100);

        reg.remove(id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_stores_workspace_and_pane_metadata() {
        let reg = SessionRegistry::new();
        let (id, _token, _capability, _rx) = reg
            .spawn(
                80,
                24,
                None,
                Some(test_shell()),
                &[],
                4,
                2,
                Some("alpha".into()),
            )
            .await
            .expect("spawn");

        let list = reg.list().await;
        let info = list.iter().find(|s| s.id == id).expect("listed");
        assert_eq!(info.workspace_id, 4);
        assert_eq!(info.pane_id, 2);
        assert_eq!(info.name.as_deref(), Some("alpha"));

        reg.remove(id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_returns_full_metadata_for_every_session() {
        let reg = SessionRegistry::new();
        let (a, _token_a, _capability_a, _rx_a) = reg
            .spawn(
                80,
                24,
                None,
                Some(test_shell()),
                &[],
                1,
                0,
                Some("a".into()),
            )
            .await
            .expect("spawn a");
        let (b, _token_b, _capability_b, _rx_b) = reg
            .spawn(90, 30, None, Some(test_shell()), &[], 2, 1, None)
            .await
            .expect("spawn b");

        let list = reg.list().await;
        assert_eq!(list.len(), 2);
        let info_a = list.iter().find(|s| s.id == a).expect("a listed");
        let info_b = list.iter().find(|s| s.id == b).expect("b listed");
        assert_eq!(info_a.workspace_id, 1);
        assert_eq!(info_a.pane_id, 0);
        assert_eq!(info_a.name.as_deref(), Some("a"));
        assert_eq!(info_b.workspace_id, 2);
        assert_eq!(info_b.pane_id, 1);
        assert_eq!(info_b.name, None);

        reg.remove(a).await;
        reg.remove(b).await;
    }

    #[tokio::test]
    async fn attach_returns_none_for_unknown_id() {
        let reg = SessionRegistry::new();
        assert!(reg.attach(999).await.is_none());
    }

    #[tokio::test]
    async fn detach_is_noop_on_unknown_id() {
        let reg = SessionRegistry::new();
        assert!(!reg.detach(999, 1).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_returns_receiver_for_known_session() {
        let reg = SessionRegistry::new();
        let (id, _original_token, _capability, _original_rx) = reg
            .spawn(80, 24, None, Some(test_shell()), &[], 0, 0, None)
            .await
            .expect("spawn");

        let (_new_token, _capability, new_rx) = reg.attach(id).await.expect("attach");
        drop(new_rx);

        reg.remove(id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detach_returns_true_for_known_session() {
        let reg = SessionRegistry::new();
        let (id, token, _capability, _rx) = reg
            .spawn(80, 24, None, Some(test_shell()), &[], 0, 0, None)
            .await
            .expect("spawn");

        assert!(reg.detach(id, token).await);

        reg.remove(id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_detach_does_not_clear_newer_attachment() {
        let reg = SessionRegistry::new();
        let (id, stale_token, _capability, _stale_rx) = reg
            .spawn(80, 24, None, Some(test_shell()), &[], 0, 0, None)
            .await
            .expect("spawn");
        let (current_token, _capability, current_rx) = reg.attach(id).await.expect("reattach");

        assert_ne!(stale_token, current_token);
        assert!(!reg.detach(id, stale_token).await);
        assert!(
            !current_rx.is_closed(),
            "a stale connection must not close the current output channel"
        );

        assert!(reg.detach(id, current_token).await);
        assert!(current_rx.is_closed());

        reg.remove(id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_rejects_a_session_whose_child_already_exited() {
        #[cfg(windows)]
        let shell_args = vec!["/Q".into(), "/D".into(), "/C".into(), "exit /B 0".into()];
        #[cfg(unix)]
        let shell_args = vec!["-c".into(), "exit 0".into()];

        let reg = SessionRegistry::new();
        let (id, _token, _capability, _rx) = reg
            .spawn(80, 24, None, Some(test_shell()), &shell_args, 3, 9, None)
            .await
            .expect("spawn one-shot session");

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let alive = reg
                .list()
                .await
                .into_iter()
                .find(|session| session.id == id)
                .is_some_and(|session| session.alive);
            if !alive {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "one-shot child did not exit within the test deadline"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert!(
            reg.attach_with_snapshot(id, 0).await.is_none(),
            "AttachSession must not install a sender after the PTY reader has exited"
        );
        reg.remove(id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_session_is_atomic_and_idempotent_for_a_pane_key() {
        let reg = std::sync::Arc::new(SessionRegistry::new());
        let first_reg = reg.clone();
        let second_reg = reg.clone();
        let (first, second) = tokio::join!(
            first_reg.ensure(
                80,
                24,
                None,
                Some(test_shell()),
                &[],
                4,
                2,
                Some("agent".into()),
                10,
            ),
            second_reg.ensure(
                80,
                24,
                None,
                Some(test_shell()),
                &[],
                4,
                2,
                Some("agent".into()),
                10,
            )
        );
        let first = first.expect("first ensure");
        let second = second.expect("second ensure");
        assert_eq!(first.session_id, second.session_id);
        assert_ne!(first.disposition, second.disposition);
        assert_eq!(reg.len().await, 1);
        reg.remove(first.session_id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_existing_never_executes_the_supplied_fallback() {
        let reg = SessionRegistry::new();
        let first = reg
            .ensure(80, 24, None, Some(test_shell()), &[], 8, 5, None, 0)
            .await
            .expect("spawn initial");
        assert_eq!(
            first.disposition,
            crate::protocol::message::EnsureDisposition::Spawned
        );

        let second = reg
            .ensure(
                80,
                24,
                None,
                Some("definitely-not-a-real-agent-executable"),
                &["--resume".into()],
                8,
                5,
                None,
                0,
            )
            .await
            .expect("existing session must win before fallback execution");
        assert_eq!(first.session_id, second.session_id);
        assert_eq!(
            second.disposition,
            crate::protocol::message::EnsureDisposition::Existing
        );
        reg.remove(first.session_id).await;
    }

    /// Sessions outlive the UI, so a reattaching client is usually a
    /// differently-sized window than the one that spawned the session.
    /// Ignoring the dimensions on the reuse path left the PTY at the old
    /// geometry: the client application kept drawing frames sized for
    /// rows the UI no longer had, and everything it addressed past the
    /// UI's last row collapsed onto it over stale content.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_existing_resizes_the_session_to_the_requested_geometry() {
        let reg = SessionRegistry::new();
        let first = reg
            .ensure(80, 24, None, Some(test_shell()), &[], 21, 7, None, 0)
            .await
            .expect("spawn initial");
        assert_eq!(
            first.disposition,
            crate::protocol::message::EnsureDisposition::Spawned
        );

        let second = reg
            .ensure(119, 35, None, Some(test_shell()), &[], 21, 7, None, 0)
            .await
            .expect("reattach to the live session");
        assert_eq!(first.session_id, second.session_id);
        assert_eq!(
            second.disposition,
            crate::protocol::message::EnsureDisposition::Existing
        );

        let info = reg
            .list()
            .await
            .into_iter()
            .find(|s| s.id == first.session_id)
            .expect("session must still be listed");
        assert_eq!(
            (info.cols, info.rows),
            (119, 35),
            "reused session must adopt the reattaching client's geometry"
        );

        reg.remove(first.session_id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn raw_spawn_rejects_a_duplicate_live_pane_key() {
        let reg = SessionRegistry::new();
        let (first, _token, _capability, _output) = reg
            .spawn(80, 24, None, Some(test_shell()), &[], 11, 3, None)
            .await
            .expect("first spawn");

        let error = reg
            .spawn(80, 24, None, Some(test_shell()), &[], 11, 3, None)
            .await
            .expect_err("duplicate key must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(reg.len().await, 1);

        reg.remove(first).await;
    }
}
