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
                // Batch all buffered chunks into a single rebuild so we
                // pay one VTE parse pass per drain rather than one per
                // PTY read. The framework also collapses any number of
                // RequestRebuild events that arrive in the same drain
                // window into a single rebuild
                // (see `RebuildCoalescer` in `unshit-app/src/app.rs`),
                // but draining here keeps the per pane terminal mutex
                // hold time bounded.
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
                    let pending_response = {
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
                        terminal.take_pending_response()
                    };
                    if !pending_response.is_empty() {
                        // Reply to host queries (DA1, DA2, DSR, CPR,
                        // XTVERSION) the parser collected. Done outside
                        // the per-terminal mutex; the write is fire and
                        // forget through the daemon shim.
                        let mut guard = shared.lock_recover();
                        if let Err(e) = guard.pty_manager.write(pane_id, &pending_response) {
                            log::warn!(
                                "pty-{}: failed to write {} bytes of query reply: {}",
                                pane_id,
                                pending_response.len(),
                                e
                            );
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

/// Cursor focus, toast bookkeeping, and deferred PTY spawn subscription.
///
/// Runs every 500 ms. Each tick:
///   * Sets `cursor_visible` to "this pane owns the focused cursor" for
///     the active pane and clears it on the others. The actual blink
///     animation is now driven by the renderer's global blink phase
///     clock (#135 Phase 1, item 2), so this flag is one shot per focus
///     change rather than a 2 Hz toggle.
///   * Advances toast lifetimes and drains fire and forget PTY write
///     errors into user visible toasts.
///   * Spawns any deferred PTYs once the renderer publishes valid cell
///     metrics.
///
/// Yields `RequestRedraw` (not `RequestRebuild`) on every tick so the
/// renderer's blink phase animation always reaches the screen, then
/// upgrades to `RequestRebuild` only when something actually changed
/// the UI tree (new toast, focus state flip, deferred spawn). Cursor
/// blink alone never triggers a tree rebuild after this change.
fn cursor_blink_subscription(shared: SharedState) -> Subscription {
    Subscription::new(
        "cursor-blink".to_string(),
        move |_sink: EventSink| -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>> {
            let shared = shared.clone();
            Box::pin(async_stream::stream! {
                let mut synced = false;
                let mut last_focus_signature: Option<(u32, bool)> = None;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let mut needs_rebuild = false;
                    {
                        let mut guard = shared.lock_recover();

                        // Per pane cursor focus: the active pane shows the
                        // cursor (the renderer animates the blink phase
                        // from a global clock); inactive panes never do.
                        // This loop only mutates state when focus actually
                        // changes, so steady state is a no op.
                        let active_id = guard.active_pane.0;
                        let win_focused = unshit::core::cell_grid::CellGrid::is_window_focused();
                        let signature = (active_id, win_focused);
                        if last_focus_signature != Some(signature) {
                            for (&id, terminal_handle) in guard.terminals.iter() {
                                let mut terminal = terminal_handle.lock_recover();
                                let should_show = id == active_id;
                                if terminal.grid().cursor_visible() != should_show {
                                    terminal.grid_mut().set_cursor_visible(should_show);
                                }
                            }
                            last_focus_signature = Some(signature);
                            // A focus change is observable in the tree
                            // (e.g. focused pane border styling), so
                            // promote this tick to a full rebuild.
                            needs_rebuild = true;
                        }

                        // Toast lifetimes are tick-driven from this same
                        // 500 ms cadence. ToastStore::with_capacity(_, 8)
                        // gives ~4 s before auto-dismiss. We rebuild only
                        // if a toast was actually dismissed so a quiet
                        // toast queue does not keep waking the tree.
                        let dismissed = guard.toasts.advance_ticks(1);
                        if !dismissed.is_empty() {
                            needs_rebuild = true;
                        }

                        // Drain any fire-and-forget PTY write failures
                        // the worker has reported since the last tick
                        // (Phase 2 of #135). The render thread never
                        // waits for daemon acks anymore, so failures
                        // surface here as user-visible toasts. 500 ms
                        // latency is acceptable for an error message;
                        // it matches the existing toast tick cadence.
                        let write_errors = guard.pty_manager.take_write_errors();
                        if !write_errors.is_empty() {
                            needs_rebuild = true;
                        }
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
                                let shell = crate::state::pane_spawn_shell(&guard);
                                for id in &all_pane_ids {
                                    if !guard.terminals.contains_key(id) {
                                        match guard.pty_manager.attach_or_spawn(
                                            *id,
                                            workspace_id,
                                            cols,
                                            rows,
                                            cwd.as_deref(),
                                            shell.as_ref(),
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
                                // Deferred spawn introduces new terminal
                                // handles into the tree; the next frame
                                // must rebuild to mount the matching grid.
                                needs_rebuild = true;
                            }
                        }
                    }
                    if needs_rebuild {
                        yield ExternalEvent::RequestRebuild;
                    } else {
                        // Cursor blink alone never rebuilds the tree
                        // (#135 Phase 1 exit criterion). The renderer
                        // animates the global blink phase from elapsed
                        // time, so a cheap repaint is all we need to
                        // make the cursor visibly toggle on screen.
                        yield ExternalEvent::RequestRedraw;
                    }
                }
            })
        },
    )
}

/// Subscription that periodically checks for renderer-computed pending
/// resizes and applies them to all terminals. Runs every 100ms for quick
/// response to window resize events.
///
/// PTY dimension sync is not user perceptible at the millisecond level
/// (the cell grid count flipping from 80 to 81 cols is invisible until
/// the next character lands), so this subscription yields
/// `RequestRedraw` rather than `RequestRebuild` (#135 Phase 1, item 3).
/// The next paint reads the new grid dimensions directly from the
/// `CellGrid` and reflows without a tree reconciliation. A real PTY
/// chunk landing in the new dimensions will request a rebuild via
/// `pty_subscription` on its own.
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
                        yield ExternalEvent::RequestRedraw;
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
