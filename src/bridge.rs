use std::collections::HashMap;
use std::io::Read;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures_core::Stream;
use unshit::app::{EventSink, ExternalEvent, Subscription};
use unshit::core::trace::{append_terminal_trace_line, terminal_trace_enabled};

use crate::state::{record_diagnostic_pty_event, MutexExt, SharedState};

const FOCUS_IN: &[u8] = b"\x1b[I";
const FOCUS_OUT: &[u8] = b"\x1b[O";

#[derive(Clone, Debug, PartialEq, Eq)]
struct TerminalNotification {
    title: String,
    text: String,
    workspace_id: u32,
    pane_id: u32,
}

fn notification_for_terminal_signal(
    signal: &crate::terminal::TerminalSignal,
    workspace_id: Option<u32>,
    pane_id: u32,
    is_codex: bool,
) -> Option<TerminalNotification> {
    let workspace_id = workspace_id?;
    match signal {
        crate::terminal::TerminalSignal::Bell if is_codex => Some(TerminalNotification {
            title: "Codex needs attention".to_string(),
            text: "Codex is waiting for you.".to_string(),
            workspace_id,
            pane_id,
        }),
        crate::terminal::TerminalSignal::Osc9(message) => Some(TerminalNotification {
            title: if is_codex {
                "Codex"
            } else {
                "Terminal notification"
            }
            .to_string(),
            text: message.clone(),
            workspace_id,
            pane_id,
        }),
        crate::terminal::TerminalSignal::Bell => None,
    }
}

fn signals_contain_bell(signals: &[crate::terminal::TerminalSignal]) -> bool {
    signals
        .iter()
        .any(|signal| matches!(signal, crate::terminal::TerminalSignal::Bell))
}

/// Whether a pane should receive focus reports. The parser-tracked mode is
/// authoritative for terminals that explicitly requested DECSET 1004. The
/// Codex fallback covers sessions attached from a legacy daemon snapshot that
/// did not persist that mode bit.
fn should_send_focus_reports(focus_reporting_active: bool, is_codex: bool) -> bool {
    focus_reporting_active || is_codex
}

/// Return the panes that should receive focus reports in stable order. Only
/// the active pane is focused while the native window is focused; every other
/// pane receives FocusOut so a background Codex session can emit its normal
/// `unfocused` notification.
fn focus_reporting_targets(state: &crate::state::AppState) -> Vec<u32> {
    let mut targets: Vec<u32> = state
        .terminals
        .iter()
        .filter_map(|(&pane_id, terminal)| {
            let focus_reporting_active = terminal.lock_recover().focus_reporting_active();
            let is_codex = crate::state::agent_tag_for_pane(state, pane_id)
                .is_some_and(|tag| tag.profile == "codex");
            should_send_focus_reports(focus_reporting_active, is_codex).then_some(pane_id)
        })
        .collect();
    targets.sort_unstable();
    targets
}

/// Codex-only target set used by the periodic bridge tick. Process
/// classification can arrive after the first attach tick, while parser-mode
/// transitions already synchronize immediately in the PTY path. Tracking this
/// narrower set catches the late classification without repeating every
/// parser-mode write on the next tick.
fn codex_focus_reporting_targets(state: &crate::state::AppState) -> Vec<u32> {
    let mut targets: Vec<u32> = state
        .terminals
        .keys()
        .copied()
        .filter(|&pane_id| {
            crate::state::agent_tag_for_pane(state, pane_id)
                .is_some_and(|tag| tag.profile == "codex")
        })
        .collect();
    targets.sort_unstable();
    targets
}

fn focus_report_bytes(window_focused: bool, active_pane: u32, pane_id: u32) -> &'static [u8] {
    if window_focused && pane_id == active_pane {
        FOCUS_IN
    } else {
        FOCUS_OUT
    }
}

