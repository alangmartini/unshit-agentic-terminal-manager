use std::collections::HashMap;
use std::io::Read;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures_core::Stream;
use unshit::app::{EventSink, ExternalEvent, Subscription};
use unshit::core::trace::{append_terminal_trace_line, terminal_trace_enabled};

use crate::state::{MutexExt, SharedState};

static PENDING_READERS: Mutex<Option<HashMap<u32, Box<dyn Read + Send>>>> = Mutex::new(None);

fn preview_bytes(bytes: &[u8], limit: usize) -> String {
    let mut preview = String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]).into_owned();
    preview = preview
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\u{1b}', "\\x1b");
    if bytes.len() > limit {
        preview.push_str("...");
    }
    preview
}

pub fn register_reader(pane_id: u32, reader: Box<dyn Read + Send>) {
    let mut guard = PENDING_READERS.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(pane_id, reader);
}

fn take_reader(pane_id: u32) -> Option<Box<dyn Read + Send>> {
    let mut guard = PENDING_READERS.lock().unwrap();
    guard.as_mut().and_then(|map| map.remove(&pane_id))
}

fn take_all_readers() -> HashMap<u32, Box<dyn Read + Send>> {
    let mut guard = PENDING_READERS.lock().unwrap();
    guard.take().unwrap_or_default()
}

/// Create a subscription that reads from a PTY stdout and feeds bytes
/// to the terminal emulator, triggering UI rebuilds.
///
/// Uses a single long-lived blocking task with a channel to avoid
/// per-read task-spawn overhead and buffer allocation.
fn pty_subscription(pane_id: u32, shared: SharedState) -> Option<Subscription> {
    let reader = take_reader(pane_id)?;

    // Wrap reader in Arc<Mutex<>> so the factory closure is Sync.
    let reader_cell: Arc<Mutex<Option<Box<dyn Read + Send>>>> = Arc::new(Mutex::new(Some(reader)));

    Some(Subscription::new(
        format!("pty-{}", pane_id),
        move |_sink: EventSink| -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>> {
            let shared = shared.clone();
            let reader_cell = reader_cell.clone();

            Box::pin(async_stream::stream! {
                // Take the reader out (one-time).
                let reader = {
                    let mut guard = reader_cell.lock().unwrap();
                    match guard.take() {
                        Some(r) => r,
                        None => return,
                    }
                };

                // Spawn a single long-lived blocking task that reads in a
                // loop and sends chunks through a channel.
                let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);

                tokio::task::spawn_blocking(move || {
                    let mut reader = reader;
                    let mut buf = [0u8; 4096];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if tx.blocking_send(buf[..n].to_vec()).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });

                // Drain channel and feed bytes to the terminal emulator.
                // Batch all buffered chunks into a single rebuild to avoid
                // triggering one full tree-rebuild per PTY read (the framework
                // does not coalesce RequestRebuild events).
                //
                // Acquire the state mutex only to look up the per-pane
                // Terminal handle, then release it before running the VTE
                // parser. `process_bytes` holds only the per-terminal mutex so
                // the render closure and other state mutators can proceed
                // concurrently on the state lock.
                while let Some(data) = rx.recv().await {
                    let terminal_handle: Option<crate::state::SharedTerminal> = {
                        let guard = shared.lock_recover();
                        guard.terminals.get(&pane_id).cloned()
                    };
                    let Some(terminal_handle) = terminal_handle else {
                        continue;
                    };

                    let mut batched = 1u32;
                    {
                        let mut terminal = terminal_handle.lock_recover();
                        terminal.process_bytes(&data);
                        while let Ok(more) = rx.try_recv() {
                            terminal.process_bytes(&more);
                            batched += 1;
                        }
                        if terminal_trace_enabled() {
                            let rows = terminal.grid().debug_rows(4, 96);
                            append_terminal_trace_line(&format!(
                                "terminal-trace stage=bridge_after_process pane={} batched={} bytes={} cursor=({}, {}) row0={:?} row1={:?} row2={:?} row3={:?}",
                                pane_id,
                                batched,
                                preview_bytes(&data, 120),
                                terminal.grid().cursor_row(),
                                terminal.grid().cursor_col(),
                                rows.first().cloned().unwrap_or_default(),
                                rows.get(1).cloned().unwrap_or_default(),
                                rows.get(2).cloned().unwrap_or_default(),
                                rows.get(3).cloned().unwrap_or_default(),
                            ));
                        }
                    }
                    if batched > 1 {
                        log::debug!("pty-{}: batched {} chunks into 1 rebuild", pane_id, batched);
                    }
                    yield ExternalEvent::RequestRebuild;
                }
            })
        },
    ))
}

