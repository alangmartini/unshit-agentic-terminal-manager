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
                let mut reader = {
                    let mut guard = reader_cell.lock().unwrap();
                    match guard.take() {
                        Some(r) => r,
                        None => return,
                    }
                };

                loop {
                    let result = tokio::task::spawn_blocking({
                        let mut buf = vec![0u8; 4096];
                        move || {
                            match reader.read(&mut buf) {
                                Ok(0) => (reader, buf, 0),
                                Ok(n) => (reader, buf, n),
                                Err(_) => (reader, buf, 0),
                            }
                        }
                    })
                    .await;

                    match result {
                        Ok((r, buf, n)) if n > 0 => {
                            reader = r;
                            {
                                let mut guard = shared.lock().expect("state mutex poisoned");
                                if let Some(terminal) = guard.terminals.get_mut(&pane_id) {
                                    terminal.process_bytes(&buf[..n]);
                                }
                            }
                            yield ExternalEvent::RequestRebuild;
                        }
                        _ => break,
                    }
                }
            })
        },
    ))
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
                            let mut guard = shared.lock().expect("state mutex poisoned");
                            let ids: Vec<u32> = guard.terminals.keys().copied().collect();
                            for id in ids {
                                guard.pty_manager.resize(id, cols, rows);
                                if let Some(t) = guard.terminals.get_mut(&id) {
                                    t.resize(rows as usize, cols as usize);
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