fn sync_terminal_focus_reporting(state: &mut crate::state::AppState, window_focused: bool) {
    let active_pane = state.active_pane.0;
    let focus_reporting_panes = focus_reporting_targets(state);

    for pane_id in focus_reporting_panes {
        let bytes = focus_report_bytes(window_focused, active_pane, pane_id);
        if let Err(error) = state.pty_manager.write(pane_id, bytes) {
            log::debug!(
                "pty-{}: could not send terminal focus report: {}",
                pane_id,
                error
            );
        }
    }
}

/// Called directly from the framework's native focus callback, before the
/// next PTY batch can arrive. This keeps Codex's `unfocused` notification
/// policy synchronized with the real window state.
pub fn report_window_focus(shared: &SharedState, focused: bool) {
    let mut state = shared.lock_recover();
    sync_terminal_focus_reporting(&mut state, focused);
}

struct PendingReader {
    generation: u64,
    reader: Box<dyn Read + Send>,
}

static PENDING_READERS: Mutex<Option<HashMap<u32, PendingReader>>> = Mutex::new(None);
static ACTIVE_READER_GENERATIONS: Mutex<Option<HashMap<u32, u64>>> = Mutex::new(None);
static NEXT_READER_GENERATION: AtomicU64 = AtomicU64::new(1);

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

fn should_patch_terminal_grid(synchronized_output_active: bool) -> bool {
    !synchronized_output_active
}

pub fn register_reader(pane_id: u32, reader: Box<dyn Read + Send>) -> u64 {
    let generation = NEXT_READER_GENERATION.fetch_add(1, Ordering::Relaxed);
    let mut guard = PENDING_READERS.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(pane_id, PendingReader { generation, reader });
    let mut active = ACTIVE_READER_GENERATIONS.lock().unwrap();
    active
        .get_or_insert_with(HashMap::new)
        .insert(pane_id, generation);
    generation
}

pub fn reader_generation(pane_id: u32) -> Option<u64> {
    ACTIVE_READER_GENERATIONS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|map| map.get(&pane_id).copied())
}

fn take_all_readers() -> HashMap<u32, PendingReader> {
    let mut guard = PENDING_READERS.lock().unwrap();
    guard.take().unwrap_or_default()
}

// Keep a continuously refilled channel from monopolizing the terminal lock.
// With the 4 KiB reader buffer, a batch contains at most 128 KiB.
const PTY_BATCH_MAX_CHUNKS: u32 = 32;
/// A terminal pane cannot show more than one new grid per 120 Hz frame.
/// Keeping snapshots at this cadence bounds visible-output latency to one
/// frame while letting the reader-channel backpressure merge excess chunks
/// before the next VTE parse and grid clone.
const PTY_GRID_SNAPSHOT_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_micros(8_333);

#[inline]
fn grid_snapshot_delay(
    last_snapshot: Option<std::time::Instant>,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    let elapsed = now.saturating_duration_since(last_snapshot?);
    if elapsed < PTY_GRID_SNAPSHOT_MIN_INTERVAL {
        Some(PTY_GRID_SNAPSHOT_MIN_INTERVAL - elapsed)
    } else {
        None
    }
}

fn process_pty_batch(
    terminal: &mut crate::terminal::Terminal,
    first: &[u8],
    queued: &mut tokio::sync::mpsc::Receiver<Vec<u8>>,
) -> (u32, usize) {
    terminal.process_bytes(first);
    let mut chunks = 1;
    let mut bytes = first.len();
    while chunks < PTY_BATCH_MAX_CHUNKS {
        let Ok(more) = queued.try_recv() else {
            break;
        };
        terminal.process_bytes(&more);
        chunks += 1;
        bytes += more.len();
    }
    (chunks, bytes)
}

