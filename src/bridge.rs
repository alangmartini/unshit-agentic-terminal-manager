use std::collections::HashMap;
use std::io::Read;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures_core::Stream;
use unshit::app::{EventSink, ExternalEvent, Subscription};

use crate::state::SharedState;

static PENDING_READERS: Mutex<Option<HashMap<u32, Box<dyn Read + Send>>>> = Mutex::new(None);

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
                while let Some(data) = rx.recv().await {
                    {
                        let mut guard = shared.lock().expect("state mutex poisoned");
                        if let Some(terminal) = guard.terminals.get_mut(&pane_id) {
                            terminal.process_bytes(&data);
                        }
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
                        let mut guard = shared.lock().expect("state mutex poisoned");

                        // Cursor blink: active pane only, and only when
                        // the window has OS focus.
                        let active_id = guard.active_pane.0;
                        let win_focused = unshit::core::cell_grid::CellGrid::is_window_focused();
                        for (&id, terminal) in guard.terminals.iter_mut() {
                            if id == active_id && win_focused {
                                terminal.grid_mut().set_cursor_visible(visible);
                            } else {
                                terminal.grid_mut().set_cursor_visible(false);
                            }
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
                            if cell_w > 0.0 && cell_h > 0.0 && w > 0.0 {
                                synced = true;
                                let (cols, rows) = crate::state::compute_pty_dimensions(
                                    w, h, cell_w, cell_h,
                                );
                                log::info!(
                                    "PTY sync: {}x{} (cell {:.2}x{:.2}, area {:.0}x{:.0})",
                                    cols, rows, cell_w, cell_h, w, h
                                );

                                // Spawn deferred PTYs for panes that have no
                                // terminal yet. Issue #5: PTYs get correct
                                // dimensions from the start.
                                let all_pane_ids: Vec<u32> = guard
                                    .panes
                                    .iter()
                                    .flat_map(|row| row.iter().map(|p| p.id.0))
                                    .collect();
                                for id in &all_pane_ids {
                                    if !guard.terminals.contains_key(id) {
                                        let terminal = crate::terminal::Terminal::new(
                                            rows as usize,
                                            cols as usize,
                                        );
                                        guard.terminals.insert(*id, terminal);
                                        match guard.pty_manager.spawn(*id, cols, rows) {
                                            Ok(reader) => {
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
                                                if let Some(t) = guard.terminals.get_mut(id) {
                                                    t.process_bytes(
                                                        format!("Failed to spawn shell: {}\r\n", e)
                                                            .as_bytes(),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }

                                // Resize any PTYs that already existed.
                                let existing_ids: Vec<u32> =
                                    guard.terminals.keys().copied().collect();
                                for id in existing_ids {
                                    guard.pty_manager.resize(id, cols, rows);
                                    if let Some(t) = guard.terminals.get_mut(&id) {
                                        t.resize(rows as usize, cols as usize);
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

/// Build the list of active subscriptions from current state.
/// Called by the framework after each tree rebuild.
pub fn build_subscriptions(shared: &SharedState) -> Vec<Subscription> {
    let mut subs = Vec::new();

    // Cursor blink timer (always active).
    subs.push(cursor_blink_subscription(shared.clone()));

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
    let guard = shared.lock().expect("state mutex poisoned");
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