/// Cursor blink and deferred PTY spawn subscription.
///
/// Every 500ms: toggles cursor visibility on the active pane. On the first
/// tick where the renderer has published valid cell metrics, spawns PTYs
/// for any panes that do not yet have a terminal (the initial pane) and
/// resizes any that already exist.
fn cursor_blink_subscription(shared: SharedState) -> Subscription {
    Subscription::new(
        "cursor-blink".to_string(),
        move |_sink: EventSink| -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>> {
            let shared = shared.clone();
            Box::pin(async_stream::stream! {
                let mut visible = true;
                let mut synced = false;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    visible = !visible;
                    {
                        let mut guard = shared.lock_recover();

                        // Cursor blink: active pane blinks when focused,
                        // shows steady cursor when window is unfocused.
                        // Inactive panes never show a cursor.
                        let active_id = guard.active_pane.0;
                        let win_focused = unshit::core::cell_grid::CellGrid::is_window_focused();
                        for (&id, terminal_handle) in guard.terminals.iter() {
                            let mut terminal = terminal_handle.lock_recover();
                            if id == active_id {
                                if win_focused {
                                    terminal.grid_mut().set_cursor_visible(visible);
                                } else {
                                    terminal.grid_mut().set_cursor_visible(true);
                                }
                            } else {
                                terminal.grid_mut().set_cursor_visible(false);
                            }
                        }

                        // Toast lifetimes are tick-driven from this same
                        // 500 ms cadence. ToastStore::with_capacity(_, 8)
                        // gives ~4 s before auto-dismiss. The ids of any
                        // dismissed toasts are intentionally ignored; the
                        // snapshot path picks up the new state on the
                        // next render.
                        let _ = guard.toasts.advance_ticks(1);

                        // Drain any fire-and-forget PTY write failures
                        // the worker has reported since the last tick
                        // (Phase 2 of #135). The render thread never
                        // waits for daemon acks anymore, so failures
                        // surface here as user-visible toasts. 500 ms
                        // latency is acceptable for an error message;
                        // it matches the existing toast tick cadence.
                        let write_errors = guard.pty_manager.take_write_errors();
                        for err in write_errors {
                            log::warn!(
                                "pty write failed for pane {}: {}",
                                err.pane_id,
                                err.error
                            );
                            crate::state::push_error_toast(
                                &mut guard,
                                format!("write failed (pane {}): {}", err.pane_id, err.error),
                            );
                        }

                        // Deferred PTY spawn and dimension sync.
                        // Wait until the renderer has published real cell
                        // metrics (cell_w > 0), then spawn PTYs for any
                        // panes missing a terminal, and resize existing ones.
                        if !synced {
                            let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
                            let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
                            let w = guard.last_grid_width;
                            let h = guard.last_grid_height;
                            log::debug!(
                                "blink sync check: cell_w={:.2} cell_h={:.2} grid_w={:.1} grid_h={:.1}",
                                cell_w, cell_h, w, h
                            );
                            if cell_w > 0.0 && cell_h > 0.0 {
                                synced = true;
                                // Use stored grid dimensions if available, otherwise
                                // fall back to 80x24. The on_resize handler may not
                                // have fired yet because it only registers when a
                                // terminal grid exists (chicken-and-egg with deferred spawn).
                                let (cols, rows) = if w > 0.0 {
                                    crate::state::compute_pty_dimensions(w, h, cell_w, cell_h)
                                } else {
                                    (80u16, 24u16)
                                };
                                log::info!(
                                    "PTY sync: {}x{} (cell {:.2}x{:.2}, area {:.0}x{:.0})",
                                    cols, rows, cell_w, cell_h, w, h
                                );

                                // Reconcile deferred panes against the daemon's
                                // surviving sessions (slice 5). If a prior UI
                                // run left a matching `(workspace_id, pane_id)`
                                // session alive, attach to it and replay its
                                // snapshot; otherwise spawn a fresh shell.
                                // Issue #5: PTYs get correct dimensions from
                                // the start.
                                let all_pane_ids: Vec<u32> = guard
                                    .panes
                                    .iter()
                                    .flat_map(|row| row.iter().map(|p| p.id.0))
                                    .collect();
                                let cwd = crate::state::active_workspace_cwd(&guard);
                                let workspace_id = crate::state::active_workspace_num(&guard);
                                for id in &all_pane_ids {
                                    if !guard.terminals.contains_key(id) {
                                        match guard.pty_manager.attach_or_spawn(
                                            *id,
                                            workspace_id,
                                            cols,
                                            rows,
                                            cwd.as_deref(),
                                        ) {
                                            Ok((Some(snapshot), reader)) => {
                                                let snap_rows = snapshot.grid.rows();
                                                let snap_cols = snapshot.grid.cols();
                                                let mut terminal = crate::terminal::Terminal::new(
                                                    snap_rows, snap_cols,
                                                );
                                                terminal.apply_snapshot(&snapshot);
                                                guard.terminals.insert(
                                                    *id,
                                                    std::sync::Arc::new(std::sync::Mutex::new(
                                                        terminal,
                                                    )),
                                                );
                                                crate::bridge::register_reader(*id, reader);
                                                log::info!(
                                                    "deferred reattach for pane {}: {}x{}",
                                                    id, snap_cols, snap_rows
                                                );
                                            }
                                            Ok((None, reader)) => {
                                                let terminal = crate::terminal::Terminal::new(
                                                    rows as usize,
                                                    cols as usize,
                                                );
                                                guard.terminals.insert(
                                                    *id,
                                                    std::sync::Arc::new(std::sync::Mutex::new(
                                                        terminal,
                                                    )),
                                                );
                                                crate::bridge::register_reader(*id, reader);
                                                log::info!(
                                                    "deferred PTY spawn for pane {}: {}x{}",
                                                    id, cols, rows
                                                );
                                            }
                                            Err(e) => {
                                                log::error!(
                                                    "failed to spawn deferred PTY for pane {}: {}",
                                                    id, e
                                                );
                                                let mut terminal = crate::terminal::Terminal::new(
                                                    rows as usize,
                                                    cols as usize,
                                                );
                                                terminal.process_bytes(
                                                    format!(
                                                        "Failed to spawn shell: {}\r\n",
                                                        e
                                                    )
                                                    .as_bytes(),
                                                );
                                                guard.terminals.insert(
                                                    *id,
                                                    std::sync::Arc::new(std::sync::Mutex::new(
                                                        terminal,
                                                    )),
                                                );
                                            }
                                        }
                                    }
                                }

                                // Resize any PTYs that already existed.
                                let existing_ids: Vec<u32> =
                                    guard.terminals.keys().copied().collect();
                                for id in existing_ids {
                                    guard.pty_manager.resize(id, cols, rows);
                                    if let Some(t) = guard.terminals.get(&id) {
                                        t.lock_recover().resize(rows as usize, cols as usize);
                                    }
                                }
                            }
                        }
                    }
                    yield ExternalEvent::RequestRebuild;
                }
            })
        },
    )
}