/// Create a subscription that reads from a PTY stdout and feeds bytes
/// to the terminal emulator, triggering UI rebuilds.
///
/// Uses a single long-lived blocking task with a channel to avoid
/// per-read task-spawn overhead and buffer allocation.
fn pty_subscription(
    pane_id: u32,
    generation: u64,
    reader: Box<dyn Read + Send>,
    shared: SharedState,
) -> Subscription {
    // Wrap reader in Arc<Mutex<>> so the factory closure is Sync.
    let reader_cell: Arc<Mutex<Option<Box<dyn Read + Send>>>> = Arc::new(Mutex::new(Some(reader)));

    Subscription::new(
        format!("pty-{pane_id}-{generation}"),
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

                // Batch ready chunks into one rebuild request, but cap each
                // batch so a continuously refilled channel releases the
                // terminal mutex and permits intermediate UI updates. The
                // framework coalesces these requests to one rebuild per frame
                // (see `RebuildCoalescer` in `unshit-app/src/app.rs`).
                //
                // Acquire the state mutex only to look up the per-pane
                // Terminal handle, then release it before running the VTE
                // parser. `process_bytes` holds only the per-terminal mutex so
                // the render closure and other state mutators can proceed
                // concurrently on the state lock.
                // Last guest-program title (OSC 0/2) pushed into the pane
                // label. Starts empty so panes without a title keep their
                // default/persisted label until the guest sets one.
                let mut last_osc_title = String::new();
                // Grid cloning is deliberately limited to the fastest target
                // present cadence. The PTY reader keeps collecting bytes in
                // its bounded channel during the short wait, so the next
                // batch represents newer output rather than dropping it.
                let mut last_grid_snapshot = None;

                while let Some(data) = rx.recv().await {
                    if let Some(delay) = grid_snapshot_delay(last_grid_snapshot, std::time::Instant::now()) {
                        tokio::time::sleep(delay).await;
                    }
                    let (
                        terminal_handle,
                        active_pane,
                        theme,
                        custom_theme,
                        selection,
                        link_hover,
                        grid_ref,
                    ) = {
                        let guard = shared.lock_recover();
                        (
                            guard.terminals.get(&pane_id).cloned(),
                            guard.active_pane.0,
                            guard.theme.clone(),
                            guard.custom_theme,
                            guard.terminal_selections.get(&pane_id).copied(),
                            guard.terminal_link_hover.filter(|hover| hover.pane == pane_id),
                            guard.mounted_terminal_grid_ref(pane_id),
                        )
                    };
                    let Some(terminal_handle) = terminal_handle else {
                        continue;
                    };

                    let (
                        pending_response,
                        pending_signals,
                        focus_reporting_changed,
                        osc_title,
                        batched,
                        total_bytes,
                        grid_patch,
                    ) = {
                        let mut terminal = terminal_handle.lock_recover();
                        let focus_reporting_was_active = terminal.focus_reporting_active();
                        let (batched, total_bytes) =
                            process_pty_batch(&mut terminal, &data, &mut rx);
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
                        // Detect guest title changes (OSC 0/2) while the
                        // terminal mutex is held; the state-lock block
                        // below pushes them into the pane/tab labels.
                        let osc_title: Option<String> = if terminal.title() != last_osc_title {
                            Some(terminal.title().to_string())
                        } else {
                            None
                        };
                        let grid_patch = should_patch_terminal_grid(terminal.synchronized_output_active())
                            .then(|| {
                                grid_ref.map(|node| {
                                    let grid = crate::ui::terminal_grid::terminal_display_snapshot(
                                        &terminal,
                                        active_pane == pane_id,
                                        &theme,
                                        &custom_theme,
                                        selection,
                                        link_hover,
                                    );
                                    (node, grid)
                                })
                            })
                            .flatten();
                        (
                            terminal.take_pending_response(),
                            terminal.take_pending_signals(),
                            terminal.focus_reporting_active() != focus_reporting_was_active,
                            osc_title,
                            batched,
                            total_bytes,
                            grid_patch,
                        )
                    };
                    let title_changed = osc_title.is_some();
                    let terminal_notifications = {
                        let mut guard = shared.lock_recover();
                        record_diagnostic_pty_event(
                            &mut guard,
                            format!(
                                "read pane={} bytes={} batched={}",
                                pane_id, total_bytes, batched
                            ),
                        );
                        if let Some(title) = osc_title {
                            // Mirror the guest window title onto the pane
                            // label the way Windows Terminal renames its
                            // tab. Manual renames win inside the mutator.
                            if crate::state::mutate_apply_osc_title(&mut guard, pane_id, &title) {
                                record_diagnostic_pty_event(
                                    &mut guard,
                                    format!("osc-title pane={} title={:?}", pane_id, title),
                                );
                            }
                            last_osc_title = title;
                        }
                        if focus_reporting_changed {
                            // Codex enables DECSET 1004 after startup. Send
                            // the current state immediately so an already
                            // unfocused window does not leave it assuming it
                            // owns terminal focus until the next native event.
                            sync_terminal_focus_reporting(
                                &mut guard,
                                unshit::core::cell_grid::CellGrid::is_window_focused(),
                            );
                        }
                        if !pending_response.is_empty() {
                            // Reply to host queries (DA1, DA2, DSR, CPR,
                            // XTVERSION) the parser collected. Done outside
                            // the per-terminal mutex; the write is fire and
                            // forget through the daemon shim.
                            if let Err(e) = guard.pty_manager.write(pane_id, &pending_response) {
                                log::warn!(
                                    "pty-{}: failed to write {} bytes of query reply: {}",
                                    pane_id,
                                    pending_response.len(),
                                    e
                                );
                            } else {
                                record_diagnostic_pty_event(
                                    &mut guard,
                                    format!(
                                        "write pane={} bytes={} source=query_reply",
                                        pane_id,
                                        pending_response.len()
                                    ),
                                );
                            }
                        }
                        let workspace_id = crate::state::workspace_num_for_pane(&guard, pane_id);
                        let is_codex = crate::state::agent_tag_for_pane(&guard, pane_id)
                            .is_some_and(|tag| tag.profile == "codex");
                        let notifications: Vec<_> = pending_signals
                            .iter()
                            .filter_map(|signal| {
                                notification_for_terminal_signal(
                                    signal,
                                    workspace_id,
                                    pane_id,
                                    is_codex,
                                )
                            })
                            .collect();
                        for notification in &notifications {
                            crate::state::push_notification_toast(
                                &mut guard,
                                notification.title.clone(),
                                notification.text.clone(),
                                notification.workspace_id,
                                notification.pane_id,
                            );
                        }
                        notifications
                    };
                    #[cfg(not(test))]
                    for notification in &terminal_notifications {
                        if let Err(error) = crate::notifications::spawn_desktop_notification_for_target(
                            notification.title.clone(),
                            notification.text.clone(),
                            notification.workspace_id,
                            notification.pane_id,
                        ) {
                            log::warn!(
                                "desktop terminal notification failed for pane {}: {}",
                                notification.pane_id,
                                error
                            );
                        }
                    }
                    if batched > 1 {
                        log::debug!("pty-{}: batched {} chunks into 1 rebuild", pane_id, batched);
                    }
                    if signals_contain_bell(&pending_signals) {
                        yield ExternalEvent::Bell;
                    }
                    if let Some((node, grid)) = grid_patch {
                        last_grid_snapshot = Some(std::time::Instant::now());
                        yield ExternalEvent::GridPatch { node, grid: Box::new(grid) };
                    }
                    if title_changed || !terminal_notifications.is_empty() {
                        yield ExternalEvent::RequestRebuild;
                    }
                }
                let retryable = {
                    let mut guard = shared.lock_recover();
                    crate::state::mark_agent_resume_stream_ended(
                        &mut guard,
                        pane_id,
                        generation,
                    )
                };
                if retryable {
                    yield ExternalEvent::RequestRebuild;
                }
            })
        },
    )
}

