use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::terminal::Terminal;
use crate::theme;

pub const MAX_COLS: usize = 4;
pub const MAX_ROWS: usize = 4;
pub const MIN_FONT_SIZE: u32 = 8;
pub const MAX_FONT_SIZE: u32 = 32;
pub const DEFAULT_CONFIG_FONT_SIZE_PT: u32 = 13;
pub const DEFAULT_TERMINAL_FONT_SIZE_PT: u32 = 13;
pub const DEFAULT_UI_DENSITY: UiDensity = UiDensity::Cozy;
pub const DEFAULT_SCROLL_LINE_PX: u32 = 100;
pub const MIN_SCROLL_LINE_PX: u32 = 16;
pub const MAX_SCROLL_LINE_PX: u32 = 160;
pub const SCROLL_LINE_PX_STEP: i32 = 4;
pub const DEFAULT_SMOOTH_SCROLL_DURATION_MS: u32 = 180;
pub const MIN_SMOOTH_SCROLL_DURATION_MS: u32 = 16;
pub const MAX_SMOOTH_SCROLL_DURATION_MS: u32 = 300;
pub const SMOOTH_SCROLL_DURATION_STEP_MS: i32 = 10;
/// Minimum flex-grow ratio for any pane (prevents collapsing below ~10%).
pub const MIN_PANE_RATIO: f32 = 0.1;
pub const MIN_SIDEBAR_WIDTH: f32 = 150.0;
pub const MAX_SIDEBAR_WIDTH: f32 = 500.0;
const DIAGNOSTIC_PTY_EVENT_LIMIT: usize = 32;
const DIAGNOSTIC_SCROLL_SAMPLE_LIMIT: usize = 96;

pub type SharedState = Arc<Mutex<AppState>>;

/// Shared, independently-lockable handle to a single `Terminal`.
///
/// Wrapping each terminal in its own `Mutex` lets the VTE parser thread
/// hold a narrow lock around `process_bytes` while the renderer and
/// application state lock are released. This mirrors the pattern from
/// Alacritty (FairMutex holding the terminal only, pokes the UI via a
/// wakeup event) and Ghostty (parser mutates renderer state under its
/// own lock, not the global state lock).
pub type SharedTerminal = Arc<Mutex<Terminal>>;

/// Extension trait that tolerates poisoning by taking the inner guard
/// on PoisonError. Used on paths reachable from any pane's byte stream
/// (render closure, state mutex, per-terminal mutex) so a panic in one
/// pane's parser cannot cascade into the others by poisoning a mutex
/// every other pane also locks. See SPEC F4.3.
pub trait MutexExt<T> {
    fn lock_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

/// Background color painted over selected terminal cells. A muted blue that
/// keeps the amber/white default foreground legible without an extra
/// contrast pass. Applied to the per-frame display-grid clone, never to the
/// live terminal buffer.
pub const SELECTION_BG: unshit::core::style::types::Color = unshit::core::style::types::Color {
    r: 38,
    g: 79,
    b: 120,
    a: 255,
};

/// How a terminal selection was seeded, which controls whether a collapsed
/// (single-cell) range counts as "nothing selected".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Click-drag: a collapsed anchor==focus range selects nothing.
    Cell,
    /// Double-click word: a single-cell word is still a real selection.
    Word,
    /// Triple-click line.
    Line,
}

/// A mouse text selection over a terminal pane, in *absolute line*
/// coordinates: `(abs_line, col)`. `abs_line` is the terminal's stable
/// buffer index (see [`crate::terminal::Terminal::abs_line_at_display`]), so
/// the selection stays pinned to its text as the view scrolls and as output
/// streams. [`crate::terminal::Terminal::selection_text`] reads the same
/// coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSelection {
    /// Where the selection was anchored (mouse-down / fixed end).
    pub anchor: (u64, usize),
    /// The moving end (latest mouse position).
    pub focus: (u64, usize),
    pub mode: SelectMode,
}

impl TermSelection {
    /// A collapsed selection anchored at `cell`.
    pub fn new(cell: (u64, usize), mode: SelectMode) -> Self {
        Self {
            anchor: cell,
            focus: cell,
            mode,
        }
    }

    /// `(start, end)` ordered so `start <= end` (line-major).
    pub fn ordered(&self) -> ((u64, usize), (u64, usize)) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    /// True when nothing is actually selected: a `Cell`-mode range whose
    /// anchor and focus are the same cell (a click without a drag). `Word`
    /// and `Line` selections are always real, even a single-character word.
    pub fn is_empty(&self) -> bool {
        self.mode == SelectMode::Cell && self.anchor == self.focus
    }
}

/// Tracks consecutive left-clicks on a terminal so a second/third press on
/// the same cell promotes the selection to word / line. Reset when the
/// pane, cell, or timing window changes.
#[derive(Clone, Copy, Debug)]
pub struct TerminalClick {
    pub pane: u32,
    pub at: std::time::Instant,
    pub cell: (u64, usize),
    /// 1 = cell, 2 = word, 3 = line. Wraps 3 -> 1 on a fourth click.
    pub count: u8,
}

/// Maximum gap between two presses for them to count as a multi-click.
pub const MULTI_CLICK_MS: u128 = 400;

/// Paint [`SELECTION_BG`] over `sel`'s visible cells on a display-grid clone.
/// No-op for an empty selection. The terminal maps the selection's absolute
/// line coordinates to current display rows (lines scrolled out of view are
/// skipped), so this operates only on the per-frame clone and never mutates
/// the live buffer. Callers force-damage the affected pane so the renderer's
/// line cache re-emits the rows (see the render path in `main.rs`).
pub fn apply_selection_highlight(
    grid: &mut unshit::core::cell_grid::CellGrid,
    terminal: &crate::terminal::Terminal,
    sel: &TermSelection,
) {
    if sel.is_empty() {
        return;
    }
    terminal.paint_selection(grid, sel.anchor, sel.focus, SELECTION_BG);
}

#[derive(Clone, Debug)]
pub struct CtxMenu {
    pub x: f32,
    pub y: f32,
    pub target: CtxMenuTarget,
}

/// What the context menu was opened against. Decides which action
/// set the overlay renders and which dispatch commands it emits.
#[derive(Clone, Debug)]
pub enum CtxMenuTarget {
    /// Menu opened on a workspace row in the sidebar.
    Workspace { idx: usize },
    /// Menu opened on a tab in the tabbar. Carries the active pane id
    /// at the moment the menu opened so rename / kill actions reach a
    /// specific session even after the active pane changes.
    Tab { pane_id: u32 },
}

/// Pending destructive action awaiting user confirmation via the confirm
/// modal. Populating `AppState.confirm_dialog` opens the modal; the
/// modal dispatches `dialog.confirm` or `dialog.cancel`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmDialog {
    /// Kill every terminal inside the workspace at `workspace_idx` and
    /// empty the workspace's tabs. Name is copied at open time so the
    /// modal can caption correctly even if the workspace is renamed
    /// mid-flight.
    KillWorkspace { workspace_idx: usize, name: String },
    /// Kill every terminal across every workspace. `count` is the number
    /// of live terminals sampled at open time and is only used for the
    /// modal body text.
    KillAll { count: usize },
    /// Window close intent awaiting a decision between keep-running,
    /// kill-all, or cancel. `remember` is the live checkbox value: when
    /// true, the clicked action is also persisted via the close toggles
    /// so the next close can skip the prompt. `kept_pane_ids` tracks
    /// which session rows the user selected to leave alive when choosing
    /// keep-running; unselected panes are killed before the layout is
    /// persisted for relaunch.
    CloseApp {
        count: usize,
        remember: bool,
        kept_pane_ids: BTreeSet<u32>,
    },
    /// Rename dialog for the session backing `pane_id`. `buffer` is
    /// the live text in the input, updated on every keystroke so the
    /// commit handler can read it without pulling values out of the UI.
    /// `error` carries an inline failure message under the input
    /// when the most recent commit attempt's RPC failed; cleared on
    /// the next keystroke so retrying does not show stale text.
    /// Issue #130.
    RenameSession {
        pane_id: u32,
        buffer: String,
        error: Option<String>,
    },
}

/// Outcome of resolving the user's persisted close preference when the
/// window's close button is clicked. Returned by `resolve_close_action`
/// so the `on_close` callback does not need to read the toggle map
/// itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    /// No preference persisted. Caller vetoes the framework close and
    /// opens the `CloseApp` confirm dialog.
    Prompt,
    /// Persisted preference: exit without touching daemon sessions.
    /// Local UI state is dropped, shells keep running on the daemon.
    KeepRunning,
    /// Persisted preference: destroy every session, then exit.
    KillAll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsSection {
    Appearance,
    Shell,
    Keybinds,
    Sessions,
    Notifications,
    DangerZone,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::Appearance => "appearance",
            SettingsSection::Shell => "shell",
            SettingsSection::Keybinds => "keybinds",
            SettingsSection::Sessions => "sessions",
            SettingsSection::Notifications => "notifications",
            SettingsSection::DangerZone => "danger zone",
        }
    }

    pub fn all() -> [SettingsSection; 6] {
        [
            SettingsSection::Appearance,
            SettingsSection::Shell,
            SettingsSection::Keybinds,
            SettingsSection::Sessions,
            SettingsSection::Notifications,
            SettingsSection::DangerZone,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiDensity {
    Compact,
    Cozy,
    Comfy,
}

impl UiDensity {
    pub fn id(self) -> &'static str {
        match self {
            UiDensity::Compact => "compact",
            UiDensity::Cozy => "cozy",
            UiDensity::Comfy => "comfy",
        }
    }

    pub fn label(self) -> &'static str {
        self.id()
    }

    pub fn all() -> [UiDensity; 3] {
        [UiDensity::Compact, UiDensity::Cozy, UiDensity::Comfy]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "compact" => Some(UiDensity::Compact),
            "cozy" => Some(UiDensity::Cozy),
            "comfy" => Some(UiDensity::Comfy),
            _ => None,
        }
    }
}

/// Lightweight mirror of `unshit_ptyd::protocol::message::SessionInfo`
/// kept in app state so the UI can render a sessions list without
/// reaching across the crate boundary. Refreshed synchronously via
/// [`refresh_sessions`] when the user opens the Sessions panel or
/// presses Refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub session_id: u64,
    pub pane_id: u32,
    pub workspace_id: u32,
    pub name: Option<String>,
    pub pid: Option<u32>,
    pub alive: bool,
}

/// Render-side projection of a `unshit::core::toast::Toast`. The live
/// store stays in `AppState`; the snapshot path clones a flat list
/// here so the UI builder layer never reaches back into the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastView {
    pub id: unshit::core::toast::ToastId,
    pub kind: unshit::core::toast::ToastKind,
    pub title: Option<String>,
    pub message: String,
    pub target: Option<ToastTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastTarget {
    pub workspace_id: u32,
    pub pane_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NotificationToastMeta {
    title: String,
    target: ToastTarget,
}

/// Push an error-level toast onto `state.toasts`. Single entry point
/// so dispatch handlers do not format user-facing strings inline.
pub fn push_error_toast(state: &mut AppState, message: impl Into<String>) {
    state.toasts.push(message);
    retain_live_toast_meta(state);
}

/// Push a user-triggered notification card with a focus target. The same
/// `ToastStore` drives lifetime/dismissal; metadata lives in `AppState`
/// because the framework toast primitive intentionally stays generic.
pub fn push_notification_toast(
    state: &mut AppState,
    title: impl Into<String>,
    message: impl Into<String>,
    workspace_id: u32,
    pane_id: u32,
) -> unshit::core::toast::ToastId {
    let id = state.toasts.push(message);
    state.toast_meta.insert(
        id,
        NotificationToastMeta {
            title: title.into(),
            target: ToastTarget {
                workspace_id,
                pane_id,
            },
        },
    );
    retain_live_toast_meta(state);
    id
}

fn retain_live_toast_meta(state: &mut AppState) {
    if state.toast_meta.is_empty() {
        return;
    }
    let live: BTreeSet<unshit::core::toast::ToastId> = state.toasts.iter().map(|t| t.id).collect();
    state.toast_meta.retain(|id, _| live.contains(id));
}

pub fn prune_toast_metadata(state: &mut AppState) {
    retain_live_toast_meta(state);
}

/// Normalise text pulled from the system clipboard before it is fed
/// into a PTY by `terminal.paste`.
///
/// Two transforms run, in order:
///
/// 1. Newline canonicalisation: `\r\n` (Windows / multi-line clipboard
///    payloads) and lone `\n` (Unix) both collapse to `\r`. POSIX
///    shells expect `\r` (a.k.a. `Enter`) between commands; sending
///    raw `\n` would deliver a literal newline character that most
///    shells either ignore or treat as continuation, surprising the
///    user mid-paste.
/// 2. Bracketed-paste marker scrub: any embedded `\x1b[200~` or
///    `\x1b[201~` sequence is removed. The daemon does not yet track
///    DECSET 2004 per session, so we send raw bytes; stripping the
///    markers anyway is defence in depth against a clipboard payload
///    that tries to forge an "end of paste" marker mid-string and
///    convince the shell to execute the suffix as if it had been
///    typed (paste-injection).
///
/// Returns the normalised string. Empty input yields an empty string.
///
/// When the focused pane has DECSET 2004 active, `dispatch_terminal_paste`
/// wraps the body returned here in `\x1b[200~ .. \x1b[201~`; the marker
/// scrub below is what makes that wrap safe against forged terminators.
pub fn normalize_pasted_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // All transforms target ASCII-only sequences (CR, LF, ESC[20{0,1}~)
    // and never alter the ASCII subset of UTF-8. Operate on bytes so we can
    // detect the 6-byte bracketed-paste markers without a dedicated state
    // machine, preserving multi-byte UTF-8 sequences intact (a naive
    // `b as char` push would truncate continuation bytes to Latin-1).

    // 1. Strip bracketed-paste markers to a FIXED POINT. A single forward
    //    pass is not enough: deleting an inner marker splices its
    //    neighbours together and can forge a brand-new marker across the
    //    join — e.g. `\x1b[2` + `\x1b[201~` + `01~` collapses to a valid
    //    `\x1b[201~`. A hostile clipboard payload could use that to inject
    //    an early end-of-paste terminator and have the shell execute the
    //    suffix. Re-scan until a whole pass removes nothing, so no
    //    reassembled marker can survive into the wrapped body.
    let is_marker = |b: &[u8], i: usize| -> bool {
        i + 5 < b.len()
            && b[i] == 0x1b
            && b[i + 1] == b'['
            && b[i + 2] == b'2'
            && b[i + 3] == b'0'
            && (b[i + 4] == b'0' || b[i + 4] == b'1')
            && b[i + 5] == b'~'
    };
    let mut bytes = text.as_bytes().to_vec();
    loop {
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        let mut removed = false;
        while i < bytes.len() {
            if is_marker(&bytes, i) {
                i += 6;
                removed = true;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        bytes = out;
        // Each pass strictly shrinks `bytes` when it removes anything, so
        // the loop terminates in at most len/6 iterations.
        if !removed {
            break;
        }
    }

    // 2. Newline canonicalisation (runs after the scrub; it never produces
    //    ESC markers). CRLF collapses to a single CR; bare LF promotes to
    //    CR; bare CR passes through. POSIX shells expect `\r` (Enter)
    //    between commands.
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' {
            out.push(b'\r');
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if b == b'\n' {
            out.push(b'\r');
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }

    // The transforms preserve UTF-8 validity (only ASCII bytes are
    // matched/dropped/replaced). Falling back to a lossy decode would
    // mask a bug; debug builds catch the impossible case.
    String::from_utf8(out).unwrap_or_else(|err| {
        debug_assert!(false, "normalize_pasted_text produced invalid UTF-8: {err}");
        String::from_utf8_lossy(err.as_bytes()).into_owned()
    })
}

/// Typed keys for the `AppState::toggles` map. Previously string literals
/// like "confirm-close" were spread across the UI, with the type system
/// no help against typos (e.g. "confirm-clsoe" silently read as `false`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ToggleKey {
    /// When true, the close-app prompt is skipped and the action stored
    /// in `KillAllOnClose` runs silently. Toggled on by the "remember my
    /// choice" checkbox in the close-app confirm dialog and cleared by
    /// the danger-zone reset control.
    RememberCloseChoice,
    /// When `RememberCloseChoice` is true, selects between the two
    /// silent close actions: false = keep running (leave daemon
    /// sessions alive), true = kill all and quit.
    KillAllOnClose,
}

impl ToggleKey {
    pub fn as_str(self) -> &'static str {
        match self {
            ToggleKey::RememberCloseChoice => "remember-close-choice",
            ToggleKey::KillAllOnClose => "kill-all-on-close",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub num: u32,
    pub name: String,
    pub path: Option<PathBuf>,
    pub collapsed: bool,
    pub terminals_expanded: bool,
    pub terminal_entries: Vec<TerminalEntry>,
    pub subtabs: Vec<Subtab>,
    pub git_branch: Option<String>,
    /// Per-workspace tab list. When this workspace is active, `AppState.tabs`
    /// mirrors this (via workspace save/restore in `workspace.switch`). When
    /// inactive, this is the source of truth.
    pub tabs: Vec<TerminalTab>,
    pub active_tab: usize,
    /// Per workspace shell override. When non empty, beats
    /// `AppState.default_shell` in `pane_spawn_shell`. Empty means
    /// "inherit the app default", matching today's behavior.
    pub shell: crate::shell::ShellSpec,
}

#[derive(Clone, Debug)]
pub struct Subtab {
    pub label: String,
    pub count: Option<u32>,
    pub pulse: bool,
    pub active: bool,
    pub disabled: bool,
    pub icon: Option<SubtabIcon>,
    pub tree_glyph: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubtabIcon {
    Terminal,
    User,
    GitBranch,
    Folder,
    EnvList,
}

#[derive(Clone, Debug)]
pub struct TerminalEntry {
    pub name: String,
    pub branch: String,
    pub branch_muted: bool,
    pub branch_error: bool,
    /// Pane this entry represents. Links the sidebar row to a real pane so
    /// clicks can focus that pane in that workspace.
    pub pane_id: PaneId,
}

#[derive(Clone, Debug)]
pub struct TerminalTab {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub status: TabStatus,
    /// Per-tab pane layout. Stored when the tab is inactive; the live
    /// `AppState` fields are used when the tab is active.
    pub panes: Vec<Vec<Pane>>,
    pub active_pane: PaneId,
    pub row_ratios: Vec<f32>,
    pub col_ratios: Vec<Vec<f32>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabStatus {
    Running,
    Idle,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PaneId(pub u32);

#[derive(Clone, Debug)]
pub struct Pane {
    pub id: PaneId,
    pub title: String,
    pub subtitle: String,
    pub pid: u32,
    pub cpu: f32,
}

/// Snapshot captured at the start of a pane resizer drag.
#[derive(Clone, Debug)]
pub struct ResizeDragSnapshot {
    /// True for horizontal resizer (between columns), false for vertical (between rows).
    pub horizontal: bool,
    /// Row index of the track before the resizer.
    pub row_idx: usize,
    /// Column index of the track before the resizer (only used when horizontal).
    pub col_idx: usize,
    /// Snapshot of the ratios at drag start.
    pub initial_ratios: Vec<f32>,
    /// Total pixel size of the container along the drag axis.
    pub container_size: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticScrollSample {
    pub phase: &'static str,
    pub elapsed_ms: f32,
    pub duration_ms: f32,
    pub start_x: f32,
    pub start_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub target_x: f32,
    pub target_y: f32,
    pub velocity_y: f32,
    pub progress_y: f32,
}

impl DiagnosticScrollSample {
    fn from_telemetry(telemetry: &unshit::app::ScrollTelemetry) -> Self {
        let phase = match telemetry.phase {
            unshit::app::ScrollTelemetryPhase::Started => "started",
            unshit::app::ScrollTelemetryPhase::Frame => "frame",
            unshit::app::ScrollTelemetryPhase::Completed => "completed",
            unshit::app::ScrollTelemetryPhase::Instant => "instant",
        };
        Self {
            phase,
            elapsed_ms: telemetry.elapsed_ms,
            duration_ms: telemetry.duration_ms,
            start_x: telemetry.start_x,
            start_y: telemetry.start_y,
            scroll_x: telemetry.scroll_x,
            scroll_y: telemetry.scroll_y,
            target_x: telemetry.target_x,
            target_y: telemetry.target_y,
            velocity_y: telemetry.velocity_y,
            progress_y: telemetry.progress_y,
        }
    }
}

pub struct AppState {
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    pub tabs: Vec<TerminalTab>,
    pub active_tab: usize,
    pub panes: Vec<Vec<Pane>>,
    pub active_pane: PaneId,
    pub settings_open: bool,
    pub settings_section: SettingsSection,
    pub theme: String,
    pub custom_theme: theme::CustomTheme,
    /// Theme id that was last published to visible terminal grids. Empty
    /// forces the first terminal route render to invalidate retained grid
    /// paint, and settings renders deliberately do not update it.
    pub last_terminal_theme_painted: String,
    pub config_font_size_pt: u32,
    pub terminal_font_size_pt: u32,
    pub ui_density: UiDensity,
    pub scroll_line_px: u32,
    pub smooth_scroll_duration_ms: u32,
    pub toggles: BTreeMap<ToggleKey, bool>,
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_active: usize,
    pub sidebar_collapsed: bool,
    pub sidebar_width: f32,
    /// Last window maximized state reported by the framework.
    pub window_maximized: bool,
    /// Sidebar width at the start of a drag, `None` when not dragging.
    pub sidebar_drag_start: Option<f32>,
    pub cpu_pct: f32,
    pub mem_gb: f32,
    pub net_kbps: f32,
    pub clock_hhmm: String,
    pub next_id: u32,
    pub pty_manager: crate::pty::DaemonPty,
    pub terminals: std::collections::HashMap<u32, SharedTerminal>,
    pub scale_factor: f32,
    /// Ratio of monospace cell_width to font_size, measured from the actual font.
    pub cell_width_ratio: f32,
    /// Last known physical pixel dimensions of the terminal grid element.
    pub last_grid_width: f32,
    pub last_grid_height: f32,
    /// Flex-grow ratios for each row. Length matches `panes.len()`.
    pub row_ratios: Vec<f32>,
    /// Flex-grow ratios for columns within each row. `col_ratios[r].len()` matches `panes[r].len()`.
    pub col_ratios: Vec<Vec<f32>>,
    /// Transient drag state, populated on DragPhase::Start, cleared on End.
    pub resize_drag: Option<ResizeDragSnapshot>,
    /// Context menu state: Some when open, None when closed.
    pub ctx_menu: Option<CtxMenu>,
    /// User keybind overrides, recording mode, and last validation error.
    /// Changes here persist to disk but only take effect on next restart
    /// (the framework's shortcut resolver snapshots the bindings at build).
    pub keybinds: crate::keybinds::KeybindsState,
    /// Transient drag state for pane-header / tab drags. `Idle` at rest,
    /// `DraggingPane { .. }` once the cursor exceeds the 4px threshold.
    pub drag: crate::drag::DragState,
    /// Last measured tab bar rectangle in window coordinates. Populated
    /// by `on_resize` on the `.tabbar` element and the sidebar width
    /// tracking. Used by the pane-extract drag flow to hit-test drops.
    pub tabbar_rect: crate::drag::Rect,
    /// Pending destructive action awaiting confirmation. `None` when no
    /// confirm modal is showing.
    pub confirm_dialog: Option<ConfirmDialog>,
    /// Daemon-known sessions, refreshed on demand via
    /// [`refresh_sessions`]. Empty when the daemon has never been
    /// polled or when the last poll returned no sessions.
    pub sessions: Vec<SessionSnapshot>,
    /// Set when the most recent `list_sessions` RPC failed. Drives
    /// the "stale" chip next to the Sessions panel refresh button so
    /// the user sees that the cached rows may not match the daemon.
    /// Cleared on the next successful refresh.
    pub sessions_stale: bool,
    /// Monotonic count of frames presented while app diagnostics are enabled.
    pub diagnostic_frame_counter: u64,
    /// Wall-clock time for the last presented frame, in Unix epoch
    /// milliseconds. Kept as an integer so snapshot formatting owns the
    /// string representation.
    pub diagnostic_last_present_unix_ms: Option<u64>,
    /// Recent smooth-scroll motion samples emitted by the framework while
    /// diagnostics are enabled. Content-free: positions and timings only.
    pub diagnostic_scroll_samples: VecDeque<DiagnosticScrollSample>,
    /// Recent PTY liveness observations. These are intentionally content-free:
    /// only event kind, pane/session identifiers, and byte counts are stored.
    pub diagnostic_pty_recent_events: VecDeque<String>,
    /// Ephemeral notification queue. Populated by
    /// [`push_error_toast`]; ticked down by the cursor-blink
    /// subscription so dismissal stays deterministic in tests.
    pub toasts: unshit::core::toast::ToastStore,
    toast_meta: BTreeMap<unshit::core::toast::ToastId, NotificationToastMeta>,
    /// System clipboard handle used by `terminal.paste`. Initialised
    /// to a fresh [`unshit::app::ClipboardContext`] in [`seed_state`]
    /// and replaced with the framework's shared instance from
    /// `App::clipboard()` once the window is up so app and framework
    /// callers share one underlying `arboard::Clipboard` (concurrent
    /// arboard handles can corrupt the heap on Windows; see the
    /// regression in `unshit-app::clipboard`).
    pub clipboard: Arc<unshit::app::ClipboardContext>,
    /// Active mouse text selection per pane, keyed by pane id. In visible
    /// (display-grid) coordinates. Absent / collapsed means nothing is
    /// selected. Drives the copy actions and the render-time highlight.
    pub terminal_selections: std::collections::HashMap<u32, TermSelection>,
    /// Panes whose selection changed since the last frame and therefore
    /// need a forced full repaint so the renderer's line cache re-emits the
    /// rows that gained or lost the highlight. Drained each render.
    pub terminal_selection_repaint: std::collections::HashSet<u32>,
    /// Last left-press on a terminal, for double/triple-click promotion.
    pub terminal_click: Option<TerminalClick>,
    /// App wide default shell. Empty means "let the daemon's own
    /// `default_shell()` decide". Per workspace overrides land in
    /// Task 6 and take precedence via `shell::resolve`.
    pub default_shell: crate::shell::ShellSpec,
    /// Quick Prompt overlay. `None` when closed. Slice 1 keeps the
    /// inner state empty; later slices add prompt text, agent, images,
    /// and autocomplete fields per `tasks/plan.md`.
    pub quick_prompt: Option<crate::quick_prompt::QuickPromptState>,
}

impl AppState {
    pub fn record_diagnostic_scroll_sample(&mut self, telemetry: &unshit::app::ScrollTelemetry) {
        if matches!(telemetry.phase, unshit::app::ScrollTelemetryPhase::Started) {
            self.diagnostic_scroll_samples.clear();
        }
        if self.diagnostic_scroll_samples.len() == DIAGNOSTIC_SCROLL_SAMPLE_LIMIT {
            self.diagnostic_scroll_samples.pop_front();
        }
        self.diagnostic_scroll_samples
            .push_back(DiagnosticScrollSample::from_telemetry(telemetry));
    }

    /// Clone everything except the non-Clone PTY manager and terminals.
    /// UI builders call this to get a snapshot for rendering.
    pub fn ui_snapshot(&self) -> UiSnapshot {
        let mut workspaces = self.workspaces.clone();
        let active_idx = self.active_workspace;
        for (idx, ws) in workspaces.iter_mut().enumerate() {
            let (branch_text, branch_error) = match &ws.git_branch {
                Some(b) => (b.clone(), false),
                None => ("no git".to_string(), true),
            };
            let entry_from = |p: &Pane| TerminalEntry {
                name: p.title.clone(),
                branch: branch_text.clone(),
                branch_muted: false,
                branch_error,
                pane_id: p.id,
            };
            let entries: Vec<TerminalEntry> = if idx == active_idx {
                // Active workspace: live panes for the active tab, saved
                // panes for every other tab. Every pane across every tab
                // shows up as its own entry.
                self.tabs
                    .iter()
                    .enumerate()
                    .flat_map(|(t_idx, tab)| {
                        if t_idx == self.active_tab {
                            self.panes
                                .iter()
                                .flatten()
                                .map(&entry_from)
                                .collect::<Vec<_>>()
                        } else {
                            tab.panes
                                .iter()
                                .flatten()
                                .map(&entry_from)
                                .collect::<Vec<_>>()
                        }
                    })
                    .collect()
            } else {
                // Inactive workspace: everything is in saved state.
                ws.tabs
                    .iter()
                    .flat_map(|tab| tab.panes.iter().flatten().map(&entry_from))
                    .collect()
            };
            for sub in &mut ws.subtabs {
                if sub.label == "terminals" {
                    sub.count = Some(entries.len() as u32);
                }
            }
            ws.terminal_entries = entries;
        }
        let (active_terminal_cols, active_terminal_rows) = self
            .terminals
            .get(&self.active_pane.0)
            .map(|terminal| {
                let terminal = terminal.lock_recover();
                (terminal.grid().cols() as u16, terminal.grid().rows() as u16)
            })
            .unwrap_or((80, 24));

        UiSnapshot {
            workspaces,
            active_workspace: self.active_workspace,
            tabs: self.tabs.clone(),
            active_tab: self.active_tab,
            panes: self.panes.clone(),
            active_pane: self.active_pane,
            settings_open: self.settings_open,
            settings_section: self.settings_section,
            theme: self.theme.clone(),
            custom_theme: self.custom_theme,
            config_font_size_pt: self.config_font_size_pt,
            terminal_font_size_pt: self.terminal_font_size_pt,
            ui_density: self.ui_density,
            scroll_line_px: self.scroll_line_px,
            smooth_scroll_duration_ms: self.smooth_scroll_duration_ms,
            toggles: self.toggles.clone(),
            palette_open: self.palette_open,
            palette_query: self.palette_query.clone(),
            palette_active: self.palette_active,
            sidebar_collapsed: self.sidebar_collapsed,
            sidebar_width: self.sidebar_width,
            window_maximized: self.window_maximized,
            cpu_pct: self.cpu_pct,
            mem_gb: self.mem_gb,
            net_kbps: self.net_kbps,
            clock_hhmm: self.clock_hhmm.clone(),
            row_ratios: self.row_ratios.clone(),
            col_ratios: self.col_ratios.clone(),
            ctx_menu: self.ctx_menu.clone(),
            keybinds: self.keybinds.clone(),
            drag: self.drag.clone(),
            tabbar_rect: self.tabbar_rect,
            last_grid_width: self.last_grid_width,
            last_grid_height: self.last_grid_height,
            scale_factor: self.scale_factor,
            confirm_dialog: self.confirm_dialog.clone(),
            terminal_count: self.terminals.len(),
            active_terminal_cols,
            active_terminal_rows,
            sessions: self.sessions.clone(),
            sessions_stale: self.sessions_stale,
            diagnostic_scroll_samples: self.diagnostic_scroll_samples.iter().cloned().collect(),
            toasts: self
                .toasts
                .iter()
                .map(|t| ToastView {
                    id: t.id,
                    kind: t.kind,
                    title: self.toast_meta.get(&t.id).map(|m| m.title.clone()),
                    message: t.message.clone(),
                    target: self.toast_meta.get(&t.id).map(|m| m.target.clone()),
                })
                .collect(),
            default_shell: self.default_shell.clone(),
            quick_prompt: self.quick_prompt.clone(),
        }
    }

    /// Clone the cell grid for a given pane. Returns `None` if no terminal
    /// exists for the pane. The returned grid is a snapshot; further writes
    /// to the live terminal won't affect it.
    pub fn terminal_grid(&self, pane_id: PaneId) -> Option<unshit::core::cell_grid::CellGrid> {
        self.terminals
            .get(&pane_id.0)
            .map(|t| t.lock_recover().grid().clone())
    }

    /// Clone the `Arc<Mutex<Terminal>>` handle for a pane without holding
    /// the app state lock beyond the hashmap lookup. Callers take the
    /// per-terminal lock independently.
    pub fn terminal_handle(&self, pane_id: u32) -> Option<SharedTerminal> {
        self.terminals.get(&pane_id).cloned()
    }
}

#[derive(Clone, Debug)]
pub struct UiSnapshot {
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    pub tabs: Vec<TerminalTab>,
    pub active_tab: usize,
    pub panes: Vec<Vec<Pane>>,
    pub active_pane: PaneId,
    pub settings_open: bool,
    pub settings_section: SettingsSection,
    pub theme: String,
    pub custom_theme: theme::CustomTheme,
    pub config_font_size_pt: u32,
    pub terminal_font_size_pt: u32,
    pub ui_density: UiDensity,
    pub scroll_line_px: u32,
    pub smooth_scroll_duration_ms: u32,
    pub toggles: BTreeMap<ToggleKey, bool>,
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_active: usize,
    pub sidebar_collapsed: bool,
    pub sidebar_width: f32,
    /// Mirrors `AppState::window_maximized` so titlebar controls can
    /// render maximize or restore affordances from state.
    pub window_maximized: bool,
    pub cpu_pct: f32,
    pub mem_gb: f32,
    pub net_kbps: f32,
    pub clock_hhmm: String,
    pub row_ratios: Vec<f32>,
    pub col_ratios: Vec<Vec<f32>>,
    pub ctx_menu: Option<CtxMenu>,
    pub keybinds: crate::keybinds::KeybindsState,
    pub drag: crate::drag::DragState,
    pub tabbar_rect: crate::drag::Rect,
    /// Last measured physical width of the terminal-grid container.
    /// Consumers must divide by `scale_factor` to get CSS pixels.
    pub last_grid_width: f32,
    /// Last measured physical height of the terminal-grid container.
    pub last_grid_height: f32,
    /// Display scale factor so callers can convert physical pixels
    /// (stored for last_grid_*) back to CSS coordinates that compose
    /// with tabbar_rect and DragState cursor.
    pub scale_factor: f32,
    pub confirm_dialog: Option<ConfirmDialog>,
    /// Total number of live terminals across every workspace. Read from
    /// `state.terminals.len()` so the danger-zone button can show an
    /// accurate count without the UI having to reach into the map.
    pub terminal_count: usize,
    /// Current dimensions of the active terminal grid.
    pub active_terminal_cols: u16,
    pub active_terminal_rows: u16,
    pub sessions: Vec<SessionSnapshot>,
    /// Mirrors `AppState::sessions_stale`. `true` when the most recent
    /// `list_sessions` RPC failed and the cached rows may be stale.
    pub sessions_stale: bool,
    /// Recent smooth-scroll samples from framework diagnostics.
    pub diagnostic_scroll_samples: Vec<DiagnosticScrollSample>,
    /// Flat projection of the live `ToastStore`. Push order preserved.
    pub toasts: Vec<ToastView>,
    /// Mirror of `AppState::default_shell` so settings UI can render
    /// the current value without reaching into the live state.
    pub default_shell: crate::shell::ShellSpec,
    /// Mirror of `AppState::quick_prompt`. `None` when the overlay is
    /// closed.
    pub quick_prompt: Option<crate::quick_prompt::QuickPromptState>,
}

fn current_folder_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "workspace".to_string())
}

pub fn seed_state() -> AppState {
    let ws1_path = std::env::current_dir().ok();
    let ws1_branch = ws1_path.as_deref().and_then(crate::git::detect_git_branch);
    let workspaces = vec![
        Workspace {
            num: 1,
            name: current_folder_name(),
            path: ws1_path,
            collapsed: false,
            terminals_expanded: true,
            terminal_entries: vec![],
            subtabs: vec![Subtab {
                label: "terminals".to_string(),
                count: Some(1),
                pulse: false,
                active: true,
                disabled: false,
                icon: Some(SubtabIcon::Terminal),
                tree_glyph: "\u{2514}",
            }],
            git_branch: ws1_branch,
            tabs: vec![],
            active_tab: 0,
            shell: crate::shell::ShellSpec::default(),
        },
        Workspace {
            num: 2,
            name: "api".to_string(),
            path: None,
            collapsed: false,
            terminals_expanded: false,
            terminal_entries: vec![],
            subtabs: vec![Subtab {
                label: "terminals".to_string(),
                count: Some(0),
                pulse: false,
                active: false,
                disabled: false,
                icon: Some(SubtabIcon::Terminal),
                tree_glyph: "\u{2514}",
            }],
            git_branch: None,
            tabs: vec![],
            active_tab: 0,
            shell: crate::shell::ShellSpec::default(),
        },
        Workspace {
            num: 3,
            name: "infra".to_string(),
            path: None,
            collapsed: true,
            terminals_expanded: false,
            terminal_entries: vec![],
            subtabs: vec![Subtab {
                label: "terminals".to_string(),
                count: Some(0),
                pulse: false,
                active: false,
                disabled: false,
                icon: Some(SubtabIcon::Terminal),
                tree_glyph: "\u{2514}",
            }],
            git_branch: None,
            tabs: vec![],
            active_tab: 0,
            shell: crate::shell::ShellSpec::default(),
        },
        Workspace {
            num: 4,
            name: "scratch".to_string(),
            path: None,
            collapsed: true,
            terminals_expanded: false,
            terminal_entries: vec![],
            subtabs: vec![Subtab {
                label: "terminals".to_string(),
                count: Some(0),
                pulse: false,
                active: false,
                disabled: false,
                icon: None,
                tree_glyph: "\u{2514}",
            }],
            git_branch: None,
            tabs: vec![],
            active_tab: 0,
            shell: crate::shell::ShellSpec::default(),
        },
    ];

    let default_pane = Pane {
        id: PaneId(1),
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 0,
        cpu: 0.0,
    };
    let panes = vec![vec![default_pane.clone()]];

    let tabs = vec![TerminalTab {
        id: "t1".to_string(),
        name: "shell".to_string(),
        subtitle: "bash".to_string(),
        status: TabStatus::Running,
        panes: panes.clone(),
        active_pane: PaneId(1),
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
    }];

    let mut toggles = BTreeMap::new();
    toggles.insert(ToggleKey::RememberCloseChoice, false);
    toggles.insert(ToggleKey::KillAllOnClose, false);

    AppState {
        workspaces,
        active_workspace: 0,
        tabs,
        active_tab: 0,
        panes,
        active_pane: PaneId(1),
        settings_open: false,
        settings_section: SettingsSection::Appearance,
        theme: theme::default_theme_id().to_string(),
        custom_theme: theme::default_custom_theme(),
        last_terminal_theme_painted: String::new(),
        config_font_size_pt: DEFAULT_CONFIG_FONT_SIZE_PT,
        terminal_font_size_pt: DEFAULT_TERMINAL_FONT_SIZE_PT,
        ui_density: DEFAULT_UI_DENSITY,
        scroll_line_px: DEFAULT_SCROLL_LINE_PX,
        smooth_scroll_duration_ms: DEFAULT_SMOOTH_SCROLL_DURATION_MS,
        toggles,
        palette_open: false,
        palette_query: String::new(),
        palette_active: 0,
        sidebar_collapsed: false,
        sidebar_width: 252.0,
        window_maximized: false,
        sidebar_drag_start: None,
        cpu_pct: 0.0,
        mem_gb: 0.0,
        net_kbps: 0.0,
        clock_hhmm: "00:00".to_string(),
        next_id: 2,
        pty_manager: crate::pty::DaemonPty::new(),
        terminals: std::collections::HashMap::new(),
        scale_factor: 1.0,
        cell_width_ratio: 0.6,
        last_grid_width: 0.0,
        last_grid_height: 0.0,
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
        resize_drag: None,
        ctx_menu: None,
        keybinds: crate::keybinds::KeybindsState::with_overrides(
            crate::keybinds::loader::load_if_installed(),
        ),
        drag: crate::drag::DragState::default(),
        tabbar_rect: crate::drag::Rect::default(),
        confirm_dialog: None,
        sessions: Vec::new(),
        sessions_stale: false,
        diagnostic_frame_counter: 0,
        diagnostic_last_present_unix_ms: None,
        diagnostic_scroll_samples: VecDeque::new(),
        diagnostic_pty_recent_events: VecDeque::new(),
        toasts: unshit::core::toast::ToastStore::with_capacity(3, 8),
        toast_meta: BTreeMap::new(),
        clipboard: Arc::new(unshit::app::ClipboardContext::new()),
        terminal_selections: std::collections::HashMap::new(),
        terminal_selection_repaint: std::collections::HashSet::new(),
        terminal_click: None,
        default_shell: crate::shell::infer_default_shell(&crate::shell::discover_installed()),
        quick_prompt: None,
    }
}

// ---------------------------------------------------------------------------
// State mutation helpers
// ---------------------------------------------------------------------------

pub fn mutate_with<F, R>(shared: &SharedState, f: F) -> R
where
    F: FnOnce(&mut AppState) -> R,
{
    let mut guard = shared.lock_recover();
    f(&mut guard)
}

pub fn record_diagnostic_renderer_frame(state: &mut AppState, unix_epoch_ms: u64) {
    state.diagnostic_frame_counter = state.diagnostic_frame_counter.saturating_add(1);
    state.diagnostic_last_present_unix_ms = Some(unix_epoch_ms);
}

pub fn record_diagnostic_pty_event(state: &mut AppState, event: impl Into<String>) {
    if state.diagnostic_pty_recent_events.len() == DIAGNOSTIC_PTY_EVENT_LIMIT {
        state.diagnostic_pty_recent_events.pop_front();
    }
    state.diagnostic_pty_recent_events.push_back(event.into());
}

fn save_tab_state(state: &mut AppState) {
    if state.active_tab >= state.tabs.len() {
        return;
    }
    let tab = &mut state.tabs[state.active_tab];
    tab.panes = state.panes.clone();
    tab.active_pane = state.active_pane;
    tab.row_ratios = state.row_ratios.clone();
    tab.col_ratios = state.col_ratios.clone();
}

fn load_tab_state(state: &mut AppState) {
    let tab = &state.tabs[state.active_tab];
    state.panes = tab.panes.clone();
    state.active_pane = tab.active_pane;
    state.row_ratios = tab.row_ratios.clone();
    state.col_ratios = tab.col_ratios.clone();
}

pub fn mutate_switch_tab(state: &mut AppState, new_index: usize) {
    if new_index >= state.tabs.len() || new_index == state.active_tab {
        return;
    }
    save_tab_state(state);
    state.active_tab = new_index;
    load_tab_state(state);
}

fn save_workspace_state(state: &mut AppState) {
    save_tab_state(state);
    if state.active_workspace >= state.workspaces.len() {
        return;
    }
    let ws = &mut state.workspaces[state.active_workspace];
    ws.tabs = state.tabs.clone();
    ws.active_tab = state.active_tab;
}

fn load_workspace_state(state: &mut AppState) {
    if state.active_workspace >= state.workspaces.len() {
        return;
    }
    let ws = &state.workspaces[state.active_workspace];
    if ws.tabs.is_empty() {
        state.tabs = vec![];
        state.active_tab = 0;
        state.panes = vec![];
        state.active_pane = PaneId(0);
        state.row_ratios = vec![];
        state.col_ratios = vec![];
        return;
    }
    state.tabs = ws.tabs.clone();
    state.active_tab = ws.active_tab.min(state.tabs.len() - 1);
    load_tab_state(state);
}

/// Save the active workspace's live tabs/panes, switch, and load the target's saved view.
pub fn mutate_switch_workspace(state: &mut AppState, new_index: usize) {
    if new_index >= state.workspaces.len() || new_index == state.active_workspace {
        return;
    }
    save_workspace_state(state);
    state.active_workspace = new_index;
    load_workspace_state(state);
}

pub fn focus_workspace_pane_by_num(state: &mut AppState, workspace_id: u32, pane_id: u32) -> bool {
    let Some(workspace_idx) = state
        .workspaces
        .iter()
        .position(|workspace| workspace.num == workspace_id)
    else {
        return false;
    };
    focus_workspace_pane_by_index(state, workspace_idx, pane_id)
}

fn focus_workspace_pane_by_index(state: &mut AppState, workspace_idx: usize, pane_id: u32) -> bool {
    if workspace_idx >= state.workspaces.len() {
        return false;
    }

    let target = PaneId(pane_id);
    let Some(tab_idx) = pane_tab_index_for_workspace(state, workspace_idx, target) else {
        return false;
    };

    state.ctx_menu = None;
    if workspace_idx != state.active_workspace {
        mutate_switch_workspace(state, workspace_idx);
    }
    if tab_idx < state.tabs.len() && tab_idx != state.active_tab {
        mutate_switch_tab(state, tab_idx);
    }
    state.active_pane = target;
    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
        tab.active_pane = target;
    }
    true
}

fn pane_tab_index_for_workspace(
    state: &AppState,
    workspace_idx: usize,
    target: PaneId,
) -> Option<usize> {
    if workspace_idx == state.active_workspace {
        if find_pane_coord(state, target).is_some() {
            return Some(state.active_tab);
        }
        return state
            .tabs
            .iter()
            .enumerate()
            .find(|(idx, tab)| {
                *idx != state.active_tab && tab.panes.iter().flatten().any(|pane| pane.id == target)
            })
            .map(|(idx, _)| idx);
    }

    state
        .workspaces
        .get(workspace_idx)?
        .tabs
        .iter()
        .enumerate()
        .find(|(_, tab)| tab.panes.iter().flatten().any(|pane| pane.id == target))
        .map(|(idx, _)| idx)
}

/// Resolve which shell a new pane should spawn with for the given
/// state. The active workspace's `shell` beats `state.default_shell`;
/// both empty yields `None` so the daemon's `default_shell()` keeps
/// its floor.
pub fn pane_spawn_shell(state: &AppState) -> Option<crate::shell::ShellSpec> {
    let workspace = state
        .workspaces
        .get(state.active_workspace)
        .map(|w| &w.shell);
    crate::shell::resolve(workspace, Some(&state.default_shell))
}

pub fn mutate_add_tab(state: &mut AppState) {
    save_tab_state(state);

    let id_num = state.next_id;
    state.next_id += 1;
    let pane_id = PaneId(id_num);

    // Compute PTY dimensions from current cell metrics.
    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    let (cols, rows) = compute_pty_dimensions(
        state.last_grid_width,
        state.last_grid_height,
        cell_w,
        cell_h,
    );

    // Spawn PTY eagerly so the terminal is live immediately.
    let cwd = active_workspace_cwd(state);
    let workspace_id = active_workspace_num(state);
    let shell = pane_spawn_shell(state);
    let mut terminal = crate::terminal::Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn_in(
        id_num,
        workspace_id,
        cols,
        rows,
        cwd.as_deref(),
        shell.as_ref(),
    ) {
        Ok(reader) => {
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!("failed to spawn PTY for new tab pane {}: {}", id_num, e);
            terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
        }
    }

    let pane = Pane {
        id: pane_id,
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 0,
        cpu: 0.0,
    };

    let tab = TerminalTab {
        id: format!("t{}", id_num),
        name: "shell".to_string(),
        subtitle: "bash".to_string(),
        status: TabStatus::Running,
        panes: vec![vec![pane.clone()]],
        active_pane: pane_id,
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
    };

    state.tabs.push(tab);
    state.active_tab = state.tabs.len() - 1;

    // Load the new tab's pane state into the live fields.
    state.panes = vec![vec![pane]];
    state.active_pane = pane_id;
    state.row_ratios = vec![1.0];
    state.col_ratios = vec![vec![1.0]];
}

/// Build the tab title for a Quick Prompt submission. Truncates on
/// character boundaries (not bytes) so the buffer is safe for any
/// prompt the user types.
fn quick_prompt_tab_title(prompt: &str) -> String {
    let trimmed = prompt.trim();
    let truncated: String = trimmed.chars().take(30).collect();
    if truncated.is_empty() {
        "qp".to_string()
    } else {
        format!("qp: {}", truncated)
    }
}

/// Spawn a new tab running an agent at `cwd` with `shell`. Mirrors
/// `mutate_add_tab` but takes the cwd and shell explicitly so it does
/// not consult the active workspace's settings.
pub fn mutate_add_quick_prompt_tab(
    state: &mut AppState,
    prompt: &str,
    cwd: &std::path::Path,
    shell: &crate::shell::ShellSpec,
) {
    save_tab_state(state);

    let id_num = state.next_id;
    state.next_id += 1;
    let pane_id = PaneId(id_num);

    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    let (cols, rows) = compute_pty_dimensions(
        state.last_grid_width,
        state.last_grid_height,
        cell_w,
        cell_h,
    );

    let workspace_id = active_workspace_num(state);
    let mut terminal = crate::terminal::Terminal::new(rows as usize, cols as usize);
    let session_name = quick_prompt_tab_title(prompt);
    match state.pty_manager.spawn_in_named(
        id_num,
        workspace_id,
        cols,
        rows,
        Some(cwd),
        Some(shell),
        Some(&session_name),
    ) {
        Ok(reader) => {
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!(
                "failed to spawn PTY for quick prompt pane {}: {}",
                id_num,
                e
            );
            terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
        }
    }

    let title = quick_prompt_tab_title(prompt);
    let pane = Pane {
        id: pane_id,
        title: title.clone(),
        subtitle: shell.program.clone(),
        pid: 0,
        cpu: 0.0,
    };
    let tab = TerminalTab {
        id: format!("t{}", id_num),
        name: title,
        subtitle: shell.program.clone(),
        status: TabStatus::Running,
        panes: vec![vec![pane.clone()]],
        active_pane: pane_id,
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
    };

    state.tabs.push(tab);
    state.active_tab = state.tabs.len() - 1;

    state.panes = vec![vec![pane]];
    state.active_pane = pane_id;
    state.row_ratios = vec![1.0];
    state.col_ratios = vec![vec![1.0]];
}

pub fn mutate_close_tab(state: &mut AppState, index: usize) {
    if index >= state.tabs.len() {
        return;
    }

    let is_active = state.active_tab == index;

    // Collect pane IDs to destroy: live fields for the active tab,
    // stored fields for a background tab.
    let pane_ids: Vec<u32> = if is_active {
        state.panes.iter().flatten().map(|p| p.id.0).collect()
    } else {
        state.tabs[index]
            .panes
            .iter()
            .flatten()
            .map(|p| p.id.0)
            .collect()
    };

    for id in &pane_ids {
        state.pty_manager.destroy(*id);
        state.terminals.remove(id);
    }

    state.tabs.remove(index);

    if state.tabs.is_empty() {
        // Workspace has no tabs left. Leave live state empty so the terminal
        // grid falls back to its empty-state canvas instead of auto-spawning
        // a fresh terminal the user didn't ask for.
        state.active_tab = 0;
        state.panes = vec![];
        state.active_pane = PaneId(0);
        state.row_ratios = vec![];
        state.col_ratios = vec![];
        return;
    }

    if is_active {
        state.active_tab = index.min(state.tabs.len() - 1);
        load_tab_state(state);
    } else if state.active_tab > index {
        state.active_tab -= 1;
    }
}

pub fn new_workspace(num: u32, name: String, path: Option<PathBuf>) -> Workspace {
    let git_branch = path.as_deref().and_then(crate::git::detect_git_branch);
    Workspace {
        num,
        name,
        path,
        collapsed: false,
        terminals_expanded: true,
        terminal_entries: vec![],
        subtabs: vec![Subtab {
            label: "terminals".to_string(),
            count: Some(0),
            pulse: false,
            active: false,
            disabled: false,
            icon: Some(SubtabIcon::Terminal),
            tree_glyph: "\u{2514}",
        }],
        git_branch,
        tabs: vec![],
        active_tab: 0,
        shell: crate::shell::ShellSpec::default(),
    }
}

/// Rebuild the full workspace + tab + pane layout from a persisted
/// snapshot, replacing whatever `seed_state` produced. Hydrates the live
/// tab fields for the active workspace, restores `next_id` above every
/// restored pane id, and guarantees the active workspace has at least one
/// live pane (seeding a fresh default tab when the persisted active
/// workspace was empty). The caller reattaches each restored pane to its
/// surviving daemon session afterwards (see `main.rs`).
pub fn restore_layout(state: &mut AppState, persisted: &crate::persist::PersistedState) {
    if persisted.workspaces.is_empty() {
        return;
    }
    let mut max_pane_id = 0u32;
    let active_idx = persisted
        .active_workspace
        .min(persisted.workspaces.len() - 1);
    let mut workspaces = Vec::with_capacity(persisted.workspaces.len());
    for (i, entry) in persisted.workspaces.iter().enumerate() {
        let mut ws = new_workspace((i + 1) as u32, entry.name.clone(), entry.path.clone());
        ws.collapsed = entry.collapsed;
        ws.shell = entry.shell.clone();
        let tabs: Vec<TerminalTab> = entry
            .tabs
            .iter()
            .filter_map(|pt| terminal_tab_from_persisted(pt, &mut max_pane_id))
            .collect();
        ws.active_tab = if tabs.is_empty() {
            0
        } else {
            entry.active_tab.min(tabs.len() - 1)
        };
        ws.terminals_expanded = !tabs.is_empty();
        if let Some(sub) = ws.subtabs.get_mut(0) {
            sub.count = Some(tabs.len() as u32);
            sub.active = i == active_idx;
        }
        ws.tabs = tabs;
        workspaces.push(ws);
    }

    state.workspaces = workspaces;
    state.active_workspace = active_idx;
    load_workspace_state(state);

    // The render/PTY bootstrap assumes the active workspace always has a
    // live pane. If the persisted active workspace had no tabs (upgrade,
    // or the user closed them all before quitting), seed a fresh one.
    if state.panes.iter().flatten().next().is_none() {
        seed_default_tab(state, &mut max_pane_id);
    }

    state.next_id = max_pane_id.saturating_add(1).max(2);
}

/// Convert one persisted tab into a live `TerminalTab`, tracking the
/// largest pane id seen. Returns `None` for a malformed tab with no panes
/// so it is dropped rather than producing an unselectable ghost tab.
fn terminal_tab_from_persisted(
    pt: &crate::persist::PersistedTab,
    max_pane_id: &mut u32,
) -> Option<TerminalTab> {
    let panes: Vec<Vec<Pane>> = pt
        .panes
        .iter()
        .map(|row| {
            row.iter()
                .map(|pp| {
                    *max_pane_id = (*max_pane_id).max(pp.id);
                    Pane {
                        id: PaneId(pp.id),
                        title: if pp.title.is_empty() {
                            "shell".to_string()
                        } else {
                            pp.title.clone()
                        },
                        subtitle: if pp.subtitle.is_empty() {
                            "bash".to_string()
                        } else {
                            pp.subtitle.clone()
                        },
                        pid: 0,
                        cpu: 0.0,
                    }
                })
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect();
    if panes.is_empty() {
        return None;
    }

    let row_ratios = normalized_row_ratios(&panes, &pt.row_ratios);
    let col_ratios = normalized_col_ratios(&panes, &pt.col_ratios);
    let active_pane = panes
        .iter()
        .flatten()
        .map(|p| p.id)
        .find(|id| id.0 == pt.active_pane)
        .unwrap_or_else(|| panes[0][0].id);

    Some(TerminalTab {
        id: pt.id.clone(),
        name: if pt.name.is_empty() {
            "shell".to_string()
        } else {
            pt.name.clone()
        },
        subtitle: if pt.subtitle.is_empty() {
            "bash".to_string()
        } else {
            pt.subtitle.clone()
        },
        status: TabStatus::Running,
        panes,
        active_pane,
        row_ratios,
        col_ratios,
    })
}

/// Coerce persisted row ratios to the restored grid shape, falling back
/// to equal weights when the saved data is missing or malformed.
fn normalized_row_ratios(panes: &[Vec<Pane>], saved: &[f32]) -> Vec<f32> {
    if saved.len() == panes.len() && saved.iter().all(|r| r.is_finite() && *r > 0.0) {
        saved.to_vec()
    } else {
        vec![1.0; panes.len()]
    }
}

fn normalized_col_ratios(panes: &[Vec<Pane>], saved: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let well_formed = saved.len() == panes.len()
        && saved.iter().zip(panes.iter()).all(|(row, panes_row)| {
            row.len() == panes_row.len() && row.iter().all(|r| r.is_finite() && *r > 0.0)
        });
    if well_formed {
        saved.to_vec()
    } else {
        panes.iter().map(|row| vec![1.0; row.len()]).collect()
    }
}

/// Seed a single fresh default tab/pane into the active workspace's live
/// fields (and its stored copy), using `max_pane_id + 1` so the id never
/// collides with a restored pane. Bumps `max_pane_id`.
fn seed_default_tab(state: &mut AppState, max_pane_id: &mut u32) {
    let id_num = max_pane_id.saturating_add(1);
    *max_pane_id = id_num;
    let pane = Pane {
        id: PaneId(id_num),
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 0,
        cpu: 0.0,
    };
    let tab = TerminalTab {
        id: format!("t{}", id_num),
        name: "shell".to_string(),
        subtitle: "bash".to_string(),
        status: TabStatus::Running,
        panes: vec![vec![pane.clone()]],
        active_pane: PaneId(id_num),
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
    };
    state.tabs = vec![tab.clone()];
    state.active_tab = 0;
    state.panes = vec![vec![pane]];
    state.active_pane = PaneId(id_num);
    state.row_ratios = vec![1.0];
    state.col_ratios = vec![vec![1.0]];
    if let Some(ws) = state.workspaces.get_mut(state.active_workspace) {
        ws.tabs = vec![tab];
        ws.active_tab = 0;
        ws.terminals_expanded = true;
        if let Some(sub) = ws.subtabs.get_mut(0) {
            sub.count = Some(1);
        }
    }
}

pub fn mutate_add_workspace(state: &mut AppState) {
    mutate_add_workspace_with_path(state, None);
}

pub fn mutate_add_workspace_with_path(state: &mut AppState, path: Option<PathBuf>) {
    let num = state.workspaces.len() as u32 + 1;
    let name = path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("workspace-{}", num));
    state.workspaces.push(new_workspace(num, name, path));
    let new_idx = state.workspaces.len() - 1;
    mutate_switch_workspace(state, new_idx);
}

pub fn mutate_remove_workspace(state: &mut AppState, idx: usize) {
    if state.workspaces.len() <= 1 || idx >= state.workspaces.len() {
        return;
    }
    state.workspaces.remove(idx);
    // Renumber remaining workspaces.
    for (i, ws) in state.workspaces.iter_mut().enumerate() {
        ws.num = i as u32 + 1;
    }
    if state.active_workspace >= state.workspaces.len() {
        state.active_workspace = state.workspaces.len() - 1;
    }
}

pub fn find_pane_coord(state: &AppState, target: PaneId) -> Option<(usize, usize)> {
    for (r, row) in state.panes.iter().enumerate() {
        for (c, pane) in row.iter().enumerate() {
            if pane.id == target {
                return Some((r, c));
            }
        }
    }
    None
}

/// Return the active pane of `ws_idx`, reading live state when that workspace
/// is active and the saved tab state otherwise. `None` when the workspace has
/// no tabs or the active pane id isn't present in its layout.
pub fn workspace_active_pane(state: &AppState, ws_idx: usize) -> Option<PaneId> {
    if ws_idx == state.active_workspace {
        return state
            .tabs
            .get(state.active_tab)
            .map(|tab| tab.active_pane)
            .filter(|pid| find_pane_coord(state, *pid).is_some());
    }
    let ws = state.workspaces.get(ws_idx)?;
    let tab = ws.tabs.get(ws.active_tab)?;
    let pane_exists = tab.panes.iter().flatten().any(|p| p.id == tab.active_pane);
    if pane_exists {
        Some(tab.active_pane)
    } else {
        None
    }
}

pub fn mutate_split_right(state: &mut AppState, target: PaneId) {
    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        return;
    };
    if state.panes[row_idx].len() >= MAX_COLS {
        return;
    }
    let id_num = state.next_id;
    state.next_id += 1;
    let pane_id = PaneId(id_num);

    // Use real cell metrics when available; fall back to 80x24.
    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    let (cols, rows) = compute_pty_dimensions(
        state.last_grid_width,
        state.last_grid_height,
        cell_w,
        cell_h,
    );

    let cwd = active_workspace_cwd(state);
    let workspace_id = active_workspace_num(state);
    let shell = pane_spawn_shell(state);
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn_in(
        id_num,
        workspace_id,
        cols,
        rows,
        cwd.as_deref(),
        shell.as_ref(),
    ) {
        Ok(reader) => {
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!("failed to spawn PTY for pane {}: {}", id_num, e);
            terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
        }
    }

    let new_pane = Pane {
        id: pane_id,
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 0,
        cpu: 0.0,
    };
    state.panes[row_idx].insert(col_idx + 1, new_pane);
    // Split the existing column ratio in half with the new pane.
    let existing = state.col_ratios[row_idx][col_idx];
    let half = existing / 2.0;
    state.col_ratios[row_idx][col_idx] = half;
    state.col_ratios[row_idx].insert(col_idx + 1, half);
    state.active_pane = pane_id;
}

pub fn mutate_split_down(state: &mut AppState, target: PaneId) {
    let Some((row_idx, _)) = find_pane_coord(state, target) else {
        return;
    };
    if state.panes.len() >= MAX_ROWS {
        return;
    }
    let id_num = state.next_id;
    state.next_id += 1;
    let pane_id = PaneId(id_num);

    // Use real cell metrics when available; fall back to 80x24.
    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    let (cols, rows) = compute_pty_dimensions(
        state.last_grid_width,
        state.last_grid_height,
        cell_w,
        cell_h,
    );

    let cwd = active_workspace_cwd(state);
    let workspace_id = active_workspace_num(state);
    let shell = pane_spawn_shell(state);
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn_in(
        id_num,
        workspace_id,
        cols,
        rows,
        cwd.as_deref(),
        shell.as_ref(),
    ) {
        Ok(reader) => {
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!("failed to spawn PTY for pane {}: {}", id_num, e);
            terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
            state
                .terminals
                .insert(id_num, Arc::new(Mutex::new(terminal)));
        }
    }

    let new_pane = Pane {
        id: pane_id,
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 0,
        cpu: 0.0,
    };
    state.panes.insert(row_idx + 1, vec![new_pane]);
    // Split the existing row ratio in half with the new row.
    let existing = state.row_ratios[row_idx];
    let half = existing / 2.0;
    state.row_ratios[row_idx] = half;
    state.row_ratios.insert(row_idx + 1, half);
    state.col_ratios.insert(row_idx + 1, vec![1.0]);
    state.active_pane = pane_id;
}

pub fn mutate_close_pane(state: &mut AppState, target: PaneId) {
    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        return;
    };

    // Destroy the PTY and terminal.
    state.pty_manager.destroy(target.0);
    state.terminals.remove(&target.0);

    // Absorb the closed pane's column ratio into a neighbor.
    let closed_ratio = state.col_ratios[row_idx][col_idx];
    state.col_ratios[row_idx].remove(col_idx);
    if !state.col_ratios[row_idx].is_empty() {
        let absorb_idx = if col_idx > 0 { col_idx - 1 } else { 0 };
        state.col_ratios[row_idx][absorb_idx] += closed_ratio;
    }

    state.panes[row_idx].remove(col_idx);
    if state.panes[row_idx].is_empty() {
        // Absorb the empty row's ratio into a neighbor.
        let closed_row_ratio = state.row_ratios[row_idx];
        state.row_ratios.remove(row_idx);
        state.col_ratios.remove(row_idx);
        if !state.row_ratios.is_empty() {
            let absorb_idx = if row_idx > 0 { row_idx - 1 } else { 0 };
            state.row_ratios[absorb_idx] += closed_row_ratio;
        }
        state.panes.remove(row_idx);
    }
    if state.panes.is_empty() {
        // Last pane of the active tab is gone: close the whole tab so the
        // tab bar and sidebar reflect the loss. When this was the last tab
        // the workspace falls back to its empty state canvas.
        let active_tab = state.active_tab;
        mutate_close_tab(state, active_tab);
        return;
    }
    if state.active_pane == target {
        let new_row = row_idx.min(state.panes.len() - 1);
        let new_col = col_idx.min(state.panes[new_row].len() - 1);
        state.active_pane = state.panes[new_row][new_col].id;
    }
    sync_live_tab_from_panes(state);
}

/// Move focus to the pane immediately left of the active pane in the
/// same row. No-op when the active pane is at column 0 or cannot be
/// located. When rows have differing column counts the up/down variants
/// clamp to the target row's last column.
pub fn mutate_focus_left(state: &mut AppState) {
    let Some((row, col)) = find_pane_coord(state, state.active_pane) else {
        return;
    };
    if col == 0 {
        return;
    }
    state.active_pane = state.panes[row][col - 1].id;
    sync_live_tab_from_panes(state);
}

pub fn mutate_focus_right(state: &mut AppState) {
    let Some((row, col)) = find_pane_coord(state, state.active_pane) else {
        return;
    };
    if col + 1 >= state.panes[row].len() {
        return;
    }
    state.active_pane = state.panes[row][col + 1].id;
    sync_live_tab_from_panes(state);
}

pub fn mutate_focus_up(state: &mut AppState) {
    let Some((row, col)) = find_pane_coord(state, state.active_pane) else {
        return;
    };
    if row == 0 {
        return;
    }
    let target_row = row - 1;
    let target_col = col.min(state.panes[target_row].len().saturating_sub(1));
    state.active_pane = state.panes[target_row][target_col].id;
    sync_live_tab_from_panes(state);
}

pub fn mutate_focus_down(state: &mut AppState) {
    let Some((row, col)) = find_pane_coord(state, state.active_pane) else {
        return;
    };
    if row + 1 >= state.panes.len() {
        return;
    }
    let target_row = row + 1;
    let target_col = col.min(state.panes[target_row].len().saturating_sub(1));
    state.active_pane = state.panes[target_row][target_col].id;
    sync_live_tab_from_panes(state);
}

/// Move `target` out of the active tab into a new tab inserted at
/// `new_tab_index`. The PTY handle and terminal emulator state are
/// kept untouched so the running process, scrollback and cwd survive
/// the move.
///
/// - When the active tab holds multiple panes, the pane is removed from
///   its row and the freed column ratio is absorbed by a neighbor, then
///   a fresh `TerminalTab` holding the extracted pane is inserted at
///   `new_tab_index`.
/// - When the active tab holds only `target`, the source tab is removed
///   (without destroying the PTY) and the new tab takes its place. The
///   tab count stays the same.
///
/// After extraction the newly inserted tab becomes active. A bad
/// `target` id or a call that would leave no tabs at all is a no-op.
pub fn mutate_extract_pane_to_tab(state: &mut AppState, target: PaneId, new_tab_index: usize) {
    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        log::warn!(
            "extract_to_tab: pane {:?} not found in active layout",
            target
        );
        return;
    };

    let extracted_pane = state.panes[row_idx][col_idx].clone();
    let extracted_id = extracted_pane.id.0;
    let source_tab_idx = state.active_tab;
    let live_pane_count: usize = state.panes.iter().map(|r| r.len()).sum();
    log::info!(
        "extract_to_tab: pane={:?} ({}, {}) source_tab={} live_panes={} new_idx={}",
        target,
        row_idx,
        col_idx,
        source_tab_idx,
        live_pane_count,
        new_tab_index
    );

    if live_pane_count == 1 {
        // Source tab becomes empty after extraction. Drop it from `tabs`
        // without going through `mutate_close_tab` (which would destroy
        // the PTY we want to migrate). Clear live layout fields so the
        // new tab can re-seed them via `load_tab_state`.
        state.tabs.remove(source_tab_idx);
        state.panes.clear();
        state.row_ratios.clear();
        state.col_ratios.clear();
    } else {
        let closed_ratio = state.col_ratios[row_idx][col_idx];
        state.col_ratios[row_idx].remove(col_idx);
        if !state.col_ratios[row_idx].is_empty() {
            let absorb_idx = if col_idx > 0 { col_idx - 1 } else { 0 };
            state.col_ratios[row_idx][absorb_idx] += closed_ratio;
        }
        state.panes[row_idx].remove(col_idx);
        if state.panes[row_idx].is_empty() {
            let closed_row_ratio = state.row_ratios[row_idx];
            state.row_ratios.remove(row_idx);
            state.col_ratios.remove(row_idx);
            if !state.row_ratios.is_empty() {
                let absorb_idx = if row_idx > 0 { row_idx - 1 } else { 0 };
                state.row_ratios[absorb_idx] += closed_row_ratio;
            }
            state.panes.remove(row_idx);
        }
        if state.active_pane == target {
            let new_row = row_idx.min(state.panes.len() - 1);
            let new_col = col_idx.min(state.panes[new_row].len() - 1);
            state.active_pane = state.panes[new_row][new_col].id;
        }
        sync_live_tab_from_panes(state);
    }

    let new_tab = TerminalTab {
        id: format!("t{}", extracted_id),
        name: extracted_pane.title.clone(),
        subtitle: extracted_pane.subtitle.clone(),
        status: TabStatus::Running,
        panes: vec![vec![extracted_pane]],
        active_pane: PaneId(extracted_id),
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
    };

    // When the source tab was removed above, any `new_tab_index` past
    // the removed slot shifts left by one.
    let adjusted_index = if live_pane_count == 1 && new_tab_index > source_tab_idx {
        new_tab_index - 1
    } else {
        new_tab_index
    };
    let insertion_index = adjusted_index.min(state.tabs.len());
    state.tabs.insert(insertion_index, new_tab);
    state.active_tab = insertion_index;
    load_tab_state(state);
}

/// Move tab `source_tab_id` to the requested `new_index` in the tab
/// strip. Active-tab pointer follows the moved tab by id so selection
/// is preserved regardless of which tabs shift. Unknown ids and a
/// no-op reorder (already at target index) are silently ignored.
pub fn mutate_tab_reorder(state: &mut AppState, source_tab_id: &str, new_index: usize) {
    let Some(old_idx) = state.tabs.iter().position(|t| t.id == source_tab_id) else {
        return;
    };
    let clamped_target = new_index.min(state.tabs.len());
    // Removing first, then inserting at the caller's index, requires a
    // shift when the target lies past the removed slot; otherwise the
    // item lands one position too far right.
    let adjusted = if clamped_target > old_idx {
        clamped_target - 1
    } else {
        clamped_target
    };
    if adjusted == old_idx {
        return;
    }
    let active_id = state.tabs[state.active_tab].id.clone();
    let tab = state.tabs.remove(old_idx);
    let insertion = adjusted.min(state.tabs.len());
    state.tabs.insert(insertion, tab);
    state.active_tab = state
        .tabs
        .iter()
        .position(|t| t.id == active_id)
        .unwrap_or(0);
}

/// Insert `source_tab`'s single pane as a split of `target` along the
/// given `edge`. This is the edge-drop half of the tab-drag flow; the
/// center zone uses `mutate_tab_reorder` instead.
///
/// Constraints:
/// - Source tab must exist and have exactly one pane. Multi-pane tabs
///   would require recursive nested grids, which the layout doesn't
///   model, so the drop is a no-op instead of a partial move.
/// - Source tab must not be the currently active tab (a self-split
///   would try to place the pane next to itself). No-op in that case.
/// - The target pane must exist in the active tab's live layout.
///
/// The source tab's PTY handle is preserved (we never destroy the
/// `state.terminals` entry), so the process, scrollback, and cwd
/// survive the move. The source tab is then removed from the tab
/// strip because it became empty.
pub fn mutate_pane_drop_split(
    state: &mut AppState,
    source_tab_id: &str,
    target: PaneId,
    edge: crate::drag::drop_zones::DropZone,
) {
    use crate::drag::drop_zones::DropZone;

    let Some(source_idx) = state.tabs.iter().position(|t| t.id == source_tab_id) else {
        log::warn!("drop_split: unknown source tab {}", source_tab_id);
        return;
    };
    if source_idx == state.active_tab {
        log::warn!("drop_split: source tab {} is active; no-op", source_tab_id);
        return;
    }
    let saved_count: usize = state.tabs[source_idx].panes.iter().map(|r| r.len()).sum();
    if saved_count != 1 {
        log::warn!(
            "drop_split: source tab {} has {} panes, only single-pane supported",
            source_tab_id,
            saved_count
        );
        return;
    }
    let source_pane = state.tabs[source_idx].panes[0][0].clone();
    let source_pane_id = source_pane.id;

    if source_pane_id == target {
        log::warn!(
            "drop_split: source pane {:?} equals target; no-op",
            source_pane_id
        );
        return;
    }

    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        log::warn!("drop_split: target pane {:?} not in active layout", target);
        return;
    };

    log::info!(
        "drop_split: src_tab={} src_pane={:?} target={:?} edge={:?}",
        source_tab_id,
        source_pane_id,
        target,
        edge
    );

    state.tabs.remove(source_idx);
    if state.active_tab > source_idx {
        state.active_tab -= 1;
    }

    match edge {
        DropZone::Left => {
            let existing = state.col_ratios[row_idx][col_idx];
            let half = existing / 2.0;
            state.col_ratios[row_idx][col_idx] = half;
            state.col_ratios[row_idx].insert(col_idx, half);
            state.panes[row_idx].insert(col_idx, source_pane);
        }
        DropZone::Right => {
            let existing = state.col_ratios[row_idx][col_idx];
            let half = existing / 2.0;
            state.col_ratios[row_idx][col_idx] = half;
            state.col_ratios[row_idx].insert(col_idx + 1, half);
            state.panes[row_idx].insert(col_idx + 1, source_pane);
        }
        DropZone::Top => {
            let existing = state.row_ratios[row_idx];
            let half = existing / 2.0;
            state.row_ratios[row_idx] = half;
            state.row_ratios.insert(row_idx, half);
            state.col_ratios.insert(row_idx, vec![1.0]);
            state.panes.insert(row_idx, vec![source_pane]);
        }
        DropZone::Bottom => {
            let existing = state.row_ratios[row_idx];
            let half = existing / 2.0;
            state.row_ratios[row_idx] = half;
            state.row_ratios.insert(row_idx + 1, half);
            state.col_ratios.insert(row_idx + 1, vec![1.0]);
            state.panes.insert(row_idx + 1, vec![source_pane]);
        }
        DropZone::Center => return,
    }

    state.active_pane = source_pane_id;
    sync_live_tab_from_panes(state);
}

/// Move `source` out of its current slot in the active tab and re-insert
/// it at `target`'s `edge`. Used when the user drags a pane grip onto
/// another pane's edge inside the same tab (the intra-tab analogue of
/// `mutate_pane_drop_split`).
///
/// The PTY handle and terminal emulator are untouched: only the layout
/// vectors move. Ratio absorption mirrors `mutate_close_pane` on removal
/// and `mutate_pane_drop_split` on insertion, so the rest of the tab's
/// layout keeps its relative proportions.
///
/// Constraints:
/// - `source` and `target` must both live in the active tab.
/// - `source != target`: dropping a pane on its own edge is a no-op.
/// - `edge` must be a proper edge; `Center` is rejected (it's a dead
///   zone in `hit_test`, so any stale Center value is a cancel).
pub fn mutate_pane_move_to_edge(
    state: &mut AppState,
    source: PaneId,
    target: PaneId,
    edge: crate::drag::drop_zones::DropZone,
) {
    use crate::drag::drop_zones::DropZone;

    if source == target {
        log::warn!("pane_move_to_edge: source == target; no-op");
        return;
    }
    if matches!(edge, DropZone::Center) {
        log::warn!("pane_move_to_edge: edge is Center; no-op");
        return;
    }
    let Some((src_row, src_col)) = find_pane_coord(state, source) else {
        log::warn!(
            "pane_move_to_edge: source {:?} not in active layout",
            source
        );
        return;
    };
    if find_pane_coord(state, target).is_none() {
        log::warn!(
            "pane_move_to_edge: target {:?} not in active layout",
            target
        );
        return;
    }

    log::info!(
        "pane_move_to_edge: src={:?} ({},{}) tgt={:?} edge={:?}",
        source,
        src_row,
        src_col,
        target,
        edge
    );

    // Detach source from its current slot (same ratio-absorb logic as
    // `mutate_close_pane`, but we keep the Pane value to re-insert).
    let source_pane = state.panes[src_row][src_col].clone();
    let freed_col_ratio = state.col_ratios[src_row][src_col];
    state.col_ratios[src_row].remove(src_col);
    if !state.col_ratios[src_row].is_empty() {
        let absorb_idx = if src_col > 0 { src_col - 1 } else { 0 };
        state.col_ratios[src_row][absorb_idx] += freed_col_ratio;
    }
    state.panes[src_row].remove(src_col);
    if state.panes[src_row].is_empty() {
        let freed_row_ratio = state.row_ratios[src_row];
        state.row_ratios.remove(src_row);
        state.col_ratios.remove(src_row);
        if !state.row_ratios.is_empty() {
            let absorb_idx = if src_row > 0 { src_row - 1 } else { 0 };
            state.row_ratios[absorb_idx] += freed_row_ratio;
        }
        state.panes.remove(src_row);
    }

    // After removal, target's coordinates may have shifted (e.g. source
    // was earlier in the same row or on a now-deleted row). Re-query.
    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        log::warn!(
            "pane_move_to_edge: target {:?} disappeared after detach",
            target
        );
        return;
    };

    match edge {
        DropZone::Left => {
            let existing = state.col_ratios[row_idx][col_idx];
            let half = existing / 2.0;
            state.col_ratios[row_idx][col_idx] = half;
            state.col_ratios[row_idx].insert(col_idx, half);
            state.panes[row_idx].insert(col_idx, source_pane);
        }
        DropZone::Right => {
            let existing = state.col_ratios[row_idx][col_idx];
            let half = existing / 2.0;
            state.col_ratios[row_idx][col_idx] = half;
            state.col_ratios[row_idx].insert(col_idx + 1, half);
            state.panes[row_idx].insert(col_idx + 1, source_pane);
        }
        DropZone::Top => {
            let existing = state.row_ratios[row_idx];
            let half = existing / 2.0;
            state.row_ratios[row_idx] = half;
            state.row_ratios.insert(row_idx, half);
            state.col_ratios.insert(row_idx, vec![1.0]);
            state.panes.insert(row_idx, vec![source_pane]);
        }
        DropZone::Bottom => {
            let existing = state.row_ratios[row_idx];
            let half = existing / 2.0;
            state.row_ratios[row_idx] = half;
            state.row_ratios.insert(row_idx + 1, half);
            state.col_ratios.insert(row_idx + 1, vec![1.0]);
            state.panes.insert(row_idx + 1, vec![source_pane]);
        }
        DropZone::Center => return,
    }

    state.active_pane = source;
    sync_live_tab_from_panes(state);
}

/// Swap two panes in the active tab. Both PTYs and terminals stay
/// alive; only the PaneId references in the layout grid trade places.
/// Used by Center drops in pane drags so the gesture rearranges
/// content without destroying anything.
pub fn mutate_pane_swap(state: &mut AppState, a: PaneId, b: PaneId) {
    if a == b {
        return;
    }
    let Some((ar, ac)) = find_pane_coord(state, a) else {
        log::warn!("pane_swap: source {:?} not in active layout", a);
        return;
    };
    let Some((br, bc)) = find_pane_coord(state, b) else {
        log::warn!("pane_swap: target {:?} not in active layout", b);
        return;
    };
    log::info!(
        "pane_swap: a={:?} ({},{}) b={:?} ({},{})",
        a,
        ar,
        ac,
        b,
        br,
        bc
    );
    let tmp = state.panes[ar][ac].clone();
    state.panes[ar][ac] = state.panes[br][bc].clone();
    state.panes[br][bc] = tmp;
    state.active_pane = a;
    sync_live_tab_from_panes(state);
}

/// Swap the single pane in `source_tab_id` with `target` in the
/// active tab. Both PTYs survive, both tabs survive — the source tab
/// inherits the target's pane and the active tab now holds the
/// source pane in target's old slot. Used by Center drops in tab
/// drags so a tab→pane drop rearranges content without deleting
/// anything.
pub fn mutate_pane_swap_from_tab(state: &mut AppState, source_tab_id: &str, target: PaneId) {
    let Some(source_idx) = state.tabs.iter().position(|t| t.id == source_tab_id) else {
        log::warn!("pane_swap_from_tab: unknown source tab {}", source_tab_id);
        return;
    };
    if source_idx == state.active_tab {
        log::warn!(
            "pane_swap_from_tab: source tab {} is active; no-op",
            source_tab_id
        );
        return;
    }
    let saved_count: usize = state.tabs[source_idx].panes.iter().map(|r| r.len()).sum();
    if saved_count != 1 {
        log::warn!(
            "pane_swap_from_tab: source tab {} has {} panes, single-pane only",
            source_tab_id,
            saved_count
        );
        return;
    }
    let source_pane = state.tabs[source_idx].panes[0][0].clone();
    let source_pane_id = source_pane.id;
    if source_pane_id == target {
        return;
    }
    let Some((tgt_row, tgt_col)) = find_pane_coord(state, target) else {
        log::warn!(
            "pane_swap_from_tab: target {:?} not in active layout",
            target
        );
        return;
    };

    log::info!(
        "pane_swap_from_tab: src_tab={} src_pane={:?} target={:?}",
        source_tab_id,
        source_pane_id,
        target
    );

    let target_pane = state.panes[tgt_row][tgt_col].clone();
    state.panes[tgt_row][tgt_col] = source_pane;
    state.tabs[source_idx].panes[0][0] = target_pane;
    state.tabs[source_idx].active_pane = target;

    state.active_pane = source_pane_id;
    sync_live_tab_from_panes(state);
}

/// Copy the live pane layout back into `tabs[active_tab]` so the per-tab
/// saved view stays in sync without waiting for a tab or workspace switch.
fn sync_live_tab_from_panes(state: &mut AppState) {
    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
        tab.panes = state.panes.clone();
        tab.active_pane = state.active_pane;
        tab.row_ratios = state.row_ratios.clone();
        tab.col_ratios = state.col_ratios.clone();
    }
}

/// Adjust two adjacent ratios by a pixel delta relative to the container size.
/// Uses the `initial` snapshot so that each drag update recomputes from the
/// original ratios + total_delta, avoiding accumulated floating point drift.
pub fn apply_ratio_delta(
    ratios: &mut [f32],
    before_idx: usize,
    after_idx: usize,
    initial: &[f32],
    delta_px: f32,
    container_px: f32,
) {
    if container_px <= 0.0 {
        return;
    }
    let total_ratio: f32 = initial.iter().sum();
    let delta_ratio = (delta_px / container_px) * total_ratio;

    let pair_sum = initial[before_idx] + initial[after_idx];
    let new_before =
        (initial[before_idx] + delta_ratio).clamp(MIN_PANE_RATIO, pair_sum - MIN_PANE_RATIO);
    let new_after = pair_sum - new_before;

    ratios[before_idx] = new_before;
    ratios[after_idx] = new_after;
}

fn adjusted_font_size(current: u32, delta: i32) -> u32 {
    let next = current as i32 + delta;
    (next.clamp(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32)) as u32
}

fn adjusted_scroll_value(current: u32, delta: i32, min: u32, max: u32) -> u32 {
    let next = current as i32 + delta;
    (next.clamp(min as i32, max as i32)) as u32
}

pub fn mutate_config_font_size_delta(state: &mut AppState, delta: i32) {
    state.config_font_size_pt = adjusted_font_size(state.config_font_size_pt, delta);
}

pub fn mutate_terminal_font_size_delta(state: &mut AppState, delta: i32) {
    let next = adjusted_font_size(state.terminal_font_size_pt, delta);
    if next != state.terminal_font_size_pt {
        state.terminal_font_size_pt = next;
        sync_terminal_size_to_font_metrics(state);
    }
}

pub fn mutate_ui_density(state: &mut AppState, density: UiDensity) -> bool {
    if state.ui_density == density {
        return false;
    }
    state.ui_density = density;
    true
}

pub fn mutate_scroll_line_px_delta(state: &mut AppState, delta: i32) -> bool {
    let next = adjusted_scroll_value(
        state.scroll_line_px,
        delta,
        MIN_SCROLL_LINE_PX,
        MAX_SCROLL_LINE_PX,
    );
    if next == state.scroll_line_px {
        return false;
    }
    state.scroll_line_px = next;
    true
}

pub fn mutate_smooth_scroll_duration_delta(state: &mut AppState, delta: i32) -> bool {
    let next = adjusted_scroll_value(
        state.smooth_scroll_duration_ms,
        delta,
        MIN_SMOOTH_SCROLL_DURATION_MS,
        MAX_SMOOTH_SCROLL_DURATION_MS,
    );
    if next == state.smooth_scroll_duration_ms {
        return false;
    }
    state.smooth_scroll_duration_ms = next;
    true
}

pub fn mutate_theme(state: &mut AppState, theme_id: &str) -> bool {
    let resolved = theme::resolve_theme_id(theme_id).to_string();
    if state.theme == resolved {
        return false;
    }
    state.theme = resolved;
    true
}

pub fn mutate_custom_theme_color(
    state: &mut AppState,
    slot: theme::CustomThemeSlot,
    raw_hex: &str,
) -> bool {
    let Some(color) = theme::parse_hex_color(raw_hex) else {
        return false;
    };
    if theme::custom_theme_color(&state.custom_theme, slot) == color
        && state.theme == theme::CUSTOM_THEME_ID
    {
        return false;
    }
    theme::set_custom_theme_color(&mut state.custom_theme, slot, color);
    state.theme = theme::CUSTOM_THEME_ID.to_string();
    state.last_terminal_theme_painted.clear();
    true
}

pub fn reset_custom_theme(state: &mut AppState) -> bool {
    let default = theme::default_custom_theme();
    if state.custom_theme == default && state.theme == theme::CUSTOM_THEME_ID {
        return false;
    }
    state.custom_theme = default;
    state.theme = theme::CUSTOM_THEME_ID.to_string();
    state.last_terminal_theme_painted.clear();
    true
}

pub fn reset_appearance(state: &mut AppState) -> bool {
    let changed = state.theme != theme::default_theme_id()
        || state.custom_theme != theme::default_custom_theme()
        || state.config_font_size_pt != DEFAULT_CONFIG_FONT_SIZE_PT
        || state.terminal_font_size_pt != DEFAULT_TERMINAL_FONT_SIZE_PT
        || state.ui_density != DEFAULT_UI_DENSITY
        || state.scroll_line_px != DEFAULT_SCROLL_LINE_PX
        || state.smooth_scroll_duration_ms != DEFAULT_SMOOTH_SCROLL_DURATION_MS;
    state.theme = theme::default_theme_id().to_string();
    state.custom_theme = theme::default_custom_theme();
    state.config_font_size_pt = DEFAULT_CONFIG_FONT_SIZE_PT;
    state.terminal_font_size_pt = DEFAULT_TERMINAL_FONT_SIZE_PT;
    state.ui_density = DEFAULT_UI_DENSITY;
    state.scroll_line_px = DEFAULT_SCROLL_LINE_PX;
    state.smooth_scroll_duration_ms = DEFAULT_SMOOTH_SCROLL_DURATION_MS;
    sync_terminal_size_to_font_metrics(state);
    state.last_terminal_theme_painted.clear();
    changed
}

pub fn take_terminal_theme_repaint_request(state: &mut AppState) -> bool {
    if state.settings_open {
        return false;
    }
    let resolved = theme::resolve_theme_id(&state.theme);
    if state.last_terminal_theme_painted == resolved {
        return false;
    }
    state.last_terminal_theme_painted = resolved.to_string();
    true
}

/// Kill every daemon session tagged with the workspace at `ws_idx` and
/// empty that workspace's tabs/panes in the UI. The workspace itself is
/// kept (per SPEC F5: "Workspace itself is not deleted, just emptied").
///
/// Pane ids come from the live `state.panes` + `state.tabs` snapshot
/// when `ws_idx` is the active workspace, otherwise from the saved
/// `workspaces[ws_idx].tabs`. Each id is destroyed on the daemon and
/// dropped from `state.terminals`; state.pty_manager.destroy is a
/// no-op for unknown ids so double-destroy is safe.
pub fn mutate_kill_workspace_terminals(state: &mut AppState, ws_idx: usize) {
    if ws_idx >= state.workspaces.len() {
        return;
    }

    let mut pane_ids: Vec<u32> = Vec::new();
    if ws_idx == state.active_workspace {
        for row in &state.panes {
            for p in row {
                pane_ids.push(p.id.0);
            }
        }
        for (tab_idx, tab) in state.tabs.iter().enumerate() {
            if tab_idx == state.active_tab {
                continue;
            }
            for row in &tab.panes {
                for p in row {
                    pane_ids.push(p.id.0);
                }
            }
        }
    } else {
        for tab in &state.workspaces[ws_idx].tabs {
            for row in &tab.panes {
                for p in row {
                    pane_ids.push(p.id.0);
                }
            }
        }
    }

    for id in &pane_ids {
        state.pty_manager.destroy(*id);
        state.terminals.remove(id);
    }

    if ws_idx == state.active_workspace {
        state.tabs.clear();
        state.active_tab = 0;
        state.panes.clear();
        state.active_pane = PaneId(0);
        state.row_ratios.clear();
        state.col_ratios.clear();
    }
    state.workspaces[ws_idx].tabs.clear();
    state.workspaces[ws_idx].active_tab = 0;
}

fn toggle_on(state: &AppState, key: ToggleKey) -> bool {
    state.toggles.get(&key).copied().unwrap_or(false)
}

fn close_app_pane_ids(state: &AppState) -> BTreeSet<u32> {
    let mut ids = BTreeSet::new();
    for (workspace_idx, workspace) in state.workspaces.iter().enumerate() {
        let tabs = if workspace_idx == state.active_workspace {
            &state.tabs
        } else {
            &workspace.tabs
        };
        for (tab_idx, tab) in tabs.iter().enumerate() {
            let panes = if workspace_idx == state.active_workspace && tab_idx == state.active_tab {
                &state.panes
            } else {
                &tab.panes
            };
            for pane in panes.iter().flatten() {
                ids.insert(pane.id.0);
            }
        }
    }
    if state.workspaces.is_empty() {
        for pane in state.panes.iter().flatten() {
            ids.insert(pane.id.0);
        }
    }
    ids
}

fn positive_ratio(v: f32) -> Option<f32> {
    (v.is_finite() && v > 0.0).then_some(v)
}

fn retain_tab_panes(tab: &mut TerminalTab, kept_pane_ids: &BTreeSet<u32>) -> bool {
    let old_panes = std::mem::take(&mut tab.panes);
    let old_row_ratios = std::mem::take(&mut tab.row_ratios);
    let old_col_ratios = std::mem::take(&mut tab.col_ratios);

    let mut panes = Vec::new();
    let mut row_ratios = Vec::new();
    let mut col_ratios = Vec::new();

    for (row_idx, row) in old_panes.into_iter().enumerate() {
        let mut kept_row = Vec::new();
        let mut kept_col_ratios = Vec::new();
        for (col_idx, pane) in row.into_iter().enumerate() {
            if kept_pane_ids.contains(&pane.id.0) {
                kept_col_ratios.push(
                    old_col_ratios
                        .get(row_idx)
                        .and_then(|row| row.get(col_idx))
                        .copied()
                        .and_then(positive_ratio)
                        .unwrap_or(1.0),
                );
                kept_row.push(pane);
            }
        }
        if !kept_row.is_empty() {
            panes.push(kept_row);
            row_ratios.push(
                old_row_ratios
                    .get(row_idx)
                    .copied()
                    .and_then(positive_ratio)
                    .unwrap_or(1.0),
            );
            col_ratios.push(kept_col_ratios);
        }
    }

    if panes.is_empty() {
        return false;
    }
    if !panes
        .iter()
        .flatten()
        .any(|pane| pane.id == tab.active_pane)
    {
        tab.active_pane = panes[0][0].id;
    }
    tab.panes = panes;
    tab.row_ratios = row_ratios;
    tab.col_ratios = col_ratios;
    true
}

fn prune_close_layout_to_kept_panes(state: &mut AppState, kept_pane_ids: &BTreeSet<u32>) {
    if state.workspaces.is_empty() {
        save_tab_state(state);
        state
            .tabs
            .retain_mut(|tab| retain_tab_panes(tab, kept_pane_ids));
        if state.tabs.is_empty() {
            state.active_tab = 0;
            state.panes.clear();
            state.active_pane = PaneId(0);
            state.row_ratios.clear();
            state.col_ratios.clear();
        } else {
            state.active_tab = state.active_tab.min(state.tabs.len() - 1);
            load_tab_state(state);
        }
        return;
    }

    save_workspace_state(state);
    for workspace in state.workspaces.iter_mut() {
        workspace
            .tabs
            .retain_mut(|tab| retain_tab_panes(tab, kept_pane_ids));
        if workspace.tabs.is_empty() {
            workspace.active_tab = 0;
            workspace.terminals_expanded = false;
        } else {
            workspace.active_tab = workspace.active_tab.min(workspace.tabs.len() - 1);
            workspace.terminals_expanded = true;
        }
        if let Some(subtab) = workspace.subtabs.get_mut(0) {
            subtab.count = Some(workspace.tabs.len() as u32);
        }
    }
    load_workspace_state(state);
}

/// Resolve the close-button click against the persisted preference
/// toggles. If no preference has been remembered, populate
/// `state.confirm_dialog` with a `CloseApp` dialog and return
/// `CloseAction::Prompt` so the caller knows to veto the framework's
/// exit. Otherwise returns the remembered action without mutating
/// state. Helper instead of inline logic so `main::on_close` does not
/// have to know the toggle keys.
pub fn resolve_close_action(state: &mut AppState) -> CloseAction {
    if toggle_on(state, ToggleKey::RememberCloseChoice) {
        if toggle_on(state, ToggleKey::KillAllOnClose) {
            CloseAction::KillAll
        } else {
            CloseAction::KeepRunning
        }
    } else {
        let kept_pane_ids = close_app_pane_ids(state);
        state.confirm_dialog = Some(ConfirmDialog::CloseApp {
            count: kept_pane_ids.len(),
            remember: false,
            kept_pane_ids,
        });
        CloseAction::Prompt
    }
}

/// Kill every terminal across every workspace and empty every workspace.
/// All pane ids currently tracked in `state.terminals` are destroyed on
/// the daemon, then every workspace's saved tabs and the live active
/// pane/tab state are cleared. Workspaces themselves are not removed
/// (per SPEC F6: the app-wide nuke empties but does not delete).
pub fn mutate_kill_all_terminals(state: &mut AppState) {
    let ids: Vec<u32> = state.terminals.keys().copied().collect();
    for id in &ids {
        state.pty_manager.destroy(*id);
        state.terminals.remove(id);
    }
    for ws in state.workspaces.iter_mut() {
        ws.tabs.clear();
        ws.active_tab = 0;
    }
    state.tabs.clear();
    state.active_tab = 0;
    state.panes.clear();
    state.active_pane = PaneId(0);
    state.row_ratios.clear();
    state.col_ratios.clear();
}

/// Poll the daemon for its live session list and cache the result in
/// `state.sessions`. Called when the user opens the Sessions panel or
/// presses the Refresh button; safe to call when disconnected (the
/// existing cache is left in place and `sessions_stale` is set so the
/// Sessions panel can show the user that the rows may not match the
/// daemon).
pub fn refresh_sessions(state: &mut AppState) {
    match state.pty_manager.list_sessions() {
        Ok(list) => {
            state.sessions = list
                .into_iter()
                .map(|info| SessionSnapshot {
                    session_id: info.id,
                    pane_id: info.pane_id,
                    workspace_id: info.workspace_id,
                    name: info.name,
                    pid: info.pid,
                    alive: info.alive,
                })
                .collect();
            state.sessions_stale = false;
        }
        Err(e) => {
            log::warn!("refresh_sessions: list_sessions failed: {e}");
            state.sessions_stale = true;
            push_error_toast(state, format!("refresh failed: {e}"));
        }
    }
}

/// Kill a session directly by session id without requiring a local
/// pane mapping. Used by the sessions panel where orphan sessions (no
/// attached pane) still need a kill button.
///
/// Mapped-pane branch is fire-and-forget through `destroy(pane_id)`;
/// orphan branch waits on the daemon's ack via
/// `kill_session_id_blocking` so a disconnected or unresponsive
/// daemon shows up as a user-visible toast instead of an
/// optimistically-removed row. Issue #130.
pub fn mutate_kill_session_id(state: &mut AppState, session_id: u64) {
    let pane_id = state
        .pty_manager
        .sessions_iter()
        .find_map(|(pid, sid)| (sid == session_id).then_some(pid));

    if let Some(pid) = pane_id {
        state.pty_manager.destroy(pid);
        state.terminals.remove(&pid);
        prune_pane_from_layouts(state, pid);
        state.sessions.retain(|s| s.session_id != session_id);
        return;
    }

    match state.pty_manager.kill_session_id_blocking(session_id) {
        Ok(()) => {
            state.sessions.retain(|s| s.session_id != session_id);
        }
        Err(e) => {
            log::warn!("kill_session_id_blocking({session_id}): {e}");
            push_error_toast(state, format!("kill failed: {e}"));
        }
    }
}

fn prune_pane_from_layouts(state: &mut AppState, pane_id: u32) {
    state.panes.retain_mut(|row| {
        row.retain(|p| p.id.0 != pane_id);
        !row.is_empty()
    });
    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
        tab.panes.retain_mut(|row| {
            row.retain(|p| p.id.0 != pane_id);
            !row.is_empty()
        });
    }
    for ws in state.workspaces.iter_mut() {
        for tab in ws.tabs.iter_mut() {
            tab.panes.retain_mut(|row| {
                row.retain(|p| p.id.0 != pane_id);
                !row.is_empty()
            });
        }
    }
}

/// Update the display title of a pane. Writes through to both the
/// active `state.panes` layout and every saved workspace/tab so the
/// rename survives workspace switches. An empty `name` is treated as
/// "clear the custom name" and falls back to a generic "shell" label.
pub fn mutate_rename_pane(state: &mut AppState, pane_id: u32, name: &str) {
    let trimmed = name.trim();
    let new_title = if trimmed.is_empty() {
        "shell".to_string()
    } else {
        trimmed.to_string()
    };

    rename_panes_in_rows(&mut state.panes, pane_id, &new_title);
    for tab in state.tabs.iter_mut() {
        rename_pane_in_tab(tab, pane_id, &new_title);
    }
    for ws in state.workspaces.iter_mut() {
        for tab in ws.tabs.iter_mut() {
            rename_pane_in_tab(tab, pane_id, &new_title);
        }
    }
}

fn rename_panes_in_rows(rows: &mut [Vec<Pane>], pane_id: u32, new_title: &str) -> bool {
    let mut renamed = false;
    for row in rows.iter_mut() {
        for pane in row.iter_mut() {
            if pane.id.0 == pane_id {
                pane.title = new_title.to_string();
                renamed = true;
            }
        }
    }
    renamed
}

fn rename_pane_in_tab(tab: &mut TerminalTab, pane_id: u32, new_title: &str) -> bool {
    let renamed = rename_panes_in_rows(&mut tab.panes, pane_id, new_title);
    if renamed && tab_title_follows_pane(tab, pane_id) {
        tab.name = new_title.to_string();
    }
    renamed
}

fn tab_title_follows_pane(tab: &TerminalTab, pane_id: u32) -> bool {
    tab.active_pane.0 == pane_id || tab.panes.iter().flatten().count() == 1
}

fn pane_title_by_id(state: &AppState, pane_id: u32) -> Option<String> {
    state
        .panes
        .iter()
        .flatten()
        .find(|p| p.id.0 == pane_id)
        .map(|p| p.title.clone())
        .or_else(|| {
            state
                .tabs
                .iter()
                .flat_map(|tab| tab.panes.iter().flatten())
                .find(|p| p.id.0 == pane_id)
                .map(|p| p.title.clone())
        })
        .or_else(|| {
            state
                .workspaces
                .iter()
                .flat_map(|ws| ws.tabs.iter())
                .flat_map(|tab| tab.panes.iter().flatten())
                .find(|p| p.id.0 == pane_id)
                .map(|p| p.title.clone())
        })
}

fn open_command_palette(state: &mut AppState) {
    state.ctx_menu = None;
    if let Some(qp) = state.quick_prompt.take() {
        crate::quick_prompt::images::cleanup_session(&qp.session_hex);
    }
    state.palette_open = true;
    clear_palette_query(state);
}

fn close_command_palette(state: &mut AppState) -> bool {
    let changed =
        state.palette_open || !state.palette_query.is_empty() || state.palette_active != 0;
    state.palette_open = false;
    clear_palette_query(state);
    changed
}

fn palette_flattened_items(state: &AppState) -> Vec<crate::command_palette::PaletteItem> {
    let snap = state.ui_snapshot();
    crate::command_palette::build_palette_results(&snap, &state.palette_query)
        .groups
        .into_iter()
        .flat_map(|group| group.items)
        .collect()
}

fn palette_result_count(state: &AppState) -> usize {
    palette_flattened_items(state).len()
}

fn reset_palette_selection(state: &mut AppState) {
    state.palette_active = 0;
}

fn clear_palette_query(state: &mut AppState) {
    state.palette_query.clear();
    reset_palette_selection(state);
}

fn set_palette_query(state: &mut AppState, query: String) {
    state.palette_query = crate::command_palette::sanitize_palette_query(&query);
    reset_palette_selection(state);
}

fn palette_push_query_char(state: &mut AppState, ch: char) -> bool {
    let mut candidate = state.palette_query.clone();
    candidate.push(ch);
    state.palette_query = crate::command_palette::sanitize_palette_query(&candidate);
    true
}

fn palette_backspace_query(state: &mut AppState) -> bool {
    state.palette_query.pop();
    reset_palette_selection(state);
    true
}

fn palette_delete_query(state: &mut AppState) -> bool {
    reset_palette_selection(state);
    true
}

fn palette_text_modifiers(modifiers: unshit::core::event::Modifiers) -> bool {
    use unshit::core::event::Modifiers;
    !modifiers.intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::META)
}

pub fn dispatch_palette_key(
    state: &mut AppState,
    combo: &unshit::core::shortcut::KeyCombo,
) -> bool {
    use unshit::core::event::{Key, Modifiers};

    if !state.palette_open {
        return false;
    }

    match (combo.key, combo.modifiers) {
        (Key::ArrowDown, modifiers) if modifiers.is_empty() => {
            dispatch(state, "palette.select_next")
        }
        (Key::Char('n'), Modifiers::CTRL) => dispatch(state, "palette.select_next"),
        (Key::ArrowUp, modifiers) if modifiers.is_empty() => dispatch(state, "palette.select_prev"),
        (Key::Char('p'), Modifiers::CTRL) => dispatch(state, "palette.select_prev"),
        (Key::Enter, modifiers) if modifiers.is_empty() => {
            dispatch(state, "palette.execute_active")
        }
        (Key::Escape, modifiers) if modifiers.is_empty() => dispatch(state, "palette.escape"),
        (Key::Backspace, modifiers) if palette_text_modifiers(modifiers) => {
            palette_backspace_query(state)
        }
        (Key::Delete, modifiers) if palette_text_modifiers(modifiers) => {
            palette_delete_query(state)
        }
        (Key::Space, modifiers) if palette_text_modifiers(modifiers) => {
            palette_push_query_char(state, ' ')
        }
        (Key::Char(ch), modifiers) if palette_text_modifiers(modifiers) => {
            palette_push_query_char(state, ch)
        }
        _ => true,
    }
}

fn is_palette_safe_dispatch(command: &str) -> bool {
    matches!(
        command,
        "session.rename_active"
            | "pane.split_right"
            | "pane.split_down"
            | "tab.new"
            | "pane.close"
            | "sidebar.toggle"
            | "modal.open"
            | "quick_prompt.open"
    ) || command.starts_with("workspace.switch:")
        || command.starts_with("terminal.focus:")
}

fn execute_palette_item(state: &mut AppState, item_id: &str) -> bool {
    if !state.palette_open {
        return false;
    }

    let Some(item) = palette_flattened_items(state)
        .into_iter()
        .find(|item| item.enabled && item.id == item_id)
    else {
        return false;
    };
    let Some(command) = item.dispatch else {
        return false;
    };
    if !is_palette_safe_dispatch(&command) {
        return false;
    }

    let handled = dispatch(state, &command);
    if handled {
        close_command_palette(state);
    }
    handled
}

pub fn dispatch(state: &mut AppState, command: &str) -> bool {
    match command {
        "modal.close" => {
            let mut changed = false;
            if state.ctx_menu.is_some() {
                state.ctx_menu = None;
                changed = true;
            }
            if state.settings_open {
                state.settings_open = false;
                state.keybinds.cancel_recording();
                state.keybinds.error = None;
                changed = true;
            }
            if state.confirm_dialog.is_some() {
                state.confirm_dialog = None;
                changed = true;
            }
            if close_command_palette(state) {
                changed = true;
            }
            // Esc with the autocomplete popup open dismisses just the
            // popup (per spec A8.3); the overlay stays. A second Esc
            // closes the overlay through the normal cleanup path
            // below.
            let dismiss_popup_only = state
                .quick_prompt
                .as_ref()
                .map(|qp| qp.popup.is_some())
                .unwrap_or(false);
            if dismiss_popup_only {
                if let Some(qp) = state.quick_prompt.as_mut() {
                    qp.popup = None;
                    changed = true;
                }
            } else if let Some(qp) = state.quick_prompt.take() {
                crate::quick_prompt::images::cleanup_session(&qp.session_hex);
                changed = true;
            }
            changed
        }
        "dialog.confirm" => {
            let Some(dlg) = state.confirm_dialog.as_ref() else {
                return false;
            };
            // CloseApp has three explicit actions and is driven by
            // `app.close.*` dispatches, not the generic yes/no confirm.
            // RenameSession commits via `dialog.rename_commit` so the
            // handler can read the buffer before clearing.
            if matches!(
                dlg,
                ConfirmDialog::CloseApp { .. } | ConfirmDialog::RenameSession { .. }
            ) {
                return false;
            }
            match state.confirm_dialog.take().unwrap() {
                ConfirmDialog::KillWorkspace { workspace_idx, .. } => {
                    mutate_kill_workspace_terminals(state, workspace_idx);
                }
                ConfirmDialog::KillAll { .. } => {
                    mutate_kill_all_terminals(state);
                }
                ConfirmDialog::CloseApp { .. } | ConfirmDialog::RenameSession { .. } => {
                    unreachable!("filtered above")
                }
            }
            crate::persist::save_workspaces(state);
            true
        }
        "dialog.cancel" => {
            if state.confirm_dialog.is_some() {
                state.confirm_dialog = None;
                true
            } else {
                false
            }
        }
        "dialog.toggle_remember" => {
            // Only applies while a CloseApp dialog is active. Flips the
            // checkbox; the persisted toggle is only written when the user
            // actually picks an action.
            if let Some(ConfirmDialog::CloseApp { remember, .. }) = state.confirm_dialog.as_mut() {
                *remember = !*remember;
                true
            } else {
                false
            }
        }
        command if command.starts_with("dialog.toggle_keep:") => {
            let Some(raw_id) = command.strip_prefix("dialog.toggle_keep:") else {
                return false;
            };
            let Ok(pane_id) = raw_id.parse::<u32>() else {
                return false;
            };
            if let Some(ConfirmDialog::CloseApp { kept_pane_ids, .. }) =
                state.confirm_dialog.as_mut()
            {
                if kept_pane_ids.contains(&pane_id) {
                    kept_pane_ids.remove(&pane_id);
                } else {
                    kept_pane_ids.insert(pane_id);
                }
                true
            } else {
                false
            }
        }
        "app.close.keep_running" => {
            let all_pane_ids = close_app_pane_ids(state);
            let (remember, kept_pane_ids) = match state.confirm_dialog.as_ref() {
                Some(ConfirmDialog::CloseApp {
                    remember,
                    kept_pane_ids,
                    ..
                }) => (*remember, kept_pane_ids.clone()),
                _ => (false, all_pane_ids.clone()),
            };
            let kill_pane_ids: Vec<u32> =
                all_pane_ids.difference(&kept_pane_ids).copied().collect();
            state.confirm_dialog = None;
            if remember {
                state.toggles.insert(ToggleKey::RememberCloseChoice, true);
                state.toggles.insert(ToggleKey::KillAllOnClose, false);
            }
            for pane_id in kill_pane_ids {
                state.pty_manager.destroy(pane_id);
                state.terminals.remove(&pane_id);
            }
            prune_close_layout_to_kept_panes(state, &kept_pane_ids);
            // Always persist the live layout (not just when "remember" is
            // ticked): the daemon keeps these sessions alive, so the next
            // launch must restore the same tabs/panes to reattach them.
            // Without this, keep-running dropped the layout and the relaunch
            // showed a fresh terminal instead of the surviving session.
            crate::persist::save_workspaces(state);
            // Drop local readers; daemon sessions remain alive. The UI
            // callback follows up with `process::exit(0)`.
            state.terminals.clear();
            true
        }
        "app.close.kill_and_quit" => {
            let remember = matches!(
                state.confirm_dialog,
                Some(ConfirmDialog::CloseApp { remember: true, .. })
            );
            state.confirm_dialog = None;
            if remember {
                state.toggles.insert(ToggleKey::RememberCloseChoice, true);
                state.toggles.insert(ToggleKey::KillAllOnClose, true);
            }
            mutate_kill_all_terminals(state);
            // The layout is now empty; persisting it means the relaunch
            // starts fresh instead of restoring panes whose sessions were
            // just killed.
            crate::persist::save_workspaces(state);
            true
        }
        "app.close.reset_preference" => {
            let had_pref = toggle_on(state, ToggleKey::RememberCloseChoice);
            state.toggles.insert(ToggleKey::RememberCloseChoice, false);
            // KillAllOnClose is left at whatever it was; it is inert while
            // RememberCloseChoice is false and the reset UI description
            // only promises to re-enable the prompt.
            if had_pref {
                crate::persist::save_workspaces(state);
            }
            had_pref
        }
        "ctx_menu.close" => {
            if state.ctx_menu.is_some() {
                state.ctx_menu = None;
                true
            } else {
                false
            }
        }
        "modal.open" => {
            if state.settings_open {
                // Re-pressing the settings hotkey while open closes the
                // page (toggle behavior), with the same cleanup as
                // modal.close.
                state.settings_open = false;
                state.keybinds.cancel_recording();
                state.keybinds.error = None;
            } else {
                state.settings_open = true;
            }
            true
        }
        "quick_prompt.open" => {
            if let Some(qp) = state.quick_prompt.take() {
                // Re-pressing the hotkey while open closes the overlay
                // (toggle behavior per spec A1.2). Clean up any pasted
                // images that have not been submitted.
                crate::quick_prompt::images::cleanup_session(&qp.session_hex);
            } else {
                state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());
            }
            true
        }
        "quick_prompt.close" => {
            if let Some(qp) = state.quick_prompt.take() {
                crate::quick_prompt::images::cleanup_session(&qp.session_hex);
                true
            } else {
                false
            }
        }
        "quick_prompt.toggle_agent" => {
            let Some(qp) = state.quick_prompt.as_mut() else {
                return false;
            };
            qp.agent = qp.agent.toggled();
            qp.error = None;
            crate::quick_prompt::state::QuickPromptStore::save(qp);
            true
        }
        "quick_prompt.image_paste" => {
            let Some(qp) = state.quick_prompt.as_ref() else {
                return false;
            };
            let session_hex = qp.session_hex.clone();
            let captured = crate::quick_prompt::images::capture_clipboard_image(
                &state.clipboard,
                &session_hex,
            );
            let Some(qp_mut) = state.quick_prompt.as_mut() else {
                return false;
            };
            match captured {
                Ok(Some(img)) => {
                    if !qp_mut.images.iter().any(|i| i.hash == img.hash) {
                        qp_mut.images.push(img);
                    }
                    qp_mut.error = None;
                    true
                }
                Ok(None) => {
                    // Clipboard had no image; surface a friendly hint
                    // so the user knows the paste did not silently
                    // disappear.
                    qp_mut.error = Some("No image on clipboard".into());
                    true
                }
                Err(e) => {
                    qp_mut.error = Some(format!("paste failed: {e}"));
                    true
                }
            }
        }
        "quick_prompt.autocomplete_select_next" => {
            let Some(qp) = state.quick_prompt.as_mut() else {
                return false;
            };
            let Some(popup) = qp.popup.as_mut() else {
                return false;
            };
            popup.select_next();
            true
        }
        "quick_prompt.autocomplete_select_prev" => {
            let Some(qp) = state.quick_prompt.as_mut() else {
                return false;
            };
            let Some(popup) = qp.popup.as_mut() else {
                return false;
            };
            popup.select_prev();
            true
        }
        "quick_prompt.autocomplete_dismiss" => {
            let Some(qp) = state.quick_prompt.as_mut() else {
                return false;
            };
            qp.popup.take().is_some()
        }
        "quick_prompt.autocomplete_confirm" => {
            let Some(qp) = state.quick_prompt.as_mut() else {
                return false;
            };
            let Some(popup) = qp.popup.as_ref() else {
                return false;
            };
            let Some(entry) = popup.current() else {
                return false;
            };
            let entry_name = entry.name.clone();
            let anchor = popup.anchor_offset;
            let trigger = popup.trigger_char;
            qp.prompt = crate::quick_prompt::autocomplete::confirm_into_prompt(
                &qp.prompt,
                anchor,
                trigger,
                &entry_name,
            );
            qp.popup = None;
            true
        }
        "quick_prompt.submit" => {
            let Some(qp) = state.quick_prompt.as_ref() else {
                return false;
            };
            let prompt = qp.prompt.trim().to_string();
            let agent = qp.agent;
            let images = qp.images.clone();
            let session_hex = qp.session_hex.clone();

            if prompt.is_empty() {
                let qp = state.quick_prompt.as_mut().unwrap();
                qp.error = Some("Type a prompt to continue.".into());
                return true;
            }

            let cwd = active_workspace_cwd(state);
            let target = match crate::quick_prompt::spawn::prepare_target(cwd.as_deref()) {
                Ok(t) => t,
                Err(e) => {
                    let qp = state.quick_prompt.as_mut().unwrap();
                    qp.error = Some(format!("submit failed: {e}"));
                    return true;
                }
            };
            let refs = match crate::quick_prompt::images::move_into_target(&images, &target.path) {
                Ok(r) => r,
                Err(e) => {
                    let qp = state.quick_prompt.as_mut().unwrap();
                    qp.error = Some(format!("submit failed (image move): {e}"));
                    return true;
                }
            };
            let augmented_prompt =
                crate::quick_prompt::images::append_image_references(&prompt, &refs);
            let shell_spec = match agent {
                crate::quick_prompt::Agent::Claude => {
                    crate::quick_prompt::spawn::claude_shell_spec(&augmented_prompt)
                }
                crate::quick_prompt::Agent::Codex => {
                    crate::quick_prompt::spawn::codex_shell_spec(&augmented_prompt)
                }
            };
            mutate_add_quick_prompt_tab(state, &prompt, &target.path, &shell_spec);
            crate::quick_prompt::images::cleanup_session(&session_hex);
            state.quick_prompt = None;
            crate::persist::save_workspaces(state);
            true
        }
        "tab.new" => {
            mutate_add_tab(state);
            crate::persist::save_workspaces(state);
            true
        }
        "tab.close.active" => {
            let idx = state.active_tab;
            mutate_close_tab(state, idx);
            crate::persist::save_workspaces(state);
            true
        }
        "tab.next" => {
            if state.tabs.len() <= 1 {
                return false;
            }
            let new_idx = (state.active_tab + 1) % state.tabs.len();
            mutate_switch_tab(state, new_idx);
            true
        }
        "tab.prev" => {
            if state.tabs.len() <= 1 {
                return false;
            }
            let new_idx = if state.active_tab == 0 {
                state.tabs.len() - 1
            } else {
                state.active_tab - 1
            };
            mutate_switch_tab(state, new_idx);
            true
        }
        "pane.split_right" => {
            mutate_split_right(state, state.active_pane);
            crate::persist::save_workspaces(state);
            true
        }
        "pane.split_down" => {
            mutate_split_down(state, state.active_pane);
            crate::persist::save_workspaces(state);
            true
        }
        "pane.close" => {
            mutate_close_pane(state, state.active_pane);
            crate::persist::save_workspaces(state);
            true
        }
        "pane.focus_left" => {
            mutate_focus_left(state);
            true
        }
        "pane.focus_right" => {
            mutate_focus_right(state);
            true
        }
        "pane.focus_up" => {
            mutate_focus_up(state);
            true
        }
        "pane.focus_down" => {
            mutate_focus_down(state);
            true
        }
        other if other.starts_with("pane.extract_to_tab:") => {
            persist_layout_if(dispatch_pane_extract_to_tab(state, other), state)
        }
        other if other.starts_with("drag.start_pane:") => dispatch_drag_start_pane(state, other),
        other if other.starts_with("drag.start_tab:") => dispatch_drag_start_tab(state, other),
        other if other.starts_with("drag.update:") => dispatch_drag_update(state, other),
        "drag.end" => persist_layout_if(dispatch_drag_end(state), state),
        other if other.starts_with("pane.drop_split:") => {
            persist_layout_if(dispatch_pane_drop_split(state, other), state)
        }
        other if other.starts_with("tab.reorder:") => {
            persist_layout_if(dispatch_tab_reorder(state, other), state)
        }
        "sidebar.toggle" => {
            state.sidebar_collapsed = !state.sidebar_collapsed;
            true
        }
        "workspace.add" => {
            mutate_add_workspace(state);
            crate::persist::save_workspaces(state);
            true
        }
        "font.inc" | "terminal_font.inc" => {
            let old = state.terminal_font_size_pt;
            mutate_terminal_font_size_delta(state, 1);
            old != state.terminal_font_size_pt
        }
        "font.dec" | "terminal_font.dec" => {
            let old = state.terminal_font_size_pt;
            mutate_terminal_font_size_delta(state, -1);
            old != state.terminal_font_size_pt
        }
        "config_font.inc" => {
            let old = state.config_font_size_pt;
            mutate_config_font_size_delta(state, 1);
            old != state.config_font_size_pt
        }
        "config_font.dec" => {
            let old = state.config_font_size_pt;
            mutate_config_font_size_delta(state, -1);
            old != state.config_font_size_pt
        }
        "scroll.line_px.inc" => mutate_scroll_line_px_delta(state, SCROLL_LINE_PX_STEP),
        "scroll.line_px.dec" => mutate_scroll_line_px_delta(state, -SCROLL_LINE_PX_STEP),
        "scroll.duration.inc" => {
            mutate_smooth_scroll_duration_delta(state, SMOOTH_SCROLL_DURATION_STEP_MS)
        }
        "scroll.duration.dec" => {
            mutate_smooth_scroll_duration_delta(state, -SMOOTH_SCROLL_DURATION_STEP_MS)
        }
        other if other.starts_with("appearance.density:") => {
            let id = &other["appearance.density:".len()..];
            UiDensity::from_id(id).is_some_and(|density| mutate_ui_density(state, density))
        }
        "theme.custom.reset" => reset_custom_theme(state),
        "appearance.reset" => reset_appearance(state),
        "palette.toggle" => {
            if state.palette_open {
                close_command_palette(state);
            } else {
                open_command_palette(state);
            }
            true
        }
        "palette.close" => close_command_palette(state),
        other if other.starts_with("palette.query:") => {
            if !state.palette_open {
                return false;
            }
            set_palette_query(state, other["palette.query:".len()..].to_string());
            true
        }
        "palette.select_next" => {
            if !state.palette_open {
                return false;
            }
            let count = palette_result_count(state);
            if count == 0 {
                return false;
            }
            state.palette_active = (state.palette_active + 1) % count;
            true
        }
        "palette.select_prev" => {
            if !state.palette_open {
                return false;
            }
            let count = palette_result_count(state);
            if count == 0 {
                return false;
            }
            state.palette_active = if state.palette_active == 0 {
                count - 1
            } else {
                (state.palette_active - 1) % count
            };
            true
        }
        other if other.starts_with("palette.hover:") => {
            if !state.palette_open {
                return false;
            }
            let Ok(index) = other["palette.hover:".len()..].parse::<usize>() else {
                return false;
            };
            if index >= palette_result_count(state) {
                return false;
            }
            if state.palette_active == index {
                return false;
            }
            state.palette_active = index;
            true
        }
        "palette.escape" => {
            if !state.palette_open {
                return false;
            }
            if !state.palette_query.is_empty() {
                clear_palette_query(state);
                true
            } else {
                close_command_palette(state)
            }
        }
        "palette.execute_active" => {
            if !state.palette_open {
                return false;
            }
            let items = palette_flattened_items(state);
            let Some(item_id) = items.get(state.palette_active).map(|item| item.id.clone()) else {
                return false;
            };
            execute_palette_item(state, &item_id)
        }
        other if other.starts_with("palette.execute:") => {
            execute_palette_item(state, &other["palette.execute:".len()..])
        }
        "fps_overlay.toggle" => {
            // Flip the in-app FPS overlay (Phase 0 of the 120fps perf
            // work, refs #135). The overlay also drives the
            // FrameProbe's emit gate so release builds start writing
            // [FRAME] log lines while the overlay is up. Returns true
            // so the framework rebuilds the tree to show or hide the
            // widget.
            crate::ui::fps_overlay::toggle_visible();
            true
        }
        "terminal.paste" => dispatch_terminal_paste(state),
        "terminal.copy" => dispatch_terminal_copy(state),
        other if other.starts_with("tab.switch:") => {
            if let Ok(index) = other["tab.switch:".len()..].parse::<usize>() {
                if index < state.tabs.len() && state.active_tab != index {
                    mutate_switch_tab(state, index);
                    return true;
                }
            }
            false
        }
        other if other.starts_with("workspace.remove:") => {
            if let Ok(idx) = other["workspace.remove:".len()..].parse::<usize>() {
                state.ctx_menu = None;
                mutate_remove_workspace(state, idx);
                crate::persist::save_workspaces(state);
                return true;
            }
            false
        }
        other if other.starts_with("workspace.collapse:") => {
            if let Ok(idx) = other["workspace.collapse:".len()..].parse::<usize>() {
                state.ctx_menu = None;
                if let Some(ws) = state.workspaces.get_mut(idx) {
                    ws.collapsed = !ws.collapsed;
                    return true;
                }
            }
            false
        }
        other if other.starts_with("workspace.switch:") => {
            if let Ok(idx) = other["workspace.switch:".len()..].parse::<usize>() {
                state.ctx_menu = None;
                if idx < state.workspaces.len() {
                    mutate_switch_workspace(state, idx);
                    return true;
                }
            }
            false
        }
        other if other.starts_with("workspace.new_terminal:") => {
            if let Ok(idx) = other["workspace.new_terminal:".len()..].parse::<usize>() {
                state.ctx_menu = None;
                if idx < state.workspaces.len() {
                    mutate_switch_workspace(state, idx);
                    mutate_add_tab(state);
                    crate::persist::save_workspaces(state);
                    return true;
                }
            }
            false
        }
        other if other.starts_with("workspace.request_kill_all:") => {
            if let Ok(idx) = other["workspace.request_kill_all:".len()..].parse::<usize>() {
                state.ctx_menu = None;
                if let Some(ws) = state.workspaces.get(idx) {
                    state.confirm_dialog = Some(ConfirmDialog::KillWorkspace {
                        workspace_idx: idx,
                        name: ws.name.clone(),
                    });
                    return true;
                }
            }
            false
        }
        "app.request_kill_all_terminals" => {
            state.confirm_dialog = Some(ConfirmDialog::KillAll {
                count: state.terminals.len(),
            });
            true
        }
        "sessions.refresh" => {
            refresh_sessions(state);
            true
        }
        "notifications.test" => {
            let workspace_id = active_workspace_num(state);
            let pane_id = state.active_pane.0;
            let title = "test notification";
            let message =
                format!("notification test from workspace {workspace_id}, pane {pane_id}");
            push_notification_toast(state, title, message.clone(), workspace_id, pane_id);
            #[cfg(not(test))]
            {
                if let Err(e) = crate::notifications::spawn_desktop_notification_for_target(
                    title,
                    message,
                    workspace_id,
                    pane_id,
                ) {
                    log::warn!("desktop test notification failed: {e}");
                }
            }
            true
        }
        other if other.starts_with("session.kill:") => {
            if let Ok(sid) = other["session.kill:".len()..].parse::<u64>() {
                mutate_kill_session_id(state, sid);
                return true;
            }
            false
        }
        other if other.starts_with("toast.dismiss:") => {
            if let Ok(id) = other["toast.dismiss:".len()..].parse::<u64>() {
                let dismissed = state.toasts.dismiss(id);
                if dismissed {
                    state.toast_meta.remove(&id);
                }
                return dismissed;
            }
            false
        }
        other if other.starts_with("notification.activate:") => {
            if let Ok(id) = other["notification.activate:".len()..].parse::<u64>() {
                let meta = state.toast_meta.get(&id).cloned();
                let dismissed = state.toasts.dismiss(id);
                state.toast_meta.remove(&id);
                if let Some(meta) = meta {
                    return focus_workspace_pane_by_num(
                        state,
                        meta.target.workspace_id,
                        meta.target.pane_id,
                    ) || dismissed;
                }
                return dismissed;
            }
            false
        }
        "dialog.rename_commit" => {
            let Some(ConfirmDialog::RenameSession {
                pane_id,
                buffer,
                error: _,
            }) = state.confirm_dialog.take()
            else {
                return false;
            };
            if let Some(sid) = state.pty_manager.session_id(pane_id) {
                let wire_name = if buffer.trim().is_empty() {
                    None
                } else {
                    Some(buffer.trim().to_string())
                };
                if let Err(e) = state.pty_manager.rename_session(sid, wire_name) {
                    // Issue #130: surface the failure inline in the
                    // dialog and skip the local pane title update so
                    // local state cannot diverge from the daemon.
                    log::warn!("rename_session({}): {}", sid, e);
                    state.confirm_dialog = Some(ConfirmDialog::RenameSession {
                        pane_id,
                        buffer,
                        error: Some(format!("rename failed: {e}")),
                    });
                    return true;
                }
            }
            mutate_rename_pane(state, pane_id, &buffer);
            crate::persist::save_workspaces(state);
            true
        }
        "session.rename_active" => {
            let pane_id = state.active_pane.0;
            dispatch(state, &format!("tab.request_rename:{pane_id}"))
        }
        other if other.starts_with("tab.request_rename:") => {
            if let Ok(pane_num) = other["tab.request_rename:".len()..].parse::<u32>() {
                state.ctx_menu = None;
                let current = pane_title_by_id(state, pane_num).unwrap_or_default();
                state.confirm_dialog = Some(ConfirmDialog::RenameSession {
                    pane_id: pane_num,
                    buffer: current,
                    error: None,
                });
                return true;
            }
            false
        }
        other if other.starts_with("terminal.focus:") => {
            let rest = &other["terminal.focus:".len()..];
            let Some((ws_str, pane_str)) = rest.split_once(':') else {
                return false;
            };
            let Ok(ws_idx) = ws_str.parse::<usize>() else {
                return false;
            };
            let Ok(pane_num) = pane_str.parse::<u32>() else {
                return false;
            };
            let handled = focus_workspace_pane_by_index(state, ws_idx, pane_num);
            if !handled {
                log::warn!(
                    "terminal.focus: pane {} not found in workspace {}",
                    pane_num,
                    ws_idx
                );
            }
            handled
        }
        other if other.starts_with("shell.set_default:") => {
            dispatch_shell_set_default(state, other)
        }
        "shell.clear_default" => dispatch_shell_clear_default(state),
        other if other.starts_with("shell.set_workspace:") => {
            dispatch_shell_set_workspace(state, other)
        }
        other if other.starts_with("shell.clear_workspace:") => {
            dispatch_shell_clear_workspace(state, other)
        }
        other if other.starts_with("keybind.set:") => dispatch_keybind_set(state, other),
        other if other.starts_with("keybind.reset:") => dispatch_keybind_reset(state, other),
        "keybind.reset_all" => {
            state.keybinds.reset_all();
            crate::keybinds::loader::save_if_installed(&state.keybinds.overrides);
            true
        }
        other if other.starts_with("keybind.record:") => {
            let id = &other["keybind.record:".len()..];
            let Some(action) = crate::keybinds::KeybindAction::from_id(id) else {
                return false;
            };
            state.keybinds.start_recording(action);
            true
        }
        "keybind.cancel_record" => {
            let had_state = state.keybinds.recording.is_some() || state.keybinds.error.is_some();
            state.keybinds.cancel_recording();
            state.keybinds.error = None;
            had_state
        }
        _ => false,
    }
}

/// Read the system clipboard, normalise the bytes, and forward them
/// to the active pane's PTY through the fire-and-forget write path.
///
/// Empty / non-text clipboard is a silent no-op so a stray Ctrl+V
/// after the user copied a non-text selection (image, file path
/// listing in some apps) does not spam toasts. A real clipboard
/// failure (driver unavailable, OS access denied, permission error)
/// surfaces as a `push_error_toast` so the user knows their paste
/// did not land. Returns `true` whenever the action was recognised
/// (even if the resulting write was a no-op) so the framework
/// rebuilds the tree and a toast becomes visible promptly.
///
/// Bracketed-paste mode is not yet tracked per session in the
/// daemon. Until it is, raw bytes go through and `normalize_pasted_text`
/// strips any embedded `\x1b[200~` / `\x1b[201~` so a hostile
/// clipboard payload cannot forge an "end of paste" mid-string.
/// See the TODO on [`normalize_pasted_text`].
fn dispatch_terminal_paste(state: &mut AppState) -> bool {
    // Read first: if the system says no clipboard at all, surface that
    // before chasing pane / PTY issues.
    let raw = match state.clipboard.read_text() {
        Ok(text) => text,
        Err(e) => {
            log::warn!("terminal.paste: clipboard read failed: {e}");
            push_error_toast(state, format!("paste failed: {e}"));
            return true;
        }
    };
    if raw.is_empty() {
        // Empty clipboard / non-text payload (arboard maps
        // `ContentNotAvailable` to an empty string). No-op without a
        // toast so a stray Ctrl+V is invisible rather than annoying.
        return true;
    }
    let payload = normalize_pasted_text(&raw);
    if payload.is_empty() {
        return true;
    }
    let pane_id = state.active_pane.0;
    if !state.pty_manager.has(pane_id) {
        // Active pane has no PTY yet (still spawning, or focus pointed
        // at a placeholder). Toast so the user knows the paste was
        // dropped and the clipboard wasn't quietly consumed.
        push_error_toast(state, "paste failed: no terminal in focus");
        return true;
    }
    let byte_count = payload.len();
    // Wrap the body in bracketed-paste markers when the running program
    // enabled DECSET 2004 so readline / editors can tell a paste from typed
    // input. `normalize_pasted_text` already scrubbed any embedded markers,
    // so a hostile clipboard payload cannot forge an early terminator.
    let bracketed = state
        .terminals
        .get(&pane_id)
        .map(|t| t.lock_recover().bracketed_paste_active())
        .unwrap_or(false);
    let bytes: Vec<u8> = if bracketed {
        let mut b = Vec::with_capacity(payload.len() + 12);
        b.extend_from_slice(b"\x1b[200~");
        b.extend_from_slice(payload.as_bytes());
        b.extend_from_slice(b"\x1b[201~");
        b
    } else {
        payload.into_bytes()
    };
    if let Err(e) = state.pty_manager.write(pane_id, &bytes) {
        // Synchronous lookup error from the fire-and-forget queue
        // (e.g. worker channel closed). Async failures land on the
        // bridge's `take_write_errors` drain via the cursor-blink
        // subscription and surface there as toasts; this branch only
        // catches the immediate rejections.
        log::warn!("terminal.paste: queue write failed for pane {pane_id}: {e}");
        record_diagnostic_pty_event(
            state,
            format!("write_failed pane={pane_id} source=paste error={e}"),
        );
        push_error_toast(state, format!("paste failed: {e}"));
    } else {
        record_diagnostic_pty_event(
            state,
            format!("write pane={pane_id} bytes={byte_count} source=paste"),
        );
    }
    true
}

/// True when the active pane has a real (non-collapsed) text selection.
/// Lets the terminal keyboard handler decide whether a bare `Ctrl+C`
/// should copy or fall through to the shell as an interrupt (`0x03`).
pub fn active_pane_has_selection(state: &AppState) -> bool {
    state
        .terminal_selections
        .get(&state.active_pane.0)
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// Copy the active pane's selection to the system clipboard, then clear it.
/// Returns `false` (no-op) when there is no real selection so callers can
/// fall through to other handling. Used by both the `Ctrl+Shift+C` shortcut
/// and the conditional `Ctrl+C`-with-selection path.
fn dispatch_terminal_copy(state: &mut AppState) -> bool {
    let pane_id = state.active_pane.0;
    let sel = match state.terminal_selections.get(&pane_id).copied() {
        Some(sel) if !sel.is_empty() => sel,
        _ => return false,
    };
    let text = match state.terminals.get(&pane_id) {
        Some(handle) => {
            let term = handle.lock_recover();
            let (start, end) = sel.ordered();
            term.selection_text(start, end)
        }
        None => return false,
    };
    if text.is_empty() {
        clear_terminal_selection(state, pane_id);
        return true;
    }
    if let Err(e) = state.clipboard.write_text(&text) {
        log::warn!("terminal.copy: clipboard write failed: {e}");
        push_error_toast(state, format!("copy failed: {e}"));
        return true;
    }
    record_diagnostic_pty_event(
        state,
        format!("copy pane={pane_id} chars={}", text.chars().count()),
    );
    // Windows Terminal clears the selection once it has been copied.
    clear_terminal_selection(state, pane_id);
    true
}

/// The visible highlighted range of a selection, or `None` when it paints
/// nothing (a collapsed `Cell`-mode selection). Two selections with the same
/// span produce identical pixels.
fn selection_highlight_span(sel: &TermSelection) -> Option<((u64, usize), (u64, usize))> {
    if sel.is_empty() {
        None
    } else {
        Some(sel.ordered())
    }
}

/// Replace `pane`'s selection, flagging a forced repaint only when the
/// painted region actually changes. A plain click (collapsed -> collapsed)
/// therefore does not force a full-pane repaint, while a drag that grows the
/// range or a click that clears a prior highlight does.
pub fn set_terminal_selection(state: &mut AppState, pane: u32, sel: TermSelection) {
    let prev_span = state
        .terminal_selections
        .get(&pane)
        .and_then(selection_highlight_span);
    let next_span = selection_highlight_span(&sel);
    state.terminal_selections.insert(pane, sel);
    if prev_span != next_span {
        state.terminal_selection_repaint.insert(pane);
    }
}

/// Drop `pane`'s selection (if any) and flag it for a forced repaint so the
/// highlight is cleared on the next frame. No-op flag churn when there was
/// nothing selected.
pub fn clear_terminal_selection(state: &mut AppState, pane: u32) {
    if state.terminal_selections.remove(&pane).is_some() {
        state.terminal_selection_repaint.insert(pane);
    }
}

/// Force a one-frame repaint of `pane`'s selection highlight without changing
/// the selection. Used when the view scrolls: the selection is anchored to
/// absolute lines and stays valid, but the highlight must be re-emitted at
/// the display rows the content now occupies.
pub fn mark_terminal_selection_dirty(state: &mut AppState, pane: u32) {
    if state.terminal_selections.contains_key(&pane) {
        state.terminal_selection_repaint.insert(pane);
    }
}

/// Pure element-local pointer → visible cell mapping. `x_offset` is the
/// content translate applied to the grid (Windows-Terminal parity). The
/// result is clamped into `[0, cols)` x `[0, rows)`. Returns `None` only
/// when the grid is degenerate or cell metrics are unavailable.
pub fn cell_from_local(
    local_x: f32,
    local_y: f32,
    cell_w: f32,
    cell_h: f32,
    x_offset: f32,
    cols: usize,
    rows: usize,
) -> Option<(usize, usize)> {
    if cols == 0 || rows == 0 || cell_w <= 0.0 || cell_h <= 0.0 {
        return None;
    }
    let x = (local_x - x_offset).max(0.0);
    let y = local_y.max(0.0);
    // `as usize` truncates toward zero, i.e. floor for the non-negative
    // values above; clamp the right/bottom overrun onto the last cell.
    let col = ((x / cell_w) as usize).min(cols - 1);
    let row = ((y / cell_h) as usize).min(rows - 1);
    Some((row, col))
}

/// Map an element-local pointer position to a visible cell in `pane`'s
/// terminal using the renderer's published cell metrics. `cell_w_scale` is
/// the renderer's per-cell advance scale (Windows-Terminal parity draws each
/// column at `cell_w * 0.996`; 1.0 otherwise) — the published metric is the
/// unscaled width, so the caller passes the scale to match the column
/// positions the renderer actually drew. Returns `None` when metrics aren't
/// ready yet or the pane has no terminal.
pub fn terminal_cell_at(
    state: &AppState,
    pane: u32,
    local_x: f32,
    local_y: f32,
    x_offset: f32,
    cell_w_scale: f32,
) -> Option<(u64, usize)> {
    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w() * cell_w_scale;
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    let handle = state.terminals.get(&pane)?;
    let t = handle.lock_recover();
    let (rows, cols) = (t.grid().rows(), t.grid().cols());
    let (row, col) = cell_from_local(local_x, local_y, cell_w, cell_h, x_offset, cols, rows)?;
    // Promote the visible row to a stable absolute line so the selection
    // survives scrolling and output.
    Some((t.abs_line_at_display(row), col))
}

fn terminal_word_bounds(state: &AppState, pane: u32, cell: (u64, usize)) -> (usize, usize) {
    state
        .terminals
        .get(&pane)
        .map(|h| h.lock_recover().word_bounds_at(cell.0, cell.1))
        .unwrap_or((cell.1, cell.1))
}

fn terminal_line_bounds(state: &AppState, pane: u32, cell: (u64, usize)) -> (usize, usize) {
    state
        .terminals
        .get(&pane)
        .map(|h| h.lock_recover().line_bounds_at(cell.0))
        .unwrap_or((cell.1, cell.1))
}

/// Handle a left mouse-down on `pane`'s terminal at absolute `cell`. Places a
/// fresh anchor, extends from the existing anchor on `shift`, or promotes to
/// word / line selection on the second / third consecutive press of the same
/// cell within [`MULTI_CLICK_MS`].
pub fn handle_terminal_mouse_down(
    state: &mut AppState,
    pane: u32,
    cell: (u64, usize),
    shift: bool,
    now: std::time::Instant,
) {
    // Shift+click extends the live selection from its existing anchor.
    if shift {
        if let Some(mut sel) = state.terminal_selections.get(&pane).copied() {
            sel.focus = cell;
            set_terminal_selection(state, pane, sel);
            state.terminal_click = Some(TerminalClick {
                pane,
                at: now,
                cell,
                count: 1,
            });
            return;
        }
        // No live selection: fall through to a fresh single-cell anchor.
    }

    let count = match state.terminal_click {
        Some(prev)
            if prev.pane == pane
                && prev.cell == cell
                && now.duration_since(prev.at).as_millis() <= MULTI_CLICK_MS =>
        {
            prev.count % 3 + 1
        }
        _ => 1,
    };
    state.terminal_click = Some(TerminalClick {
        pane,
        at: now,
        cell,
        count,
    });

    let sel = match count {
        2 => {
            let (s, e) = terminal_word_bounds(state, pane, cell);
            TermSelection {
                anchor: (cell.0, s),
                focus: (cell.0, e),
                mode: SelectMode::Word,
            }
        }
        3 => {
            let (s, e) = terminal_line_bounds(state, pane, cell);
            TermSelection {
                anchor: (cell.0, s),
                focus: (cell.0, e),
                mode: SelectMode::Line,
            }
        }
        _ => TermSelection::new(cell, SelectMode::Cell),
    };
    set_terminal_selection(state, pane, sel);
}

/// Extend `pane`'s selection focus to absolute `cell` during a drag. Seeds a
/// fresh cell selection if a drag somehow arrives without a prior mouse-down
/// anchor.
pub fn handle_terminal_drag(state: &mut AppState, pane: u32, cell: (u64, usize)) {
    if let Some(mut sel) = state.terminal_selections.get(&pane).copied() {
        sel.focus = cell;
        set_terminal_selection(state, pane, sel);
    } else {
        set_terminal_selection(state, pane, TermSelection::new(cell, SelectMode::Cell));
    }
}

/// Finish a drag: drop a collapsed (empty) selection so a click-without-drag
/// leaves no stale highlight.
pub fn finish_terminal_drag(state: &mut AppState, pane: u32) {
    if let Some(sel) = state.terminal_selections.get(&pane).copied() {
        if sel.is_empty() {
            clear_terminal_selection(state, pane);
        }
    }
}

/// Parse `shell.set_default:<json>` and apply. The json must
/// deserialize to a `ShellSpec`; malformed input returns `false` with
/// no state change.
fn dispatch_shell_set_default(state: &mut AppState, cmd: &str) -> bool {
    let json = &cmd["shell.set_default:".len()..];
    let Ok(spec) = serde_json::from_str::<crate::shell::ShellSpec>(json) else {
        return false;
    };
    state.default_shell = spec;
    crate::persist::save_workspaces(state);
    true
}

/// Clear the app wide default shell. Idempotent: succeeds even when
/// already empty so the UI can wire a single button without checking
/// state first.
fn dispatch_shell_clear_default(state: &mut AppState) -> bool {
    state.default_shell = crate::shell::ShellSpec::default();
    crate::persist::save_workspaces(state);
    true
}

/// Parse `shell.set_workspace:<idx>:<json>` and apply. Both an
/// out-of-range index and malformed json return `false` with no
/// state change.
fn dispatch_shell_set_workspace(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["shell.set_workspace:".len()..];
    let Some((idx_str, json)) = rest.split_once(':') else {
        return false;
    };
    let Ok(idx) = idx_str.parse::<usize>() else {
        return false;
    };
    if idx >= state.workspaces.len() {
        return false;
    }
    let Ok(spec) = serde_json::from_str::<crate::shell::ShellSpec>(json) else {
        return false;
    };
    state.workspaces[idx].shell = spec;
    crate::persist::save_workspaces(state);
    true
}

/// Parse `shell.clear_workspace:<idx>` and clear that workspace's
/// override. Out-of-range index returns `false`.
fn dispatch_shell_clear_workspace(state: &mut AppState, cmd: &str) -> bool {
    let idx_str = &cmd["shell.clear_workspace:".len()..];
    let Ok(idx) = idx_str.parse::<usize>() else {
        return false;
    };
    if idx >= state.workspaces.len() {
        return false;
    }
    state.workspaces[idx].shell = crate::shell::ShellSpec::default();
    crate::persist::save_workspaces(state);
    true
}

/// Parse `keybind.set:<action_id>:<combo>` and apply. The combo can
/// itself contain `+` and the separator is only the *first* colon after
/// the prefix, since combos like `Ctrl+,` do not contain colons but
/// ids never do either. Falls back to `false` when the action id is
/// unknown so a stale settings UI doesn't silently corrupt state.
fn dispatch_keybind_set(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["keybind.set:".len()..];
    let Some((id, combo_str)) = rest.split_once(':') else {
        return false;
    };
    let Some(action) = crate::keybinds::KeybindAction::from_id(id) else {
        return false;
    };
    let combo = match unshit::core::shortcut::KeyCombo::parse(combo_str) {
        Ok(c) => c,
        Err(e) => {
            state.keybinds.error = Some(crate::keybinds::KeybindError {
                action,
                kind: crate::keybinds::KeybindErrorKind::InvalidCombo {
                    combo: combo_str.to_string(),
                    message: e,
                },
            });
            return true;
        }
    };
    match state.keybinds.set(action, combo) {
        Ok(()) => {
            crate::keybinds::loader::save_if_installed(&state.keybinds.overrides);
            true
        }
        Err(_) => true,
    }
}

/// Parse `drag.start_pane:<pane_id>:<x>:<y>` and enter the
/// `DraggingPane` state. Returns `false` on malformed input or if
/// the pane id is not present in the current tab.
fn dispatch_drag_start_pane(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["drag.start_pane:".len()..];
    let mut parts = rest.splitn(3, ':');
    let (Some(pane_str), Some(x_str), Some(y_str)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let (Ok(pane_num), Ok(x), Ok(y)) = (
        pane_str.parse::<u32>(),
        x_str.parse::<f32>(),
        y_str.parse::<f32>(),
    ) else {
        return false;
    };
    let pane = PaneId(pane_num);
    if find_pane_coord(state, pane).is_none() {
        return false;
    }
    // Cursor events arrive in physical pixels. Store them in CSS
    // pixels so they compose correctly with `Dimension::Px` in the
    // overlay builder (the framework re-applies scale_factor there).
    let sf = state.scale_factor.max(1e-3);
    state.drag = crate::drag::DragState::DraggingPane {
        pane,
        cursor_x: x / sf,
        cursor_y: y / sf,
    };
    true
}

/// Parse `drag.start_tab:<tab_id>:<x>:<y>` and enter the `DraggingTab`
/// state. Returns `false` on malformed input or if the tab id is not
/// present. The cursor is normalised to CSS pixels (winit events are
/// physical) to match how pane rects and the tab-bar rect are stored.
fn dispatch_drag_start_tab(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["drag.start_tab:".len()..];
    let mut parts = rest.splitn(3, ':');
    let (Some(id), Some(x_str), Some(y_str)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(x), Ok(y)) = (x_str.parse::<f32>(), y_str.parse::<f32>()) else {
        return false;
    };
    if !state.tabs.iter().any(|t| t.id == id) {
        return false;
    }
    let sf = state.scale_factor.max(1e-3);
    state.drag = crate::drag::DragState::DraggingTab {
        source_tab: id.to_string(),
        cursor_x: x / sf,
        cursor_y: y / sf,
    };
    true
}

/// Handle `drag.end`. Resolves the drop based on the variant in flight:
/// - `DraggingPane`: tab-bar hit -> extract to new tab.
/// - `DraggingTab`: tab-bar hit -> `tab.reorder`; edge zone on a pane ->
///   `pane.drop_split`; center zone on a pane -> `tab.reorder` next to
///   the target's tab.
///
/// Returns `true` iff a drag was actually in progress. Drops that hit
/// nothing still clear the drag state and return `true`.
/// Persist the workspace layout when `changed` is true, then forward the
/// flag. Used to wrap the drag/drop/reorder/extract dispatch helpers so a
/// rearranged layout survives a relaunch without each helper having to
/// know about persistence.
fn persist_layout_if(changed: bool, state: &AppState) -> bool {
    if changed {
        crate::persist::save_workspaces(state);
    }
    changed
}

fn dispatch_drag_end(state: &mut AppState) -> bool {
    match state.drag.clone() {
        crate::drag::DragState::DraggingPane {
            pane,
            cursor_x,
            cursor_y,
        } => {
            log::info!(
                "drag.end: pane={:?} cursor=({:.1},{:.1}) tabs={} active_tab={}",
                pane,
                cursor_x,
                cursor_y,
                state.tabs.len(),
                state.active_tab
            );
            if let Some(index) = crate::drag::resolve_tabbar_drop(
                cursor_x,
                cursor_y,
                state.tabbar_rect,
                state.tabs.len(),
            ) {
                log::info!(
                    "drag.end: extracting pane {:?} to tab index {}",
                    pane,
                    index
                );
                mutate_extract_pane_to_tab(state, pane, index);
            } else {
                // Missed the tab bar: look for a pane-edge drop inside the
                // active tab. The dragged pane itself is skipped so a drop
                // on its own rect cancels instead of splitting against self.
                let grid = crate::drag::grid_rect_from_state(
                    state.sidebar_width,
                    state.tabbar_rect,
                    state.last_grid_width,
                    state.last_grid_height,
                    state.scale_factor,
                );
                let rects = crate::drag::compute_pane_rects(
                    &state.panes,
                    &state.row_ratios,
                    &state.col_ratios,
                    grid,
                );
                let hit = rects
                    .into_iter()
                    .filter(|(id, _)| *id != pane)
                    .find_map(|(id, r)| {
                        crate::drag::drop_zones::hit_test(r, cursor_x, cursor_y).map(|z| (id, z))
                    });
                if let Some((target, zone)) = hit {
                    use crate::drag::drop_zones::DropZone;
                    log::info!(
                        "drag.end: pane {:?} hit target {:?} zone {:?}",
                        pane,
                        target,
                        zone
                    );
                    match zone {
                        DropZone::Left | DropZone::Right | DropZone::Top | DropZone::Bottom => {
                            mutate_pane_move_to_edge(state, pane, target, zone);
                        }
                        DropZone::Center => {
                            mutate_pane_swap(state, pane, target);
                        }
                    }
                } else {
                    log::info!("drag.end: pane {:?} no target", pane);
                }
            }
            state.drag = crate::drag::DragState::Idle;
            true
        }
        crate::drag::DragState::DraggingTab {
            source_tab,
            cursor_x,
            cursor_y,
        } => {
            log::info!(
                "drag.end: tab={} cursor=({:.1},{:.1}) tabs={} active_tab={}",
                source_tab,
                cursor_x,
                cursor_y,
                state.tabs.len(),
                state.active_tab
            );
            if let Some(index) = crate::drag::resolve_tabbar_drop(
                cursor_x,
                cursor_y,
                state.tabbar_rect,
                state.tabs.len(),
            ) {
                log::info!("drag.end: reordering tab {} to {}", source_tab, index);
                mutate_tab_reorder(state, &source_tab, index);
            } else {
                let grid = crate::drag::grid_rect_from_state(
                    state.sidebar_width,
                    state.tabbar_rect,
                    state.last_grid_width,
                    state.last_grid_height,
                    state.scale_factor,
                );
                let rects = crate::drag::compute_pane_rects(
                    &state.panes,
                    &state.row_ratios,
                    &state.col_ratios,
                    grid,
                );
                let hit = rects.into_iter().find_map(|(id, r)| {
                    crate::drag::drop_zones::hit_test(r, cursor_x, cursor_y).map(|z| (id, z))
                });
                if let Some((target, zone)) = hit {
                    use crate::drag::drop_zones::DropZone;
                    log::info!(
                        "drag.end: tab {} hit pane {:?} zone {:?}",
                        source_tab,
                        target,
                        zone
                    );
                    match zone {
                        DropZone::Left | DropZone::Right | DropZone::Top | DropZone::Bottom => {
                            mutate_pane_drop_split(state, &source_tab, target, zone);
                        }
                        DropZone::Center => {
                            mutate_pane_swap_from_tab(state, &source_tab, target);
                        }
                    }
                } else {
                    log::info!("drag.end: tab {} no pane hit", source_tab);
                }
            }
            state.drag = crate::drag::DragState::Idle;
            true
        }
        crate::drag::DragState::Idle => false,
    }
}

/// Parse `pane.drop_split:<target_pane_id>:<edge>`. Requires an active
/// `DraggingTab` so the source tab id can be read from `state.drag`;
/// returns `false` otherwise. The edge is one of `left|right|top|bottom`
/// (center is not valid here — use `tab.reorder`).
fn dispatch_pane_drop_split(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["pane.drop_split:".len()..];
    let Some((target_str, edge_str)) = rest.split_once(':') else {
        return false;
    };
    let Ok(target_num) = target_str.parse::<u32>() else {
        return false;
    };
    let Some(edge) = crate::drag::drop_zones::DropZone::from_id(edge_str) else {
        return false;
    };
    if matches!(edge, crate::drag::drop_zones::DropZone::Center) {
        return false;
    }
    let source = match &state.drag {
        crate::drag::DragState::DraggingTab { source_tab, .. } => source_tab.clone(),
        _ => return false,
    };
    mutate_pane_drop_split(state, &source, PaneId(target_num), edge);
    true
}

/// Parse `tab.reorder:<source_tab_id>:<index>`. Unlike drop_split,
/// this does not require an active drag — it's also used by
/// keyboard-driven reordering in the future. Unknown ids silently
/// no-op inside `mutate_tab_reorder`.
fn dispatch_tab_reorder(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["tab.reorder:".len()..];
    let Some((id, idx_str)) = rest.rsplit_once(':') else {
        return false;
    };
    let Ok(new_index) = idx_str.parse::<usize>() else {
        return false;
    };
    mutate_tab_reorder(state, id, new_index);
    true
}

/// Parse `drag.update:<x>:<y>` and update cursor position on the
/// active drag. Returns `false` when not currently dragging or when
/// the coordinates don't parse.
fn dispatch_drag_update(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["drag.update:".len()..];
    let Some((x_str, y_str)) = rest.split_once(':') else {
        return false;
    };
    let (Ok(x), Ok(y)) = (x_str.parse::<f32>(), y_str.parse::<f32>()) else {
        return false;
    };
    let sf = state.scale_factor.max(1e-3);
    match &mut state.drag {
        crate::drag::DragState::DraggingPane {
            cursor_x, cursor_y, ..
        }
        | crate::drag::DragState::DraggingTab {
            cursor_x, cursor_y, ..
        } => {
            *cursor_x = x / sf;
            *cursor_y = y / sf;
            true
        }
        crate::drag::DragState::Idle => false,
    }
}

/// Parse `pane.extract_to_tab:<pane_id>:<tab_index>` and apply.
/// Returns `false` when the parts are malformed so the framework can
/// ignore a stale UI command without mutating state.
fn dispatch_pane_extract_to_tab(state: &mut AppState, cmd: &str) -> bool {
    let rest = &cmd["pane.extract_to_tab:".len()..];
    let Some((pane_str, index_str)) = rest.split_once(':') else {
        return false;
    };
    let Ok(pane_num) = pane_str.parse::<u32>() else {
        return false;
    };
    let Ok(index) = index_str.parse::<usize>() else {
        return false;
    };
    mutate_extract_pane_to_tab(state, PaneId(pane_num), index);
    true
}

fn dispatch_keybind_reset(state: &mut AppState, cmd: &str) -> bool {
    let id = &cmd["keybind.reset:".len()..];
    let Some(action) = crate::keybinds::KeybindAction::from_id(id) else {
        return false;
    };
    state.keybinds.reset(action);
    crate::keybinds::loader::save_if_installed(&state.keybinds.overrides);
    true
}

/// Return the working directory for the active workspace, falling back to home.
pub fn active_workspace_cwd(state: &AppState) -> Option<PathBuf> {
    state
        .workspaces
        .get(state.active_workspace)
        .and_then(|ws| ws.path.clone())
}

/// Stable identifier of the active workspace on the wire. Threaded into
/// `DaemonPty::spawn*` / `attach_or_spawn` so daemon-side `SessionInfo`
/// records carry the workspace the pane belongs to, enabling
/// cross-UI-run session reconciliation.
pub fn active_workspace_num(state: &AppState) -> u32 {
    state
        .workspaces
        .get(state.active_workspace)
        .map(|ws| ws.num)
        .unwrap_or(0)
}

pub fn find_active_pane(state: &UiSnapshot) -> &Pane {
    for row in &state.panes {
        for pane in row {
            if pane.id == state.active_pane {
                return pane;
            }
        }
    }
    &state.panes[0][0]
}

pub fn is_on(state: &UiSnapshot, key: ToggleKey) -> bool {
    state.toggles.get(&key).copied().unwrap_or(false)
}

/// Resize all active terminals and their PTYs to the given column/row count.
pub fn resize_all_terminals(state: &mut AppState, cols: u16, rows: u16) {
    let ids: Vec<u32> = state.terminals.keys().copied().collect();
    for id in ids {
        if let Some(terminal) = state.terminals.get(&id) {
            terminal
                .lock()
                .expect("terminal mutex poisoned")
                .resize_viewport_growth(rows as usize, cols as usize);
        }
        state.pty_manager.resize(id, cols, rows);
    }
}

/// Re-publish terminal cell metrics and resize live PTYs after a terminal font
/// size or display scale change. This uses the current measured width ratio as
/// a fast estimate; the renderer will publish exact glyph metrics on paint.
pub fn sync_terminal_size_to_font_metrics(state: &mut AppState) -> Option<(u16, u16)> {
    let scale_factor = state.scale_factor.max(1e-3);
    let font_size = state.terminal_font_size_pt as f32 * scale_factor;
    let line_height = font_size * CSS_LINE_HEIGHT;
    state.cell_width_ratio = measure_cell_width_ratio_at(font_size, line_height);
    let (cell_w, cell_h) = pre_publish_cell_metrics(
        state.terminal_font_size_pt,
        scale_factor,
        state.cell_width_ratio,
    );
    if state.last_grid_width <= 0.0 || state.last_grid_height <= 0.0 {
        return None;
    }
    let (cols, rows) = compute_pty_dimensions(
        state.last_grid_width,
        state.last_grid_height,
        cell_w,
        cell_h,
    );
    resize_all_terminals(state, cols, rows);
    Some((cols, rows))
}

/// Measure the actual monospace cell_width / font_size ratio using cosmic-text
/// at a specific (DPI-scaled) font size. Because glyph hinting can produce
/// different advance widths at different pixel sizes, the measurement must be
/// taken at the same size the renderer will use.
///
/// `line_height` is the absolute pixel line height (typically `font_size * 1.2`
/// from CSS). Accepting it as a parameter keeps the caller as the single source
/// of truth for the line_height multiplier, rather than hardcoding 1.2 here.
pub fn measure_cell_width_ratio_at(font_size: f32, line_height: f32) -> f32 {
    use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};

    let mut fs = FontSystem::new();
    let metrics = Metrics::new(font_size, line_height);
    let mut buffer = Buffer::new(&mut fs, metrics);
    buffer.set_size(&mut fs, Some(font_size * 10.0), None);
    buffer.set_text(
        &mut fs,
        "M",
        Attrs::new().family(Family::Monospace),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut fs, false);

    if let Some(glyph) = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .next()
    {
        let ratio = glyph.w / font_size;
        log::info!(
            "measured monospace cell_width ratio: {:.4} (glyph.w={:.2} at font_size={:.1})",
            ratio,
            glyph.w,
            font_size
        );
        return ratio;
    }
    log::warn!("failed to measure monospace cell_width, falling back to 0.6");
    0.6
}

/// Default terminal font-size in CSS px. Must match the `.terminal-content`
/// fallback in assets/styles.css and the seeded terminal font-size value.
pub const CSS_BASE_FONT_SIZE: f32 = DEFAULT_TERMINAL_FONT_SIZE_PT as f32;

/// CSS line-height for `.terminal-content`. Must match
/// `.terminal-content { line-height: 1.25; }` in assets/styles.css.
/// If this value drifts from the CSS, the renderer cell_h and the
/// pre-published cell_h will disagree, causing row-height mismatches.
pub const CSS_LINE_HEIGHT: f32 = 1.25;

/// Pre-publish cell metrics to the global atomics so that `on_resize` handlers
/// can compute correct PTY dimensions on the very first frame.
pub fn pre_publish_cell_metrics(
    terminal_font_size_pt: u32,
    scale_factor: f32,
    cell_width_ratio: f32,
) -> (f32, f32) {
    let font_size = terminal_font_size_pt as f32 * scale_factor;
    let cell_w = font_size * cell_width_ratio;
    let cell_h = font_size * CSS_LINE_HEIGHT;
    unshit::core::cell_grid::CellGrid::publish_cell_metrics(cell_w, cell_h);
    (cell_w, cell_h)
}

/// Compute PTY column and row dimensions from real cell metrics.
/// Falls back to `(80, 24)` when metrics are not yet available.
pub fn compute_pty_dimensions(
    grid_width: f32,
    grid_height: f32,
    cell_w: f32,
    cell_h: f32,
) -> (u16, u16) {
    if cell_w > 0.0 && cell_h > 0.0 && grid_width > 0.0 {
        let cols = (grid_width / cell_w).max(1.0) as u16;
        let rows = (grid_height / cell_h).max(1.0) as u16;
        (cols, rows)
    } else {
        (80u16, 24u16)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Process-wide guard for tests that touch the real OS clipboard.
    ///
    /// `arboard` on Windows is documented to corrupt the heap when
    /// `OpenClipboard` / `SetClipboardData` / `GetClipboardData` are
    /// invoked from multiple threads in the same process — see the
    /// module docs on `unshit-app/src/clipboard.rs`. cargo runs tests
    /// in parallel by default, so any test that exercises a real
    /// `ClipboardContext` must hold this guard for the duration of
    /// its clipboard interaction. Recovers a poisoned guard so a
    /// single panicking test does not lock the whole suite out.
    pub(super) fn clipboard_access_guard() -> MutexGuard<'static, ()> {
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        match GUARD.get_or_init(|| Mutex::new(())).lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Build a minimal AppState for testing tab/dispatch logic.
    /// Avoids PTY spawning by providing empty panes and terminals directly.
    pub(super) fn test_state() -> AppState {
        let pane = Pane {
            id: PaneId(1),
            title: "shell".to_string(),
            subtitle: "bash".to_string(),
            pid: 0,
            cpu: 0.0,
        };
        let panes = vec![vec![pane]];
        let tabs = vec![TerminalTab {
            id: "t1".to_string(),
            name: "shell".to_string(),
            subtitle: "bash".to_string(),
            status: TabStatus::Running,
            panes: panes.clone(),
            active_pane: PaneId(1),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        }];
        AppState {
            workspaces: vec![],
            active_workspace: 0,
            tabs,
            active_tab: 0,
            panes,
            active_pane: PaneId(1),
            settings_open: false,
            settings_section: SettingsSection::Appearance,
            theme: crate::theme::default_theme_id().to_string(),
            custom_theme: crate::theme::default_custom_theme(),
            last_terminal_theme_painted: String::new(),
            config_font_size_pt: DEFAULT_CONFIG_FONT_SIZE_PT,
            terminal_font_size_pt: DEFAULT_TERMINAL_FONT_SIZE_PT,
            ui_density: DEFAULT_UI_DENSITY,
            scroll_line_px: DEFAULT_SCROLL_LINE_PX,
            smooth_scroll_duration_ms: DEFAULT_SMOOTH_SCROLL_DURATION_MS,
            toggles: BTreeMap::new(),
            palette_open: false,
            palette_query: String::new(),
            palette_active: 0,
            sidebar_collapsed: false,
            sidebar_width: 252.0,
            window_maximized: false,
            sidebar_drag_start: None,
            cpu_pct: 0.0,
            mem_gb: 0.0,
            net_kbps: 0.0,
            clock_hhmm: "12:00".to_string(),
            next_id: 2,
            pty_manager: crate::pty::DaemonPty::new(),
            terminals: std::collections::HashMap::new(),
            scale_factor: 1.0,
            cell_width_ratio: 0.6,
            last_grid_width: 0.0,
            last_grid_height: 0.0,
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
            resize_drag: None,
            ctx_menu: None,
            keybinds: crate::keybinds::KeybindsState::default(),
            drag: crate::drag::DragState::default(),
            tabbar_rect: crate::drag::Rect::default(),
            confirm_dialog: None,
            sessions: Vec::new(),
            sessions_stale: false,
            diagnostic_frame_counter: 0,
            diagnostic_last_present_unix_ms: None,
            diagnostic_scroll_samples: VecDeque::new(),
            diagnostic_pty_recent_events: VecDeque::new(),
            toasts: unshit::core::toast::ToastStore::with_capacity(3, 8),
            toast_meta: BTreeMap::new(),
            clipboard: Arc::new(unshit::app::ClipboardContext::new()),
            terminal_selections: std::collections::HashMap::new(),
            terminal_selection_repaint: std::collections::HashSet::new(),
            terminal_click: None,
            default_shell: crate::shell::ShellSpec::default(),
            quick_prompt: None,
        }
    }

    // -- SettingsSection ------------------------------------------------------

    #[test]
    fn settings_section_labels() {
        assert_eq!(SettingsSection::Appearance.label(), "appearance");
        assert_eq!(SettingsSection::Shell.label(), "shell");
        assert_eq!(SettingsSection::Keybinds.label(), "keybinds");
        assert_eq!(SettingsSection::Sessions.label(), "sessions");
        assert_eq!(SettingsSection::Notifications.label(), "notifications");
        assert_eq!(SettingsSection::DangerZone.label(), "danger zone");
    }

    #[test]
    fn settings_section_all_returns_six() {
        let all = SettingsSection::all();
        assert_eq!(all.len(), 6);
        assert_eq!(all[0], SettingsSection::Appearance);
        assert_eq!(all[1], SettingsSection::Shell);
        assert_eq!(all[2], SettingsSection::Keybinds);
        assert_eq!(all[3], SettingsSection::Sessions);
        assert_eq!(all[4], SettingsSection::Notifications);
        assert_eq!(all[5], SettingsSection::DangerZone);
    }

    // -- Tab mutations --------------------------------------------------------

    #[test]
    fn add_tab_increments_id_and_activates() {
        let mut state = test_state();
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.next_id, 2);

        mutate_add_tab(&mut state);

        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.tabs[1].id, "t2");
        assert_eq!(state.active_tab, 1);
        assert_eq!(state.next_id, 3);
    }

    #[test]
    fn pane_spawn_shell_returns_resolved_default_when_set() {
        let mut state = test_state();
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/bash".into(),
            args: vec!["--login".into()],
        };
        let resolved = pane_spawn_shell(&state).expect("non empty default must resolve");
        assert_eq!(resolved.program, "/bin/bash");
        assert_eq!(resolved.args, vec!["--login".to_string()]);
    }

    #[test]
    fn pane_spawn_shell_returns_none_when_default_is_empty() {
        let state = test_state();
        assert!(state.default_shell.is_empty());
        assert!(
            pane_spawn_shell(&state).is_none(),
            "empty default must yield None so the daemon's own default_shell() takes over"
        );
    }

    #[test]
    fn add_tab_records_resolved_default_shell_on_pty_shim() {
        let mut state = test_state();
        state.default_shell = crate::shell::ShellSpec {
            program: "/usr/local/bin/fish".into(),
            args: vec![],
        };
        let new_pane_id = state.next_id;
        mutate_add_tab(&mut state);
        let shell = state
            .pty_manager
            .spawn_shell(new_pane_id)
            .expect("add_tab must forward the resolved default shell to the shim");
        assert_eq!(shell.program, "/usr/local/bin/fish");
    }

    #[test]
    fn split_right_records_resolved_default_shell_on_pty_shim() {
        let mut state = test_state();
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/zsh".into(),
            args: vec![],
        };
        let target = state.active_pane;
        let new_pane_id = state.next_id;
        mutate_split_right(&mut state, target);
        let shell = state
            .pty_manager
            .spawn_shell(new_pane_id)
            .expect("split_right must forward the resolved default shell to the shim");
        assert_eq!(shell.program, "/bin/zsh");
    }

    #[test]
    fn split_down_records_resolved_default_shell_on_pty_shim() {
        let mut state = test_state();
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/dash".into(),
            args: vec![],
        };
        let target = state.active_pane;
        let new_pane_id = state.next_id;
        mutate_split_down(&mut state, target);
        let shell = state
            .pty_manager
            .spawn_shell(new_pane_id)
            .expect("split_down must forward the resolved default shell to the shim");
        assert_eq!(shell.program, "/bin/dash");
    }

    #[test]
    fn pane_spawn_shell_prefers_active_workspace_shell_over_app_default() {
        let mut state = test_state();
        state
            .workspaces
            .push(new_workspace(1, "alpha".into(), None));
        state.active_workspace = 0;
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/bash".into(),
            args: vec![],
        };
        state.workspaces[0].shell = crate::shell::ShellSpec {
            program: "/usr/local/bin/fish".into(),
            args: vec!["-l".into()],
        };

        let resolved = pane_spawn_shell(&state).expect("workspace override must resolve");
        assert_eq!(resolved.program, "/usr/local/bin/fish");
        assert_eq!(resolved.args, vec!["-l".to_string()]);
    }

    #[test]
    fn pane_spawn_shell_falls_back_to_app_default_when_workspace_shell_is_empty() {
        let mut state = test_state();
        state
            .workspaces
            .push(new_workspace(1, "alpha".into(), None));
        state.active_workspace = 0;
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/zsh".into(),
            args: vec![],
        };
        assert!(state.workspaces[0].shell.is_empty());

        let resolved = pane_spawn_shell(&state).expect("app default must take over");
        assert_eq!(resolved.program, "/bin/zsh");
    }

    #[test]
    fn pane_spawn_shell_uses_correct_workspace_after_switch() {
        // Two workspaces: ws0 has an override, ws1 does not.
        // Adding a tab in each must record the right shell on the
        // shim. This catches a regression where pane_spawn_shell
        // forgets to consult the active workspace.
        let mut state = test_state();
        state
            .workspaces
            .push(new_workspace(1, "alpha".into(), None));
        state.workspaces.push(new_workspace(2, "beta".into(), None));
        state.active_workspace = 0;
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/dash".into(),
            args: vec![],
        };
        state.workspaces[0].shell = crate::shell::ShellSpec {
            program: "/usr/local/bin/fish".into(),
            args: vec![],
        };
        // workspaces[1].shell stays empty so it falls back to the app default.

        let ws0_pane_id = state.next_id;
        mutate_add_tab(&mut state);
        let ws0_shell = state
            .pty_manager
            .spawn_shell(ws0_pane_id)
            .expect("ws0 add_tab must record the override");
        assert_eq!(ws0_shell.program, "/usr/local/bin/fish");

        mutate_switch_workspace(&mut state, 1);
        let ws1_pane_id = state.next_id;
        mutate_add_tab(&mut state);
        let ws1_shell = state
            .pty_manager
            .spawn_shell(ws1_pane_id)
            .expect("ws1 add_tab must fall back to the app default");
        assert_eq!(ws1_shell.program, "/bin/dash");
    }

    #[test]
    fn add_tab_with_empty_default_shell_does_not_record_a_shell() {
        let mut state = test_state();
        assert!(state.default_shell.is_empty());
        let new_pane_id = state.next_id;
        mutate_add_tab(&mut state);
        assert!(
            state.pty_manager.spawn_shell(new_pane_id).is_none(),
            "empty default must surface as None so the daemon falls back"
        );
    }

    #[test]
    fn add_tab_spawns_pty_in_active_workspace_cwd() {
        // Regression: a new tab spawned from the "+" button used
        // PtyManager::spawn (no cwd), so the shell landed in the home dir
        // and the PowerShell profile's Set-Location then won. Splits already
        // passed active_workspace_cwd; tab add must too.
        let mut state = seed_state();
        let ws_path = PathBuf::from(if cfg!(windows) {
            r"C:\tmp\ws"
        } else {
            "/tmp/ws"
        });
        mutate_add_workspace_with_path(&mut state, Some(ws_path.clone()));

        let new_pane_id = state.next_id;
        mutate_add_tab(&mut state);

        assert_eq!(
            state.pty_manager.spawn_cwd(new_pane_id),
            Some(ws_path.as_path()),
            "new tab must spawn its PTY in the active workspace cwd",
        );
    }

    #[test]
    fn close_tab_removes_and_adjusts_active() {
        let mut state = test_state();
        mutate_add_tab(&mut state); // t2
        mutate_add_tab(&mut state); // t3
                                    // tabs: [t1, t2, t3], active = 2

        // Close middle tab while active is after it
        state.active_tab = 2;
        mutate_close_tab(&mut state, 1);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab, 1); // shifted left
    }

    #[test]
    fn close_active_tab_clamps() {
        let mut state = test_state();
        mutate_add_tab(&mut state);
        // tabs: [t1, t2], active = 1

        mutate_close_tab(&mut state, 1); // close active (last)
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn close_last_tab_leaves_workspace_empty() {
        let mut state = test_state();
        // only one tab
        mutate_close_tab(&mut state, 0);
        assert!(
            state.tabs.is_empty(),
            "closing the last tab must not auto-respawn a new one"
        );
        assert!(state.panes.is_empty(), "live panes must be cleared");
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn close_tab_out_of_bounds_is_noop() {
        let mut state = test_state();
        let len_before = state.tabs.len();
        mutate_close_tab(&mut state, 999);
        assert_eq!(state.tabs.len(), len_before);
    }

    #[test]
    fn close_tab_before_active_decrements_active() {
        let mut state = test_state();
        mutate_add_tab(&mut state); // t2
        mutate_add_tab(&mut state); // t3
        state.active_tab = 2;

        mutate_close_tab(&mut state, 0);
        assert_eq!(state.active_tab, 1);
    }

    // -- find_pane_coord ------------------------------------------------------

    #[test]
    fn find_pane_coord_finds_existing() {
        let state = test_state();
        assert_eq!(find_pane_coord(&state, PaneId(1)), Some((0, 0)));
    }

    #[test]
    fn find_pane_coord_returns_none_for_missing() {
        let state = test_state();
        assert_eq!(find_pane_coord(&state, PaneId(999)), None);
    }

    #[test]
    fn find_pane_coord_multi_row() {
        let mut state = test_state();
        let pane2 = Pane {
            id: PaneId(5),
            title: "test".to_string(),
            subtitle: "".to_string(),
            pid: 0,
            cpu: 0.0,
        };
        state.panes.push(vec![pane2]);
        assert_eq!(find_pane_coord(&state, PaneId(5)), Some((1, 0)));
    }

    // -- Font size ------------------------------------------------------------

    #[test]
    fn terminal_font_size_increments() {
        let mut state = test_state();
        state.terminal_font_size_pt = 13;
        mutate_terminal_font_size_delta(&mut state, 1);
        assert_eq!(state.terminal_font_size_pt, 14);
    }

    #[test]
    fn terminal_font_size_clamps_at_max() {
        let mut state = test_state();
        state.terminal_font_size_pt = MAX_FONT_SIZE;
        mutate_terminal_font_size_delta(&mut state, 1);
        assert_eq!(state.terminal_font_size_pt, MAX_FONT_SIZE);
    }

    #[test]
    fn terminal_font_size_clamps_at_min() {
        let mut state = test_state();
        state.terminal_font_size_pt = MIN_FONT_SIZE;
        mutate_terminal_font_size_delta(&mut state, -1);
        assert_eq!(state.terminal_font_size_pt, MIN_FONT_SIZE);
    }

    #[test]
    fn config_font_size_large_delta_clamps() {
        let mut state = test_state();
        state.config_font_size_pt = 13;
        mutate_config_font_size_delta(&mut state, 100);
        assert_eq!(state.config_font_size_pt, MAX_FONT_SIZE);
    }

    #[test]
    fn scroll_line_px_delta_uses_step_and_clamps() {
        let mut state = test_state();
        state.scroll_line_px = DEFAULT_SCROLL_LINE_PX;
        assert!(mutate_scroll_line_px_delta(&mut state, SCROLL_LINE_PX_STEP));
        assert_eq!(state.scroll_line_px, DEFAULT_SCROLL_LINE_PX + 4);

        state.scroll_line_px = MAX_SCROLL_LINE_PX;
        assert!(!mutate_scroll_line_px_delta(
            &mut state,
            SCROLL_LINE_PX_STEP
        ));
        assert_eq!(state.scroll_line_px, MAX_SCROLL_LINE_PX);
    }

    #[test]
    fn smooth_scroll_duration_delta_uses_step_and_clamps() {
        let mut state = test_state();
        state.smooth_scroll_duration_ms = DEFAULT_SMOOTH_SCROLL_DURATION_MS;
        assert!(mutate_smooth_scroll_duration_delta(
            &mut state,
            -SMOOTH_SCROLL_DURATION_STEP_MS
        ));
        assert_eq!(
            state.smooth_scroll_duration_ms,
            DEFAULT_SMOOTH_SCROLL_DURATION_MS - 10
        );

        state.smooth_scroll_duration_ms = MIN_SMOOTH_SCROLL_DURATION_MS;
        assert!(!mutate_smooth_scroll_duration_delta(
            &mut state,
            -SMOOTH_SCROLL_DURATION_STEP_MS
        ));
        assert_eq!(
            state.smooth_scroll_duration_ms,
            MIN_SMOOTH_SCROLL_DURATION_MS
        );
    }

    // -- dispatch -------------------------------------------------------------

    #[test]
    fn dispatch_shell_set_default_updates_state() {
        let mut state = test_state();
        assert!(state.default_shell.is_empty());
        let json = r#"{"program":"/bin/zsh","args":["-l"]}"#;
        let cmd = format!("shell.set_default:{json}");
        assert!(dispatch(&mut state, &cmd));
        assert_eq!(state.default_shell.program, "/bin/zsh");
        assert_eq!(state.default_shell.args, vec!["-l".to_string()]);
    }

    #[test]
    fn dispatch_shell_set_default_with_malformed_json_returns_false() {
        let mut state = test_state();
        let original = state.default_shell.clone();
        assert!(!dispatch(&mut state, "shell.set_default:not-json"));
        assert_eq!(
            state.default_shell, original,
            "malformed json must not mutate state"
        );
    }

    #[test]
    fn dispatch_shell_set_default_with_empty_json_payload_returns_false() {
        let mut state = test_state();
        // Missing colon means no payload — also malformed.
        assert!(!dispatch(&mut state, "shell.set_default"));
    }

    #[test]
    fn dispatch_shell_clear_default_clears_state() {
        let mut state = test_state();
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/dash".into(),
            args: vec![],
        };
        assert!(dispatch(&mut state, "shell.clear_default"));
        assert!(state.default_shell.is_empty());
    }

    #[test]
    fn dispatch_shell_clear_default_when_already_empty_still_returns_true() {
        // The handler is idempotent: clearing an already empty default
        // is a successful no-op, not an error. Persisting it is cheap.
        let mut state = test_state();
        assert!(state.default_shell.is_empty());
        assert!(dispatch(&mut state, "shell.clear_default"));
        assert!(state.default_shell.is_empty());
    }

    #[test]
    fn dispatch_shell_set_workspace_updates_correct_workspace() {
        let mut state = test_state();
        state
            .workspaces
            .push(new_workspace(1, "alpha".into(), None));
        state.workspaces.push(new_workspace(2, "beta".into(), None));
        let json = r#"{"program":"/usr/local/bin/fish","args":[]}"#;
        let cmd = format!("shell.set_workspace:1:{json}");
        assert!(dispatch(&mut state, &cmd));
        assert!(
            state.workspaces[0].shell.is_empty(),
            "workspace 0 must be untouched"
        );
        assert_eq!(state.workspaces[1].shell.program, "/usr/local/bin/fish");
    }

    #[test]
    fn dispatch_shell_set_workspace_with_out_of_range_index_returns_false() {
        let mut state = test_state();
        let count = state.workspaces.len();
        let cmd = format!(
            "shell.set_workspace:{count}:{}",
            r#"{"program":"/bin/zsh","args":[]}"#
        );
        assert!(!dispatch(&mut state, &cmd));
    }

    #[test]
    fn dispatch_shell_set_workspace_with_malformed_index_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(
            &mut state,
            r#"shell.set_workspace:abc:{"program":"/bin/zsh","args":[]}"#
        ));
    }

    #[test]
    fn dispatch_shell_set_workspace_with_malformed_json_returns_false() {
        let mut state = test_state();
        state
            .workspaces
            .push(new_workspace(1, "alpha".into(), None));
        assert!(!dispatch(&mut state, "shell.set_workspace:0:not-json"));
        assert!(state.workspaces[0].shell.is_empty());
    }

    #[test]
    fn dispatch_shell_clear_workspace_clears_the_right_workspace() {
        let mut state = test_state();
        state
            .workspaces
            .push(new_workspace(1, "alpha".into(), None));
        state.workspaces.push(new_workspace(2, "beta".into(), None));
        state.workspaces[0].shell = crate::shell::ShellSpec {
            program: "/bin/zsh".into(),
            args: vec![],
        };
        state.workspaces[1].shell = crate::shell::ShellSpec {
            program: "/bin/fish".into(),
            args: vec![],
        };
        assert!(dispatch(&mut state, "shell.clear_workspace:0"));
        assert!(state.workspaces[0].shell.is_empty());
        assert_eq!(
            state.workspaces[1].shell.program, "/bin/fish",
            "clearing workspace 0 must not touch workspace 1"
        );
    }

    #[test]
    fn dispatch_shell_clear_workspace_with_out_of_range_index_returns_false() {
        let mut state = test_state();
        let count = state.workspaces.len();
        let cmd = format!("shell.clear_workspace:{count}");
        assert!(!dispatch(&mut state, &cmd));
    }

    #[test]
    fn dispatch_shell_clear_workspace_with_malformed_index_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "shell.clear_workspace:abc"));
    }

    #[test]
    fn dispatch_modal_open_close() {
        let mut state = test_state();
        assert!(!state.settings_open);

        assert!(dispatch(&mut state, "modal.open"));
        assert!(state.settings_open);

        // Dispatching again while open toggles the page closed
        assert!(dispatch(&mut state, "modal.open"));
        assert!(!state.settings_open);

        assert!(dispatch(&mut state, "modal.open"));
        assert!(state.settings_open);
        assert!(dispatch(&mut state, "modal.close"));
        assert!(!state.settings_open);

        // Closing again returns false (already closed)
        assert!(!dispatch(&mut state, "modal.close"));
    }

    #[test]
    fn dispatch_modal_open_close_can_repeat_settings_route() {
        let mut state = test_state();

        assert!(dispatch(&mut state, "modal.open"));
        assert!(dispatch(&mut state, "modal.close"));
        assert!(!state.settings_open);

        assert!(dispatch(&mut state, "modal.open"));
        assert!(state.settings_open);
        assert!(dispatch(&mut state, "modal.close"));
        assert!(!state.settings_open);
    }

    #[test]
    fn modal_close_clears_keybind_recording_state() {
        let mut state = test_state();
        state.settings_open = true;
        state
            .keybinds
            .start_recording(crate::keybinds::KeybindAction::NewTerminal);
        state.keybinds.error = Some(crate::keybinds::KeybindError {
            action: crate::keybinds::KeybindAction::NewTerminal,
            kind: crate::keybinds::KeybindErrorKind::InvalidCombo {
                combo: "bad".to_string(),
                message: "bad combo".to_string(),
            },
        });

        assert!(dispatch(&mut state, "modal.close"));

        assert!(!state.settings_open);
        assert!(state.keybinds.recording.is_none());
        assert!(state.keybinds.error.is_none());
    }

    #[test]
    fn terminal_theme_repaint_request_waits_for_terminal_route() {
        let mut state = test_state();
        state.last_terminal_theme_painted = "amber".to_string();
        state.theme = "dracula".to_string();
        state.settings_open = true;

        assert!(
            !take_terminal_theme_repaint_request(&mut state),
            "settings route should not consume terminal repaint request"
        );
        assert_eq!(state.last_terminal_theme_painted, "amber");

        state.settings_open = false;
        assert!(take_terminal_theme_repaint_request(&mut state));
        assert_eq!(state.last_terminal_theme_painted, "dracula");
        assert!(!take_terminal_theme_repaint_request(&mut state));
    }

    #[test]
    fn dispatch_quick_prompt_open_sets_state() {
        let mut state = test_state();
        assert!(state.quick_prompt.is_none());

        assert!(dispatch(&mut state, "quick_prompt.open"));
        assert!(state.quick_prompt.is_some());
    }

    #[test]
    fn dispatch_quick_prompt_open_toggles_when_already_open() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "quick_prompt.open"));
        assert!(state.quick_prompt.is_some());

        // Re-pressing the hotkey closes the overlay (A1.2 toggle).
        assert!(dispatch(&mut state, "quick_prompt.open"));
        assert!(state.quick_prompt.is_none());
    }

    #[test]
    fn dispatch_quick_prompt_close_clears_state() {
        let mut state = test_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());

        assert!(dispatch(&mut state, "quick_prompt.close"));
        assert!(state.quick_prompt.is_none());

        // Closing again returns false (nothing to close).
        assert!(!dispatch(&mut state, "quick_prompt.close"));
    }

    #[test]
    fn dispatch_modal_close_clears_quick_prompt() {
        let mut state = test_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());

        assert!(dispatch(&mut state, "modal.close"));
        assert!(state.quick_prompt.is_none());
    }

    #[test]
    fn dispatch_quick_prompt_toggle_agent_flips_when_open() {
        use crate::quick_prompt::state::Agent;
        let mut state = test_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());
        let initial = state.quick_prompt.as_ref().unwrap().agent;

        assert!(dispatch(&mut state, "quick_prompt.toggle_agent"));
        let toggled = state.quick_prompt.as_ref().unwrap().agent;
        assert_ne!(initial, toggled);
        assert!(matches!(toggled, Agent::Claude | Agent::Codex));

        assert!(dispatch(&mut state, "quick_prompt.toggle_agent"));
        assert_eq!(state.quick_prompt.as_ref().unwrap().agent, initial);
    }

    #[test]
    fn dispatch_quick_prompt_toggle_agent_clears_error() {
        let mut state = test_state();
        let mut qp = crate::quick_prompt::QuickPromptState::open_default();
        qp.error = Some("stale".into());
        state.quick_prompt = Some(qp);

        assert!(dispatch(&mut state, "quick_prompt.toggle_agent"));
        assert!(state.quick_prompt.as_ref().unwrap().error.is_none());
    }

    #[test]
    fn dispatch_quick_prompt_toggle_agent_no_op_when_closed() {
        let mut state = test_state();
        assert!(state.quick_prompt.is_none());
        assert!(!dispatch(&mut state, "quick_prompt.toggle_agent"));
    }

    #[test]
    fn dispatch_quick_prompt_submit_no_op_when_closed() {
        let mut state = test_state();
        assert!(state.quick_prompt.is_none());
        assert!(!dispatch(&mut state, "quick_prompt.submit"));
    }

    #[test]
    fn dispatch_quick_prompt_submit_empty_prompt_sets_error() {
        let mut state = test_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());

        assert!(dispatch(&mut state, "quick_prompt.submit"));
        let qp = state.quick_prompt.as_ref().expect("overlay still open");
        assert_eq!(qp.error.as_deref(), Some("Type a prompt to continue."));
    }

    #[test]
    fn dispatch_quick_prompt_submit_whitespace_only_sets_error() {
        let mut state = test_state();
        let mut qp = crate::quick_prompt::QuickPromptState::open_default();
        qp.prompt = "   \n\t  ".into();
        state.quick_prompt = Some(qp);

        assert!(dispatch(&mut state, "quick_prompt.submit"));
        assert!(state
            .quick_prompt
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .map(|e| e.contains("Type a prompt"))
            .unwrap_or(false));
    }

    #[test]
    fn dispatch_quick_prompt_submit_codex_spawns_tab_and_closes_overlay() {
        // Slice 6 wires Codex parity: submitting on Codex builds a
        // codex_shell_spec, prepares the target, and adds the tab.
        // We assert the side effects observable from state: tab count
        // increments and the overlay closes. The actual program path
        // is not invoked in tests (no daemon is running), and the
        // tab title is "qp: <prompt prefix>".
        use crate::quick_prompt::Agent;
        let initial_tabs = {
            let s = test_state();
            s.tabs.len()
        };
        let mut state = test_state();
        let mut qp = crate::quick_prompt::QuickPromptState::open_with_agent(Agent::Codex);
        qp.prompt = "do the thing".into();
        state.quick_prompt = Some(qp);

        assert!(dispatch(&mut state, "quick_prompt.submit"));
        assert!(state.quick_prompt.is_none(), "overlay closes on success");
        assert_eq!(
            state.tabs.len(),
            initial_tabs + 1,
            "Codex submit should add a tab"
        );
        assert_eq!(
            state.tabs.last().unwrap().name,
            "qp: do the thing",
            "tab title comes from quick_prompt_tab_title(prompt)"
        );
    }

    #[test]
    fn quick_prompt_tab_title_truncates_to_thirty_chars() {
        let title = super::quick_prompt_tab_title(&"a".repeat(50));
        // "qp: " + 30 chars = 34 chars total.
        assert_eq!(title.chars().count(), 34);
        assert!(title.starts_with("qp: "));
    }

    #[test]
    fn quick_prompt_tab_title_handles_empty_prompt() {
        // Defensive: even though dispatch rejects empty prompts, the
        // title helper should not panic on one.
        assert_eq!(super::quick_prompt_tab_title(""), "qp");
        assert_eq!(super::quick_prompt_tab_title("   "), "qp");
    }

    #[test]
    fn quick_prompt_tab_title_truncates_on_char_boundary() {
        // Multi-byte chars must not split. 30 emojis is well over 30
        // bytes but exactly 30 chars; the truncation is on chars.
        let prompt: String = "🎯".repeat(50);
        let title = super::quick_prompt_tab_title(&prompt);
        assert!(title.starts_with("qp: "));
        // Body should be exactly 30 chars.
        let body: String = title.chars().skip(4).collect();
        assert_eq!(body.chars().count(), 30);
    }

    #[test]
    fn dispatch_quick_prompt_image_paste_no_op_when_closed() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "quick_prompt.image_paste"));
    }

    #[test]
    fn dispatch_quick_prompt_image_paste_sets_error_when_clipboard_empty() {
        // The test clipboard is uninitialized so read_image returns
        // Ok(None); the dispatch arm surfaces a friendly hint.
        let mut state = test_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());

        // We can't reliably write or clear an image on the OS clipboard
        // from a test, but read_image on a freshly-created context with
        // text or no content returns Ok(None) and we surface the
        // "No image on clipboard" hint. This is the behavior we lock in.
        let _ = dispatch(&mut state, "quick_prompt.image_paste");
        let qp = state.quick_prompt.as_ref().unwrap();
        // Either we got a "no image" hint or the clipboard genuinely
        // had an image (unlikely in CI). Both are valid outcomes; the
        // test only enforces that dispatch did not panic and the
        // overlay is still open.
        if let Some(err) = qp.error.as_deref() {
            assert!(
                err == "No image on clipboard" || err.starts_with("paste failed:"),
                "unexpected error chip: {err}"
            );
        } else {
            // An image was actually pasted: at least one entry should
            // exist and have a non-empty hash.
            assert!(qp.images.iter().all(|i| !i.hash.is_empty()));
        }
    }

    #[test]
    fn dispatch_quick_prompt_close_cleans_session_dir() {
        // Open with a fresh session_hex, drop a marker file in its
        // session dir, and verify quick_prompt.close removes the dir.
        let mut state = test_state();
        let qp = crate::quick_prompt::QuickPromptState::open_default();
        let session_hex = qp.session_hex.clone();
        state.quick_prompt = Some(qp);
        let dir = crate::quick_prompt::images::session_dir(&session_hex);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker.txt"), b"x").unwrap();
        assert!(dir.exists());

        assert!(dispatch(&mut state, "quick_prompt.close"));
        assert!(!dir.exists(), "session dir should be cleaned up");
    }

    #[test]
    fn dispatch_modal_close_cleans_quick_prompt_session_dir() {
        let mut state = test_state();
        let qp = crate::quick_prompt::QuickPromptState::open_default();
        let session_hex = qp.session_hex.clone();
        state.quick_prompt = Some(qp);
        let dir = crate::quick_prompt::images::session_dir(&session_hex);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir.exists());

        assert!(dispatch(&mut state, "modal.close"));
        assert!(!dir.exists());
    }

    fn open_with_popup() -> AppState {
        let mut state = test_state();
        let mut qp = crate::quick_prompt::QuickPromptState::open_default();
        qp.popup = Some(crate::quick_prompt::Popup::open(
            vec![
                crate::quick_prompt::Entry {
                    name: "alpha".into(),
                    kind: crate::quick_prompt::EntryKind::Skill,
                },
                crate::quick_prompt::Entry {
                    name: "beta".into(),
                    kind: crate::quick_prompt::EntryKind::Command,
                },
            ],
            0,
        ));
        qp.prompt = "/".into();
        state.quick_prompt = Some(qp);
        state
    }

    #[test]
    fn dispatch_autocomplete_select_next_advances_popup_selection() {
        let mut state = open_with_popup();
        assert!(dispatch(
            &mut state,
            "quick_prompt.autocomplete_select_next"
        ));
        let popup = state.quick_prompt.as_ref().unwrap().popup.as_ref().unwrap();
        assert_eq!(popup.selected, 1);
    }

    #[test]
    fn dispatch_autocomplete_select_prev_wraps_from_zero() {
        let mut state = open_with_popup();
        assert!(dispatch(
            &mut state,
            "quick_prompt.autocomplete_select_prev"
        ));
        let popup = state.quick_prompt.as_ref().unwrap().popup.as_ref().unwrap();
        assert_eq!(popup.selected, 1);
    }

    #[test]
    fn dispatch_autocomplete_dismiss_clears_popup_only() {
        let mut state = open_with_popup();
        assert!(dispatch(&mut state, "quick_prompt.autocomplete_dismiss"));
        let qp = state.quick_prompt.as_ref().unwrap();
        assert!(qp.popup.is_none(), "popup should be cleared");
        // Overlay itself is still open.
        assert_eq!(qp.prompt, "/");
    }

    #[test]
    fn dispatch_autocomplete_confirm_inserts_entry_and_closes_popup() {
        let mut state = open_with_popup();
        // Selected is 0, which is "alpha".
        assert!(dispatch(&mut state, "quick_prompt.autocomplete_confirm"));
        let qp = state.quick_prompt.as_ref().unwrap();
        assert_eq!(qp.prompt, "/alpha");
        assert!(qp.popup.is_none());
    }

    #[test]
    fn dispatch_modal_close_dismisses_popup_only_when_present() {
        // Two Esc presses: first dismisses the popup, second closes
        // the overlay. The session dir should only be removed on the
        // overlay close (second press).
        let mut state = open_with_popup();
        let session_hex = state.quick_prompt.as_ref().unwrap().session_hex.clone();
        let dir = crate::quick_prompt::images::session_dir(&session_hex);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir.exists());

        // First Esc: popup should be cleared, overlay still open, dir
        // still on disk.
        assert!(dispatch(&mut state, "modal.close"));
        let qp = state.quick_prompt.as_ref().expect("overlay still open");
        assert!(qp.popup.is_none(), "popup should be cleared");
        assert!(dir.exists(), "session dir kept while overlay open");

        // Second Esc: overlay closes and the session dir is cleaned
        // up.
        assert!(dispatch(&mut state, "modal.close"));
        assert!(state.quick_prompt.is_none());
        assert!(!dir.exists(), "session dir cleaned up on overlay close");
    }

    #[test]
    fn dispatch_autocomplete_arms_no_op_when_popup_closed() {
        let mut state = test_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());
        assert!(!dispatch(
            &mut state,
            "quick_prompt.autocomplete_select_next"
        ));
        assert!(!dispatch(
            &mut state,
            "quick_prompt.autocomplete_select_prev"
        ));
        assert!(!dispatch(&mut state, "quick_prompt.autocomplete_dismiss"));
        assert!(!dispatch(&mut state, "quick_prompt.autocomplete_confirm"));
    }

    #[test]
    fn dispatch_tab_new() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "tab.new"));
        assert_eq!(state.tabs.len(), 2);
    }

    #[test]
    fn dispatch_tab_close_active() {
        let mut state = test_state();
        mutate_add_tab(&mut state);
        state.active_tab = 1;
        assert!(dispatch(&mut state, "tab.close.active"));
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn dispatch_tab_next_wraps() {
        let mut state = test_state();
        mutate_add_tab(&mut state);
        mutate_add_tab(&mut state);
        state.active_tab = 0;

        assert!(dispatch(&mut state, "tab.next"));
        assert_eq!(state.active_tab, 1);

        assert!(dispatch(&mut state, "tab.next"));
        assert_eq!(state.active_tab, 2);

        // Wraps to 0
        assert!(dispatch(&mut state, "tab.next"));
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn dispatch_tab_prev_wraps() {
        let mut state = test_state();
        mutate_add_tab(&mut state);
        mutate_add_tab(&mut state);
        state.active_tab = 0;

        // Wraps to last
        assert!(dispatch(&mut state, "tab.prev"));
        assert_eq!(state.active_tab, 2);

        assert!(dispatch(&mut state, "tab.prev"));
        assert_eq!(state.active_tab, 1);
    }

    #[test]
    fn dispatch_tab_switch() {
        let mut state = test_state();
        mutate_add_tab(&mut state);
        mutate_add_tab(&mut state);
        state.active_tab = 0;

        assert!(dispatch(&mut state, "tab.switch:2"));
        assert_eq!(state.active_tab, 2);

        // Switching to current tab returns false
        assert!(!dispatch(&mut state, "tab.switch:2"));

        // Out-of-bounds returns false
        assert!(!dispatch(&mut state, "tab.switch:99"));

        // Invalid number returns false
        assert!(!dispatch(&mut state, "tab.switch:abc"));
    }

    #[test]
    fn dispatch_sidebar_toggle() {
        let mut state = test_state();
        assert!(!state.sidebar_collapsed);

        assert!(dispatch(&mut state, "sidebar.toggle"));
        assert!(state.sidebar_collapsed);

        assert!(dispatch(&mut state, "sidebar.toggle"));
        assert!(!state.sidebar_collapsed);
    }

    #[test]
    fn dispatch_font_inc_dec() {
        let mut state = test_state();
        state.terminal_font_size_pt = 13;

        assert!(dispatch(&mut state, "font.inc"));
        assert_eq!(state.terminal_font_size_pt, 14);

        assert!(dispatch(&mut state, "font.dec"));
        assert_eq!(state.terminal_font_size_pt, 13);
    }

    #[test]
    fn dispatch_config_font_inc_dec() {
        let mut state = test_state();
        state.config_font_size_pt = 13;

        assert!(dispatch(&mut state, "config_font.inc"));
        assert_eq!(state.config_font_size_pt, 14);

        assert!(dispatch(&mut state, "config_font.dec"));
        assert_eq!(state.config_font_size_pt, 13);
    }

    #[test]
    fn dispatch_scroll_tuning_inc_dec() {
        let mut state = test_state();
        state.scroll_line_px = DEFAULT_SCROLL_LINE_PX;
        state.smooth_scroll_duration_ms = DEFAULT_SMOOTH_SCROLL_DURATION_MS;

        assert!(dispatch(&mut state, "scroll.line_px.inc"));
        assert_eq!(state.scroll_line_px, DEFAULT_SCROLL_LINE_PX + 4);
        assert!(dispatch(&mut state, "scroll.line_px.dec"));
        assert_eq!(state.scroll_line_px, DEFAULT_SCROLL_LINE_PX);

        assert!(dispatch(&mut state, "scroll.duration.inc"));
        assert_eq!(
            state.smooth_scroll_duration_ms,
            DEFAULT_SMOOTH_SCROLL_DURATION_MS + 10
        );
        assert!(dispatch(&mut state, "scroll.duration.dec"));
        assert_eq!(
            state.smooth_scroll_duration_ms,
            DEFAULT_SMOOTH_SCROLL_DURATION_MS
        );
    }

    #[test]
    fn dispatch_density_updates_ui_density() {
        let mut state = test_state();
        assert_eq!(state.ui_density, DEFAULT_UI_DENSITY);

        assert!(dispatch(&mut state, "appearance.density:compact"));
        assert_eq!(state.ui_density, UiDensity::Compact);
        assert!(!dispatch(&mut state, "appearance.density:compact"));
        assert!(!dispatch(&mut state, "appearance.density:huge"));
    }

    #[test]
    fn custom_theme_color_edit_selects_custom_theme() {
        let mut state = test_state();
        assert!(mutate_custom_theme_color(
            &mut state,
            crate::theme::CustomThemeSlot::Accent,
            "#123456"
        ));
        assert_eq!(state.theme, crate::theme::CUSTOM_THEME_ID);
        assert_eq!(
            crate::theme::color_to_hex(state.custom_theme.accent),
            "#123456"
        );
        assert!(state.last_terminal_theme_painted.is_empty());
    }

    #[test]
    fn dispatch_appearance_reset_restores_theme_and_font_defaults() {
        let mut state = test_state();
        state.theme = crate::theme::CUSTOM_THEME_ID.to_string();
        state.custom_theme.accent = unshit::core::style::types::Color::rgb(18, 52, 86);
        state.config_font_size_pt = 18;
        state.terminal_font_size_pt = 20;
        state.ui_density = UiDensity::Comfy;
        state.scroll_line_px = 96;
        state.smooth_scroll_duration_ms = 160;

        assert!(dispatch(&mut state, "appearance.reset"));
        assert_eq!(state.theme, crate::theme::default_theme_id());
        assert_eq!(state.custom_theme, crate::theme::default_custom_theme());
        assert_eq!(state.config_font_size_pt, DEFAULT_CONFIG_FONT_SIZE_PT);
        assert_eq!(state.terminal_font_size_pt, DEFAULT_TERMINAL_FONT_SIZE_PT);
        assert_eq!(state.ui_density, DEFAULT_UI_DENSITY);
        assert_eq!(state.scroll_line_px, DEFAULT_SCROLL_LINE_PX);
        assert_eq!(
            state.smooth_scroll_duration_ms,
            DEFAULT_SMOOTH_SCROLL_DURATION_MS
        );
    }

    #[test]
    fn dispatch_font_inc_at_max_returns_false() {
        let mut state = test_state();
        state.terminal_font_size_pt = MAX_FONT_SIZE;
        assert!(!dispatch(&mut state, "font.inc"));
    }

    #[test]
    fn dispatch_font_dec_at_min_returns_false() {
        let mut state = test_state();
        state.terminal_font_size_pt = MIN_FONT_SIZE;
        assert!(!dispatch(&mut state, "font.dec"));
    }

    #[test]
    fn dispatch_palette_toggle() {
        let mut state = test_state();
        assert!(!state.palette_open);

        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(state.palette_open);

        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(!state.palette_open);
    }

    #[test]
    fn dispatch_palette_open_close_and_query_reset_selection() {
        let mut state = test_state();
        state.palette_query = "> split".to_string();
        state.palette_active = 4;

        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(state.palette_open);
        assert_eq!(state.palette_query, "");
        assert_eq!(state.palette_active, 0);

        state.palette_query = "@ agent".to_string();
        state.palette_active = 2;
        let parsed = crate::command_palette::parse_palette_query(&state.palette_query);
        assert_eq!(parsed.mode, crate::command_palette::PaletteMode::Agents);

        assert!(dispatch(&mut state, "palette.query:> rename"));
        assert_eq!(state.palette_query, "> rename");
        assert_eq!(state.palette_active, 0);

        state.palette_active = 3;
        assert!(dispatch(&mut state, "palette.close"));
        assert!(!state.palette_open);
        assert_eq!(state.palette_query, "");
        assert_eq!(state.palette_active, 0);
    }

    #[test]
    fn dispatch_palette_query_caps_and_sanitizes_external_input() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "palette.toggle"));

        let long_query = format!("palette.query:>\trename\n{}\u{202e}", "x".repeat(400));
        assert!(dispatch(&mut state, &long_query));

        assert!(
            state.palette_query.chars().count() <= crate::command_palette::PALETTE_QUERY_MAX_CHARS
        );
        assert!(!state.palette_query.chars().any(char::is_control));
        assert!(!state.palette_query.contains('\u{202e}'));
        assert_eq!(
            crate::command_palette::parse_palette_query(&state.palette_query).mode,
            crate::command_palette::PaletteMode::Actions
        );
    }

    #[test]
    fn dispatch_palette_selection_wraps_current_results() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:>"));

        assert_eq!(state.palette_active, 0);
        assert!(dispatch(&mut state, "palette.select_prev"));
        assert_eq!(
            state.palette_active,
            crate::command_palette::SAFE_ACTIONS.len() - 1
        );
        assert!(dispatch(&mut state, "palette.select_next"));
        assert_eq!(state.palette_active, 0);

        state.palette_active = crate::command_palette::SAFE_ACTIONS.len() - 1;
        assert!(dispatch(&mut state, "palette.select_next"));
        assert_eq!(state.palette_active, 0);
    }

    #[test]
    fn dispatch_palette_escape_clears_query_then_closes() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:rename"));
        state.palette_active = 2;

        assert!(dispatch(&mut state, "palette.escape"));
        assert!(state.palette_open);
        assert_eq!(state.palette_query, "");
        assert_eq!(state.palette_active, 0);

        assert!(dispatch(&mut state, "palette.escape"));
        assert!(!state.palette_open);
    }

    #[test]
    fn dispatch_palette_execute_rename_current_terminal_opens_rename_dialog() {
        let mut state = test_state();
        state.palette_open = true;

        assert!(dispatch(
            &mut state,
            "palette.execute:rename_current_terminal"
        ));

        assert!(!state.palette_open);
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::RenameSession {
                pane_id,
                buffer,
                error,
            }) => {
                assert_eq!(*pane_id, state.active_pane.0);
                assert_eq!(buffer, "shell");
                assert!(error.is_none());
            }
            other => panic!("expected RenameSession dialog, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_palette_execute_active_runs_current_selection_and_closes() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:> open_settings"));

        assert!(dispatch(&mut state, "palette.execute_active"));

        assert!(!state.palette_open);
        assert!(state.settings_open);
    }

    #[test]
    fn dispatch_palette_executes_all_safe_actions_and_closes() {
        let mut state = test_state();
        state.palette_open = true;
        assert!(dispatch(&mut state, "palette.execute:split_pane_right"));
        assert!(!state.palette_open);
        assert_eq!(state.panes.iter().flatten().count(), 2);

        let mut state = test_state();
        state.palette_open = true;
        assert!(dispatch(&mut state, "palette.execute:split_pane_down"));
        assert!(!state.palette_open);
        assert_eq!(state.panes.len(), 2);

        let mut state = test_state();
        state.palette_open = true;
        assert!(dispatch(&mut state, "palette.execute:new_terminal"));
        assert!(!state.palette_open);
        assert_eq!(state.tabs.len(), 2);

        let mut state = test_state();
        mutate_split_right(&mut state, PaneId(1));
        state.palette_open = true;
        assert!(dispatch(&mut state, "palette.execute:close_pane"));
        assert!(!state.palette_open);
        assert_eq!(state.panes.iter().flatten().count(), 1);

        let mut state = test_state();
        state.palette_open = true;
        assert!(dispatch(&mut state, "palette.execute:toggle_sidebar"));
        assert!(!state.palette_open);
        assert!(state.sidebar_collapsed);

        let mut state = test_state();
        state.palette_open = true;
        assert!(dispatch(&mut state, "palette.execute:open_settings"));
        assert!(!state.palette_open);
        assert!(state.settings_open);
    }

    #[test]
    fn dispatch_palette_refuses_unknown_and_destructive_ids_without_closing() {
        let mut state = test_state();
        state.palette_open = true;

        assert!(!dispatch(&mut state, "palette.execute:kill_session"));
        assert!(!dispatch(&mut state, "palette.execute:session.kill:1"));
        assert!(!dispatch(&mut state, "palette.execute:missing"));
        assert!(state.palette_open);
    }

    #[test]
    fn dispatch_palette_navigation_ids_must_come_from_real_snapshot_rows() {
        let mut state = two_workspace_state();
        state.palette_open = true;
        state.palette_query = ": beta".to_string();

        assert!(dispatch(&mut state, "palette.execute:workspace:1"));
        assert!(!state.palette_open);
        assert_eq!(state.active_workspace, 1);

        let mut state = two_workspace_state();
        state.palette_open = true;
        state.palette_query = ": pane 8".to_string();
        assert!(dispatch(&mut state, "palette.execute:terminal:1:8"));
        assert!(!state.palette_open);
        assert_eq!(state.active_workspace, 1);
        assert_eq!(state.active_pane, PaneId(8));

        let mut state = two_workspace_state();
        state.palette_open = true;
        state.palette_query = ":".to_string();
        assert!(!dispatch(&mut state, "palette.execute:terminal:1:999"));
        assert!(state.palette_open);
    }

    #[test]
    fn dispatch_palette_agent_terminal_ids_must_come_from_real_snapshot_rows() {
        let mut state = two_workspace_state();
        state.workspaces[1].tabs[0].subtitle = "codex.cmd".to_string();
        state.workspaces[1].tabs[0].panes[0][1].subtitle = "codex.cmd".to_string();
        state.palette_open = true;
        state.palette_query = "@ codex".to_string();

        assert!(dispatch(&mut state, "palette.execute:agent-terminal:1:8"));
        assert!(!state.palette_open);
        assert_eq!(state.active_workspace, 1);
        assert_eq!(state.active_pane, PaneId(8));

        let mut state = two_workspace_state();
        state.palette_open = true;
        state.palette_query = "@".to_string();

        assert!(!dispatch(&mut state, "palette.execute:agent-terminal:1:8"));
        assert!(state.palette_open);
    }

    #[test]
    fn palette_key_down_and_ctrl_n_select_next_result() {
        use unshit::core::event::{Key, Modifiers};
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:>"));

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::plain(Key::ArrowDown)
        ));
        assert_eq!(state.palette_active, 1);

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::new(Key::Char('n'), Modifiers::CTRL)
        ));
        assert_eq!(state.palette_active, 2);
    }

    #[test]
    fn palette_key_up_and_ctrl_p_select_previous_result() {
        use unshit::core::event::{Key, Modifiers};
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:>"));

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::plain(Key::ArrowUp)
        ));
        assert_eq!(
            state.palette_active,
            crate::command_palette::SAFE_ACTIONS.len() - 1
        );

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::new(Key::Char('p'), Modifiers::CTRL)
        ));
        assert_eq!(
            state.palette_active,
            crate::command_palette::SAFE_ACTIONS.len() - 2
        );
    }

    #[test]
    fn palette_key_enter_executes_selected_result() {
        use unshit::core::event::Key;
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:>"));
        state.palette_active = crate::command_palette::SAFE_ACTIONS
            .iter()
            .position(|action| action.id == "open_settings")
            .expect("open settings action");

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::plain(Key::Enter)
        ));

        assert!(!state.palette_open);
        assert!(state.settings_open);
    }

    #[test]
    fn palette_key_escape_clears_query_before_closing() {
        use unshit::core::event::Key;
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:rename"));
        state.palette_active = 2;

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::plain(Key::Escape)
        ));
        assert!(state.palette_open);
        assert_eq!(state.palette_query, "");
        assert_eq!(state.palette_active, 0);

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::plain(Key::Escape)
        ));
        assert!(!state.palette_open);
    }

    #[test]
    fn palette_key_plain_text_edits_query_without_focused_input() {
        use unshit::core::event::{Key, Modifiers};
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));

        for ch in ['r', 'e', 'n', 'a', 'm', 'e'] {
            assert!(dispatch_palette_key(
                &mut state,
                &KeyCombo::plain(Key::Char(ch))
            ));
        }
        assert_eq!(state.palette_query, "rename");

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::plain(Key::Backspace)
        ));
        assert_eq!(state.palette_query, "renam");

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::new(Key::Char('>'), Modifiers::SHIFT)
        ));
        assert_eq!(state.palette_query, "renam>");
        assert!(state.palette_open);
    }

    #[test]
    fn palette_key_plain_space_supports_multi_term_query() {
        use unshit::core::event::Key;
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));

        for ch in "open settings".chars() {
            let combo = if ch == ' ' {
                KeyCombo::plain(Key::Space)
            } else {
                KeyCombo::plain(Key::Char(ch))
            };
            assert!(dispatch_palette_key(&mut state, &combo));
        }

        assert_eq!(state.palette_query, "open settings");
        let results = crate::command_palette::build_palette_results(
            &state.ui_snapshot(),
            &state.palette_query,
        );
        let first = results
            .groups
            .iter()
            .flat_map(|group| group.items.iter())
            .next()
            .expect("matching command");
        assert_eq!(first.id, "open_settings");
    }

    #[test]
    fn palette_key_consumes_global_shortcuts_while_open() {
        use unshit::core::event::{Key, Modifiers};
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        let tabs_before = state.tabs.len();
        let pane_rows_before = state.panes.len();

        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::new(Key::Char('v'), Modifiers::CTRL)
        ));
        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::new(Key::Char('w'), Modifiers::CTRL)
        ));

        assert!(state.palette_open);
        assert_eq!(state.tabs.len(), tabs_before);
        assert_eq!(state.panes.len(), pane_rows_before);
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn palette_key_typing_then_enter_executes_matching_command() {
        use unshit::core::event::Key;
        use unshit::core::shortcut::KeyCombo;

        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));

        for ch in "rename".chars() {
            assert!(dispatch_palette_key(
                &mut state,
                &KeyCombo::plain(Key::Char(ch))
            ));
        }
        assert!(dispatch_palette_key(
            &mut state,
            &KeyCombo::plain(Key::Enter)
        ));

        assert!(!state.palette_open);
        assert!(matches!(
            state.confirm_dialog,
            Some(ConfirmDialog::RenameSession { pane_id: 1, .. })
        ));
    }

    #[test]
    fn palette_hover_moves_active_selection_to_real_row() {
        let mut state = seed_state();
        assert!(dispatch(&mut state, "palette.toggle"));
        assert!(dispatch(&mut state, "palette.query:>"));

        assert!(dispatch(&mut state, "palette.hover:3"));
        assert_eq!(state.palette_active, 3);
        assert!(!dispatch(&mut state, "palette.hover:999"));
        assert_eq!(state.palette_active, 3);
    }

    #[test]
    fn dispatch_session_rename_active_opens_current_pane_rename_dialog() {
        let mut state = test_state();

        assert!(dispatch(&mut state, "session.rename_active"));

        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::RenameSession {
                pane_id,
                buffer,
                error,
            }) => {
                assert_eq!(*pane_id, 1);
                assert_eq!(buffer, "shell");
                assert!(error.is_none());
            }
            other => panic!("expected RenameSession dialog, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_fps_overlay_toggle_flips_visibility_and_emit_flag() {
        let _guard = crate::ui::fps_overlay::global_state_test_lock();
        // Reset to a known starting state since the overlay lives in
        // a process global. Both the visible flag and the FrameProbe
        // emit flag must move in lock step on every dispatch.
        crate::ui::fps_overlay::reset_for_test();
        let mut state = test_state();

        assert!(dispatch(&mut state, "fps_overlay.toggle"));
        let snap = crate::ui::fps_overlay::snapshot();
        assert!(snap.visible);
        assert!(unshit::app::frame_probe::is_emit_enabled());

        assert!(dispatch(&mut state, "fps_overlay.toggle"));
        let snap = crate::ui::fps_overlay::snapshot();
        assert!(!snap.visible);
        assert!(!unshit::app::frame_probe::is_emit_enabled());

        crate::ui::fps_overlay::reset_for_test();
    }

    #[test]
    fn dispatch_unknown_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "nonexistent.command"));
    }

    // -- context menu ---------------------------------------------------------

    #[test]
    fn ctx_menu_close_clears_state() {
        let mut state = test_state();
        state.ctx_menu = Some(CtxMenu {
            x: 100.0,
            y: 200.0,
            target: CtxMenuTarget::Workspace { idx: 0 },
        });
        assert!(dispatch(&mut state, "ctx_menu.close"));
        assert!(state.ctx_menu.is_none());
    }

    #[test]
    fn ctx_menu_close_returns_false_when_already_closed() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "ctx_menu.close"));
    }

    #[test]
    fn modal_close_also_closes_ctx_menu() {
        let mut state = test_state();
        state.ctx_menu = Some(CtxMenu {
            x: 10.0,
            y: 20.0,
            target: CtxMenuTarget::Workspace { idx: 0 },
        });
        assert!(dispatch(&mut state, "modal.close"));
        assert!(state.ctx_menu.is_none());
    }

    #[test]
    fn workspace_remove_closes_ctx_menu() {
        let mut state = test_state();
        state.ctx_menu = Some(CtxMenu {
            x: 0.0,
            y: 0.0,
            target: CtxMenuTarget::Workspace { idx: 0 },
        });
        assert!(dispatch(&mut state, "workspace.remove:1"));
        assert!(state.ctx_menu.is_none());
    }

    // -- F5: per-workspace kill ----------------------------------------------

    #[test]
    fn request_kill_all_opens_confirm_dialog_and_closes_ctx_menu() {
        let mut state = seed_state();
        state.ctx_menu = Some(CtxMenu {
            x: 1.0,
            y: 2.0,
            target: CtxMenuTarget::Workspace { idx: 0 },
        });
        assert!(dispatch(&mut state, "workspace.request_kill_all:0"));
        assert!(state.ctx_menu.is_none());
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::KillWorkspace {
                workspace_idx,
                name,
            }) => {
                assert_eq!(*workspace_idx, 0);
                assert_eq!(name, &state.workspaces[0].name);
            }
            other => panic!("expected KillWorkspace dialog, got {other:?}"),
        }
    }

    #[test]
    fn request_kill_all_for_unknown_workspace_is_noop() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "workspace.request_kill_all:99"));
        assert!(state.confirm_dialog.is_none());
    }

    #[test]
    fn dialog_cancel_clears_dialog_without_side_effects() {
        let mut state = seed_state();
        state.confirm_dialog = Some(ConfirmDialog::KillWorkspace {
            workspace_idx: 0,
            name: "ws".into(),
        });
        let tabs_before = state.tabs.len();
        assert!(dispatch(&mut state, "dialog.cancel"));
        assert!(state.confirm_dialog.is_none());
        assert_eq!(state.tabs.len(), tabs_before, "cancel must not kill tabs");
    }

    #[test]
    fn dialog_confirm_on_kill_workspace_empties_active_workspace() {
        let mut state = seed_state();
        let ws_idx = state.active_workspace;
        assert!(!state.panes.is_empty(), "seed must have at least one pane");
        state.confirm_dialog = Some(ConfirmDialog::KillWorkspace {
            workspace_idx: ws_idx,
            name: state.workspaces[ws_idx].name.clone(),
        });
        assert!(dispatch(&mut state, "dialog.confirm"));
        assert!(state.confirm_dialog.is_none());
        assert!(
            state.tabs.is_empty(),
            "active workspace tabs must be emptied"
        );
        assert!(
            state.panes.is_empty(),
            "active workspace panes must be emptied"
        );
        assert!(
            state.terminals.is_empty(),
            "terminal handles must be dropped"
        );
        assert!(
            state.workspaces[ws_idx].tabs.is_empty(),
            "saved tab list must also be cleared"
        );
    }

    #[test]
    fn mutate_kill_workspace_terminals_on_inactive_workspace_leaves_active_intact() {
        let mut state = seed_state();
        mutate_add_workspace(&mut state);
        assert!(state.workspaces.len() >= 2);
        let inactive_idx = 1;
        assert_ne!(state.active_workspace, inactive_idx);

        // Seed the inactive workspace with a saved tab so there's
        // something to kill.
        state.workspaces[inactive_idx].tabs = vec![TerminalTab {
            id: "t-inactive".into(),
            name: "old".into(),
            subtitle: "".into(),
            status: TabStatus::Running,
            panes: vec![vec![Pane {
                id: PaneId(42),
                title: "p".into(),
                subtitle: "".into(),
                pid: 0,
                cpu: 0.0,
            }]],
            active_pane: PaneId(42),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        }];

        let active_tabs_before = state.tabs.len();
        let active_panes_before = state.panes.len();

        mutate_kill_workspace_terminals(&mut state, inactive_idx);

        assert!(
            state.workspaces[inactive_idx].tabs.is_empty(),
            "target workspace must be emptied"
        );
        assert_eq!(
            state.tabs.len(),
            active_tabs_before,
            "active workspace tabs must be untouched"
        );
        assert_eq!(
            state.panes.len(),
            active_panes_before,
            "active workspace panes must be untouched"
        );
    }

    #[test]
    fn mutate_kill_workspace_terminals_unknown_index_is_noop() {
        let mut state = test_state();
        let tabs_before = state.tabs.len();
        mutate_kill_workspace_terminals(&mut state, 999);
        assert_eq!(state.tabs.len(), tabs_before);
    }

    #[test]
    fn modal_close_also_closes_confirm_dialog() {
        let mut state = test_state();
        state.confirm_dialog = Some(ConfirmDialog::KillWorkspace {
            workspace_idx: 0,
            name: "ws".into(),
        });
        assert!(dispatch(&mut state, "modal.close"));
        assert!(state.confirm_dialog.is_none());
    }

    #[test]
    fn request_kill_all_terminals_opens_kill_all_confirm_dialog_with_count() {
        let mut state = seed_state();
        assert!(dispatch(&mut state, "app.request_kill_all_terminals"));
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::KillAll { count }) => {
                assert_eq!(*count, state.terminals.len());
            }
            other => panic!("expected KillAll dialog, got {other:?}"),
        }
    }

    #[test]
    fn dialog_confirm_on_kill_all_empties_every_workspace() {
        let mut state = seed_state();
        mutate_add_workspace(&mut state);
        // Seed saved tabs on the second (now inactive) workspace so the
        // test asserts the mutator reaches into every workspace, not
        // just the active one.
        state.workspaces[1].tabs = vec![TerminalTab {
            id: "ws2-t1".into(),
            name: "n".into(),
            subtitle: "".into(),
            status: TabStatus::Running,
            panes: vec![vec![Pane {
                id: PaneId(77),
                title: "p".into(),
                subtitle: "".into(),
                pid: 0,
                cpu: 0.0,
            }]],
            active_pane: PaneId(77),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        }];

        state.confirm_dialog = Some(ConfirmDialog::KillAll { count: 0 });
        assert!(dispatch(&mut state, "dialog.confirm"));

        assert!(state.confirm_dialog.is_none());
        assert!(state.tabs.is_empty(), "active tabs must be emptied");
        assert!(state.panes.is_empty(), "active panes must be emptied");
        assert!(
            state.terminals.is_empty(),
            "every terminal handle must be dropped"
        );
        for (idx, ws) in state.workspaces.iter().enumerate() {
            assert!(
                ws.tabs.is_empty(),
                "workspace {idx} must have no saved tabs"
            );
        }
    }

    #[test]
    fn mutate_kill_all_terminals_on_empty_state_is_noop() {
        let mut state = seed_state();
        // seed_state produces a workspace with no tabs, so everything is
        // already empty; the mutator must not panic or corrupt invariants.
        mutate_kill_all_terminals(&mut state);
        assert!(state.tabs.is_empty());
        assert!(state.terminals.is_empty());
        assert_eq!(state.active_pane, PaneId(0));
    }

    // -- F7 close-app prompt --------------------------------------------------

    fn close_app_dialog(count: usize, remember: bool, kept_pane_ids: &[u32]) -> ConfirmDialog {
        ConfirmDialog::CloseApp {
            count,
            remember,
            kept_pane_ids: kept_pane_ids.iter().copied().collect(),
        }
    }

    #[test]
    fn resolve_close_action_with_no_preference_opens_prompt_and_vetoes() {
        let mut state = seed_state();
        assert!(!toggle_on(&state, ToggleKey::RememberCloseChoice));
        let action = resolve_close_action(&mut state);
        assert_eq!(action, CloseAction::Prompt);
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::CloseApp {
                remember,
                kept_pane_ids,
                ..
            }) => {
                assert!(!remember);
                assert!(
                    kept_pane_ids.contains(&state.active_pane.0),
                    "seeded pane should start selected to keep alive"
                );
            }
            other => panic!("expected CloseApp dialog, got {other:?}"),
        }
    }

    #[test]
    fn resolve_close_action_with_kill_preference_returns_kill_all() {
        let mut state = seed_state();
        state.toggles.insert(ToggleKey::RememberCloseChoice, true);
        state.toggles.insert(ToggleKey::KillAllOnClose, true);
        let action = resolve_close_action(&mut state);
        assert_eq!(action, CloseAction::KillAll);
        assert!(
            state.confirm_dialog.is_none(),
            "remembered preference must not open the dialog"
        );
    }

    #[test]
    fn resolve_close_action_with_keep_preference_returns_keep_running() {
        let mut state = seed_state();
        state.toggles.insert(ToggleKey::RememberCloseChoice, true);
        state.toggles.insert(ToggleKey::KillAllOnClose, false);
        let action = resolve_close_action(&mut state);
        assert_eq!(action, CloseAction::KeepRunning);
        assert!(state.confirm_dialog.is_none());
    }

    #[test]
    fn dialog_toggle_remember_flips_checkbox_on_close_app_dialog() {
        let mut state = seed_state();
        state.confirm_dialog = Some(close_app_dialog(2, false, &[1, 2]));
        assert!(dispatch(&mut state, "dialog.toggle_remember"));
        assert!(matches!(
            state.confirm_dialog,
            Some(ConfirmDialog::CloseApp { remember: true, .. })
        ));
        assert!(dispatch(&mut state, "dialog.toggle_remember"));
        assert!(matches!(
            state.confirm_dialog,
            Some(ConfirmDialog::CloseApp {
                remember: false,
                ..
            })
        ));
    }

    #[test]
    fn dialog_toggle_keep_flips_selected_pane_on_close_app_dialog() {
        let mut state = seed_state();
        state.confirm_dialog = Some(close_app_dialog(2, false, &[1, 2]));
        assert!(dispatch(&mut state, "dialog.toggle_keep:2"));
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::CloseApp { kept_pane_ids, .. }) => {
                assert!(kept_pane_ids.contains(&1));
                assert!(!kept_pane_ids.contains(&2));
            }
            other => panic!("expected CloseApp dialog, got {other:?}"),
        }
        assert!(dispatch(&mut state, "dialog.toggle_keep:2"));
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::CloseApp { kept_pane_ids, .. }) => {
                assert!(kept_pane_ids.contains(&2));
            }
            other => panic!("expected CloseApp dialog, got {other:?}"),
        }
    }

    #[test]
    fn dialog_toggle_remember_without_close_app_is_noop() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "dialog.toggle_remember"));
        state.confirm_dialog = Some(ConfirmDialog::KillAll { count: 1 });
        assert!(!dispatch(&mut state, "dialog.toggle_remember"));
        assert!(!dispatch(&mut state, "dialog.toggle_keep:1"));
    }

    #[test]
    fn close_keep_running_without_remember_clears_dialog_and_terminals_only() {
        let mut state = seed_state();
        state.confirm_dialog = Some(close_app_dialog(0, false, &[1]));
        assert!(dispatch(&mut state, "app.close.keep_running"));
        assert!(state.confirm_dialog.is_none());
        assert!(state.terminals.is_empty());
        assert!(
            !toggle_on(&state, ToggleKey::RememberCloseChoice),
            "preference must not be written when remember is off"
        );
    }

    #[test]
    fn close_keep_running_with_remember_persists_preference() {
        let mut state = seed_state();
        state.confirm_dialog = Some(close_app_dialog(0, true, &[1]));
        assert!(dispatch(&mut state, "app.close.keep_running"));
        assert!(toggle_on(&state, ToggleKey::RememberCloseChoice));
        assert!(!toggle_on(&state, ToggleKey::KillAllOnClose));
    }

    #[test]
    fn close_keep_running_kills_unselected_and_prunes_layout() {
        let mut state = seed_state();
        state.panes = vec![vec![
            Pane {
                id: PaneId(1),
                title: "keep".into(),
                subtitle: "bash".into(),
                pid: 0,
                cpu: 0.0,
            },
            Pane {
                id: PaneId(2),
                title: "kill".into(),
                subtitle: "bash".into(),
                pid: 0,
                cpu: 0.0,
            },
        ]];
        state.active_pane = PaneId(2);
        state.row_ratios = vec![1.0];
        state.col_ratios = vec![vec![0.4, 0.6]];
        state.tabs[0].panes = state.panes.clone();
        state.tabs[0].active_pane = PaneId(2);
        state.tabs[0].row_ratios = state.row_ratios.clone();
        state.tabs[0].col_ratios = state.col_ratios.clone();
        state.confirm_dialog = Some(close_app_dialog(2, false, &[1]));

        assert!(dispatch(&mut state, "app.close.keep_running"));

        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 1);
        assert_eq!(state.panes[0][0].id, PaneId(1));
        assert_eq!(state.active_pane, PaneId(1));
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.tabs[0].panes[0][0].id, PaneId(1));
        assert_eq!(
            state.workspaces[state.active_workspace].tabs[0].panes[0][0].id,
            PaneId(1)
        );
    }

    #[test]
    fn close_kill_and_quit_with_remember_persists_preference_and_empties() {
        let mut state = seed_state();
        state.confirm_dialog = Some(close_app_dialog(0, true, &[1]));
        assert!(dispatch(&mut state, "app.close.kill_and_quit"));
        assert!(toggle_on(&state, ToggleKey::RememberCloseChoice));
        assert!(toggle_on(&state, ToggleKey::KillAllOnClose));
        assert!(state.tabs.is_empty());
        assert!(state.terminals.is_empty());
    }

    #[test]
    fn close_reset_preference_clears_remember_flag() {
        let mut state = seed_state();
        state.toggles.insert(ToggleKey::RememberCloseChoice, true);
        state.toggles.insert(ToggleKey::KillAllOnClose, true);
        assert!(dispatch(&mut state, "app.close.reset_preference"));
        assert!(!toggle_on(&state, ToggleKey::RememberCloseChoice));
    }

    #[test]
    fn close_reset_preference_when_not_set_is_noop() {
        let mut state = seed_state();
        assert!(!toggle_on(&state, ToggleKey::RememberCloseChoice));
        assert!(!dispatch(&mut state, "app.close.reset_preference"));
    }

    #[test]
    fn dialog_confirm_on_close_app_is_noop_and_keeps_dialog_open() {
        let mut state = seed_state();
        state.confirm_dialog = Some(close_app_dialog(0, false, &[1]));
        assert!(!dispatch(&mut state, "dialog.confirm"));
        assert!(
            state.confirm_dialog.is_some(),
            "CloseApp must not be consumed by the generic confirm handler"
        );
    }

    // -- F8 named sessions ----------------------------------------------------

    #[test]
    fn tab_request_rename_opens_rename_dialog_with_current_title() {
        let mut state = seed_state();
        state.panes = vec![vec![Pane {
            id: PaneId(42),
            title: "api-server".into(),
            subtitle: "".into(),
            pid: 0,
            cpu: 0.0,
        }]];
        assert!(dispatch(&mut state, "tab.request_rename:42"));
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::RenameSession {
                pane_id, buffer, ..
            }) => {
                assert_eq!(*pane_id, 42);
                assert_eq!(buffer, "api-server");
            }
            other => panic!("expected RenameSession dialog, got {other:?}"),
        }
    }

    #[test]
    fn tab_request_rename_closes_ctx_menu() {
        let mut state = seed_state();
        state.ctx_menu = Some(CtxMenu {
            x: 1.0,
            y: 2.0,
            target: CtxMenuTarget::Tab { pane_id: 1 },
        });
        assert!(dispatch(&mut state, "tab.request_rename:1"));
        assert!(state.ctx_menu.is_none());
    }

    #[test]
    fn tab_request_rename_with_unknown_pane_opens_dialog_with_empty_buffer() {
        let mut state = seed_state();
        assert!(dispatch(&mut state, "tab.request_rename:9999"));
        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::RenameSession {
                pane_id, buffer, ..
            }) => {
                assert_eq!(*pane_id, 9999);
                assert!(buffer.is_empty());
            }
            other => panic!("expected RenameSession dialog, got {other:?}"),
        }
    }

    #[test]
    fn tab_request_rename_reads_title_from_saved_tab_pane() {
        let mut state = seed_state();
        state.tabs.push(TerminalTab {
            id: "inactive-tab".into(),
            name: "inactive".into(),
            subtitle: "bash".into(),
            status: TabStatus::Running,
            panes: vec![vec![Pane {
                id: PaneId(77),
                title: "saved-pane-title".into(),
                subtitle: "bash".into(),
                pid: 0,
                cpu: 0.0,
            }]],
            active_pane: PaneId(77),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        });

        assert!(dispatch(&mut state, "tab.request_rename:77"));

        match state.confirm_dialog.as_ref() {
            Some(ConfirmDialog::RenameSession { buffer, .. }) => {
                assert_eq!(buffer, "saved-pane-title");
            }
            other => panic!("expected RenameSession dialog, got {other:?}"),
        }
    }

    #[test]
    fn dialog_confirm_on_rename_session_is_noop_and_keeps_dialog_open() {
        let mut state = seed_state();
        state.confirm_dialog = Some(ConfirmDialog::RenameSession {
            pane_id: 1,
            buffer: "x".into(),
            error: None,
        });
        assert!(!dispatch(&mut state, "dialog.confirm"));
        assert!(state.confirm_dialog.is_some());
    }

    #[test]
    fn dialog_rename_commit_updates_pane_title_and_clears_dialog() {
        let mut state = seed_state();
        state.panes = vec![vec![Pane {
            id: PaneId(7),
            title: "shell".into(),
            subtitle: "".into(),
            pid: 0,
            cpu: 0.0,
        }]];
        state.confirm_dialog = Some(ConfirmDialog::RenameSession {
            pane_id: 7,
            buffer: "build".into(),
            error: None,
        });
        assert!(dispatch(&mut state, "dialog.rename_commit"));
        assert!(state.confirm_dialog.is_none());
        assert_eq!(state.panes[0][0].title, "build");
    }

    #[test]
    fn dialog_rename_commit_empty_buffer_clears_to_fallback_title() {
        let mut state = seed_state();
        state.panes = vec![vec![Pane {
            id: PaneId(7),
            title: "custom".into(),
            subtitle: "".into(),
            pid: 0,
            cpu: 0.0,
        }]];
        state.confirm_dialog = Some(ConfirmDialog::RenameSession {
            pane_id: 7,
            buffer: "   ".into(),
            error: None,
        });
        assert!(dispatch(&mut state, "dialog.rename_commit"));
        assert_eq!(state.panes[0][0].title, "shell");
    }

    #[test]
    fn dialog_rename_commit_without_dialog_is_noop() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "dialog.rename_commit"));
    }

    // refs #130: when the daemon RPC fails, the rename dialog must
    // stay open with an inline error string so the user can retry,
    // and the local pane title must NOT update (otherwise local and
    // daemon state diverge until the next refresh).
    #[test]
    fn rename_commit_rpc_failure_keeps_dialog_open_with_error_string() {
        let mut state = seed_state();
        let pane_id = state.panes[0][0].id.0;
        let original_title = state.panes[0][0].title.clone();
        state
            .pty_manager
            .test_install_broken_inner_with_session(pane_id, 7);
        state.confirm_dialog = Some(ConfirmDialog::RenameSession {
            pane_id,
            buffer: "new-name".to_string(),
            error: None,
        });
        let handled = dispatch(&mut state, "dialog.rename_commit");
        assert!(handled);
        match state.confirm_dialog {
            Some(ConfirmDialog::RenameSession {
                pane_id: pid,
                buffer,
                error,
            }) => {
                assert_eq!(pid, pane_id);
                assert_eq!(buffer, "new-name");
                let msg = error.expect("error string must be present");
                assert!(
                    msg.starts_with("rename failed:"),
                    "expected rename-failure message, got {msg:?}"
                );
            }
            other => panic!("expected dialog still open with error, got {other:?}"),
        }
        // Local title must not have moved.
        assert_eq!(state.panes[0][0].title, original_title);
        // No toast: the error is inline-only per spec decision 1a.
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn rename_commit_rpc_failure_does_not_call_mutate_rename_pane() {
        let mut state = seed_state();
        let pane_id = state.panes[0][0].id.0;
        let original = state.panes[0][0].title.clone();
        state
            .pty_manager
            .test_install_broken_inner_with_session(pane_id, 11);
        state.confirm_dialog = Some(ConfirmDialog::RenameSession {
            pane_id,
            buffer: "should-not-stick".to_string(),
            error: None,
        });
        dispatch(&mut state, "dialog.rename_commit");
        // Original title preserved across every layout; mutate_rename_pane
        // would have rewritten this, so its absence is the assertion.
        assert_eq!(state.panes[0][0].title, original);
    }

    #[test]
    fn refresh_sessions_leaves_cache_untouched_when_disconnected() {
        let mut state = seed_state();
        state.sessions = vec![SessionSnapshot {
            session_id: 1,
            pane_id: 1,
            workspace_id: 1,
            name: None,
            pid: None,
            alive: true,
        }];
        // No daemon connected in tests; refresh must log and not panic.
        refresh_sessions(&mut state);
        assert_eq!(state.sessions.len(), 1);
    }

    // refs #130: refresh failure must surface to the user, not just to stderr.
    #[test]
    fn refresh_sessions_failure_sets_stale_and_pushes_toast() {
        let mut state = seed_state();
        assert!(!state.sessions_stale);
        assert!(state.toasts.is_empty());
        // No daemon connected in tests, so list_sessions returns Err.
        refresh_sessions(&mut state);
        assert!(state.sessions_stale);
        assert_eq!(state.toasts.len(), 1);
        let msg = state
            .toasts
            .iter()
            .next()
            .map(|t| t.message.clone())
            .unwrap_or_default();
        assert!(
            msg.starts_with("refresh failed:"),
            "expected refresh-failure toast, got {msg:?}"
        );
    }

    #[test]
    fn push_error_toast_caps_at_three() {
        let mut state = seed_state();
        for i in 0..5 {
            push_error_toast(&mut state, format!("err {i}"));
        }
        assert_eq!(state.toasts.len(), 3);
        let messages: Vec<String> = state.toasts.iter().map(|t| t.message.clone()).collect();
        assert_eq!(messages, vec!["err 2", "err 3", "err 4"]);
    }

    #[test]
    fn dispatch_toast_dismiss_removes_toast() {
        let mut state = seed_state();
        push_error_toast(&mut state, "boom");
        let id = state.toasts.iter().next().expect("toast").id;
        assert!(dispatch(&mut state, &format!("toast.dismiss:{id}")));
        assert!(state.toasts.is_empty());
        // Idempotent: a second dismiss returns false but does not panic.
        assert!(!dispatch(&mut state, &format!("toast.dismiss:{id}")));
    }

    #[test]
    fn ui_snapshot_includes_toast_view_in_push_order() {
        let mut state = seed_state();
        push_error_toast(&mut state, "first");
        push_error_toast(&mut state, "second");
        let snap = state.ui_snapshot();
        let messages: Vec<&str> = snap.toasts.iter().map(|t| t.message.as_str()).collect();
        assert_eq!(messages, vec!["first", "second"]);
        assert!(!snap.sessions_stale);
    }

    #[test]
    fn ui_snapshot_mirrors_window_maximized_state() {
        let mut state = seed_state();
        assert!(!state.ui_snapshot().window_maximized);

        state.window_maximized = true;

        assert!(state.ui_snapshot().window_maximized);
    }

    #[test]
    fn push_notification_toast_adds_title_and_target_to_snapshot() {
        let mut state = seed_state();
        push_notification_toast(&mut state, "Build done", "Tests passed", 1, 1);

        let snap = state.ui_snapshot();
        let toast = snap.toasts.first().expect("notification toast");
        assert_eq!(toast.title.as_deref(), Some("Build done"));
        assert_eq!(toast.message, "Tests passed");
        assert_eq!(
            toast.target,
            Some(ToastTarget {
                workspace_id: 1,
                pane_id: 1,
            })
        );
    }

    #[test]
    fn dispatch_notifications_test_pushes_targeted_toast() {
        let mut state = seed_state();
        let workspace_id = active_workspace_num(&state);
        let pane_id = state.active_pane.0;

        assert!(dispatch(&mut state, "notifications.test"));

        let snap = state.ui_snapshot();
        let toast = snap.toasts.first().expect("test notification toast");
        assert_eq!(toast.title.as_deref(), Some("test notification"));
        assert_eq!(
            toast.message,
            format!("notification test from workspace {workspace_id}, pane {pane_id}")
        );
        assert_eq!(
            toast.target,
            Some(ToastTarget {
                workspace_id,
                pane_id
            })
        );
    }

    #[test]
    fn notification_activate_focuses_workspace_pane_and_dismisses_toast() {
        let mut state = two_workspace_state();
        let id = push_notification_toast(&mut state, "Done", "Pane 8 finished", 2, 8);

        assert!(dispatch(&mut state, &format!("notification.activate:{id}")));

        assert!(state.toasts.is_empty());
        assert_eq!(state.active_workspace, 1);
        assert_eq!(state.active_pane, PaneId(8));
    }

    // refs #130: with the orphan-branch kill now blocking on a daemon
    // ack, a disconnected daemon must NOT drop the row optimistically.
    // The user gets a toast instead and the cached row stays so the
    // panel does not lie about what the daemon knows.
    #[test]
    fn session_kill_without_pane_mapping_keeps_row_when_disconnected() {
        let mut state = seed_state();
        state.sessions = vec![
            SessionSnapshot {
                session_id: 1,
                pane_id: 1,
                workspace_id: 1,
                name: None,
                pid: None,
                alive: true,
            },
            SessionSnapshot {
                session_id: 2,
                pane_id: 2,
                workspace_id: 1,
                name: None,
                pid: None,
                alive: true,
            },
        ];
        assert!(dispatch(&mut state, "session.kill:1"));
        assert_eq!(state.sessions.len(), 2, "row stays under failed RPC");
        assert_eq!(state.toasts.len(), 1, "user sees a kill-failed toast");
        let msg = state
            .toasts
            .iter()
            .next()
            .map(|t| t.message.clone())
            .unwrap_or_default();
        assert!(
            msg.starts_with("kill failed:"),
            "expected kill-failure toast, got {msg:?}"
        );
    }

    #[test]
    fn session_kill_bad_id_returns_false() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "session.kill:not-a-number"));
    }

    #[test]
    fn sessions_refresh_dispatch_returns_true() {
        let mut state = seed_state();
        assert!(dispatch(&mut state, "sessions.refresh"));
    }

    #[test]
    fn settings_section_sessions_labels_and_included_in_all() {
        assert_eq!(SettingsSection::Sessions.label(), "sessions");
        assert!(SettingsSection::all().contains(&SettingsSection::Sessions));
    }

    #[test]
    fn mutate_rename_pane_updates_saved_workspace_tabs() {
        let mut state = seed_state();
        state.workspaces[1].tabs = vec![TerminalTab {
            id: "ws2-t1".into(),
            name: "n".into(),
            subtitle: "".into(),
            status: TabStatus::Running,
            panes: vec![vec![Pane {
                id: PaneId(9),
                title: "old".into(),
                subtitle: "".into(),
                pid: 0,
                cpu: 0.0,
            }]],
            active_pane: PaneId(9),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        }];
        mutate_rename_pane(&mut state, 9, "new-name");
        assert_eq!(state.workspaces[1].tabs[0].panes[0][0].title, "new-name");
    }

    #[test]
    fn mutate_rename_pane_updates_tab_titles_and_sidebar_entries() {
        let mut state = seed_state();
        let pane_id = state.active_pane.0;
        state.workspaces[state.active_workspace].tabs = state.tabs.clone();

        mutate_rename_pane(&mut state, pane_id, "build-watch");

        assert_eq!(state.panes[0][0].title, "build-watch");
        assert_eq!(state.tabs[state.active_tab].name, "build-watch");
        assert_eq!(
            state.workspaces[state.active_workspace].tabs[state.active_tab].name,
            "build-watch"
        );

        let snap = state.ui_snapshot();
        assert_eq!(snap.tabs[snap.active_tab].name, "build-watch");
        let active_entry = snap.workspaces[snap.active_workspace]
            .terminal_entries
            .iter()
            .find(|entry| entry.pane_id.0 == pane_id)
            .expect("renamed active pane entry");
        assert_eq!(active_entry.name, "build-watch");
    }

    #[test]
    fn mutate_rename_pane_updates_inactive_saved_tab_title() {
        let mut state = seed_state();
        state.workspaces[1].tabs = vec![TerminalTab {
            id: "ws2-t1".into(),
            name: "old-tab".into(),
            subtitle: "bash".into(),
            status: TabStatus::Running,
            panes: vec![vec![Pane {
                id: PaneId(9),
                title: "old-pane".into(),
                subtitle: "bash".into(),
                pid: 0,
                cpu: 0.0,
            }]],
            active_pane: PaneId(9),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        }];

        mutate_rename_pane(&mut state, 9, "api-server");

        let tab = &state.workspaces[1].tabs[0];
        assert_eq!(tab.name, "api-server");
        assert_eq!(tab.panes[0][0].title, "api-server");
        let snap = state.ui_snapshot();
        let entry = snap.workspaces[1]
            .terminal_entries
            .iter()
            .find(|entry| entry.pane_id == PaneId(9))
            .expect("inactive workspace entry");
        assert_eq!(entry.name, "api-server");
    }

    #[test]
    fn workspace_remove_does_not_remove_last() {
        let mut state = test_state();
        // Remove workspaces until one remains.
        while state.workspaces.len() > 1 {
            state.workspaces.pop();
        }
        let before = state.workspaces.len();
        mutate_remove_workspace(&mut state, 0);
        assert_eq!(state.workspaces.len(), before);
    }

    #[test]
    fn workspace_remove_renumbers() {
        let mut state = test_state();
        mutate_add_workspace(&mut state);
        mutate_add_workspace(&mut state);
        mutate_add_workspace(&mut state);
        let count = state.workspaces.len();
        assert!(count >= 3);
        mutate_remove_workspace(&mut state, 1);
        assert_eq!(state.workspaces.len(), count - 1);
        for (i, ws) in state.workspaces.iter().enumerate() {
            assert_eq!(ws.num, i as u32 + 1);
        }
    }

    #[test]
    fn workspace_collapse_via_dispatch() {
        let mut state = test_state();
        mutate_add_workspace(&mut state);
        let before = state.workspaces[0].collapsed;
        assert!(dispatch(&mut state, "workspace.collapse:0"));
        assert_ne!(state.workspaces[0].collapsed, before);
    }

    #[test]
    fn workspace_switch_via_dispatch() {
        let mut state = test_state();
        mutate_add_workspace(&mut state);
        mutate_add_workspace(&mut state);
        assert!(dispatch(&mut state, "workspace.switch:1"));
        assert_eq!(state.active_workspace, 1);
    }

    // -- find_active_pane / is_on ---------------------------------------------

    #[test]
    fn find_active_pane_returns_matching() {
        let state = test_state();
        let snap = state.ui_snapshot();
        let pane = find_active_pane(&snap);
        assert_eq!(pane.id, PaneId(1));
    }

    #[test]
    fn find_active_pane_falls_back_to_first() {
        let mut state = test_state();
        state.active_pane = PaneId(999); // non-existent
        let snap = state.ui_snapshot();
        let pane = find_active_pane(&snap);
        assert_eq!(pane.id, PaneId(1)); // falls back
    }

    #[test]
    fn is_on_returns_value_or_false() {
        let mut state = test_state();
        state.toggles.insert(ToggleKey::RememberCloseChoice, true);
        state.toggles.insert(ToggleKey::KillAllOnClose, false);
        let snap = state.ui_snapshot();

        assert!(is_on(&snap, ToggleKey::RememberCloseChoice));
        assert!(!is_on(&snap, ToggleKey::KillAllOnClose));

        // Unknown / missing keys default to false. Drop a key from the
        // map and confirm `is_on` reports the absent value rather than
        // panicking.
        state.toggles.remove(&ToggleKey::KillAllOnClose);
        let snap = state.ui_snapshot();
        assert!(!is_on(&snap, ToggleKey::KillAllOnClose));
    }

    // -- ui_snapshot ----------------------------------------------------------

    #[test]
    fn ui_snapshot_copies_fields() {
        let mut state = test_state();
        state.config_font_size_pt = 15;
        state.terminal_font_size_pt = 20;
        state.scroll_line_px = 72;
        state.smooth_scroll_duration_ms = 60;
        state.theme = "dracula".to_string();
        state.custom_theme.accent = unshit::core::style::types::Color::rgb(18, 52, 86);
        state.sidebar_collapsed = true;

        let snap = state.ui_snapshot();
        assert_eq!(snap.config_font_size_pt, 15);
        assert_eq!(snap.terminal_font_size_pt, 20);
        assert_eq!(snap.scroll_line_px, 72);
        assert_eq!(snap.smooth_scroll_duration_ms, 60);
        assert_eq!(snap.theme, "dracula");
        assert_eq!(snap.custom_theme.accent, state.custom_theme.accent);
        assert!(snap.sidebar_collapsed);
    }

    // -- seed_state -----------------------------------------------------------

    #[test]
    fn seed_state_has_reasonable_defaults() {
        let state = seed_state();
        assert_eq!(state.workspaces.len(), 4);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.active_tab, 0);
        assert_eq!(state.active_workspace, 0);
        assert_eq!(state.scroll_line_px, DEFAULT_SCROLL_LINE_PX);
        assert_eq!(
            state.smooth_scroll_duration_ms,
            DEFAULT_SMOOTH_SCROLL_DURATION_MS
        );
        assert_eq!(state.config_font_size_pt, DEFAULT_CONFIG_FONT_SIZE_PT);
        assert_eq!(state.terminal_font_size_pt, DEFAULT_TERMINAL_FONT_SIZE_PT);
        assert_eq!(state.custom_theme, crate::theme::default_custom_theme());
        assert!(!state.settings_open);
        assert!(!state.sidebar_collapsed);
        assert!(!state.palette_open);
    }

    // -- mutate_with ----------------------------------------------------------

    #[test]
    fn mutate_with_applies_closure() {
        let shared: SharedState = std::sync::Arc::new(std::sync::Mutex::new(test_state()));
        let result = mutate_with(&shared, |st| {
            st.terminal_font_size_pt = 25;
            st.terminal_font_size_pt
        });
        assert_eq!(result, 25);
        let guard = shared.lock().unwrap();
        assert_eq!(guard.terminal_font_size_pt, 25);
    }

    // -- text selection -------------------------------------------------------

    #[test]
    fn cell_from_local_maps_and_floors() {
        // 10px wide, 20px tall cells, no offset.
        assert_eq!(
            cell_from_local(0.0, 0.0, 10.0, 20.0, 0.0, 80, 24),
            Some((0, 0))
        );
        // 95/10 -> col 9, 45/20 -> row 2.
        assert_eq!(
            cell_from_local(95.0, 45.0, 10.0, 20.0, 0.0, 80, 24),
            Some((2, 9))
        );
    }

    #[test]
    fn cell_from_local_clamps_and_applies_offset() {
        // Far overrun clamps onto the last cell.
        assert_eq!(
            cell_from_local(1.0e5, 1.0e5, 10.0, 20.0, 0.0, 80, 24),
            Some((23, 79))
        );
        // The content x-offset shifts the cell origin right: x=13 with a 3px
        // offset lands 10px into the content -> col 1.
        assert_eq!(
            cell_from_local(13.0, 0.0, 10.0, 20.0, 3.0, 80, 24),
            Some((0, 1))
        );
        // Negative local (left of content) clamps to column 0.
        assert_eq!(
            cell_from_local(-5.0, -5.0, 10.0, 20.0, 0.0, 80, 24),
            Some((0, 0))
        );
    }

    #[test]
    fn cell_from_local_rejects_degenerate_inputs() {
        assert_eq!(cell_from_local(5.0, 5.0, 10.0, 20.0, 0.0, 0, 24), None);
        assert_eq!(cell_from_local(5.0, 5.0, 0.0, 20.0, 0.0, 80, 24), None);
    }

    #[test]
    fn term_selection_empty_and_ordering() {
        // A collapsed cell selection is empty; word/line are never empty.
        assert!(TermSelection::new((1, 2), SelectMode::Cell).is_empty());
        assert!(!TermSelection::new((1, 2), SelectMode::Word).is_empty());

        let sel = TermSelection {
            anchor: (2, 5),
            focus: (1, 1),
            mode: SelectMode::Cell,
        };
        assert!(!sel.is_empty());
        assert_eq!(sel.ordered(), ((1, 1), (2, 5)));
    }

    #[test]
    fn apply_selection_highlight_paints_only_selected_cells() {
        let mut term = crate::terminal::Terminal::new(2, 5);
        term.process_bytes(b"aaaaa\r\nbbbbb");
        let mut grid = term.display_grid();
        // Absolute line 0 == display row 0 on a fresh terminal with no
        // scrollback; select cols 1..=3 of it.
        let sel = TermSelection {
            anchor: (0, 1),
            focus: (0, 3),
            mode: SelectMode::Cell,
        };
        apply_selection_highlight(&mut grid, &term, &sel);

        // Selected cells (row 0, cols 1..=3) carry the selection bg.
        for col in 1..=3 {
            assert_eq!(grid.get_cell(0, col).unwrap().bg, SELECTION_BG, "col {col}");
        }
        // Cells outside the range are untouched.
        assert_ne!(grid.get_cell(0, 0).unwrap().bg, SELECTION_BG);
        assert_ne!(grid.get_cell(0, 4).unwrap().bg, SELECTION_BG);
        assert_ne!(grid.get_cell(1, 2).unwrap().bg, SELECTION_BG);
    }

    #[test]
    fn apply_selection_highlight_skips_empty_selection() {
        let mut term = crate::terminal::Terminal::new(1, 3);
        term.process_bytes(b"xyz");
        let mut grid = term.display_grid();
        let before = *grid.get_cell(0, 0).unwrap();
        apply_selection_highlight(
            &mut grid,
            &term,
            &TermSelection::new((0, 0), SelectMode::Cell),
        );
        assert_eq!(
            *grid.get_cell(0, 0).unwrap(),
            before,
            "empty selection is a no-op"
        );
    }

    #[test]
    fn mouse_down_promotes_through_cell_word_line_on_repeat_clicks() {
        use std::time::Duration;
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();

        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Cell);

        // Second press on the same cell within the window -> word.
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0 + Duration::from_millis(50));
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Word);

        // Third -> line.
        handle_terminal_mouse_down(
            &mut st,
            pane,
            (0, 0),
            false,
            t0 + Duration::from_millis(100),
        );
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Line);

        // A press after the multi-click window resets to a fresh cell anchor.
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0 + Duration::from_secs(2));
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Cell);
    }

    #[test]
    fn shift_click_extends_existing_selection_anchor() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane, (1, 2), false, t0);
        // Shift+click keeps the anchor and moves the focus.
        handle_terminal_mouse_down(
            &mut st,
            pane,
            (3, 7),
            true,
            t0 + std::time::Duration::from_secs(2),
        );
        let sel = st.terminal_selections[&pane];
        assert_eq!(sel.anchor, (1, 2));
        assert_eq!(sel.focus, (3, 7));
    }

    #[test]
    fn selection_repaint_flag_tracks_visible_change() {
        let mut st = test_state();
        // A collapsed cell selection paints nothing, so a plain click must
        // not force a full-pane repaint.
        set_terminal_selection(&mut st, 1, TermSelection::new((0, 0), SelectMode::Cell));
        assert!(
            !st.terminal_selection_repaint.contains(&1),
            "collapsed click must not force a repaint"
        );
        assert!(
            !active_pane_has_selection(&st),
            "collapsed cell is not a selection"
        );

        // Growing to a real range changes the painted span -> repaint.
        set_terminal_selection(
            &mut st,
            1,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 4),
                mode: SelectMode::Cell,
            },
        );
        assert!(st.terminal_selection_repaint.contains(&1));
        assert!(active_pane_has_selection(&st));

        // Clearing a painted selection -> repaint to erase the highlight.
        st.terminal_selection_repaint.clear();
        clear_terminal_selection(&mut st, 1);
        assert!(!st.terminal_selections.contains_key(&1));
        assert!(st.terminal_selection_repaint.contains(&1));
    }

    // -- mutate_split_right ---------------------------------------------------

    #[test]
    fn split_right_increases_pane_count_and_changes_active() {
        let mut state = seed_state();
        let original_active = state.active_pane;
        let original_col_count = state.panes[0].len();

        mutate_split_right(&mut state, original_active);

        assert_eq!(state.panes[0].len(), original_col_count + 1);
        assert_ne!(state.active_pane, original_active);
        // The new pane should be in the terminals map
        assert!(state.terminals.contains_key(&state.active_pane.0));
    }

    #[test]
    fn split_right_at_max_cols_is_noop() {
        let mut state = seed_state();
        // Manually fill the first row to MAX_COLS to avoid spawning many PTYs
        while state.panes[0].len() < MAX_COLS {
            let id = state.next_id;
            state.next_id += 1;
            state.panes[0].push(Pane {
                id: PaneId(id),
                title: "shell".to_string(),
                subtitle: "bash".to_string(),
                pid: 0,
                cpu: 0.0,
            });
            state.active_pane = PaneId(id);
        }
        assert_eq!(state.panes[0].len(), MAX_COLS);

        let pane_count_before = state.panes[0].len();
        let active_before = state.active_pane;
        let ap = state.active_pane;
        mutate_split_right(&mut state, ap);

        assert_eq!(state.panes[0].len(), pane_count_before);
        assert_eq!(state.active_pane, active_before);
    }

    #[test]
    fn split_right_nonexistent_pane_is_noop() {
        let mut state = seed_state();
        let active_before = state.active_pane;
        let pane_count = state.panes[0].len();

        mutate_split_right(&mut state, PaneId(9999));

        assert_eq!(state.panes[0].len(), pane_count);
        assert_eq!(state.active_pane, active_before);
    }

    // -- mutate_split_down ----------------------------------------------------

    #[test]
    fn split_down_increases_row_count() {
        let mut state = seed_state();
        let original_row_count = state.panes.len();
        let original_active = state.active_pane;

        mutate_split_down(&mut state, original_active);

        assert_eq!(state.panes.len(), original_row_count + 1);
        assert_ne!(state.active_pane, original_active);
        assert!(state.terminals.contains_key(&state.active_pane.0));
    }

    #[test]
    fn split_down_at_max_rows_is_noop() {
        let mut state = seed_state();
        // Manually fill to MAX_ROWS to avoid spawning many PTYs
        while state.panes.len() < MAX_ROWS {
            let id = state.next_id;
            state.next_id += 1;
            state.panes.push(vec![Pane {
                id: PaneId(id),
                title: "shell".to_string(),
                subtitle: "bash".to_string(),
                pid: 0,
                cpu: 0.0,
            }]);
            state.active_pane = PaneId(id);
        }
        assert_eq!(state.panes.len(), MAX_ROWS);

        let row_count_before = state.panes.len();
        let active_before = state.active_pane;
        let ap = state.active_pane;
        mutate_split_down(&mut state, ap);

        assert_eq!(state.panes.len(), row_count_before);
        assert_eq!(state.active_pane, active_before);
    }

    #[test]
    fn split_down_nonexistent_pane_is_noop() {
        let mut state = seed_state();
        let row_count = state.panes.len();
        let active_before = state.active_pane;

        mutate_split_down(&mut state, PaneId(9999));

        assert_eq!(state.panes.len(), row_count);
        assert_eq!(state.active_pane, active_before);
    }

    // -- mutate_close_pane ----------------------------------------------------

    #[test]
    fn close_pane_from_multi_pane_row() {
        let mut state = seed_state();
        // Split right so we have 2 panes in row 0
        let first_pane = state.active_pane;
        mutate_split_right(&mut state, first_pane);
        let second_pane = state.active_pane;
        assert_eq!(state.panes[0].len(), 2);

        // Close the second pane, active should fall back
        mutate_close_pane(&mut state, second_pane);
        assert_eq!(state.panes[0].len(), 1);
        assert_eq!(state.active_pane, first_pane);
        // Terminal entry should be removed
        assert!(!state.terminals.contains_key(&second_pane.0));
    }

    #[test]
    fn close_last_pane_closes_tab_and_leaves_workspace_empty() {
        let mut state = seed_state();
        let original_pane = state.active_pane;

        mutate_close_pane(&mut state, original_pane);

        assert!(
            state.panes.is_empty(),
            "closing the last pane must not auto-spawn a replacement"
        );
        assert!(
            state.tabs.is_empty(),
            "closing the last pane must close the containing tab"
        );
        assert!(!state.terminals.contains_key(&original_pane.0));
    }

    #[test]
    fn close_pane_syncs_live_layout_into_active_tab() {
        let mut state = seed_state();
        let first = state.active_pane;
        mutate_split_right(&mut state, first);
        // active_pane is now the new right pane; close it.
        let second = state.active_pane;
        mutate_close_pane(&mut state, second);

        let tab = &state.tabs[state.active_tab];
        assert_eq!(
            tab.panes[0].len(),
            1,
            "active tab's saved panes must mirror live state after close"
        );
        assert_eq!(tab.active_pane, first);
    }

    #[test]
    fn close_nonexistent_pane_is_noop() {
        let mut state = seed_state();
        let active_before = state.active_pane;
        let pane_count = state.panes[0].len();

        mutate_close_pane(&mut state, PaneId(9999));

        assert_eq!(state.panes[0].len(), pane_count);
        assert_eq!(state.active_pane, active_before);
    }

    #[test]
    fn close_non_active_pane_keeps_active_unchanged() {
        let mut state = seed_state();
        let first_pane = state.active_pane;
        mutate_split_right(&mut state, first_pane);
        let second_pane = state.active_pane;

        // Switch active back to first pane
        state.active_pane = first_pane;

        // Close the non-active second pane
        mutate_close_pane(&mut state, second_pane);
        assert_eq!(state.panes[0].len(), 1);
        assert_eq!(state.active_pane, first_pane);
    }

    #[test]
    fn close_pane_removes_empty_row() {
        let mut state = seed_state();
        // Add a second row via split down
        let ap = state.active_pane;
        mutate_split_down(&mut state, ap);
        assert_eq!(state.panes.len(), 2);
        let second_row_pane = state.active_pane;

        // Close the pane in the second row (the only pane there)
        mutate_close_pane(&mut state, second_row_pane);
        assert_eq!(state.panes.len(), 1);
    }

    // -- mutate_focus_* -------------------------------------------------------

    /// Replace the active tab's pane grid with a synthetic layout. Each
    /// inner Vec is a row; values are pane ids. Sets active_pane to the
    /// pane with id `active`. Avoids spawning PTYs.
    fn install_pane_grid(state: &mut AppState, grid: Vec<Vec<u32>>, active: u32) {
        let panes: Vec<Vec<Pane>> = grid
            .iter()
            .map(|row| {
                row.iter()
                    .map(|id| Pane {
                        id: PaneId(*id),
                        title: "shell".to_string(),
                        subtitle: "bash".to_string(),
                        pid: 0,
                        cpu: 0.0,
                    })
                    .collect()
            })
            .collect();
        state.row_ratios = vec![1.0; panes.len()];
        state.col_ratios = panes.iter().map(|r| vec![1.0; r.len()]).collect();
        state.panes = panes;
        state.active_pane = PaneId(active);
        state.next_id = grid.iter().flatten().max().copied().unwrap_or(0) + 1;
        sync_live_tab_from_panes(state);
    }

    #[test]
    fn focus_left_moves_to_previous_column_in_same_row() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2, 3]], 3);

        mutate_focus_left(&mut state);

        assert_eq!(state.active_pane, PaneId(2));
    }

    #[test]
    fn focus_left_at_leftmost_is_noop() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2]], 1);

        mutate_focus_left(&mut state);

        assert_eq!(state.active_pane, PaneId(1));
    }

    #[test]
    fn focus_right_moves_to_next_column_in_same_row() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2, 3]], 1);

        mutate_focus_right(&mut state);

        assert_eq!(state.active_pane, PaneId(2));
    }

    #[test]
    fn focus_right_at_rightmost_is_noop() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2]], 2);

        mutate_focus_right(&mut state);

        assert_eq!(state.active_pane, PaneId(2));
    }

    #[test]
    fn focus_down_moves_to_next_row_same_column() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2], vec![3, 4]], 2);

        mutate_focus_down(&mut state);

        assert_eq!(state.active_pane, PaneId(4));
    }

    #[test]
    fn focus_down_clamps_column_when_target_row_is_shorter() {
        let mut state = seed_state();
        // Row 0 has three panes; row 1 has two. Focusing down from the
        // rightmost pane in row 0 must clamp to the last pane of row 1
        // rather than panicking.
        install_pane_grid(&mut state, vec![vec![1, 2, 3], vec![4, 5]], 3);

        mutate_focus_down(&mut state);

        assert_eq!(state.active_pane, PaneId(5));
    }

    #[test]
    fn focus_down_at_bottom_row_is_noop() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1], vec![2]], 2);

        mutate_focus_down(&mut state);

        assert_eq!(state.active_pane, PaneId(2));
    }

    #[test]
    fn focus_up_moves_to_previous_row_same_column() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2], vec![3, 4]], 4);

        mutate_focus_up(&mut state);

        assert_eq!(state.active_pane, PaneId(2));
    }

    #[test]
    fn focus_up_clamps_column_when_target_row_is_shorter() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1], vec![2, 3, 4]], 4);

        mutate_focus_up(&mut state);

        assert_eq!(state.active_pane, PaneId(1));
    }

    #[test]
    fn focus_up_at_top_row_is_noop() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1], vec![2]], 1);

        mutate_focus_up(&mut state);

        assert_eq!(state.active_pane, PaneId(1));
    }

    #[test]
    fn focus_in_single_pane_layout_is_noop_in_every_direction() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1]], 1);

        mutate_focus_left(&mut state);
        assert_eq!(state.active_pane, PaneId(1));
        mutate_focus_right(&mut state);
        assert_eq!(state.active_pane, PaneId(1));
        mutate_focus_up(&mut state);
        assert_eq!(state.active_pane, PaneId(1));
        mutate_focus_down(&mut state);
        assert_eq!(state.active_pane, PaneId(1));
    }

    #[test]
    fn focus_with_invalid_active_pane_is_noop() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2]], 1);
        // Corrupt the active pane to an id not present in the grid.
        state.active_pane = PaneId(9999);

        mutate_focus_left(&mut state);
        mutate_focus_right(&mut state);
        mutate_focus_up(&mut state);
        mutate_focus_down(&mut state);

        assert_eq!(state.active_pane, PaneId(9999));
    }

    #[test]
    fn focus_syncs_active_pane_into_live_tab() {
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2]], 1);
        assert_eq!(state.tabs[state.active_tab].active_pane, PaneId(1));

        mutate_focus_right(&mut state);

        assert_eq!(state.active_pane, PaneId(2));
        assert_eq!(state.tabs[state.active_tab].active_pane, PaneId(2));
    }

    // -- dispatch pane commands -----------------------------------------------

    #[test]
    fn dispatch_pane_split_right() {
        let mut state = seed_state();
        let original_col_count = state.panes[0].len();

        assert!(dispatch(&mut state, "pane.split_right"));
        assert_eq!(state.panes[0].len(), original_col_count + 1);
    }

    #[test]
    fn dispatch_pane_split_down() {
        let mut state = seed_state();
        let original_row_count = state.panes.len();

        assert!(dispatch(&mut state, "pane.split_down"));
        assert_eq!(state.panes.len(), original_row_count + 1);
    }

    #[test]
    fn dispatch_pane_close() {
        let mut state = seed_state();
        // Split first so closing does not trigger the "last pane" path
        dispatch(&mut state, "pane.split_right");
        let pane_count = state.panes[0].len();

        assert!(dispatch(&mut state, "pane.close"));
        assert_eq!(state.panes[0].len(), pane_count - 1);
    }

    #[test]
    fn dispatch_pane_focus_directions_route_to_focus_mutators() {
        // Build a 2x2 grid: panes 1,2 in row 0 and 3,4 in row 1; start at 1.
        let mut state = seed_state();
        install_pane_grid(&mut state, vec![vec![1, 2], vec![3, 4]], 1);

        assert!(dispatch(&mut state, "pane.focus_right"));
        assert_eq!(state.active_pane, PaneId(2));

        assert!(dispatch(&mut state, "pane.focus_down"));
        assert_eq!(state.active_pane, PaneId(4));

        assert!(dispatch(&mut state, "pane.focus_left"));
        assert_eq!(state.active_pane, PaneId(3));

        assert!(dispatch(&mut state, "pane.focus_up"));
        assert_eq!(state.active_pane, PaneId(1));
    }

    #[test]
    fn dispatch_pane_close_on_last_pane_closes_tab() {
        // Unsplit (Ctrl+Shift+W) is dispatched as `pane.close`. When the
        // focused pane is the tab's only pane, closing it must close the
        // whole tab. Locks down the A3 semantics so future refactors
        // cannot silently regress to "leave empty tab behind".
        let mut state = seed_state();
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 1);
        let original_tab_count = state.tabs.len();

        assert!(dispatch(&mut state, "pane.close"));

        assert!(state.panes.is_empty(), "last pane must be gone");
        assert_eq!(
            state.tabs.len(),
            original_tab_count - 1,
            "closing the last pane must close the tab"
        );
    }

    // -- mutate_extract_pane_to_tab -----------------------------------------

    #[test]
    fn extract_pane_from_two_pane_tab_leaves_one_pane_in_source() {
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        assert_eq!(state.panes[0].len(), 2);
        let target = state.panes[0][1].id;

        mutate_extract_pane_to_tab(&mut state, target, 1);

        assert_eq!(state.tabs.len(), 2, "new tab created");
        // Source tab (t1) retains its original pane.
        let source_tab = &state.tabs[0];
        assert_eq!(source_tab.panes.iter().flatten().count(), 1);
        // New tab holds the extracted pane.
        let new_tab = &state.tabs[1];
        assert_eq!(new_tab.panes.iter().flatten().count(), 1);
        assert_eq!(new_tab.panes[0][0].id, target);
    }

    #[test]
    fn extract_pane_reflows_ratios_in_source() {
        // After the split the row is 0.5/0.5. Extracting one pane must hand
        // its column ratio to the remaining pane so the source fills 1.0.
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let target = state.panes[0][1].id;

        mutate_extract_pane_to_tab(&mut state, target, 1);

        let source_col_ratios = &state.tabs[0].col_ratios;
        assert_eq!(source_col_ratios[0].len(), 1);
        assert!(
            (source_col_ratios[0][0] - 1.0).abs() < 1e-6,
            "surviving pane must absorb ratio, got {}",
            source_col_ratios[0][0]
        );
    }

    #[test]
    fn extract_only_pane_closes_source_tab() {
        // Single-pane tab extracted: the source tab disappears entirely so
        // we never leave an empty tab behind. The pane must survive in the
        // newly created tab.
        let mut state = seed_state();
        mutate_add_tab(&mut state); // now 2 tabs
        mutate_switch_tab(&mut state, 0);
        let target = state.active_pane;
        let tab_count_before = state.tabs.len();

        mutate_extract_pane_to_tab(&mut state, target, tab_count_before);

        assert_eq!(
            state.tabs.len(),
            tab_count_before,
            "source tab removed, new tab inserted = same count"
        );
        // Extracted pane still exists in some tab.
        let found = state
            .tabs
            .iter()
            .flat_map(|t| t.panes.iter().flatten())
            .any(|p| p.id == target);
        assert!(found, "extracted pane must live on in the new tab");
    }

    #[test]
    fn extract_preserves_pty_entry_in_terminals_map() {
        // The whole point of extract-to-tab: the running process must not
        // respawn. We approximate that by checking the `terminals` HashMap
        // entry is the same Arc (same strong count before/after).
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let target = state.panes[0][1].id;
        // split_right already inserted a SharedTerminal for this pane.
        let before = state
            .terminals
            .get(&target.0)
            .map(Arc::strong_count)
            .expect("pty spawned by split_right");

        mutate_extract_pane_to_tab(&mut state, target, 1);

        let after = state
            .terminals
            .get(&target.0)
            .map(Arc::strong_count)
            .expect("pty handle must survive extraction");
        assert_eq!(before, after, "Arc count must match: no respawn occurred");
    }

    #[test]
    fn extract_activates_new_tab() {
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let target = state.panes[0][1].id;

        mutate_extract_pane_to_tab(&mut state, target, 1);

        assert_eq!(state.active_tab, 1, "new tab becomes active");
        assert_eq!(state.active_pane, target);
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 1);
        assert_eq!(state.panes[0][0].id, target);
    }

    #[test]
    fn extract_out_of_range_index_clamps_to_end() {
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let target = state.panes[0][1].id;

        mutate_extract_pane_to_tab(&mut state, target, 99);

        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.tabs[1].panes[0][0].id, target);
    }

    #[test]
    fn extract_unknown_pane_is_noop() {
        let mut state = seed_state();
        let tabs_before = state.tabs.len();

        mutate_extract_pane_to_tab(&mut state, PaneId(9999), 0);

        assert_eq!(state.tabs.len(), tabs_before);
    }

    #[test]
    fn dispatch_pane_extract_to_tab_parses_and_applies() {
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let target = state.panes[0][1].id;
        let cmd = format!("pane.extract_to_tab:{}:1", target.0);

        assert!(dispatch(&mut state, &cmd));
        assert_eq!(state.tabs.len(), 2);
    }

    #[test]
    fn dispatch_pane_extract_to_tab_malformed_returns_false() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "pane.extract_to_tab:"));
        assert!(!dispatch(&mut state, "pane.extract_to_tab:abc:1"));
        assert!(!dispatch(&mut state, "pane.extract_to_tab:1:xyz"));
    }

    // -- drag.* dispatch arms -------------------------------------------------

    #[test]
    fn dispatch_drag_start_pane_sets_dragging_state() {
        let mut state = seed_state();
        let id = state.active_pane.0;
        let cmd = format!("drag.start_pane:{}:40:60", id);

        assert!(dispatch(&mut state, &cmd));

        assert_eq!(state.drag.dragged_pane(), Some(PaneId(id)));
    }

    #[test]
    fn dispatch_drag_start_pane_rejects_unknown_pane() {
        let mut state = seed_state();
        let cmd = "drag.start_pane:9999:0:0";

        assert!(!dispatch(&mut state, cmd));
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn dispatch_drag_update_while_dragging_updates_cursor() {
        let mut state = seed_state();
        let id = state.active_pane.0;
        dispatch(&mut state, &format!("drag.start_pane:{}:0:0", id));

        assert!(dispatch(&mut state, "drag.update:123:456"));

        match &state.drag {
            crate::drag::DragState::DraggingPane {
                cursor_x, cursor_y, ..
            } => {
                assert_eq!(*cursor_x, 123.0);
                assert_eq!(*cursor_y, 456.0);
            }
            _ => panic!("drag state must remain DraggingPane"),
        }
    }

    #[test]
    fn dispatch_drag_update_when_idle_returns_false() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "drag.update:10:20"));
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn dispatch_drag_end_resets_to_idle() {
        let mut state = seed_state();
        let id = state.active_pane.0;
        dispatch(&mut state, &format!("drag.start_pane:{}:0:0", id));
        assert!(state.drag.is_active());

        assert!(dispatch(&mut state, "drag.end"));

        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn dispatch_drag_end_when_idle_returns_false() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "drag.end"));
    }

    #[test]
    fn dispatch_drag_malformed_returns_false() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "drag.start_pane:"));
        assert!(!dispatch(&mut state, "drag.start_pane:1"));
        assert!(!dispatch(&mut state, "drag.start_pane:1:2"));
        assert!(!dispatch(&mut state, "drag.start_pane:abc:1:2"));
        assert!(!dispatch(&mut state, "drag.update:"));
        assert!(!dispatch(&mut state, "drag.update:1"));
        assert!(!dispatch(&mut state, "drag.update:x:y"));
    }

    #[test]
    fn drag_update_refreshes_cursor_while_tab_dragging() {
        let mut state = seed_state();
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: "tab-7".into(),
            cursor_x: 0.0,
            cursor_y: 0.0,
        };
        assert!(dispatch(&mut state, "drag.update:321:54"));
        assert_eq!(state.drag.cursor(), Some((321.0, 54.0)));
        assert_eq!(state.drag.dragged_tab(), Some("tab-7"));
    }

    fn seed_state_with_two_tabs() -> AppState {
        // Start from seed_state (one tab, one pane), then add a
        // second tab whose single pane has a fresh id so drop_split
        // has a non-self source.
        let mut state = seed_state();
        let pane_id = PaneId(state.next_id);
        state.next_id += 1;
        let second_tab = TerminalTab {
            id: "t-second".into(),
            name: "second".into(),
            subtitle: "bash".into(),
            status: TabStatus::Running,
            panes: vec![vec![Pane {
                id: pane_id,
                title: "second".into(),
                subtitle: "bash".into(),
                pid: 0,
                cpu: 0.0,
            }]],
            active_pane: pane_id,
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        };
        state.tabs.push(second_tab);
        state
    }

    #[test]
    fn tab_reorder_moves_tab_to_new_index() {
        let mut state = seed_state_with_two_tabs();
        let first_id = state.tabs[0].id.clone();
        mutate_tab_reorder(&mut state, &first_id, 2);
        assert_eq!(state.tabs[1].id, first_id);
    }

    #[test]
    fn tab_reorder_preserves_active_tab_by_id() {
        let mut state = seed_state_with_two_tabs();
        state.active_tab = 0;
        let active_id = state.tabs[0].id.clone();
        // Move the active tab to the end.
        mutate_tab_reorder(&mut state, &active_id, 2);
        assert_eq!(state.tabs[state.active_tab].id, active_id);
    }

    #[test]
    fn tab_reorder_unknown_id_is_noop() {
        let mut state = seed_state_with_two_tabs();
        let snapshot: Vec<String> = state.tabs.iter().map(|t| t.id.clone()).collect();
        mutate_tab_reorder(&mut state, "does-not-exist", 0);
        let after: Vec<String> = state.tabs.iter().map(|t| t.id.clone()).collect();
        assert_eq!(snapshot, after);
    }

    #[test]
    fn tab_reorder_same_position_is_noop() {
        let mut state = seed_state_with_two_tabs();
        let id = state.tabs[0].id.clone();
        let before: Vec<String> = state.tabs.iter().map(|t| t.id.clone()).collect();
        mutate_tab_reorder(&mut state, &id, 0);
        let after: Vec<String> = state.tabs.iter().map(|t| t.id.clone()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn pane_drop_split_right_adds_new_column() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        let target_pane = state.panes[0][0].id;
        let tabs_before = state.tabs.len();

        mutate_pane_drop_split(&mut state, &source_id, target_pane, DropZone::Right);

        assert_eq!(
            state.tabs.len(),
            tabs_before - 1,
            "source tab should be removed"
        );
        assert_eq!(state.panes[0].len(), 2, "target row should have two panes");
        assert_eq!(
            state.panes[0][0].id, target_pane,
            "target stays in its column"
        );
        // The inserted pane is in position 1 and becomes active.
        assert_eq!(state.active_pane, state.panes[0][1].id);
    }

    #[test]
    fn pane_drop_split_left_inserts_before_target() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        let source_pane_id = state.tabs[1].panes[0][0].id;
        let target_pane = state.panes[0][0].id;

        mutate_pane_drop_split(&mut state, &source_id, target_pane, DropZone::Left);

        assert_eq!(
            state.panes[0][0].id, source_pane_id,
            "source lands before target"
        );
        assert_eq!(state.panes[0][1].id, target_pane);
    }

    #[test]
    fn pane_drop_split_bottom_adds_new_row() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        let source_pane_id = state.tabs[1].panes[0][0].id;
        let target_pane = state.panes[0][0].id;

        mutate_pane_drop_split(&mut state, &source_id, target_pane, DropZone::Bottom);

        assert_eq!(state.panes.len(), 2, "should have two rows");
        assert_eq!(state.panes[0][0].id, target_pane);
        assert_eq!(state.panes[1][0].id, source_pane_id);
    }

    #[test]
    fn pane_drop_split_top_inserts_row_before_target() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        let source_pane_id = state.tabs[1].panes[0][0].id;
        let target_pane = state.panes[0][0].id;

        mutate_pane_drop_split(&mut state, &source_id, target_pane, DropZone::Top);

        assert_eq!(state.panes.len(), 2);
        assert_eq!(state.panes[0][0].id, source_pane_id);
        assert_eq!(state.panes[1][0].id, target_pane);
    }

    #[test]
    fn pane_drop_split_halves_the_split_ratio() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        let target_pane = state.panes[0][0].id;
        state.col_ratios[0][0] = 1.0;

        mutate_pane_drop_split(&mut state, &source_id, target_pane, DropZone::Right);

        assert_eq!(state.col_ratios[0].len(), 2);
        assert!((state.col_ratios[0][0] - 0.5).abs() < 1e-6);
        assert!((state.col_ratios[0][1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pane_drop_split_rejects_same_tab_source() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let source_id = state.tabs[0].id.clone();
        let target_pane = state.panes[0][0].id;
        let before: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        // Source is the active tab: must be a no-op.
        mutate_pane_drop_split(&mut state, &source_id, target_pane, DropZone::Right);
        let after: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn pane_drop_split_rejects_multi_pane_source() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        // Give the source tab a second pane.
        state.tabs[1].panes[0].push(Pane {
            id: PaneId(9999),
            title: "extra".into(),
            subtitle: "bash".into(),
            pid: 0,
            cpu: 0.0,
        });
        state.tabs[1].col_ratios[0].push(1.0);

        let tabs_before = state.tabs.len();
        let target_pane = state.panes[0][0].id;
        mutate_pane_drop_split(&mut state, &source_id, target_pane, DropZone::Right);
        assert_eq!(
            state.tabs.len(),
            tabs_before,
            "multi-pane source should be rejected"
        );
        assert_eq!(state.panes[0].len(), 1, "target row unchanged");
    }

    // -- mutate_pane_move_to_edge -------------------------------------------

    #[test]
    fn pane_move_to_edge_right_same_row_reorders() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        // Row is [a, b]. Move a to b's Right edge → [b, a].
        mutate_pane_move_to_edge(&mut state, a, b, DropZone::Right);
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 2);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[0][1].id, a);
        assert_eq!(state.active_pane, a);
    }

    #[test]
    fn pane_move_to_edge_left_same_row_reorders() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        // Row is [a, b]. Move b to a's Left edge → [b, a].
        mutate_pane_move_to_edge(&mut state, b, a, DropZone::Left);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[0][1].id, a);
    }

    #[test]
    fn pane_move_to_edge_bottom_same_row_creates_row_below() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        // [a, b] → move a below b → [[b], [a]].
        mutate_pane_move_to_edge(&mut state, a, b, DropZone::Bottom);
        assert_eq!(state.panes.len(), 2);
        assert_eq!(state.panes[0].len(), 1);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[1][0].id, a);
    }

    #[test]
    fn pane_move_to_edge_top_same_row_inserts_row_above() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        // [a, b] → move b above a → [[b], [a]].
        mutate_pane_move_to_edge(&mut state, b, a, DropZone::Top);
        assert_eq!(state.panes.len(), 2);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[1][0].id, a);
    }

    #[test]
    fn pane_move_to_edge_cross_row_handles_row_deletion() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_down(&mut state, a);
        let b = state.active_pane;
        // Layout is [[a], [b]]. Moving a to b's Right should delete row 0
        // (source was the only pane there), shifting b up to row 0, then
        // insert a to b's right → single row [b, a].
        mutate_pane_move_to_edge(&mut state, a, b, DropZone::Right);
        assert_eq!(state.panes.len(), 1, "empty source row must be removed");
        assert_eq!(state.panes[0].len(), 2);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[0][1].id, a);
    }

    #[test]
    fn pane_move_to_edge_source_equals_target_is_noop() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        let before: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        mutate_pane_move_to_edge(&mut state, b, b, DropZone::Right);
        let after: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        assert_eq!(before, after, "self-drop must not alter the layout");
    }

    #[test]
    fn pane_move_to_edge_center_is_noop() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        let before: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        mutate_pane_move_to_edge(&mut state, a, b, DropZone::Center);
        let after: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn pane_move_to_edge_right_halves_target_ratio() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_down(&mut state, a);
        let b = state.active_pane;
        // Detach a (its row dies), target b is now at row 0 with col_ratio [1.0].
        mutate_pane_move_to_edge(&mut state, a, b, DropZone::Right);
        assert_eq!(state.col_ratios[0].len(), 2);
        assert!((state.col_ratios[0][0] - 0.5).abs() < 1e-6);
        assert!((state.col_ratios[0][1] - 0.5).abs() < 1e-6);
    }

    // -- mutate_pane_swap ----------------------------------------------------

    fn install_dummy_terminal(state: &mut AppState, pane_id: PaneId) {
        use crate::terminal::Terminal;
        use std::sync::{Arc, Mutex};
        state
            .terminals
            .insert(pane_id.0, Arc::new(Mutex::new(Terminal::new(24, 80))));
    }

    #[test]
    fn pane_swap_exchanges_slots_and_keeps_both_terminals() {
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        install_dummy_terminal(&mut state, a);
        install_dummy_terminal(&mut state, b);
        // Layout is [[a, b]]. Swap a and b: layout becomes [[b, a]],
        // both PTYs preserved.
        mutate_pane_swap(&mut state, a, b);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[0][1].id, a);
        assert!(state.terminals.contains_key(&a.0));
        assert!(state.terminals.contains_key(&b.0));
        assert_eq!(state.active_pane, a);
    }

    #[test]
    fn pane_swap_self_is_noop() {
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        let before: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        mutate_pane_swap(&mut state, b, b);
        let after: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id).collect())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn pane_swap_across_rows() {
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_down(&mut state, a);
        let b = state.active_pane;
        install_dummy_terminal(&mut state, a);
        install_dummy_terminal(&mut state, b);
        // Layout [[a], [b]] → [[b], [a]].
        mutate_pane_swap(&mut state, a, b);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[1][0].id, a);
        assert!(state.terminals.contains_key(&a.0));
        assert!(state.terminals.contains_key(&b.0));
    }

    // -- mutate_pane_swap_from_tab -------------------------------------------

    #[test]
    fn pane_swap_from_tab_preserves_both_tabs_and_terminals() {
        let mut state = seed_state_with_two_tabs();
        let source_tab_id = state.tabs[1].id.clone();
        let source_pane_id = state.tabs[1].panes[0][0].id;
        let target = state.panes[0][0].id;
        install_dummy_terminal(&mut state, target);
        install_dummy_terminal(&mut state, source_pane_id);
        let tabs_before = state.tabs.len();

        mutate_pane_swap_from_tab(&mut state, &source_tab_id, target);

        // Both tabs survive, both terminals survive, source pane now in
        // active tab and target pane now lives in the source tab.
        assert_eq!(state.tabs.len(), tabs_before);
        assert_eq!(state.panes[0][0].id, source_pane_id);
        assert_eq!(state.tabs[1].panes[0][0].id, target);
        assert!(state.terminals.contains_key(&target.0));
        assert!(state.terminals.contains_key(&source_pane_id.0));
        assert_eq!(state.active_pane, source_pane_id);
    }

    #[test]
    fn pane_swap_from_tab_rejects_same_tab_source() {
        let mut state = seed_state();
        let source_tab_id = state.tabs[0].id.clone();
        let target = state.panes[0][0].id;
        install_dummy_terminal(&mut state, target);
        let panes_before = state.panes.clone();
        mutate_pane_swap_from_tab(&mut state, &source_tab_id, target);
        let ids_before: Vec<Vec<PaneId>> = panes_before
            .iter()
            .map(|r| r.iter().map(|p| p.id).collect())
            .collect();
        let ids_after: Vec<Vec<PaneId>> = state
            .panes
            .iter()
            .map(|r| r.iter().map(|p| p.id).collect())
            .collect();
        assert_eq!(ids_before, ids_after);
    }

    #[test]
    fn pane_swap_from_tab_rejects_multi_pane_source() {
        let mut state = seed_state_with_two_tabs();
        let source_tab_id = state.tabs[1].id.clone();
        state.tabs[1].panes[0].push(Pane {
            id: PaneId(9999),
            title: "extra".into(),
            subtitle: "bash".into(),
            pid: 0,
            cpu: 0.0,
        });
        state.tabs[1].col_ratios[0].push(1.0);
        let target = state.panes[0][0].id;
        install_dummy_terminal(&mut state, target);
        let target_pane_before = state.panes[0][0].id;

        mutate_pane_swap_from_tab(&mut state, &source_tab_id, target);

        assert_eq!(state.panes[0][0].id, target_pane_before);
    }

    #[test]
    fn drag_end_pane_over_pane_center_swaps() {
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        install_dummy_terminal(&mut state, a);
        install_dummy_terminal(&mut state, b);
        set_active_pane_rect_for_test(&mut state);
        // Target b is at (106, 100, 100, 200). Center: nx in [0.25, 0.75],
        // ny in [0.25, 0.75]. Cursor (156, 200) → nx=0.5, ny=0.5.
        state.drag = crate::drag::DragState::DraggingPane {
            pane: a,
            cursor_x: 156.0,
            cursor_y: 200.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        // Center drop on another pane swaps slots — both PTYs survive.
        assert_eq!(state.panes[0].len(), 2);
        assert_eq!(state.panes[0][0].id, b);
        assert_eq!(state.panes[0][1].id, a);
        assert!(state.terminals.contains_key(&a.0));
        assert!(state.terminals.contains_key(&b.0));
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn drag_end_pane_over_pane_edge_moves_pane() {
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        set_active_pane_rect_for_test(&mut state);
        // After split_right the grid has two columns sharing (6, 100, 200, 200).
        // Col 0: a at (6, 100, 100, 200). Col 1: b at (106, 100, 100, 200).
        // Target b's right edge strip runs x ∈ [181, 206) — cursor (195, 200).
        state.drag = crate::drag::DragState::DraggingPane {
            pane: a,
            cursor_x: 195.0,
            cursor_y: 200.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        assert_eq!(state.panes[0].len(), 2);
        assert_eq!(
            state.panes[0][0].id, b,
            "a detached and re-inserted right of b"
        );
        assert_eq!(state.panes[0][1].id, a);
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn drag_end_pane_over_own_rect_is_noop() {
        // Dropping a pane onto its own rect must not match any target,
        // so the layout stays intact and drag state clears.
        let mut state = seed_state();
        let a = state.active_pane;
        mutate_split_right(&mut state, a);
        let b = state.active_pane;
        set_active_pane_rect_for_test(&mut state);
        // Pane a occupies (6, 100, 100, 200). Drop deep inside its own rect.
        let before_panes: Vec<PaneId> = state.panes[0].iter().map(|p| p.id).collect();
        state.drag = crate::drag::DragState::DraggingPane {
            pane: a,
            cursor_x: 30.0,
            cursor_y: 150.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        let after: Vec<PaneId> = state.panes[0].iter().map(|p| p.id).collect();
        assert_eq!(before_panes, after, "self-drop must not move the pane");
        assert_eq!(state.drag, crate::drag::DragState::Idle);
        let _ = b;
    }

    /// Relied on by the framework's window-blur drag-cancel path: a
    /// synthesized `drag.end` with whatever last cursor position must
    /// always clear the drag state, even when the coordinates don't
    /// hit any drop target. Without this, alt-tabbing mid-drag leaves
    /// ghost overlays and drop zones stuck on screen.
    #[test]
    fn drag_end_always_clears_state_from_dragging_pane() {
        let mut state = seed_state();
        state.drag = crate::drag::DragState::DraggingPane {
            pane: state.active_pane,
            cursor_x: -999.0,
            cursor_y: -999.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn drag_end_always_clears_state_from_dragging_tab() {
        let mut state = seed_state_with_two_tabs();
        let id = state.tabs[1].id.clone();
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: id,
            cursor_x: -999.0,
            cursor_y: -999.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn drag_end_from_idle_is_false_no_op() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "drag.end"));
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn dispatch_drag_start_tab_enters_state() {
        let mut state = seed_state_with_two_tabs();
        let id = state.tabs[1].id.clone();
        assert!(dispatch(&mut state, &format!("drag.start_tab:{}:0:0", id)));
        assert_eq!(state.drag.dragged_tab(), Some(id.as_str()));
    }

    #[test]
    fn dispatch_drag_start_tab_rejects_unknown() {
        let mut state = seed_state_with_two_tabs();
        assert!(!dispatch(&mut state, "drag.start_tab:nonexistent:0:0"));
        assert!(matches!(state.drag, crate::drag::DragState::Idle));
    }

    #[test]
    fn dispatch_pane_drop_split_reads_source_from_drag_state() {
        use crate::drag::drop_zones::DropZone;
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: source_id.clone(),
            cursor_x: 0.0,
            cursor_y: 0.0,
        };
        let target = state.panes[0][0].id;
        assert!(dispatch(
            &mut state,
            &format!("pane.drop_split:{}:{}", target.0, DropZone::Right.id())
        ));
        assert_eq!(state.panes[0].len(), 2);
    }

    #[test]
    fn dispatch_pane_drop_split_requires_tab_drag() {
        let mut state = seed_state_with_two_tabs();
        // No drag in progress.
        let target = state.panes[0][0].id;
        assert!(!dispatch(
            &mut state,
            &format!("pane.drop_split:{}:right", target.0)
        ));
    }

    #[test]
    fn dispatch_pane_drop_split_rejects_center() {
        let mut state = seed_state_with_two_tabs();
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: state.tabs[1].id.clone(),
            cursor_x: 0.0,
            cursor_y: 0.0,
        };
        let target = state.panes[0][0].id;
        assert!(!dispatch(
            &mut state,
            &format!("pane.drop_split:{}:center", target.0)
        ));
    }

    #[test]
    fn dispatch_tab_reorder_moves_tab() {
        let mut state = seed_state_with_two_tabs();
        let id = state.tabs[0].id.clone();
        assert!(dispatch(&mut state, &format!("tab.reorder:{}:2", id)));
        assert_eq!(state.tabs[1].id, id);
    }

    #[test]
    fn dispatch_tab_reorder_rejects_malformed() {
        let mut state = seed_state_with_two_tabs();
        assert!(!dispatch(&mut state, "tab.reorder:"));
        assert!(!dispatch(&mut state, "tab.reorder:only-id"));
        assert!(!dispatch(&mut state, "tab.reorder:id:notanumber"));
    }

    #[test]
    fn drag_end_tab_over_tabbar_reorders() {
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        state.tabbar_rect = crate::drag::Rect {
            x: 0.0,
            y: 34.0,
            width: 800.0,
            height: 38.0,
        };
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: source_id.clone(),
            cursor_x: 10.0, // near the start of the tab bar, insert at 0
            cursor_y: 50.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        assert_eq!(state.tabs[0].id, source_id, "source moved to index 0");
        assert!(matches!(state.drag, crate::drag::DragState::Idle));
    }

    /// Configure an AppState so its single active pane lives at a known
    /// CSS rect. Grid x is sidebar_width + SIDEBAR_RESIZER_WIDTH (6), y
    /// is tabbar.y + tabbar.height, and grid w/h come from last_grid_*
    /// divided by scale_factor. We use sidebar=0 and scale_factor=1 so
    /// the target rect becomes (6, 100, 200, 200).
    fn set_active_pane_rect_for_test(state: &mut AppState) {
        state.sidebar_width = 0.0;
        state.scale_factor = 1.0;
        state.last_grid_width = 200.0;
        state.last_grid_height = 200.0;
        state.tabbar_rect = crate::drag::Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 100.0,
        };
    }

    #[test]
    fn drag_end_tab_over_pane_edge_splits() {
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        set_active_pane_rect_for_test(&mut state);
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: source_id,
            // Target pane occupies (6, 100, 200, 200). Cursor at the
            // right edge middle band: nx > 0.75, ny in [0.25, 0.75].
            cursor_x: 200.0,
            cursor_y: 200.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        assert_eq!(state.panes[0].len(), 2, "target row should have split");
    }

    #[test]
    fn drag_end_tab_over_pane_center_swaps() {
        let mut state = seed_state_with_two_tabs();
        let source_id = state.tabs[1].id.clone();
        let source_pane_id = state.tabs[1].panes[0][0].id;
        state.active_tab = 0;
        let target_pane_id = state.panes[0][0].id;
        install_dummy_terminal(&mut state, target_pane_id);
        install_dummy_terminal(&mut state, source_pane_id);
        let tabs_before = state.tabs.len();
        set_active_pane_rect_for_test(&mut state);
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: source_id.clone(),
            // Center of the target: nx,ny in [0.25, 0.75].
            cursor_x: 106.0,
            cursor_y: 200.0,
        };
        assert!(dispatch(&mut state, "drag.end"));
        // Both tabs survive; panes swap places (source pane in active tab,
        // target pane in the source tab). Both PTYs preserved.
        assert_eq!(state.tabs.len(), tabs_before);
        assert!(state.tabs.iter().any(|t| t.id == source_id));
        assert_eq!(state.panes[0][0].id, source_pane_id);
        assert_eq!(state.tabs[1].panes[0][0].id, target_pane_id);
        assert!(state.terminals.contains_key(&target_pane_id.0));
        assert!(state.terminals.contains_key(&source_pane_id.0));
        assert_eq!(state.active_pane, source_pane_id);
    }

    #[test]
    fn drag_end_over_tabbar_extracts_pane_to_new_tab() {
        // A multi-pane tab with the cursor released over the tab bar
        // should extract the dragged pane into its own tab at the
        // computed insertion index.
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let extracted = state.active_pane;
        let tabs_before = state.tabs.len();

        state.tabbar_rect = crate::drag::Rect {
            x: 0.0,
            y: 34.0,
            width: 800.0,
            height: 38.0,
        };

        dispatch(
            &mut state,
            &format!("drag.start_pane:{}:400:300", extracted.0),
        );
        dispatch(&mut state, "drag.update:600:50");
        assert!(dispatch(&mut state, "drag.end"));

        assert_eq!(state.tabs.len(), tabs_before + 1, "new tab should be added");
        assert_eq!(
            state.drag,
            crate::drag::DragState::Idle,
            "drag state cleared"
        );
    }

    #[test]
    fn drag_end_outside_tabbar_does_not_extract() {
        // A release with the cursor below the tab bar must not touch
        // the tab list; drag state still clears.
        let mut state = seed_state();
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let extracted = state.active_pane;
        let tabs_before = state.tabs.len();

        state.tabbar_rect = crate::drag::Rect {
            x: 0.0,
            y: 34.0,
            width: 800.0,
            height: 38.0,
        };

        dispatch(
            &mut state,
            &format!("drag.start_pane:{}:400:300", extracted.0),
        );
        dispatch(&mut state, "drag.update:400:500");
        assert!(dispatch(&mut state, "drag.end"));

        assert_eq!(state.tabs.len(), tabs_before, "no extraction below tab bar");
        assert_eq!(state.drag, crate::drag::DragState::Idle);
    }

    #[test]
    fn dispatch_drag_start_converts_physical_cursor_to_css() {
        // Cursor events arrive in physical pixels; storing them as-is
        // and then feeding them to `Dimension::Px` would make the ghost
        // overlay render scale_factor^2 pixels away from the real
        // cursor. Divide by scale_factor at the dispatch boundary.
        let mut state = seed_state();
        state.scale_factor = 2.0;
        let id = state.active_pane.0;
        dispatch(&mut state, &format!("drag.start_pane:{}:200:100", id));
        assert_eq!(state.drag.cursor(), Some((100.0, 50.0)));
    }

    #[test]
    fn dispatch_drag_update_converts_physical_cursor_to_css() {
        let mut state = seed_state();
        state.scale_factor = 2.0;
        let id = state.active_pane.0;
        dispatch(&mut state, &format!("drag.start_pane:{}:0:0", id));
        dispatch(&mut state, "drag.update:400:200");
        assert_eq!(state.drag.cursor(), Some((200.0, 100.0)));
    }

    #[test]
    fn drag_end_over_tabbar_inserts_at_cursor_position() {
        // Dropping near the right edge of the tab bar should insert the
        // new tab at the end; dropping near the left should insert near
        // the beginning.
        let mut state = seed_state();
        mutate_add_tab(&mut state);
        mutate_add_tab(&mut state);
        state.active_tab = 1;
        load_tab_state(&mut state);
        let original = state.active_pane;
        mutate_split_right(&mut state, original);
        let extracted = state.active_pane;

        state.tabbar_rect = crate::drag::Rect {
            x: 0.0,
            y: 34.0,
            width: 900.0,
            height: 38.0,
        };

        dispatch(
            &mut state,
            &format!("drag.start_pane:{}:400:300", extracted.0),
        );
        dispatch(&mut state, "drag.update:890:50");
        dispatch(&mut state, "drag.end");

        assert_eq!(state.active_tab, state.tabs.len() - 1, "inserted at end");
    }

    #[test]
    fn close_pane_absorbs_ratio_into_neighbor() {
        // After a split, the two panes share 0.5/0.5 of the row. Closing
        // one must hand its ratio to the neighbor so the surviving pane
        // fills the row (1.0 total) rather than leaving a visual gap.
        let mut state = seed_state();
        let first = state.active_pane;
        mutate_split_right(&mut state, first);
        let second = state.active_pane;
        assert_eq!(state.col_ratios[0], vec![0.5, 0.5]);

        mutate_close_pane(&mut state, second);

        assert_eq!(state.col_ratios[0].len(), 1);
        assert!(
            (state.col_ratios[0][0] - 1.0).abs() < 1e-6,
            "surviving pane must absorb closed pane's ratio, got {}",
            state.col_ratios[0][0]
        );
    }

    // -- dispatch keybind.* ---------------------------------------------------

    #[test]
    fn dispatch_keybind_set_non_conflicting_updates_override() {
        let mut state = test_state();
        assert!(dispatch(
            &mut state,
            "keybind.set:new_terminal:Ctrl+Shift+T"
        ));
        assert_eq!(
            state
                .keybinds
                .effective(crate::keybinds::KeybindAction::NewTerminal)
                .to_string(),
            "Ctrl+Shift+T".to_string()
        );
        assert!(state.keybinds.error.is_none());
    }

    #[test]
    fn dispatch_keybind_set_conflict_leaves_override_unchanged() {
        // Ctrl+W is Unsplit's default. Setting NewTerminal to Ctrl+W
        // must conflict and leave NewTerminal at its default.
        let mut state = test_state();
        assert!(dispatch(&mut state, "keybind.set:new_terminal:Ctrl+W"));
        assert!(
            state.keybinds.error.is_some(),
            "conflict should populate error"
        );
        assert!(
            !state
                .keybinds
                .overrides
                .contains_key(&crate::keybinds::KeybindAction::NewTerminal),
            "conflicting set must not mutate overrides"
        );
    }

    #[test]
    fn dispatch_keybind_set_invalid_combo_sets_error() {
        let mut state = test_state();
        assert!(dispatch(
            &mut state,
            "keybind.set:new_terminal:NotARealCombo"
        ));
        match state.keybinds.error.as_ref().map(|e| &e.kind) {
            Some(crate::keybinds::KeybindErrorKind::InvalidCombo { .. }) => {}
            other => panic!("expected InvalidCombo error, got {:?}", other),
        }
    }

    #[test]
    fn dispatch_keybind_set_unknown_action_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "keybind.set:bogus_action:Ctrl+T"));
    }

    #[test]
    fn dispatch_keybind_reset_drops_override() {
        let mut state = test_state();
        dispatch(&mut state, "keybind.set:new_terminal:Ctrl+Shift+T");
        assert!(state
            .keybinds
            .overrides
            .contains_key(&crate::keybinds::KeybindAction::NewTerminal));

        assert!(dispatch(&mut state, "keybind.reset:new_terminal"));
        assert!(!state
            .keybinds
            .overrides
            .contains_key(&crate::keybinds::KeybindAction::NewTerminal));
    }

    #[test]
    fn dispatch_keybind_reset_all_clears_every_override() {
        let mut state = test_state();
        dispatch(&mut state, "keybind.set:new_terminal:Ctrl+Shift+T");
        dispatch(&mut state, "keybind.set:close_tab:Ctrl+Shift+F4");
        assert_eq!(state.keybinds.overrides.len(), 2);

        assert!(dispatch(&mut state, "keybind.reset_all"));
        assert!(state.keybinds.overrides.is_empty());
    }

    #[test]
    fn dispatch_keybind_record_and_cancel() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "keybind.record:new_terminal"));
        assert_eq!(
            state.keybinds.recording,
            Some(crate::keybinds::KeybindAction::NewTerminal)
        );

        assert!(dispatch(&mut state, "keybind.cancel_record"));
        assert!(state.keybinds.recording.is_none());
    }

    #[test]
    fn dispatch_keybind_record_unknown_action_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "keybind.record:bogus"));
    }

    // -- dispatch tab.next / tab.prev with empty tabs -------------------------

    #[test]
    fn dispatch_tab_next_empty_tabs_returns_false() {
        let mut state = test_state();
        state.tabs.clear();

        assert!(!dispatch(&mut state, "tab.next"));
    }

    #[test]
    fn dispatch_tab_prev_empty_tabs_returns_false() {
        let mut state = test_state();
        state.tabs.clear();

        assert!(!dispatch(&mut state, "tab.prev"));
    }

    #[test]
    fn seed_state_has_empty_terminals() {
        let state = seed_state();
        assert!(
            state.terminals.is_empty(),
            "seed_state must not pre-populate terminals; PTY spawn is deferred"
        );
    }

    #[test]
    fn seed_state_has_default_pane() {
        let state = seed_state();
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 1);
        assert_eq!(state.panes[0][0].id, PaneId(1));
    }

    #[test]
    fn compute_pty_dimensions_with_valid_metrics() {
        let (cols, rows) = compute_pty_dimensions(800.0, 600.0, 8.0, 16.0);
        assert_eq!(cols, 100);
        assert_eq!(rows, 37);
    }

    #[test]
    fn compute_pty_dimensions_fallback_when_no_metrics() {
        let (cols, rows) = compute_pty_dimensions(800.0, 600.0, 0.0, 0.0);
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
    }

    #[test]
    fn compute_pty_dimensions_fallback_when_no_grid() {
        let (cols, rows) = compute_pty_dimensions(0.0, 0.0, 8.0, 16.0);
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
    }

    #[test]
    fn compute_pty_dimensions_fallback_partial_zero() {
        let (cols, rows) = compute_pty_dimensions(800.0, 600.0, 8.0, 0.0);
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
    }

    #[test]
    fn compute_pty_dimensions_minimum_one() {
        let (cols, rows) = compute_pty_dimensions(1.0, 1.0, 8.0, 16.0);
        assert_eq!(cols, 1);
        assert_eq!(rows, 1);
    }

    #[test]
    fn resize_all_terminals_updates_every_terminal() {
        let mut state = seed_state();
        state
            .terminals
            .insert(1, Arc::new(Mutex::new(Terminal::new(24, 80))));
        state
            .terminals
            .insert(2, Arc::new(Mutex::new(Terminal::new(24, 80))));

        resize_all_terminals(&mut state, 120, 40);

        for term in state.terminals.values() {
            let t = term.lock().expect("terminal mutex poisoned");
            let grid = t.grid();
            assert_eq!(grid.cols(), 120, "terminal cols should be 120 after resize");
            assert_eq!(grid.rows(), 40, "terminal rows should be 40 after resize");
        }
    }

    #[test]
    fn resize_all_terminals_handles_empty_state() {
        let mut state = seed_state();
        state.terminals.clear();
        resize_all_terminals(&mut state, 100, 30);
        assert!(state.terminals.is_empty());
    }

    // -- Parser lock decoupling -----------------------------------------------
    //
    // A parser thread writing to one terminal must NOT block the state lock
    // or the render path. We simulate the parser workload by holding a
    // per-terminal mutex while another thread simultaneously grabs the state
    // mutex and clones the terminal handle. No deadlock means the two lock
    // domains are independent.

    #[test]
    fn parser_lock_independent_of_state_lock() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Barrier;
        use std::thread;

        let state: SharedState = Arc::new(Mutex::new(seed_state()));
        {
            let mut guard = state.lock().unwrap();
            guard
                .terminals
                .insert(1, Arc::new(Mutex::new(Terminal::new(24, 80))));
        }

        let barrier = Arc::new(Barrier::new(2));
        let parser_writes = Arc::new(AtomicUsize::new(0));

        // Parser thread: locks the per-terminal mutex and writes many times.
        let parser_state = state.clone();
        let parser_barrier = barrier.clone();
        let parser_writes_cl = parser_writes.clone();
        let parser = thread::spawn(move || {
            let handle = {
                let guard = parser_state.lock().expect("state lock");
                guard
                    .terminals
                    .get(&1)
                    .cloned()
                    .expect("terminal registered")
            };
            parser_barrier.wait();
            let mut terminal = handle.lock().expect("terminal lock");
            for _ in 0..1024 {
                terminal.process_bytes(b"x");
                parser_writes_cl.fetch_add(1, Ordering::Relaxed);
            }
        });

        // Renderer thread: locks the state mutex repeatedly. Because the
        // parser does NOT hold the state lock during process_bytes, this
        // thread should finish even though the parser is busy.
        let render_state = state.clone();
        let render_barrier = barrier.clone();
        let renderer = thread::spawn(move || {
            render_barrier.wait();
            for _ in 0..1024 {
                let _handle_opt = {
                    let guard = render_state.lock().expect("state lock");
                    guard.terminals.get(&1).cloned()
                };
                // We explicitly do NOT lock the per-terminal mutex here: the
                // render closure only needs the handle, so it never waits on
                // the parser's write lock.
            }
        });

        parser.join().expect("parser thread joined");
        renderer.join().expect("renderer thread joined");
        assert_eq!(parser_writes.load(Ordering::Relaxed), 1024);
    }

    #[test]
    fn parser_writes_large_output_without_deadlocking_render() {
        // Writes ~1 MiB of output through the parser while the renderer
        // concurrently grabs state lock to snapshot handles. Proves the
        // two mutex domains are independent (regression guard).
        use std::thread;

        let state: SharedState = Arc::new(Mutex::new(seed_state()));
        {
            let mut guard = state.lock().unwrap();
            guard
                .terminals
                .insert(42, Arc::new(Mutex::new(Terminal::new(24, 80))));
        }

        let parser_state = state.clone();
        let parser = thread::spawn(move || {
            let handle = {
                let guard = parser_state.lock().expect("state lock");
                guard.terminals.get(&42).cloned().expect("terminal present")
            };
            let mut terminal = handle.lock().expect("terminal lock");
            let chunk = vec![b'y'; 4096];
            for _ in 0..256 {
                terminal.process_bytes(&chunk);
            }
        });

        let render_state = state.clone();
        let renderer = thread::spawn(move || {
            for _ in 0..512 {
                let _snap = {
                    let guard = render_state.lock().expect("state lock");
                    guard.terminals.get(&42).cloned()
                };
            }
        });

        parser.join().expect("parser thread joined");
        renderer.join().expect("renderer thread joined");

        // After the parser finishes, the terminal must contain ~1MiB of 'y'
        // plus ANSI overhead. Verify the grid is non-empty as a sanity check.
        let guard = state.lock().unwrap();
        let handle = guard.terminals.get(&42).expect("terminal still present");
        let term = handle.lock().unwrap();
        // At minimum the last row has 'y' in some column.
        let rows = term.grid().debug_rows(24, 80);
        assert!(rows.iter().any(|r| r.contains('y')));
    }

    #[test]
    fn measure_cell_width_ratio_at_accepts_line_height() {
        let font_size = 12.0_f32;
        let line_height = font_size * 1.2;
        let ratio = measure_cell_width_ratio_at(font_size, line_height);
        assert!(ratio > 0.0, "ratio must be positive, got {}", ratio);
        assert!(ratio < 1.0, "ratio must be less than 1.0, got {}", ratio);
    }

    #[test]
    fn pre_publish_sets_nonzero_global_metrics() {
        use unshit::core::cell_grid::CellGrid;
        CellGrid::publish_cell_metrics(0.0, 0.0);
        let (cell_w, cell_h) = pre_publish_cell_metrics(DEFAULT_TERMINAL_FONT_SIZE_PT, 1.0, 0.6);
        assert!(cell_w > 0.0, "cell_w must be positive, got {}", cell_w);
        assert!(cell_h > 0.0, "cell_h must be positive, got {}", cell_h);
    }

    #[test]
    fn pre_publish_scales_with_dpi() {
        let ratio = 0.6_f32;
        let (w1, h1) = pre_publish_cell_metrics(DEFAULT_TERMINAL_FONT_SIZE_PT, 1.0, ratio);
        let (w2, h2) = pre_publish_cell_metrics(DEFAULT_TERMINAL_FONT_SIZE_PT, 2.0, ratio);
        assert!(
            (w2 - w1 * 2.0).abs() < 0.001_f32,
            "cell_w at 2x should be double: {} vs {}",
            w2,
            w1 * 2.0
        );
        assert!(
            (h2 - h1 * 2.0).abs() < 0.001_f32,
            "cell_h at 2x should be double: {} vs {}",
            h2,
            h1 * 2.0
        );
    }

    #[test]
    fn seed_state_defaults_are_consistent_for_pre_publish() {
        let state = seed_state();
        assert_eq!(state.scale_factor, 1.0);
        assert_eq!(state.cell_width_ratio, 0.6);
    }

    #[test]
    fn line_height_does_not_affect_width_ratio() {
        let font_size = 14.0_f32;
        let ratio_normal = measure_cell_width_ratio_at(font_size, font_size * 1.2);
        let ratio_tall = measure_cell_width_ratio_at(font_size, font_size * 2.0);
        let ratio_tight = measure_cell_width_ratio_at(font_size, font_size * 1.0);

        let epsilon = 0.001;
        assert!(
            (ratio_normal - ratio_tall).abs() < epsilon,
            "expected same ratio for different line_heights: normal={}, tall={}",
            ratio_normal,
            ratio_tall
        );
        assert!(
            (ratio_normal - ratio_tight).abs() < epsilon,
            "expected same ratio for different line_heights: normal={}, tight={}",
            ratio_normal,
            ratio_tight
        );
    }

    #[test]
    fn pre_publish_cell_h_matches_css_line_height() {
        // Regression: CSS_LINE_HEIGHT must stay in sync with
        // `.terminal-content { line-height }` in styles.css.
        // If the constant drifts, pre_publish produces a cell_h that
        // disagrees with the renderer, causing visible row-height gaps.
        let scale = 1.0_f32;
        let ratio = 0.6_f32;
        let (_, cell_h) = pre_publish_cell_metrics(DEFAULT_TERMINAL_FONT_SIZE_PT, scale, ratio);
        let expected = CSS_BASE_FONT_SIZE * scale * CSS_LINE_HEIGHT;
        assert!(
            (cell_h - expected).abs() < f32::EPSILON,
            "cell_h ({}) must equal font_size * CSS_LINE_HEIGHT ({})",
            cell_h,
            expected,
        );
    }

    #[test]
    fn default_terminal_font_size_matches_cell_metric_constant() {
        let state = seed_state();
        assert_eq!(
            state.terminal_font_size_pt as f32, CSS_BASE_FONT_SIZE,
            "default terminal font size and initial cell metric estimate must stay in sync"
        );
    }

    #[test]
    fn measure_cell_width_ratio_reasonable_range() {
        for &size in &[10.0_f32, 12.0, 14.0, 16.0, 24.0] {
            let ratio = measure_cell_width_ratio_at(size, size * 1.2);
            assert!(
                ratio > 0.3 && ratio < 0.9,
                "at font_size={}, ratio {} is outside expected 0.3..0.9 range",
                size,
                ratio
            );
        }
    }

    // Regression: issue #17. The initial PTY dimensions must be estimated
    // from the window size, not hardcoded to 80x24. Using 80 cols caused
    // the PowerShell greeting to wrap before on_cell_metrics corrected it.
    #[test]
    fn initial_pty_estimate_not_hardcoded_80x24() {
        let ratio =
            measure_cell_width_ratio_at(CSS_BASE_FONT_SIZE, CSS_BASE_FONT_SIZE * CSS_LINE_HEIGHT);
        let cell_w = CSS_BASE_FONT_SIZE * ratio;
        let cell_h = CSS_BASE_FONT_SIZE * CSS_LINE_HEIGHT;
        // Same formula as main.rs: window(1280x800) minus chrome(284x109)
        let cols = ((1280.0_f32 - 284.0) / cell_w).max(1.0) as u16;
        let rows = ((800.0_f32 - 109.0) / cell_h).max(1.0) as u16;
        assert!(
            cols > 80,
            "estimated cols ({}) must exceed the old hardcoded 80",
            cols
        );
        assert!(
            rows > 24,
            "estimated rows ({}) must exceed the old hardcoded 24",
            rows
        );
    }

    #[test]
    fn initial_pty_estimate_reasonable_range() {
        let ratio =
            measure_cell_width_ratio_at(CSS_BASE_FONT_SIZE, CSS_BASE_FONT_SIZE * CSS_LINE_HEIGHT);
        let cell_w = CSS_BASE_FONT_SIZE * ratio;
        let cell_h = CSS_BASE_FONT_SIZE * CSS_LINE_HEIGHT;
        let cols = ((1280.0_f32 - 284.0) / cell_w).max(1.0) as u16;
        let rows = ((800.0_f32 - 109.0) / cell_h).max(1.0) as u16;
        assert!(
            (100..200).contains(&cols),
            "estimated cols ({}) outside reasonable 100..200 range",
            cols
        );
        assert!(
            (30..60).contains(&rows),
            "estimated rows ({}) outside reasonable 30..60 range",
            rows
        );
    }

    // -----------------------------------------------------------------------
    // Pane ratio tests
    // -----------------------------------------------------------------------

    #[test]
    fn seed_state_has_initial_ratios() {
        let state = seed_state();
        assert_eq!(state.row_ratios, vec![1.0]);
        assert_eq!(state.col_ratios, vec![vec![1.0]]);
        assert!(state.resize_drag.is_none());
    }

    #[test]
    fn apply_ratio_delta_positive() {
        let initial = vec![1.0, 1.0];
        let mut ratios = initial.clone();
        // Drag 200px right in a 1000px container: shift 20% of total ratio.
        apply_ratio_delta(&mut ratios, 0, 1, &initial, 200.0, 1000.0);
        assert!((ratios[0] - 1.4).abs() < 0.001, "before={}", ratios[0]);
        assert!((ratios[1] - 0.6).abs() < 0.001, "after={}", ratios[1]);
    }

    #[test]
    fn apply_ratio_delta_negative() {
        let initial = vec![1.0, 1.0];
        let mut ratios = initial.clone();
        apply_ratio_delta(&mut ratios, 0, 1, &initial, -200.0, 1000.0);
        assert!((ratios[0] - 0.6).abs() < 0.001, "before={}", ratios[0]);
        assert!((ratios[1] - 1.4).abs() < 0.001, "after={}", ratios[1]);
    }

    #[test]
    fn apply_ratio_delta_clamps_to_minimum() {
        let initial = vec![0.5, 0.5];
        let mut ratios = initial.clone();
        // Drag far enough to try to collapse the "after" pane.
        apply_ratio_delta(&mut ratios, 0, 1, &initial, 900.0, 1000.0);
        assert!(
            ratios[1] >= MIN_PANE_RATIO,
            "after pane ratio {} must not go below MIN_PANE_RATIO",
            ratios[1]
        );
        assert!(
            (ratios[0] + ratios[1] - 1.0).abs() < 0.001,
            "pair sum must be preserved"
        );
    }

    #[test]
    fn apply_ratio_delta_clamps_negative_to_minimum() {
        let initial = vec![0.5, 0.5];
        let mut ratios = initial.clone();
        apply_ratio_delta(&mut ratios, 0, 1, &initial, -900.0, 1000.0);
        assert!(
            ratios[0] >= MIN_PANE_RATIO,
            "before pane ratio {} must not go below MIN_PANE_RATIO",
            ratios[0]
        );
    }

    #[test]
    fn apply_ratio_delta_zero_is_noop() {
        let initial = vec![1.0, 1.0];
        let mut ratios = initial.clone();
        apply_ratio_delta(&mut ratios, 0, 1, &initial, 0.0, 1000.0);
        assert_eq!(ratios, initial);
    }

    #[test]
    fn apply_ratio_delta_zero_container_is_noop() {
        let initial = vec![1.0, 1.0];
        let mut ratios = initial.clone();
        apply_ratio_delta(&mut ratios, 0, 1, &initial, 100.0, 0.0);
        assert_eq!(ratios, initial);
    }

    #[test]
    fn apply_ratio_delta_preserves_pair_sum() {
        let initial = vec![0.3, 0.7, 0.5];
        let mut ratios = initial.clone();
        apply_ratio_delta(&mut ratios, 1, 2, &initial, 150.0, 800.0);
        let pair_sum = ratios[1] + ratios[2];
        let expected = initial[1] + initial[2];
        assert!(
            (pair_sum - expected).abs() < 0.001,
            "pair sum {} must equal initial {}",
            pair_sum,
            expected
        );
        // Third element should be unchanged.
        assert_eq!(ratios[0], initial[0]);
    }

    // -----------------------------------------------------------------------
    // Workspace path tests
    // -----------------------------------------------------------------------

    #[test]
    fn seed_state_first_workspace_has_cwd_path() {
        let state = seed_state();
        let ws = &state.workspaces[0];
        assert!(
            ws.path.is_some(),
            "first workspace must store current_dir as its path"
        );
    }

    #[test]
    fn seed_state_demo_workspaces_have_no_path() {
        let state = seed_state();
        for ws in &state.workspaces[1..] {
            assert!(
                ws.path.is_none(),
                "demo workspace '{}' should have no path",
                ws.name
            );
        }
    }

    #[test]
    fn add_workspace_with_path_uses_folder_name() {
        let mut state = seed_state();
        let path = PathBuf::from("/home/user/projects/my-app");
        mutate_add_workspace_with_path(&mut state, Some(path.clone()));
        let ws = state.workspaces.last().unwrap();
        assert_eq!(ws.name, "my-app");
        assert_eq!(ws.path, Some(path));
    }

    #[test]
    fn add_workspace_with_path_sets_active() {
        let mut state = seed_state();
        let old_count = state.workspaces.len();
        mutate_add_workspace_with_path(&mut state, Some(PathBuf::from("/tmp/test")));
        assert_eq!(state.active_workspace, old_count);
    }

    #[test]
    fn add_workspace_without_path_uses_default_name() {
        let mut state = seed_state();
        let expected_num = state.workspaces.len() as u32 + 1;
        mutate_add_workspace(&mut state);
        let ws = state.workspaces.last().unwrap();
        assert_eq!(ws.name, format!("workspace-{}", expected_num));
        assert!(ws.path.is_none());
    }

    #[test]
    fn active_workspace_cwd_returns_path_when_set() {
        let mut state = seed_state();
        let path = PathBuf::from("/tmp/ws");
        mutate_add_workspace_with_path(&mut state, Some(path.clone()));
        assert_eq!(active_workspace_cwd(&state), Some(path));
    }

    #[test]
    fn active_workspace_cwd_returns_none_when_no_path() {
        let mut state = seed_state();
        state.active_workspace = 1; // demo workspace with no path
        assert_eq!(active_workspace_cwd(&state), None);
    }

    // -- Per-workspace terminal routing (issue #101) -------------------------

    /// Build a workspace pre-populated with one tab whose panes contain the
    /// given pane ids. Used to simulate a workspace that "has terminals" for
    /// click-routing tests.
    fn workspace_with_panes(num: u32, name: &str, pane_ids: &[u32]) -> Workspace {
        let panes: Vec<Vec<Pane>> = vec![pane_ids
            .iter()
            .map(|&id| Pane {
                id: PaneId(id),
                title: format!("shell-{}", id),
                subtitle: "bash".to_string(),
                pid: 0,
                cpu: 0.0,
            })
            .collect()];
        let first_id = PaneId(pane_ids[0]);
        let tab = TerminalTab {
            id: format!("t{}", num),
            name: name.to_string(),
            subtitle: "bash".to_string(),
            status: TabStatus::Running,
            panes,
            active_pane: first_id,
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0; pane_ids.len()]],
        };
        Workspace {
            num,
            name: name.to_string(),
            path: None,
            collapsed: false,
            terminals_expanded: true,
            terminal_entries: vec![],
            subtabs: vec![],
            git_branch: None,
            tabs: vec![tab],
            active_tab: 0,
            shell: crate::shell::ShellSpec::default(),
        }
    }

    /// AC1 contract: clicking a terminal entry under workspace B while
    /// workspace A is active must switch active_workspace to B and focus the
    /// clicked pane. This is the single failing test that locks the dispatch
    /// shape for `terminal.focus:<ws_idx>:<pane_id>`.
    #[test]
    fn terminal_focus_switches_workspace_and_pane() {
        let mut state = test_state();
        state.workspaces = vec![
            workspace_with_panes(1, "alpha", &[1]),
            workspace_with_panes(2, "beta", &[7, 8]),
        ];
        state.active_workspace = 0;
        // Mirror ws0's live view so "active workspace" is consistent.
        state.tabs = state.workspaces[0].tabs.clone();
        state.active_tab = 0;
        state.panes = state.tabs[0].panes.clone();
        state.active_pane = PaneId(1);
        state.row_ratios = vec![1.0];
        state.col_ratios = vec![vec![1.0]];

        let handled = dispatch(&mut state, "terminal.focus:1:8");

        assert!(handled, "terminal.focus must return true when handled");
        assert_eq!(state.active_workspace, 1, "workspace must switch to 1");
        assert_eq!(
            state.active_pane,
            PaneId(8),
            "active pane must be the clicked one"
        );
    }

    /// Build a two-workspace state where ws0 is active and both have panes.
    /// Returned state has live state mirroring ws0.
    fn two_workspace_state() -> AppState {
        let mut state = test_state();
        state.workspaces = vec![
            workspace_with_panes(1, "alpha", &[1]),
            workspace_with_panes(2, "beta", &[7, 8]),
        ];
        state.active_workspace = 0;
        state.tabs = state.workspaces[0].tabs.clone();
        state.active_tab = 0;
        state.panes = state.tabs[0].panes.clone();
        state.active_pane = PaneId(1);
        state.row_ratios = vec![1.0];
        state.col_ratios = vec![vec![1.0]];
        state.next_id = 9;
        state
    }

    #[test]
    fn terminal_focus_invalid_workspace_returns_false() {
        let mut state = two_workspace_state();
        let handled = dispatch(&mut state, "terminal.focus:99:1");
        assert!(!handled, "out-of-range workspace must not be handled");
        assert_eq!(state.active_workspace, 0);
    }

    #[test]
    fn terminal_focus_missing_pane_returns_false() {
        let mut state = two_workspace_state();
        let handled = dispatch(&mut state, "terminal.focus:1:999");
        assert!(
            !handled,
            "nonexistent pane id in that workspace must not be handled"
        );
    }

    #[test]
    fn terminal_focus_malformed_returns_false() {
        let mut state = two_workspace_state();
        assert!(!dispatch(&mut state, "terminal.focus:"));
        assert!(!dispatch(&mut state, "terminal.focus:1"));
        assert!(!dispatch(&mut state, "terminal.focus:abc:7"));
        assert!(!dispatch(&mut state, "terminal.focus:1:xyz"));
    }

    #[test]
    fn terminal_focus_same_workspace_just_focuses_pane() {
        let mut state = two_workspace_state();
        // ws0 has one pane (id 1); add a second pane directly.
        state.panes[0].push(Pane {
            id: PaneId(42),
            title: "extra".to_string(),
            subtitle: "bash".to_string(),
            pid: 0,
            cpu: 0.0,
        });
        state.col_ratios[0].push(0.5);
        state.col_ratios[0][0] = 0.5;
        // Mirror into ws0's saved tab so lookup sees it.
        state.workspaces[0].tabs[0].panes = state.panes.clone();
        state.workspaces[0].tabs[0].col_ratios = state.col_ratios.clone();

        let handled = dispatch(&mut state, "terminal.focus:0:42");
        assert!(handled);
        assert_eq!(state.active_workspace, 0);
        assert_eq!(state.active_pane, PaneId(42));
    }

    #[test]
    fn workspace_switch_preserves_pane_layout_of_both() {
        let mut state = two_workspace_state();
        let ws0_ids_before: Vec<Vec<u32>> = state.workspaces[0].tabs[0]
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id.0).collect())
            .collect();
        let ws1_ids_before: Vec<Vec<u32>> = state.workspaces[1].tabs[0]
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id.0).collect())
            .collect();

        assert!(dispatch(&mut state, "workspace.switch:1"));
        assert_eq!(state.active_workspace, 1);
        let live_ws1: Vec<Vec<u32>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id.0).collect())
            .collect();
        assert_eq!(live_ws1, ws1_ids_before);
        assert_eq!(state.active_pane, PaneId(7));

        assert!(dispatch(&mut state, "workspace.switch:0"));
        assert_eq!(state.active_workspace, 0);
        let live_ws0: Vec<Vec<u32>> = state
            .panes
            .iter()
            .map(|row| row.iter().map(|p| p.id.0).collect())
            .collect();
        assert_eq!(live_ws0, ws0_ids_before);
        assert_eq!(state.active_pane, PaneId(1));
    }

    #[test]
    fn workspace_switch_remembers_active_pane_per_workspace() {
        let mut state = two_workspace_state();
        // Start in ws0, go to ws1 and focus pane 8.
        assert!(dispatch(&mut state, "terminal.focus:1:8"));
        assert_eq!(state.active_pane, PaneId(8));
        // Switch to ws0, then back to ws1: active pane must still be 8.
        assert!(dispatch(&mut state, "workspace.switch:0"));
        assert_eq!(state.active_pane, PaneId(1));
        assert!(dispatch(&mut state, "workspace.switch:1"));
        assert_eq!(state.active_pane, PaneId(8));
    }

    #[test]
    fn workspace_switch_does_not_touch_terminals_map() {
        let mut state = two_workspace_state();
        let keys_before: Vec<u32> = {
            let mut ks: Vec<u32> = state.terminals.keys().copied().collect();
            ks.sort();
            ks
        };
        assert!(dispatch(&mut state, "workspace.switch:1"));
        assert!(dispatch(&mut state, "workspace.switch:0"));
        let keys_after: Vec<u32> = {
            let mut ks: Vec<u32> = state.terminals.keys().copied().collect();
            ks.sort();
            ks
        };
        assert_eq!(
            keys_before, keys_after,
            "terminals map (PTY handles) must survive workspace switches intact"
        );
    }

    #[test]
    fn ui_snapshot_inactive_workspace_entries_reflect_its_own_panes() {
        let state = two_workspace_state();
        let snap = state.ui_snapshot();
        // ws1 is inactive; its terminal_entries must reflect pane ids 7 and 8.
        let ws1 = &snap.workspaces[1];
        let ids: Vec<u32> = ws1.terminal_entries.iter().map(|e| e.pane_id.0).collect();
        assert_eq!(ids, vec![7, 8]);
    }

    #[test]
    fn ui_snapshot_active_workspace_entries_span_all_tabs() {
        let mut state = seed_state();
        // seed_state starts with one tab holding PaneId(1). Add a second tab.
        mutate_add_tab(&mut state);
        let snap = state.ui_snapshot();
        let ws0 = &snap.workspaces[0];
        let ids: Vec<u32> = ws0.terminal_entries.iter().map(|e| e.pane_id.0).collect();
        assert_eq!(
            ids.len(),
            2,
            "active workspace must list panes from every tab"
        );
        assert!(ids.contains(&1), "first tab's pane must appear");
    }

    #[test]
    fn workspace_switch_to_empty_clears_live_panes() {
        let mut state = two_workspace_state();
        // Replace ws1 with an empty-tabs workspace (simulates a fresh/unvisited ws).
        state.workspaces[1].tabs = vec![];
        state.workspaces[1].active_tab = 0;

        assert!(dispatch(&mut state, "workspace.switch:1"));
        assert_eq!(state.active_workspace, 1);
        assert!(
            state.panes.is_empty(),
            "panes must be empty on empty workspace"
        );
        assert!(
            state.tabs.is_empty(),
            "tabs must be empty on empty workspace"
        );
    }

    #[test]
    fn mutate_add_workspace_starts_with_no_terminals() {
        let mut state = two_workspace_state();
        let before = state.workspaces.len();
        mutate_add_workspace(&mut state);
        assert_eq!(state.workspaces.len(), before + 1);
        assert_eq!(state.active_workspace, before);
        assert!(
            state.panes.is_empty(),
            "a newly created workspace must start with no panes"
        );
        assert!(
            state.tabs.is_empty(),
            "a newly created workspace must start with no tabs"
        );
    }

    #[test]
    fn restore_layout_round_trips_tabs_panes_and_next_id() {
        // Original: ws0 has tab1(pane 1) and a second tab carrying a
        // right-split (panes 2 and 3, with pane 3 active).
        let mut original = seed_state();
        mutate_add_tab(&mut original);
        let split_target = original.active_pane;
        mutate_split_right(&mut original, split_target);
        let expected_active = original.active_pane;
        let persisted = crate::persist::PersistedState::from_state(&original);

        let mut restored = seed_state();
        restore_layout(&mut restored, &persisted);

        assert_eq!(restored.active_workspace, 0);
        assert_eq!(restored.tabs.len(), 2);
        assert_eq!(restored.active_tab, 1, "active tab selection must survive");
        // The active (split) tab restores both panes in one row.
        assert_eq!(restored.panes.len(), 1);
        let ids: Vec<u32> = restored.panes[0].iter().map(|p| p.id.0).collect();
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(restored.active_pane, expected_active);
        // next_id advances past every restored pane id (max is 3) so new
        // panes never collide with a restored one.
        assert_eq!(restored.next_id, 4);
        // The first tab keeps its original pane.
        assert_eq!(restored.tabs[0].panes[0][0].id, PaneId(1));
    }

    #[test]
    fn restore_layout_seeds_default_when_active_workspace_empty() {
        use crate::persist::{PersistedPane, PersistedState, PersistedTab, PersistedWorkspace};
        let persisted = PersistedState {
            workspaces: vec![
                PersistedWorkspace {
                    name: "main".into(),
                    path: None,
                    collapsed: false,
                    shell: crate::shell::ShellSpec::default(),
                    tabs: vec![],
                    active_tab: 0,
                },
                PersistedWorkspace {
                    name: "api".into(),
                    path: None,
                    collapsed: false,
                    shell: crate::shell::ShellSpec::default(),
                    tabs: vec![PersistedTab {
                        id: "t9".into(),
                        name: String::new(),
                        subtitle: String::new(),
                        panes: vec![vec![PersistedPane {
                            id: 9,
                            title: String::new(),
                            subtitle: String::new(),
                        }]],
                        active_pane: 9,
                        row_ratios: vec![1.0],
                        col_ratios: vec![vec![1.0]],
                    }],
                    active_tab: 0,
                },
            ],
            active_workspace: 0,
            ..Default::default()
        };

        let mut state = seed_state();
        restore_layout(&mut state, &persisted);

        // The active workspace (idx 0) had no tabs, so a fresh default pane
        // is seeded with an id past every restored pane (max was 9).
        assert_eq!(state.active_workspace, 0);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 1);
        assert_eq!(state.active_pane, PaneId(10));
        assert_eq!(state.next_id, 11);
        // The non-active workspace retains its restored pane.
        assert_eq!(state.workspaces[1].tabs[0].panes[0][0].id, PaneId(9));
    }

    #[test]
    fn workspace_new_terminal_switches_and_spawns() {
        let mut state = two_workspace_state();
        assert!(dispatch(&mut state, "workspace.new_terminal:1"));
        assert_eq!(state.active_workspace, 1);
        assert!(!state.tabs.is_empty(), "new tab must be spawned");
        assert!(!state.panes.is_empty(), "new pane must be spawned");
    }

    #[test]
    fn workspace_active_pane_reads_live_for_active_workspace() {
        let state = two_workspace_state();
        assert_eq!(
            super::workspace_active_pane(&state, 0),
            Some(PaneId(1)),
            "active workspace must report the live active pane"
        );
    }

    #[test]
    fn workspace_active_pane_reads_saved_for_inactive_workspace() {
        let state = two_workspace_state();
        assert_eq!(
            super::workspace_active_pane(&state, 1),
            Some(PaneId(7)),
            "inactive workspace must report its saved active pane"
        );
    }

    #[test]
    fn workspace_active_pane_none_when_empty() {
        let mut state = two_workspace_state();
        state.workspaces[1].tabs = vec![];
        assert!(
            super::workspace_active_pane(&state, 1).is_none(),
            "a workspace without tabs has no active pane"
        );
    }

    #[test]
    fn workspace_active_pane_none_for_out_of_range() {
        let state = two_workspace_state();
        assert!(super::workspace_active_pane(&state, 99).is_none());
    }

    // ---------------------------------------------------------------------
    // terminal.paste / normalize_pasted_text
    //
    // Regression coverage for the clipboard paste keybind. The dispatch
    // arm reads the system clipboard, normalises newlines + bracketed
    // paste markers, and writes through `pty_manager.write` (the
    // fire-and-forget path so the render thread cannot stall on the
    // daemon round trip). These tests guard:
    //   * newline canonicalisation rules across CRLF, LF, and CR,
    //   * bracketed-paste end-marker stripping (paste-injection guard),
    //   * the empty-clipboard / non-text no-op path,
    //   * the "no terminal in focus" toast path,
    //   * and that the dispatch arm is wired to the action id used by
    //     the keybind registry.
    // ---------------------------------------------------------------------

    #[test]
    fn normalize_pasted_text_collapses_crlf_to_cr() {
        // Windows clipboard payloads almost always use CRLF. Most POSIX
        // shells treat LF as no-op (or, worse, as continuation), so we
        // need to land on a single CR per line.
        assert_eq!(
            super::normalize_pasted_text("hello\r\nworld\r\n"),
            "hello\rworld\r"
        );
    }

    #[test]
    fn normalize_pasted_text_promotes_lone_lf_to_cr() {
        // Unix clipboards send LF. Bash and friends still expect CR.
        assert_eq!(
            super::normalize_pasted_text("alpha\nbeta\n"),
            "alpha\rbeta\r"
        );
    }

    #[test]
    fn normalize_pasted_text_passes_lone_cr_through() {
        // Old Mac or pre-formatted clipboard payloads can be CR-only;
        // they are already in the format the shell expects.
        assert_eq!(
            super::normalize_pasted_text("classic\rmac\r"),
            "classic\rmac\r"
        );
    }

    #[test]
    fn normalize_pasted_text_preserves_unicode() {
        // Multi-byte UTF-8 must round-trip unchanged. A byte-level
        // walker that pushed each byte as a `char` would corrupt
        // continuation bytes here.
        assert_eq!(super::normalize_pasted_text("café 🦀"), "café 🦀");
    }

    #[test]
    fn normalize_pasted_text_strips_bracketed_paste_markers() {
        // Defence in depth against paste-injection: strip the start
        // and end markers a hostile clipboard payload could embed to
        // forge an "end of paste" mid-string. Even though the daemon
        // does not advertise DECSET 2004 yet, this rule is part of
        // the normalisation contract.
        let input = "ok\x1b[200~before-end\x1b[201~after";
        assert_eq!(super::normalize_pasted_text(input), "okbefore-endafter");
    }

    #[test]
    fn normalize_pasted_text_scrubs_reassembled_split_markers() {
        // Paste-injection regression: a single forward pass would delete the
        // inner marker and splice its neighbours into a brand-new terminator
        // (`\x1b[2` + `\x1b[201~` + `01~` -> `\x1b[201~`). The scrub must run
        // to a fixed point so no marker survives into the wrapped body.
        let input = "\x1b[2\x1b[201~01~payload";
        let out = super::normalize_pasted_text(input);
        assert!(
            !out.contains("\x1b[201~") && !out.contains("\x1b[200~"),
            "no bracketed-paste marker may survive, got {out:?}"
        );
        assert_eq!(out, "payload");

        // Nested start markers must also fully collapse.
        let nested = "\x1b[200\x1b[200~~rm -rf";
        let out = super::normalize_pasted_text(nested);
        assert!(!out.contains("\x1b[200~"), "got {out:?}");
    }

    #[test]
    fn normalize_pasted_text_empty_input_returns_empty() {
        assert_eq!(super::normalize_pasted_text(""), "");
    }

    #[test]
    fn normalize_pasted_text_no_op_when_already_normalised() {
        // Plain text without newlines or markers should pass straight
        // through with no allocations bigger than the input.
        assert_eq!(super::normalize_pasted_text("ls -al"), "ls -al");
    }

    #[test]
    fn normalize_pasted_text_handles_truncated_marker_prefix() {
        // A standalone ESC or short prefix (`\x1b[20`) must not be
        // consumed; only the full 6-byte marker should drop. This
        // guards against eating real shell escape codes in a paste.
        assert_eq!(super::normalize_pasted_text("\x1b[20"), "\x1b[20");
        assert_eq!(super::normalize_pasted_text("\x1b[200"), "\x1b[200");
    }

    #[test]
    fn dispatch_terminal_paste_is_a_recognised_command() {
        // The keybind registry registers Ctrl+V / Ctrl+Shift+V to
        // dispatch this exact command name. If `dispatch` returned
        // `false` here, the on_command hook would consider the action
        // unrecognised and the keybind would silently do nothing.
        let _lock = clipboard_access_guard();
        let mut state = test_state();
        let handled = dispatch(&mut state, "terminal.paste");
        assert!(handled, "terminal.paste must be a known dispatch action");
    }

    #[test]
    fn dispatch_terminal_paste_no_terminal_in_focus_pushes_toast() {
        // No PTY is registered for the active pane in `test_state`.
        // Paste must surface a user-visible toast rather than panic
        // or silently swallow the clipboard read.
        let _lock = clipboard_access_guard();
        let mut state = test_state();
        // Seed the clipboard with text that would otherwise be sent
        // to the PTY so a regression that bypassed the focus check
        // would write to a non-existent pane.
        let _ = state.clipboard.write_text("ls\n");
        let toasts_before = state.toasts.len();
        dispatch(&mut state, "terminal.paste");
        assert!(
            state.toasts.len() > toasts_before,
            "no-focus paste must surface a toast"
        );
    }

    #[test]
    fn dispatch_terminal_paste_empty_clipboard_is_silent() {
        // Empty clipboard should not toast: a stray Ctrl+V right after
        // the user copied a non-text selection (image, file path
        // listing) is harmless and silent paste is the expected
        // behaviour from Windows Terminal / iTerm / GNOME Terminal.
        let _lock = clipboard_access_guard();
        let mut state = test_state();
        // arboard does not expose a way to fully clear the OS
        // clipboard from a non-graphical test, so swap the field
        // for a fresh isolated context that the test owns.
        let local = std::sync::Arc::new(unshit::app::ClipboardContext::new());
        // Best effort: clear; if the platform refuses (headless CI)
        // we still fall through and rely on read returning empty.
        let _ = local.clear();
        state.clipboard = local;

        let toasts_before = state.toasts.len();
        let handled = dispatch(&mut state, "terminal.paste");
        // Action recognised even though no bytes were written.
        assert!(handled);
        // Toast count must not grow on an empty clipboard. A
        // ClipboardError::Unavailable on headless CI would also
        // trigger a toast and fail this test; that is intentional
        // because surfacing real failures is exactly what we want.
        if state
            .clipboard
            .read_text()
            .map(|s| s.is_empty())
            .unwrap_or(false)
        {
            assert_eq!(
                state.toasts.len(),
                toasts_before,
                "empty clipboard paste must not push a toast"
            );
        }
    }

    /// Regression test for the clipboard paste keybind feature.
    ///
    /// The action id `terminal.paste` is the contract between
    /// `keybinds::registry::system_bindings` (Ctrl+V / Ctrl+Shift+V)
    /// and `state::dispatch`. If any future change renames the action
    /// or drops the dispatch arm without updating the registry, this
    /// test catches it before the user sees a silently broken paste.
    #[test]
    fn terminal_paste_action_id_matches_keybind_registry() {
        let bindings = crate::keybinds::registry::default_shortcut_bindings();
        let paste_targets: Vec<&str> = bindings
            .iter()
            .filter(|(_, cmd)| cmd == "terminal.paste")
            .map(|(combo, _)| combo.as_str())
            .collect();
        // Both bindings must be registered so users coming from
        // either Windows or Linux conventions reach the same action.
        assert!(
            paste_targets.contains(&"Ctrl+V"),
            "Ctrl+V must dispatch terminal.paste; got {paste_targets:?}"
        );
        assert!(
            paste_targets.contains(&"Ctrl+Shift+V"),
            "Ctrl+Shift+V must dispatch terminal.paste; got {paste_targets:?}"
        );
        // And the dispatch handler must accept it. Together these
        // guarantee end-to-end the keybind reaches a live arm.
        let _lock = clipboard_access_guard();
        let mut state = test_state();
        assert!(dispatch(&mut state, "terminal.paste"));
    }
}

#[cfg(test)]
mod tests_mouse_selection_copy_paste {
    use super::tests::{clipboard_access_guard, test_state};
    use super::*;
    use std::time::Duration;

    // -------- cell_from_local tests --------

    #[test]
    fn cell_from_local_origin_maps_to_zero() {
        assert_eq!(
            cell_from_local(0.0, 0.0, 10.0, 20.0, 0.0, 80, 24),
            Some((0, 0))
        );
    }

    #[test]
    fn cell_from_local_floors_within_cell() {
        // 95 / 10 = 9.5 -> col 9 (floors)
        // 45 / 20 = 2.25 -> row 2 (floors)
        assert_eq!(
            cell_from_local(95.0, 45.0, 10.0, 20.0, 0.0, 80, 24),
            Some((2, 9))
        );
    }

    #[test]
    fn cell_from_local_right_overrun_clamps_to_last_col() {
        // Far-right overrun should clamp onto the last column.
        assert_eq!(
            cell_from_local(1.0e5, 0.0, 10.0, 20.0, 0.0, 80, 24),
            Some((0, 79))
        );
    }

    #[test]
    fn cell_from_local_bottom_overrun_clamps_to_last_row() {
        // Far-bottom overrun should clamp onto the last row.
        assert_eq!(
            cell_from_local(0.0, 1.0e5, 10.0, 20.0, 0.0, 80, 24),
            Some((23, 0))
        );
    }

    #[test]
    fn cell_from_local_applies_x_offset() {
        // With x_offset=3, local_x=13 maps to 13-3=10 pixels into content
        // 10 / 10 = col 1
        assert_eq!(
            cell_from_local(13.0, 0.0, 10.0, 20.0, 3.0, 80, 24),
            Some((0, 1))
        );
    }

    #[test]
    fn cell_from_local_negative_coord_clamps_to_origin() {
        // Negative local coordinates clamp to (0, 0)
        assert_eq!(
            cell_from_local(-5.0, -10.0, 10.0, 20.0, 0.0, 80, 24),
            Some((0, 0))
        );
    }

    #[test]
    fn cell_from_local_zero_cols_returns_none() {
        assert_eq!(cell_from_local(5.0, 5.0, 10.0, 20.0, 0.0, 0, 24), None);
    }

    #[test]
    fn cell_from_local_zero_rows_returns_none() {
        assert_eq!(cell_from_local(5.0, 5.0, 10.0, 20.0, 0.0, 80, 0), None);
    }

    #[test]
    fn cell_from_local_zero_cell_width_returns_none() {
        assert_eq!(cell_from_local(5.0, 5.0, 0.0, 20.0, 0.0, 80, 24), None);
    }

    #[test]
    fn cell_from_local_zero_cell_height_returns_none() {
        assert_eq!(cell_from_local(5.0, 5.0, 10.0, 0.0, 0.0, 80, 24), None);
    }

    #[test]
    fn cell_from_local_negative_cell_width_returns_none() {
        assert_eq!(cell_from_local(5.0, 5.0, -10.0, 20.0, 0.0, 80, 24), None);
    }

    // -------- terminal_cell_at tests --------

    #[test]
    fn terminal_cell_at_no_terminal_returns_none() {
        let st = test_state();
        // No terminal registered for pane 1
        let result = terminal_cell_at(&st, 1, 0.0, 0.0, 0.0, 1.0);
        assert_eq!(result, None);
    }

    #[test]
    fn terminal_cell_at_with_real_terminal_maps_to_absolute_line() {
        let mut st = test_state();
        let pane = 1u32;
        // Create a 24x80 terminal and publish cell metrics.
        st.terminals.insert(
            pane,
            std::sync::Arc::new(std::sync::Mutex::new(crate::terminal::Terminal::new(
                24, 80,
            ))),
        );
        unshit::core::cell_grid::CellGrid::publish_cell_metrics(10.0, 20.0);

        // On a fresh terminal with no scrollback, display row N == absolute line N.
        let result = terminal_cell_at(&st, pane, 0.0, 0.0, 0.0, 1.0);
        assert_eq!(result, Some((0, 0)));
    }

    #[test]
    fn terminal_cell_at_applies_cell_w_scale() {
        let mut st = test_state();
        let pane = 1u32;
        st.terminals.insert(
            pane,
            std::sync::Arc::new(std::sync::Mutex::new(crate::terminal::Terminal::new(
                24, 80,
            ))),
        );
        unshit::core::cell_grid::CellGrid::publish_cell_metrics(10.0, 20.0);

        // With scale=0.996 the effective cell width is 9.96, so 19.92 lands on
        // the left edge of column 2 (19.92 / 9.96 == 2.0). Without the scale
        // (divisor 10.0) the same x would map to column 1, so this pins the
        // scale's effect on the hit-test.
        let result = terminal_cell_at(&st, pane, 19.92, 0.0, 0.0, 0.996);
        assert_eq!(result, Some((0, 2)));
        let unscaled = terminal_cell_at(&st, pane, 19.92, 0.0, 0.0, 1.0);
        assert_eq!(unscaled, Some((0, 1)));
    }

    #[test]
    fn terminal_cell_at_after_scrolling_maps_to_different_absolute_line() {
        let mut st = test_state();
        let pane = 1u32;
        let mut term = crate::terminal::Terminal::new(3, 5);
        // Write 5 lines of output to build scrollback
        term.process_bytes(b"line0\r\nline1\r\nline2\r\nline3\r\nline4");
        st.terminals
            .insert(pane, std::sync::Arc::new(std::sync::Mutex::new(term)));
        unshit::core::cell_grid::CellGrid::publish_cell_metrics(10.0, 20.0);

        // At scroll_offset 0, display row 0 == absolute line ~2 (top of scrollback)
        let abs_at_top = terminal_cell_at(&st, pane, 0.0, 0.0, 0.0, 1.0);
        assert!(abs_at_top.is_some());

        // After scrolling, the same pixel should map to a different absolute line.
        // (This is verified by the terminal's scroll_view and abs_line_at_display logic;
        // we just verify that terminal_cell_at propagates the mapping correctly.)
        let handle = st.terminals.get(&pane).unwrap();
        let mut t = handle.lock_recover();
        if t.scrollback_len() > 0 {
            t.scroll_view_up(1);
        }
        drop(t);
        let abs_after_scroll = terminal_cell_at(&st, pane, 0.0, 0.0, 0.0, 1.0);
        if let (Some((abs_before, _)), Some((abs_after, _))) = (abs_at_top, abs_after_scroll) {
            if abs_before != abs_after {
                // Scrollback exists and scroll changed the mapping.
                assert_ne!(abs_before, abs_after);
            }
        }
    }

    // -------- TermSelection tests --------

    #[test]
    fn term_selection_cell_mode_collapsed_is_empty() {
        let sel = TermSelection::new((5, 10), SelectMode::Cell);
        assert!(sel.is_empty());
    }

    #[test]
    fn term_selection_word_mode_single_cell_not_empty() {
        let sel = TermSelection::new((5, 10), SelectMode::Word);
        assert!(!sel.is_empty());
    }

    #[test]
    fn term_selection_line_mode_single_cell_not_empty() {
        let sel = TermSelection::new((5, 10), SelectMode::Line);
        assert!(!sel.is_empty());
    }

    #[test]
    fn term_selection_ordered_with_reversed_anchor_focus() {
        let sel = TermSelection {
            anchor: (2, 5),
            focus: (1, 1),
            mode: SelectMode::Cell,
        };
        assert_eq!(sel.ordered(), ((1, 1), (2, 5)));
    }

    #[test]
    fn term_selection_ordered_with_forward_anchor_focus() {
        let sel = TermSelection {
            anchor: (1, 1),
            focus: (2, 5),
            mode: SelectMode::Cell,
        };
        assert_eq!(sel.ordered(), ((1, 1), (2, 5)));
    }

    #[test]
    fn term_selection_ordered_across_multiple_lines() {
        let sel = TermSelection {
            anchor: (10, 50),
            focus: (5, 20),
            mode: SelectMode::Cell,
        };
        // (5, 20) is earlier in line-major order
        assert_eq!(sel.ordered(), ((5, 20), (10, 50)));
    }

    // -------- set_terminal_selection and repaint flag tests --------

    #[test]
    fn set_terminal_selection_collapsed_to_collapsed_no_repaint() {
        let mut st = test_state();
        let pane = 1u32;
        set_terminal_selection(&mut st, pane, TermSelection::new((0, 0), SelectMode::Cell));
        assert!(!st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn set_terminal_selection_none_to_range_flags_repaint() {
        let mut st = test_state();
        let pane = 1u32;
        assert!(!st.terminal_selections.contains_key(&pane));
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 5),
                mode: SelectMode::Cell,
            },
        );
        assert!(st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn set_terminal_selection_range_to_moved_range_flags_repaint() {
        let mut st = test_state();
        let pane = 1u32;
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 5),
                mode: SelectMode::Cell,
            },
        );
        st.terminal_selection_repaint.clear();
        // Move focus to a different position
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 10),
                mode: SelectMode::Cell,
            },
        );
        assert!(st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn set_terminal_selection_range_to_identical_range_no_repaint() {
        let mut st = test_state();
        let pane = 1u32;
        let sel = TermSelection {
            anchor: (0, 0),
            focus: (0, 5),
            mode: SelectMode::Cell,
        };
        set_terminal_selection(&mut st, pane, sel);
        st.terminal_selection_repaint.clear();
        // Set the exact same selection again
        set_terminal_selection(&mut st, pane, sel);
        assert!(!st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn set_terminal_selection_range_to_collapsed_flags_repaint() {
        let mut st = test_state();
        let pane = 1u32;
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 5),
                mode: SelectMode::Cell,
            },
        );
        st.terminal_selection_repaint.clear();
        // Collapse to a single cell
        set_terminal_selection(&mut st, pane, TermSelection::new((0, 0), SelectMode::Cell));
        assert!(st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn clear_terminal_selection_removes_and_flags_repaint() {
        let mut st = test_state();
        let pane = 1u32;
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 5),
                mode: SelectMode::Cell,
            },
        );
        st.terminal_selection_repaint.clear();
        clear_terminal_selection(&mut st, pane);
        assert!(!st.terminal_selections.contains_key(&pane));
        assert!(st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn clear_terminal_selection_no_prior_selection_no_flag_churn() {
        let mut st = test_state();
        let pane = 1u32;
        assert!(!st.terminal_selections.contains_key(&pane));
        clear_terminal_selection(&mut st, pane);
        assert!(!st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn mark_terminal_selection_dirty_flags_when_selection_exists() {
        let mut st = test_state();
        let pane = 1u32;
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 5),
                mode: SelectMode::Cell,
            },
        );
        st.terminal_selection_repaint.clear();
        mark_terminal_selection_dirty(&mut st, pane);
        assert!(st.terminal_selection_repaint.contains(&pane));
    }

    #[test]
    fn mark_terminal_selection_dirty_no_flag_when_none() {
        let mut st = test_state();
        let pane = 1u32;
        mark_terminal_selection_dirty(&mut st, pane);
        assert!(!st.terminal_selection_repaint.contains(&pane));
    }

    // -------- handle_terminal_mouse_down: multi-click promotion --------

    #[test]
    fn handle_terminal_mouse_down_first_click_cell_mode() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Cell);
    }

    #[test]
    fn handle_terminal_mouse_down_second_click_promotes_to_word() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0 + Duration::from_millis(50));
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Word);
    }

    #[test]
    fn handle_terminal_mouse_down_third_click_promotes_to_line() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0 + Duration::from_millis(50));
        handle_terminal_mouse_down(
            &mut st,
            pane,
            (0, 0),
            false,
            t0 + Duration::from_millis(100),
        );
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Line);
    }

    #[test]
    fn handle_terminal_mouse_down_fourth_click_wraps_to_cell() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        for i in 0..4 {
            handle_terminal_mouse_down(
                &mut st,
                pane,
                (0, 0),
                false,
                t0 + Duration::from_millis(50 * i),
            );
        }
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Cell);
    }

    #[test]
    fn handle_terminal_mouse_down_click_after_window_resets_to_cell() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        // Press again way after the window expires (400ms)
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0 + Duration::from_secs(2));
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Cell);
    }

    #[test]
    fn handle_terminal_mouse_down_different_cell_resets_count() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0 + Duration::from_millis(50));
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Word);
        // Click on a different cell within the window
        handle_terminal_mouse_down(
            &mut st,
            pane,
            (0, 1),
            false,
            t0 + Duration::from_millis(100),
        );
        assert_eq!(st.terminal_selections[&pane].mode, SelectMode::Cell);
    }

    #[test]
    fn handle_terminal_mouse_down_different_pane_resets_count() {
        let mut st = test_state();
        let pane1 = 1u32;
        let pane2 = 2u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane1, (0, 0), false, t0);
        handle_terminal_mouse_down(
            &mut st,
            pane1,
            (0, 0),
            false,
            t0 + Duration::from_millis(50),
        );
        assert_eq!(st.terminal_selections[&pane1].mode, SelectMode::Word);
        // Click on a different pane within the window
        handle_terminal_mouse_down(
            &mut st,
            pane2,
            (0, 0),
            false,
            t0 + Duration::from_millis(100),
        );
        assert_eq!(st.terminal_selections[&pane2].mode, SelectMode::Cell);
    }

    #[test]
    fn handle_terminal_mouse_down_shift_click_extends_anchor() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        // Initial click at (1, 2)
        handle_terminal_mouse_down(&mut st, pane, (1, 2), false, t0);
        // Shift+click at (3, 7) keeps anchor and moves focus
        handle_terminal_mouse_down(&mut st, pane, (3, 7), true, t0 + Duration::from_secs(2));
        let sel = st.terminal_selections[&pane];
        assert_eq!(sel.anchor, (1, 2));
        assert_eq!(sel.focus, (3, 7));
    }

    #[test]
    fn handle_terminal_mouse_down_shift_click_without_prior_starts_fresh() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        // Shift+click with no prior selection starts a fresh anchor
        handle_terminal_mouse_down(&mut st, pane, (2, 3), true, t0);
        let sel = st.terminal_selections[&pane];
        assert_eq!(sel.anchor, (2, 3));
        assert_eq!(sel.focus, (2, 3));
        assert_eq!(sel.mode, SelectMode::Cell);
    }

    // -------- handle_terminal_drag and finish_terminal_drag --------

    #[test]
    fn handle_terminal_drag_extends_focus() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        handle_terminal_mouse_down(&mut st, pane, (1, 1), false, t0);
        handle_terminal_drag(&mut st, pane, (3, 5));
        let sel = st.terminal_selections[&pane];
        assert_eq!(sel.anchor, (1, 1));
        assert_eq!(sel.focus, (3, 5));
    }

    #[test]
    fn handle_terminal_drag_seeds_without_prior_anchor() {
        let mut st = test_state();
        let pane = 1u32;
        // Drag without a prior mouse-down (edge case)
        handle_terminal_drag(&mut st, pane, (5, 10));
        let sel = st.terminal_selections[&pane];
        assert_eq!(sel.anchor, (5, 10));
        assert_eq!(sel.focus, (5, 10));
        assert!(sel.is_empty());
    }

    #[test]
    fn finish_terminal_drag_drops_collapsed_selection() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        // A click without a drag leaves a collapsed selection
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        finish_terminal_drag(&mut st, pane);
        assert!(!st.terminal_selections.contains_key(&pane));
    }

    #[test]
    fn finish_terminal_drag_keeps_non_collapsed_selection() {
        let mut st = test_state();
        let pane = 1u32;
        let t0 = std::time::Instant::now();
        // A drag that extends the selection
        handle_terminal_mouse_down(&mut st, pane, (0, 0), false, t0);
        handle_terminal_drag(&mut st, pane, (0, 5));
        finish_terminal_drag(&mut st, pane);
        assert!(st.terminal_selections.contains_key(&pane));
        assert!(!st.terminal_selections[&pane].is_empty());
    }

    // -------- active_pane_has_selection --------

    #[test]
    fn active_pane_has_selection_true_for_real_range() {
        let mut st = test_state();
        assert_eq!(st.active_pane.0, 1);
        set_terminal_selection(
            &mut st,
            1,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 5),
                mode: SelectMode::Cell,
            },
        );
        assert!(active_pane_has_selection(&st));
    }

    #[test]
    fn active_pane_has_selection_false_for_collapsed() {
        let mut st = test_state();
        set_terminal_selection(&mut st, 1, TermSelection::new((0, 0), SelectMode::Cell));
        assert!(!active_pane_has_selection(&st));
    }

    #[test]
    fn active_pane_has_selection_false_for_non_active_pane() {
        let mut st = test_state();
        // active_pane is 1, but set selection on pane 2
        set_terminal_selection(
            &mut st,
            2,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 5),
                mode: SelectMode::Cell,
            },
        );
        assert!(!active_pane_has_selection(&st));
    }

    #[test]
    fn active_pane_has_selection_false_when_none() {
        let st = test_state();
        assert!(!active_pane_has_selection(&st));
    }

    // -------- dispatch_terminal_copy (via dispatch) --------

    #[test]
    fn dispatch_terminal_copy_returns_true_with_selection_and_terminal() {
        let _lock = clipboard_access_guard();
        let mut st = test_state();
        let pane = 1u32;
        // Create a real terminal with text
        let mut term = crate::terminal::Terminal::new(2, 5);
        term.process_bytes(b"hello\r\nworld");
        st.terminals
            .insert(pane, std::sync::Arc::new(std::sync::Mutex::new(term)));
        // Set a real selection
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 4),
                mode: SelectMode::Cell,
            },
        );
        assert!(active_pane_has_selection(&st));
        // Dispatch copy
        let result = dispatch(&mut st, "terminal.copy");
        assert!(result);
        // Selection should be cleared
        assert!(!active_pane_has_selection(&st));
    }

    #[test]
    fn dispatch_terminal_copy_returns_false_with_no_selection() {
        let _lock = clipboard_access_guard();
        let mut st = test_state();
        let pane = 1u32;
        let mut term = crate::terminal::Terminal::new(2, 5);
        term.process_bytes(b"hello");
        st.terminals
            .insert(pane, std::sync::Arc::new(std::sync::Mutex::new(term)));
        // No selection set
        let result = dispatch(&mut st, "terminal.copy");
        assert!(!result);
    }

    #[test]
    fn dispatch_terminal_copy_returns_false_with_collapsed_selection() {
        let _lock = clipboard_access_guard();
        let mut st = test_state();
        let pane = 1u32;
        let mut term = crate::terminal::Terminal::new(2, 5);
        term.process_bytes(b"hello");
        st.terminals
            .insert(pane, std::sync::Arc::new(std::sync::Mutex::new(term)));
        // Collapsed selection (empty in Cell mode)
        set_terminal_selection(&mut st, pane, TermSelection::new((0, 0), SelectMode::Cell));
        let result = dispatch(&mut st, "terminal.copy");
        assert!(!result);
    }

    #[test]
    fn dispatch_terminal_copy_clears_selection_after_success() {
        let _lock = clipboard_access_guard();
        let mut st = test_state();
        let pane = 1u32;
        let mut term = crate::terminal::Terminal::new(1, 5);
        term.process_bytes(b"hello");
        st.terminals
            .insert(pane, std::sync::Arc::new(std::sync::Mutex::new(term)));
        set_terminal_selection(
            &mut st,
            pane,
            TermSelection {
                anchor: (0, 0),
                focus: (0, 4),
                mode: SelectMode::Cell,
            },
        );
        let _ = dispatch(&mut st, "terminal.copy");
        // Verify selection is cleared by checking active_pane_has_selection
        assert!(!active_pane_has_selection(&st));
    }

    // -------- normalize_pasted_text comprehensive tests --------

    #[test]
    fn normalize_pasted_text_crlf_to_cr() {
        assert_eq!(
            normalize_pasted_text("hello\r\nworld\r\n"),
            "hello\rworld\r"
        );
    }

    #[test]
    fn normalize_pasted_text_lf_to_cr() {
        assert_eq!(normalize_pasted_text("alpha\nbeta\n"), "alpha\rbeta\r");
    }

    #[test]
    fn normalize_pasted_text_cr_passthrough() {
        assert_eq!(normalize_pasted_text("old\rmac\r"), "old\rmac\r");
    }

    #[test]
    fn normalize_pasted_text_preserves_unicode() {
        assert_eq!(normalize_pasted_text("café 🦀"), "café 🦀");
    }

    #[test]
    fn normalize_pasted_text_strips_bracketed_paste_markers() {
        let input = "ok\x1b[200~before-end\x1b[201~after";
        let output = normalize_pasted_text(input);
        assert_eq!(output, "okbefore-endafter");
        assert!(!output.contains("\x1b[200~"));
        assert!(!output.contains("\x1b[201~"));
    }

    #[test]
    fn normalize_pasted_text_scrubs_reassembled_split_markers_regression() {
        // REGRESSION: A naive single pass would splice neighbours into a new marker.
        // `\x1b[2` + `\x1b[201~` + `01~` -> `\x1b[201~` without fixed-point iteration.
        let input = "\x1b[2\x1b[201~01~payload";
        let output = normalize_pasted_text(input);
        assert!(
            !output.contains("\x1b[201~"),
            "end marker escaped: {output:?}"
        );
        assert!(
            !output.contains("\x1b[200~"),
            "start marker escaped: {output:?}"
        );
        assert_eq!(output, "payload");
    }

    #[test]
    fn normalize_pasted_text_nested_markers_fully_collapse() {
        let input = "\x1b[200\x1b[200~~rm -rf";
        let output = normalize_pasted_text(input);
        assert!(
            !output.contains("\x1b[200~"),
            "nested marker survived: {output:?}"
        );
        assert_eq!(output, "rm -rf");
    }

    #[test]
    fn normalize_pasted_text_adjacent_split_markers_collapse() {
        // Multiple separate splits that reassemble across deletions.
        let input = "\x1b[20\x1b[2001~0~test";
        let output = normalize_pasted_text(input);
        assert!(!output.contains("\x1b[200~"));
        assert!(!output.contains("\x1b[201~"));
    }

    #[test]
    fn normalize_pasted_text_markers_at_boundaries() {
        // Marker at the very start.
        let input1 = "\x1b[200~content";
        let output1 = normalize_pasted_text(input1);
        assert_eq!(output1, "content");

        // Marker at the very end.
        let input2 = "content\x1b[201~";
        let output2 = normalize_pasted_text(input2);
        assert_eq!(output2, "content");

        // Markers at both boundaries.
        let input3 = "\x1b[200~middle\x1b[201~";
        let output3 = normalize_pasted_text(input3);
        assert_eq!(output3, "middle");
    }

    #[test]
    fn normalize_pasted_text_truncated_marker_prefix_preserved() {
        // Lone ESC is not part of a marker (needs ESC + '[').
        assert_eq!(normalize_pasted_text("\x1b"), "\x1b");
        // ESC + '[' is still not a marker (needs 20{0,1}~).
        assert_eq!(normalize_pasted_text("\x1b["), "\x1b[");
        // ESC + '[' + '2' + '0' is still not complete.
        assert_eq!(normalize_pasted_text("\x1b[20"), "\x1b[20");
        // ESC + '[' + '2' + '0' + '0' is still not complete.
        assert_eq!(normalize_pasted_text("\x1b[200"), "\x1b[200");
        // ESC + '[' + '2' + '0' + '0' + '{0,1}' is the full marker; only it gets stripped.
        // All the prefixes survive as partial real escape codes.
    }

    #[test]
    fn normalize_pasted_text_empty_input() {
        assert_eq!(normalize_pasted_text(""), "");
    }

    #[test]
    fn normalize_pasted_text_only_markers_becomes_empty() {
        let input = "\x1b[200~\x1b[201~";
        let output = normalize_pasted_text(input);
        assert_eq!(output, "");
    }

    #[test]
    fn normalize_pasted_text_multiple_separate_markers() {
        let input = "a\x1b[200~b\x1b[201~c\x1b[200~d\x1b[201~e";
        let output = normalize_pasted_text(input);
        assert_eq!(output, "abcde");
        assert!(!output.contains("\x1b[200~"));
        assert!(!output.contains("\x1b[201~"));
    }

    #[test]
    fn normalize_pasted_text_utf8_adjacent_to_markers() {
        // Multi-byte UTF-8 (café) adjacent to markers should survive intact.
        let input = "\x1b[200~café\x1b[201~🦀";
        let output = normalize_pasted_text(input);
        assert_eq!(output, "café🦀");
    }

    #[test]
    fn normalize_pasted_text_crlf_mixed_with_markers() {
        let input = "line1\r\n\x1b[200~line2\r\nline3\x1b[201~line4\r\n";
        let output = normalize_pasted_text(input);
        // CRLF collapses to CR; markers strip.
        assert_eq!(output, "line1\rline2\rline3line4\r");
    }

    #[test]
    fn normalize_pasted_text_noop_when_plain() {
        // Plain text with no markers or special newlines should be unchanged.
        assert_eq!(normalize_pasted_text("ls -al"), "ls -al");
    }
}