/// Subscription that periodically checks for renderer-computed pending
/// resizes and applies them to all terminals. Runs every 100ms for quick
/// response to window resize events.
fn resize_poll_subscription(shared: SharedState) -> Subscription {
    Subscription::new(
        "resize-poll",
        move |_sink: EventSink| -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>> {
            let shared = shared.clone();
            Box::pin(async_stream::stream! {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if let Some((cols, rows)) =
                        unshit::core::cell_grid::CellGrid::take_pending_resize()
                    {
                        {
                            let mut guard = shared.lock_recover();
                            let ids: Vec<u32> = guard.terminals.keys().copied().collect();
                            for id in ids {
                                guard.pty_manager.resize(id, cols, rows);
                                if let Some(t) = guard.terminals.get(&id) {
                                    t.lock_recover().resize(rows as usize, cols as usize);
                                }
                            }
                        } // guard drops before yield
                        yield ExternalEvent::RequestRebuild;
                    }
                }
            })
        },
    )
}

/// Build the list of active subscriptions from current state.
/// Called by the framework after each tree rebuild.
pub fn build_subscriptions(shared: &SharedState) -> Vec<Subscription> {
    let mut subs = Vec::new();

    // Cursor blink timer (always active).
    subs.push(cursor_blink_subscription(shared.clone()));

    // Resize poll: checks for renderer-published pending resizes.
    subs.push(resize_poll_subscription(shared.clone()));

    // Pick up any newly registered readers and create subscriptions for them.
    let pending = take_all_readers();
    for (pane_id, reader) in pending {
        register_reader(pane_id, reader);
        if let Some(sub) = pty_subscription(pane_id, shared.clone()) {
            subs.push(sub);
        }
    }

    // For existing terminals, emit identity-only subscriptions so the
    // framework keeps already-running streams alive.
    let guard = shared.lock_recover();
    for &pane_id in guard.terminals.keys() {
        subs.push(Subscription::new(
            format!("pty-{}", pane_id),
            move |_sink: EventSink| -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>> {
                Box::pin(async_stream::stream! {
                    // Yield nothing. The framework identity system keeps the
                    // original stream running; this factory only fires if
                    // the previous subscription was cancelled.
                    let _: ExternalEvent = std::future::pending().await;
                    // unreachable, but gives the stream the right Item type
                    yield ExternalEvent::RequestRebuild;
                })
            },
        ));
    }

    subs
}