/// Cursor focus, toast bookkeeping, and deferred PTY spawn subscription.
///
/// Runs every 500 ms. Each tick:
///   * Tracks active-pane changes without mutating terminal-owned cursor
///     visibility. TUI cursor visibility (`CSI ?25h/l`) stays in the
///     terminal grid; active/inactive pane masking happens on the render
///     snapshot clone. The actual blink animation is driven by the
///     renderer's global blink phase clock (#135 Phase 1, item 2).
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
                let mut last_codex_focus_targets: Vec<u32> = Vec::new();
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let mut needs_rebuild = false;
                    {
                        let mut guard = shared.lock_recover();

                        // Cursor visibility belongs to the terminal stream:
                        // `CSI ?25l` must be able to hide the cursor while a
                        // TUI draws its own prompt cursor. Render snapshots
                        // hide inactive panes by cloning the grid and clearing
                        // the clone's cursor flag, so focus tracking here only
                        // requests a rebuild if a missed active-pane transition
                        // needs to be reflected in the tree.
                        let active_id = guard.active_pane.0;
                        let win_focused = unshit::core::cell_grid::CellGrid::is_window_focused();
                        let signature = (active_id, win_focused);
                        let codex_focus_targets = codex_focus_reporting_targets(&guard);
                        let active_pane_changed = last_focus_signature
                            .map_or(true, |(last_active, _)| last_active != active_id);
                        let focus_changed = last_focus_signature != Some(signature);
                        let codex_targets_changed =
                            last_codex_focus_targets != codex_focus_targets;
                        if active_pane_changed || codex_targets_changed {
                            last_focus_signature = Some(signature);
                            last_codex_focus_targets = codex_focus_targets;
                            // Pane switches turn the old terminal into a
                            // background session and make the new one active.
                            // Keep DECSET 1004 clients synchronized even
                            // though active-pane state is mutated by several
                            // UI paths.
                            sync_terminal_focus_reporting(&mut guard, win_focused);
                        } else if focus_changed {
                            // The native focus callback owns window-only
                            // transitions and already sent the matching
                            // report. Keep the signature current so this tick
                            // still requests the appropriate tree rebuild.
                            last_focus_signature = Some(signature);
                        }
                        if focus_changed {
                            // A pane focus change is observable in the tree
                            // (e.g. focused pane border styling and cursor
                            // masking), so promote this tick to a rebuild.
                            needs_rebuild = true;
                        }

                        // Toast lifetimes are tick-driven from this same
                        // 500 ms cadence. ToastStore::with_capacity(_, 8)
                        // gives ~4 s before auto-dismiss. We rebuild only
                        // if a toast was actually dismissed so a quiet
                        // toast queue does not keep waking the tree.
                        let dismissed = guard.toasts.advance_ticks(1);
                        if !dismissed.is_empty() {
                            crate::state::prune_toast_metadata(&mut guard);
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
                            record_diagnostic_pty_event(
                                &mut guard,
                                format!("write_failed pane={} error={}", err.pane_id, err.error),
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
                                    // Editor panes have no PTY by design;
                                    // spawning one here would shadow the
                                    // editor with a shell session.
                                    if !guard.terminals.contains_key(id)
                                        && !guard.editors.contains_key(id)
                                        && !guard.flows.contains_key(id)
                                    {
                                        let spawn_plan = crate::state::pane_agent_spawn_plan(
                                            &guard,
                                            *id,
                                            cwd.clone(),
                                            shell.clone(),
                                        );
                                        let launch_prepared =
                                            crate::state::prepare_agent_resume_launch(
                                                &mut guard,
                                                *id,
                                                workspace_id,
                                                &spawn_plan,
                                                "deferred",
                                            );
                                        let reconcile_result = if launch_prepared {
                                            guard.pty_manager.attach_or_spawn(
                                                *id,
                                                workspace_id,
                                                cols,
                                                rows,
                                                spawn_plan.cwd.as_deref(),
                                                spawn_plan.shell.as_ref(),
                                            )
                                        } else {
                                            Err(std::io::Error::new(
                                                std::io::ErrorKind::PermissionDenied,
                                                "agent recovery launch preflight was not durable",
                                            ))
                                        };
                                        match reconcile_result {
                                            Ok((Some(snapshot), reader)) => {
                                                let snap_rows = snapshot.grid.rows();
                                                let snap_cols = snapshot.grid.cols();
                                                let mut terminal = crate::terminal::Terminal::new(
                                                    snap_rows, snap_cols,
                                                );
                                                terminal.apply_snapshot(&snapshot);
                                                terminal.set_telemetry_pane(*id);
                                                guard.terminals.insert(
                                                    *id,
                                                    std::sync::Arc::new(std::sync::Mutex::new(
                                                        terminal,
                                                    )),
                                                );
                                                crate::bridge::register_reader(*id, reader);
                                                if let Some(session_id) =
                                                    guard.pty_manager.session_id(*id)
                                                {
                                                    record_diagnostic_pty_event(
                                                        &mut guard,
                                                        format!(
                                                            "attach pane={} session={} source=deferred",
                                                            id, session_id
                                                        ),
                                                    );
                                                }
                                                crate::state::apply_agent_spawn_outcome(
                                                    &mut guard,
                                                    *id,
                                                    workspace_id,
                                                    &spawn_plan,
                                                    true,
                                                    "deferred",
                                                );
                                                log::info!(
                                                    "deferred reattach for pane {}: {}x{}",
                                                    id, snap_cols, snap_rows
                                                );
                                            }
                                            Ok((None, reader)) => {
                                                let mut terminal = crate::terminal::Terminal::new(
                                                    rows as usize,
                                                    cols as usize,
                                                );
                                                terminal.set_telemetry_pane(*id);
                                                guard.terminals.insert(
                                                    *id,
                                                    std::sync::Arc::new(std::sync::Mutex::new(
                                                        terminal,
                                                    )),
                                                );
                                                crate::bridge::register_reader(*id, reader);
                                                if let Some(session_id) =
                                                    guard.pty_manager.session_id(*id)
                                                {
                                                    record_diagnostic_pty_event(
                                                        &mut guard,
                                                        format!(
                                                            "spawn pane={} session={} source=deferred",
                                                            id, session_id
                                                        ),
                                                    );
                                                }
                                                crate::state::apply_agent_spawn_outcome(
                                                    &mut guard,
                                                    *id,
                                                    workspace_id,
                                                    &spawn_plan,
                                                    false,
                                                    "deferred",
                                                );
                                                log::info!(
                                                    "deferred PTY spawn for pane {}: {}x{}",
                                                    id, cols, rows
                                                );
                                            }
                                            Err(e) => {
                                                crate::state::record_agent_spawn_failure(
                                                    &mut guard,
                                                    *id,
                                                    workspace_id,
                                                    &spawn_plan,
                                                    "deferred",
                                                    &e,
                                                );
                                                log::error!(
                                                    "failed to spawn deferred PTY for pane {}: {}",
                                                    id, e
                                                );
                                                let mut terminal = crate::terminal::Terminal::new(
                                                    rows as usize,
                                                    cols as usize,
                                                );
                                                terminal.set_telemetry_pane(*id);
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
                                    if let Some(t) = guard.terminals.get(&id) {
                                        t.lock_recover()
                                            .resize_viewport_growth(rows as usize, cols as usize);
                                    }
                                    guard.pty_manager.resize(id, cols, rows);
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
                        let should_redraw = {
                            let mut guard = shared.lock_recover();
                            let pane_count: usize = guard.panes.iter().map(Vec::len).sum();

                            // Renderer pending resize has no pane identity. In a split layout,
                            // different panes can legitimately have different terminal sizes, so
                            // applying one pane's pending size to every PTY creates a resize
                            // feedback loop between adjacent panes. The per-pane on_resize handler
                            // owns split-pane sizing; this global fallback is only safe for a
                            // single visible pane.
                            if pane_count == 1 {
                                let ids: Vec<u32> = guard.terminals.keys().copied().collect();
                                for id in ids {
                                    if let Some(t) = guard.terminals.get(&id) {
                                        t.lock_recover()
                                            .resize_viewport_growth(rows as usize, cols as usize);
                                    }
                                    guard.pty_manager.resize(id, cols, rows);
                                }
                                true
                            } else {
                                false
                            }
                        }; // guard drops before yield
                        if should_redraw {
                            yield ExternalEvent::RequestRedraw;
                        }
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

    // Local notification IPC: accepts `terminal-manager notify` calls
    // from child processes running inside managed terminals.
    subs.push(crate::notifications::notification_subscription(
        shared.clone(),
    ));

    // Pick up any newly registered readers and create subscriptions for them.
    let pending = take_all_readers();
    let replaced_panes = pending
        .keys()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    for (pane_id, pending) in pending {
        subs.push(pty_subscription(
            pane_id,
            pending.generation,
            pending.reader,
            shared.clone(),
        ));
    }

    // For existing terminals, emit identity-only subscriptions so the
    // framework keeps already-running streams alive.
    let guard = shared.lock_recover();
    for &pane_id in guard.terminals.keys() {
        if replaced_panes.contains(&pane_id) {
            continue;
        }
        let generation = reader_generation(pane_id).unwrap_or(0);
        subs.push(Subscription::new(
            format!("pty-{pane_id}-{generation}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_pty_batch_answers_queries_without_waiting_for_more_output() {
        let (_tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut terminal = crate::terminal::Terminal::new(4, 12);
        let input = b"x\x1b[6n";
        assert_eq!(
            process_pty_batch(&mut terminal, input, &mut rx),
            (1, input.len())
        );
        assert_eq!(terminal.take_pending_response(), b"\x1b[1;2R");
    }

    #[test]
    fn pty_batches_release_queued_tail_and_preserve_terminal_state() {
        let input =
            "hello\u{4e2d}\u{1f642}\x1b[31mred\x1b[0m\r\n\x1b]2;batch-title\x07\x1b[>c".repeat(50);
        let chunks: Vec<Vec<u8>> = input.as_bytes().chunks(7).map(<[u8]>::to_vec).collect();
        let (tx, mut rx) = tokio::sync::mpsc::channel(chunks.len());
        for chunk in &chunks[1..] {
            tx.try_send(chunk.clone()).unwrap();
        }
        drop(tx);
        let mut actual = crate::terminal::Terminal::new(4, 12);
        let mut expected = crate::terminal::Terminal::new(4, 12);
        let mut first = chunks[0].clone();
        let mut consumed = 0usize;
        let mut batches = 0;
        loop {
            let (count, bytes) = process_pty_batch(&mut actual, &first, &mut rx);
            assert!(
                count <= PTY_BATCH_MAX_CHUNKS,
                "one batch consumed the queued tail"
            );
            let end = consumed + count as usize;
            assert_eq!(
                bytes,
                chunks[consumed..end].iter().map(Vec::len).sum::<usize>()
            );
            for chunk in &chunks[consumed..end] {
                expected.process_bytes(chunk);
            }
            assert_eq!(actual.cursor_position(), expected.cursor_position());
            assert_eq!(actual.title(), expected.title());
            assert_eq!(actual.scrollback_len(), expected.scrollback_len());
            assert_eq!(
                actual.take_pending_response(),
                expected.take_pending_response()
            );
            for row in 0..4 {
                for col in 0..12 {
                    assert_eq!(
                        actual.grid().get_cell(row, col),
                        expected.grid().get_cell(row, col)
                    );
                }
            }
            let last = actual.abs_line_at_display(3);
            assert_eq!(
                actual.selection_text((0, 0), (last, 11)),
                expected.selection_text((0, 0), (last, 11))
            );
            consumed = end;
            batches += 1;
            match rx.try_recv() {
                Ok(next) => first = next,
                Err(_) => break,
            }
        }
        assert!(
            batches > 1,
            "sustained output must allow intermediate updates"
        );
        assert_eq!(consumed, chunks.len(), "every input byte must be processed");
    }

    #[test]
    fn terminal_updates_patch_when_not_in_synchronized_output() {
        assert!(should_patch_terminal_grid(false));
    }

    #[test]
    fn terminal_updates_do_not_patch_mid_synchronized_output_frame() {
        assert!(!should_patch_terminal_grid(true));
    }

    #[test]
    fn codex_bell_maps_to_an_attention_notification() {
        let notification = notification_for_terminal_signal(
            &crate::terminal::TerminalSignal::Bell,
            Some(3),
            7,
            true,
        )
        .expect("Codex bell notification");

        assert_eq!(notification.title, "Codex needs attention");
        assert_eq!(notification.text, "Codex is waiting for you.");
        assert_eq!((notification.workspace_id, notification.pane_id), (3, 7));
    }

    #[test]
    fn generic_bell_does_not_create_a_desktop_notification() {
        assert_eq!(
            notification_for_terminal_signal(
                &crate::terminal::TerminalSignal::Bell,
                Some(3),
                7,
                false,
            ),
            None
        );
    }

    #[test]
    fn only_bel_signals_drive_the_framework_bell() {
        assert!(!signals_contain_bell(&[
            crate::terminal::TerminalSignal::Osc9("approval requested".into(),)
        ]));
        assert!(signals_contain_bell(&[
            crate::terminal::TerminalSignal::Osc9("approval requested".into()),
            crate::terminal::TerminalSignal::Bell,
        ]));
    }

    #[test]
    fn osc9_maps_message_to_the_emitting_pane() {
        let notification = notification_for_terminal_signal(
            &crate::terminal::TerminalSignal::Osc9("approval requested; edit files".into()),
            Some(5),
            11,
            true,
        )
        .expect("OSC 9 notification");

        assert_eq!(notification.title, "Codex");
        assert_eq!(notification.text, "approval requested; edit files");
        assert_eq!((notification.workspace_id, notification.pane_id), (5, 11));
    }

    #[test]
    fn focus_reports_only_the_active_terminal_as_focused() {
        assert_eq!(focus_report_bytes(true, 2, 2), FOCUS_IN);
        assert_eq!(focus_report_bytes(true, 2, 1), FOCUS_OUT);
        assert_eq!(focus_report_bytes(false, 2, 2), FOCUS_OUT);
    }

    #[test]
    fn known_codex_panes_use_focus_reports_after_legacy_snapshot_reattach() {
        assert!(should_send_focus_reports(false, true));
    }

    #[test]
    fn ordinary_shells_without_decset_1004_do_not_get_focus_reports() {
        assert!(!should_send_focus_reports(false, false));
    }

    #[test]
    fn explicit_decset_1004_still_enables_focus_reports_for_any_terminal() {
        assert!(should_send_focus_reports(true, false));
    }

    #[test]
    fn focus_reporting_targets_are_sorted_and_exclude_unknown_shells() {
        let mut state = crate::state::seed_state();
        for pane_id in [9, 3, 5] {
            state.terminals.insert(
                pane_id,
                std::sync::Arc::new(std::sync::Mutex::new(crate::terminal::Terminal::new(2, 2))),
            );
        }
        state.pane_agents.insert(
            9,
            crate::agents::AgentTag::new("codex", crate::agents::AgentTagSource::Process),
        );
        state.pane_agents.insert(
            3,
            crate::agents::AgentTag::new("codex", crate::agents::AgentTagSource::Process),
        );
        state.pane_agents.insert(
            5,
            crate::agents::AgentTag::new("shell", crate::agents::AgentTagSource::Process),
        );

        assert_eq!(focus_reporting_targets(&state), vec![3, 9]);
        assert_eq!(codex_focus_reporting_targets(&state), vec![3, 9]);
    }

    #[test]
    fn grid_snapshot_delay_caps_visible_updates_at_120hz() {
        let start = std::time::Instant::now();
        assert_eq!(
            grid_snapshot_delay(Some(start), start),
            Some(PTY_GRID_SNAPSHOT_MIN_INTERVAL)
        );
        assert_eq!(
            grid_snapshot_delay(
                Some(start),
                start + PTY_GRID_SNAPSHOT_MIN_INTERVAL - std::time::Duration::from_micros(1)
            ),
            Some(std::time::Duration::from_micros(1))
        );
        assert_eq!(
            grid_snapshot_delay(Some(start), start + PTY_GRID_SNAPSHOT_MIN_INTERVAL),
            None
        );
    }
}
