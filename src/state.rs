use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::terminal::Terminal;

pub const MAX_COLS: usize = 4;
pub const MAX_ROWS: usize = 4;
pub const MIN_FONT_SIZE: u32 = 8;
pub const MAX_FONT_SIZE: u32 = 32;
/// Minimum flex-grow ratio for any pane (prevents collapsing below ~10%).
pub const MIN_PANE_RATIO: f32 = 0.1;
pub const MIN_SIDEBAR_WIDTH: f32 = 150.0;
pub const MAX_SIDEBAR_WIDTH: f32 = 500.0;

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
    /// so the next close can skip the prompt.
    CloseApp { count: usize, remember: bool },
    /// Rename dialog for the session backing `pane_id`. `buffer` is
    /// the live text in the input, updated on every keystroke so the
    /// commit handler can read it without pulling values out of the UI.
    RenameSession { pane_id: u32, buffer: String },
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
    General,
    Appearance,
    Shell,
    Keybinds,
    Agents,
    Sessions,
    DangerZone,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::General => "general",
            SettingsSection::Appearance => "appearance",
            SettingsSection::Shell => "shell",
            SettingsSection::Keybinds => "keybinds",
            SettingsSection::Agents => "agents",
            SettingsSection::Sessions => "sessions",
            SettingsSection::DangerZone => "danger zone",
        }
    }

    pub fn all() -> [SettingsSection; 7] {
        [
            SettingsSection::General,
            SettingsSection::Appearance,
            SettingsSection::Shell,
            SettingsSection::Keybinds,
            SettingsSection::Agents,
            SettingsSection::Sessions,
            SettingsSection::DangerZone,
        ]
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
    pub message: String,
}

/// Push an error-level toast onto `state.toasts`. Single entry point
/// so dispatch handlers do not format user-facing strings inline.
pub fn push_error_toast(state: &mut AppState, message: impl Into<String>) {
    state.toasts.push(message);
}

/// Typed keys for the `AppState::toggles` map. Previously string literals
/// like "confirm-close" were spread across the UI, with the type system
/// no help against typos (e.g. "confirm-clsoe" silently read as `false`).
///
/// Agent enable/disable flags used to live here too (`AgentClaude`, etc.)
/// but were moved to `AppState::agents` so general/appearance toggles and
/// agent rows are not mixed in one map. See `AgentKey`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ToggleKey {
    RestoreOnStartup,
    ConfirmClose,
    StartMinimized,
    CheckUpdates,
    GlowEffect,
    BackgroundTexture,
    FontLigatures,
    ShellIntegration,
    ScrollOnOutput,
    BellNotification,
    AutoDiscovery,
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
            ToggleKey::RestoreOnStartup => "restore-on-startup",
            ToggleKey::ConfirmClose => "confirm-close",
            ToggleKey::StartMinimized => "start-minimized",
            ToggleKey::CheckUpdates => "check-updates",
            ToggleKey::GlowEffect => "glow-effect",
            ToggleKey::BackgroundTexture => "background-texture",
            ToggleKey::FontLigatures => "font-ligatures",
            ToggleKey::ShellIntegration => "shell-integration",
            ToggleKey::ScrollOnOutput => "scroll-on-output",
            ToggleKey::BellNotification => "bell-notification",
            ToggleKey::AutoDiscovery => "auto-discovery",
            ToggleKey::RememberCloseChoice => "remember-close-choice",
            ToggleKey::KillAllOnClose => "kill-all-on-close",
        }
    }
}

/// Identifies which configured agent an `Agent` entry represents. Kept
/// separate from `ToggleKey` because agents are a list of records (icon,
/// path, status, enabled) rather than booleans against a well-known name.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum AgentKey {
    Claude,
    Amp,
    Codex,
}

impl AgentKey {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKey::Claude => "agent-claude",
            AgentKey::Amp => "agent-amp",
            AgentKey::Codex => "agent-codex",
        }
    }
}

/// A single configured agent and whether it is enabled. The full set lives
/// in `AppState::agents` as a `Vec<Agent>` so new agents can be added
/// without touching the generic toggles map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Agent {
    pub key: AgentKey,
    pub enabled: bool,
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
    pub font_size_pt: u32,
    pub toggles: BTreeMap<ToggleKey, bool>,
    /// Configured agents and their enabled state. Separate from `toggles`
    /// because agents are records (icon, path, status, enabled) whereas
    /// `toggles` holds boolean feature flags.
    pub agents: Vec<Agent>,
    pub palette_open: bool,
    pub sidebar_collapsed: bool,
    pub sidebar_width: f32,
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
    /// Ephemeral notification queue. Populated by
    /// [`push_error_toast`]; ticked down by the cursor-blink
    /// subscription so dismissal stays deterministic in tests.
    pub toasts: unshit::core::toast::ToastStore,
}

impl AppState {
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
            font_size_pt: self.font_size_pt,
            toggles: self.toggles.clone(),
            agents: self.agents.clone(),
            palette_open: self.palette_open,
            sidebar_collapsed: self.sidebar_collapsed,
            sidebar_width: self.sidebar_width,
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
            sessions: self.sessions.clone(),
            sessions_stale: self.sessions_stale,
            toasts: self
                .toasts
                .iter()
                .map(|t| ToastView {
                    id: t.id,
                    kind: t.kind,
                    message: t.message.clone(),
                })
                .collect(),
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
    pub font_size_pt: u32,
    pub toggles: BTreeMap<ToggleKey, bool>,
    pub agents: Vec<Agent>,
    pub palette_open: bool,
    pub sidebar_collapsed: bool,
    pub sidebar_width: f32,
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
    pub sessions: Vec<SessionSnapshot>,
    /// Mirrors `AppState::sessions_stale`. `true` when the most recent
    /// `list_sessions` RPC failed and the cached rows may be stale.
    pub sessions_stale: bool,
    /// Flat projection of the live `ToastStore`. Push order preserved.
    pub toasts: Vec<ToastView>,
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
    toggles.insert(ToggleKey::RestoreOnStartup, true);
    toggles.insert(ToggleKey::ConfirmClose, true);
    toggles.insert(ToggleKey::StartMinimized, false);
    toggles.insert(ToggleKey::CheckUpdates, true);
    toggles.insert(ToggleKey::GlowEffect, true);
    toggles.insert(ToggleKey::BackgroundTexture, true);
    toggles.insert(ToggleKey::FontLigatures, true);
    toggles.insert(ToggleKey::ShellIntegration, true);
    toggles.insert(ToggleKey::ScrollOnOutput, true);
    toggles.insert(ToggleKey::BellNotification, false);
    toggles.insert(ToggleKey::AutoDiscovery, true);
    toggles.insert(ToggleKey::RememberCloseChoice, false);
    toggles.insert(ToggleKey::KillAllOnClose, false);

    let agents = default_agents();

    AppState {
        workspaces,
        active_workspace: 0,
        tabs,
        active_tab: 0,
        panes,
        active_pane: PaneId(1),
        settings_open: false,
        settings_section: SettingsSection::General,
        theme: "amber".to_string(),
        font_size_pt: 13,
        toggles,
        agents,
        palette_open: false,
        sidebar_collapsed: false,
        sidebar_width: 252.0,
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
        toasts: unshit::core::toast::ToastStore::with_capacity(3, 8),
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
    let mut terminal = crate::terminal::Terminal::new(rows as usize, cols as usize);
    match state
        .pty_manager
        .spawn_in(id_num, workspace_id, cols, rows, cwd.as_deref())
    {
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
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state
        .pty_manager
        .spawn_in(id_num, workspace_id, cols, rows, cwd.as_deref())
    {
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
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state
        .pty_manager
        .spawn_in(id_num, workspace_id, cols, rows, cwd.as_deref())
    {
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

pub fn mutate_font_size_delta(state: &mut AppState, delta: i32) {
    let next = state.font_size_pt as i32 + delta;
    state.font_size_pt = (next.clamp(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32)) as u32;
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
        state.confirm_dialog = Some(ConfirmDialog::CloseApp {
            count: state.terminals.len(),
            remember: false,
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

    for row in state.panes.iter_mut() {
        for pane in row.iter_mut() {
            if pane.id.0 == pane_id {
                pane.title = new_title.clone();
            }
        }
    }
    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
        for row in tab.panes.iter_mut() {
            for pane in row.iter_mut() {
                if pane.id.0 == pane_id {
                    pane.title = new_title.clone();
                }
            }
        }
    }
    for ws in state.workspaces.iter_mut() {
        for tab in ws.tabs.iter_mut() {
            for row in tab.panes.iter_mut() {
                for pane in row.iter_mut() {
                    if pane.id.0 == pane_id {
                        pane.title = new_title.clone();
                    }
                }
            }
        }
    }
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
                changed = true;
            }
            if state.confirm_dialog.is_some() {
                state.confirm_dialog = None;
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
        "app.close.keep_running" => {
            let remember = matches!(
                state.confirm_dialog,
                Some(ConfirmDialog::CloseApp { remember: true, .. })
            );
            state.confirm_dialog = None;
            if remember {
                state.toggles.insert(ToggleKey::RememberCloseChoice, true);
                state.toggles.insert(ToggleKey::KillAllOnClose, false);
                crate::persist::save_workspaces(state);
            }
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
                crate::persist::save_workspaces(state);
            }
            mutate_kill_all_terminals(state);
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
            if !state.settings_open {
                state.settings_open = true;
                true
            } else {
                false
            }
        }
        "tab.new" => {
            mutate_add_tab(state);
            true
        }
        "tab.close.active" => {
            let idx = state.active_tab;
            mutate_close_tab(state, idx);
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
            true
        }
        "pane.split_down" => {
            mutate_split_down(state, state.active_pane);
            true
        }
        "pane.close" => {
            mutate_close_pane(state, state.active_pane);
            true
        }
        other if other.starts_with("pane.extract_to_tab:") => {
            dispatch_pane_extract_to_tab(state, other)
        }
        other if other.starts_with("drag.start_pane:") => dispatch_drag_start_pane(state, other),
        other if other.starts_with("drag.start_tab:") => dispatch_drag_start_tab(state, other),
        other if other.starts_with("drag.update:") => dispatch_drag_update(state, other),
        "drag.end" => dispatch_drag_end(state),
        other if other.starts_with("pane.drop_split:") => dispatch_pane_drop_split(state, other),
        other if other.starts_with("tab.reorder:") => dispatch_tab_reorder(state, other),
        "sidebar.toggle" => {
            state.sidebar_collapsed = !state.sidebar_collapsed;
            true
        }
        "workspace.add" => {
            mutate_add_workspace(state);
            crate::persist::save_workspaces(state);
            true
        }
        "font.inc" => {
            let old = state.font_size_pt;
            mutate_font_size_delta(state, 1);
            old != state.font_size_pt
        }
        "font.dec" => {
            let old = state.font_size_pt;
            mutate_font_size_delta(state, -1);
            old != state.font_size_pt
        }
        "palette.toggle" => {
            state.palette_open = !state.palette_open;
            true
        }
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
        other if other.starts_with("session.kill:") => {
            if let Ok(sid) = other["session.kill:".len()..].parse::<u64>() {
                mutate_kill_session_id(state, sid);
                return true;
            }
            false
        }
        other if other.starts_with("toast.dismiss:") => {
            if let Ok(id) = other["toast.dismiss:".len()..].parse::<u64>() {
                return state.toasts.dismiss(id);
            }
            false
        }
        "dialog.rename_commit" => {
            let Some(ConfirmDialog::RenameSession { pane_id, buffer }) =
                state.confirm_dialog.take()
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
                    log::warn!("rename_session({}): {}", sid, e);
                }
            }
            mutate_rename_pane(state, pane_id, &buffer);
            crate::persist::save_workspaces(state);
            true
        }
        other if other.starts_with("tab.request_rename:") => {
            if let Ok(pane_num) = other["tab.request_rename:".len()..].parse::<u32>() {
                state.ctx_menu = None;
                let current = state
                    .panes
                    .iter()
                    .flat_map(|row| row.iter())
                    .find(|p| p.id.0 == pane_num)
                    .map(|p| p.title.clone())
                    .unwrap_or_default();
                state.confirm_dialog = Some(ConfirmDialog::RenameSession {
                    pane_id: pane_num,
                    buffer: current,
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
            if ws_idx >= state.workspaces.len() {
                log::warn!("terminal.focus: workspace index {} out of range", ws_idx);
                return false;
            }
            let target = PaneId(pane_num);
            let pane_exists = if ws_idx == state.active_workspace {
                find_pane_coord(state, target).is_some()
            } else {
                let ws = &state.workspaces[ws_idx];
                ws.tabs
                    .get(ws.active_tab)
                    .map(|tab| tab.panes.iter().flatten().any(|p| p.id == target))
                    .unwrap_or(false)
            };
            if !pane_exists {
                log::warn!(
                    "terminal.focus: pane {} not found in workspace {}",
                    pane_num,
                    ws_idx
                );
                return false;
            }
            state.ctx_menu = None;
            mutate_switch_workspace(state, ws_idx);
            state.active_pane = target;
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.active_pane = target;
            }
            true
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

/// Default agent list used when seeding or resetting state. Kept in state.rs
/// so `AppState`/`UiSnapshot` share one source of truth for which agents
/// exist. The settings UI walks this list (plus static metadata) to render
/// rows.
pub fn default_agents() -> Vec<Agent> {
    vec![
        Agent {
            key: AgentKey::Claude,
            enabled: true,
        },
        Agent {
            key: AgentKey::Amp,
            enabled: true,
        },
        Agent {
            key: AgentKey::Codex,
            enabled: false,
        },
    ]
}

/// Whether the agent identified by `key` is enabled. Returns `false` if
/// no entry is present (same semantics as `is_on` for unknown toggle keys).
pub fn agent_enabled(state: &UiSnapshot, key: AgentKey) -> bool {
    state
        .agents
        .iter()
        .find(|a| a.key == key)
        .map(|a| a.enabled)
        .unwrap_or(false)
}

/// Flip the enabled flag for `key` in `state.agents`. Other agents are
/// left untouched. No-op if the key is not present.
pub fn mutate_toggle_agent(state: &mut AppState, key: AgentKey) {
    if let Some(agent) = state.agents.iter_mut().find(|a| a.key == key) {
        agent.enabled = !agent.enabled;
    }
}

/// Resize all active terminals and their PTYs to the given column/row count.
pub fn resize_all_terminals(state: &mut AppState, cols: u16, rows: u16) {
    let ids: Vec<u32> = state.terminals.keys().copied().collect();
    for id in ids {
        state.pty_manager.resize(id, cols, rows);
        if let Some(terminal) = state.terminals.get(&id) {
            terminal
                .lock()
                .expect("terminal mutex poisoned")
                .resize(rows as usize, cols as usize);
        }
    }
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

/// CSS base font-size in px. Must match `--t-md` in assets/styles.css.
pub const CSS_BASE_FONT_SIZE: f32 = 12.0;

/// CSS line-height for `.terminal-content`. Must match
/// `.terminal-content { line-height: 1.2; }` in assets/styles.css.
/// If this value drifts from the CSS, the renderer cell_h and the
/// pre-published cell_h will disagree, causing row-height mismatches.
pub const CSS_LINE_HEIGHT: f32 = 1.2;

/// Pre-publish cell metrics to the global atomics so that `on_resize` handlers
/// can compute correct PTY dimensions on the very first frame.
pub fn pre_publish_cell_metrics(scale_factor: f32, cell_width_ratio: f32) -> (f32, f32) {
    let font_size = CSS_BASE_FONT_SIZE * scale_factor;
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

    /// Build a minimal AppState for testing tab/dispatch logic.
    /// Avoids PTY spawning by providing empty panes and terminals directly.
    fn test_state() -> AppState {
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
            settings_section: SettingsSection::General,
            theme: "amber".to_string(),
            font_size_pt: 13,
            toggles: BTreeMap::new(),
            agents: default_agents(),
            palette_open: false,
            sidebar_collapsed: false,
            sidebar_width: 252.0,
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
            toasts: unshit::core::toast::ToastStore::with_capacity(3, 8),
        }
    }

    // -- SettingsSection ------------------------------------------------------

    #[test]
    fn settings_section_labels() {
        assert_eq!(SettingsSection::General.label(), "general");
        assert_eq!(SettingsSection::Appearance.label(), "appearance");
        assert_eq!(SettingsSection::Shell.label(), "shell");
        assert_eq!(SettingsSection::Keybinds.label(), "keybinds");
        assert_eq!(SettingsSection::Agents.label(), "agents");
        assert_eq!(SettingsSection::DangerZone.label(), "danger zone");
    }

    #[test]
    fn settings_section_all_returns_seven() {
        let all = SettingsSection::all();
        assert_eq!(all.len(), 7);
        assert_eq!(all[0], SettingsSection::General);
        assert_eq!(all[4], SettingsSection::Agents);
        assert_eq!(all[5], SettingsSection::Sessions);
        assert_eq!(all[6], SettingsSection::DangerZone);
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
    fn font_size_increments() {
        let mut state = test_state();
        state.font_size_pt = 13;
        mutate_font_size_delta(&mut state, 1);
        assert_eq!(state.font_size_pt, 14);
    }

    #[test]
    fn font_size_clamps_at_max() {
        let mut state = test_state();
        state.font_size_pt = MAX_FONT_SIZE;
        mutate_font_size_delta(&mut state, 1);
        assert_eq!(state.font_size_pt, MAX_FONT_SIZE);
    }

    #[test]
    fn font_size_clamps_at_min() {
        let mut state = test_state();
        state.font_size_pt = MIN_FONT_SIZE;
        mutate_font_size_delta(&mut state, -1);
        assert_eq!(state.font_size_pt, MIN_FONT_SIZE);
    }

    #[test]
    fn font_size_large_delta_clamps() {
        let mut state = test_state();
        state.font_size_pt = 13;
        mutate_font_size_delta(&mut state, 100);
        assert_eq!(state.font_size_pt, MAX_FONT_SIZE);
    }

    // -- dispatch -------------------------------------------------------------

    #[test]
    fn dispatch_modal_open_close() {
        let mut state = test_state();
        assert!(!state.settings_open);

        assert!(dispatch(&mut state, "modal.open"));
        assert!(state.settings_open);

        // Opening again returns false (already open)
        assert!(!dispatch(&mut state, "modal.open"));

        assert!(dispatch(&mut state, "modal.close"));
        assert!(!state.settings_open);

        // Closing again returns false (already closed)
        assert!(!dispatch(&mut state, "modal.close"));
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
        state.font_size_pt = 13;

        assert!(dispatch(&mut state, "font.inc"));
        assert_eq!(state.font_size_pt, 14);

        assert!(dispatch(&mut state, "font.dec"));
        assert_eq!(state.font_size_pt, 13);
    }

    #[test]
    fn dispatch_font_inc_at_max_returns_false() {
        let mut state = test_state();
        state.font_size_pt = MAX_FONT_SIZE;
        assert!(!dispatch(&mut state, "font.inc"));
    }

    #[test]
    fn dispatch_font_dec_at_min_returns_false() {
        let mut state = test_state();
        state.font_size_pt = MIN_FONT_SIZE;
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

    #[test]
    fn resolve_close_action_with_no_preference_opens_prompt_and_vetoes() {
        let mut state = seed_state();
        assert!(!toggle_on(&state, ToggleKey::RememberCloseChoice));
        let action = resolve_close_action(&mut state);
        assert_eq!(action, CloseAction::Prompt);
        assert!(matches!(
            state.confirm_dialog,
            Some(ConfirmDialog::CloseApp {
                remember: false,
                ..
            })
        ));
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
        state.confirm_dialog = Some(ConfirmDialog::CloseApp {
            count: 2,
            remember: false,
        });
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
    fn dialog_toggle_remember_without_close_app_is_noop() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "dialog.toggle_remember"));
        state.confirm_dialog = Some(ConfirmDialog::KillAll { count: 1 });
        assert!(!dispatch(&mut state, "dialog.toggle_remember"));
    }

    #[test]
    fn close_keep_running_without_remember_clears_dialog_and_terminals_only() {
        let mut state = seed_state();
        state.confirm_dialog = Some(ConfirmDialog::CloseApp {
            count: 0,
            remember: false,
        });
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
        state.confirm_dialog = Some(ConfirmDialog::CloseApp {
            count: 0,
            remember: true,
        });
        assert!(dispatch(&mut state, "app.close.keep_running"));
        assert!(toggle_on(&state, ToggleKey::RememberCloseChoice));
        assert!(!toggle_on(&state, ToggleKey::KillAllOnClose));
    }

    #[test]
    fn close_kill_and_quit_with_remember_persists_preference_and_empties() {
        let mut state = seed_state();
        state.confirm_dialog = Some(ConfirmDialog::CloseApp {
            count: 0,
            remember: true,
        });
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
        state.confirm_dialog = Some(ConfirmDialog::CloseApp {
            count: 0,
            remember: false,
        });
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
            Some(ConfirmDialog::RenameSession { pane_id, buffer }) => {
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
            Some(ConfirmDialog::RenameSession { pane_id, buffer }) => {
                assert_eq!(*pane_id, 9999);
                assert!(buffer.is_empty());
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
        });
        assert!(dispatch(&mut state, "dialog.rename_commit"));
        assert_eq!(state.panes[0][0].title, "shell");
    }

    #[test]
    fn dialog_rename_commit_without_dialog_is_noop() {
        let mut state = seed_state();
        assert!(!dispatch(&mut state, "dialog.rename_commit"));
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
        state.toggles.insert(ToggleKey::GlowEffect, true);
        state.toggles.insert(ToggleKey::BackgroundTexture, false);
        let snap = state.ui_snapshot();

        assert!(is_on(&snap, ToggleKey::GlowEffect));
        assert!(!is_on(&snap, ToggleKey::BackgroundTexture));
        assert!(!is_on(&snap, ToggleKey::ConfirmClose));
    }

    // -- agents field (refs #107) --------------------------------------------

    #[test]
    fn default_agents_has_three_entries() {
        let agents = default_agents();
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[0].key, AgentKey::Claude);
        assert_eq!(agents[1].key, AgentKey::Amp);
        assert_eq!(agents[2].key, AgentKey::Codex);
    }

    #[test]
    fn seed_state_has_expected_agent_entries() {
        let state = seed_state();
        assert_eq!(state.agents.len(), 3);
        assert!(state.agents.iter().any(|a| a.key == AgentKey::Claude));
        assert!(state.agents.iter().any(|a| a.key == AgentKey::Amp));
        assert!(state.agents.iter().any(|a| a.key == AgentKey::Codex));
    }

    #[test]
    fn seed_state_agent_defaults_match_legacy_toggles() {
        // Prior to the split these lived in `toggles`: claude=on, amp=on, codex=off.
        let state = seed_state();
        assert!(agent_enabled(&state.ui_snapshot(), AgentKey::Claude));
        assert!(agent_enabled(&state.ui_snapshot(), AgentKey::Amp));
        assert!(!agent_enabled(&state.ui_snapshot(), AgentKey::Codex));
    }

    #[test]
    fn agent_enabled_returns_false_when_missing() {
        let mut state = test_state();
        state.agents.clear();
        let snap = state.ui_snapshot();
        assert!(!agent_enabled(&snap, AgentKey::Claude));
    }

    #[test]
    fn mutate_toggle_agent_flips_only_target() {
        // Regression: toggling one agent must not change sibling agents.
        let mut state = test_state();
        let before_amp = agent_enabled(&state.ui_snapshot(), AgentKey::Amp);
        let before_codex = agent_enabled(&state.ui_snapshot(), AgentKey::Codex);

        mutate_toggle_agent(&mut state, AgentKey::Claude);

        let snap = state.ui_snapshot();
        assert!(!agent_enabled(&snap, AgentKey::Claude));
        assert_eq!(agent_enabled(&snap, AgentKey::Amp), before_amp);
        assert_eq!(agent_enabled(&snap, AgentKey::Codex), before_codex);
    }

    #[test]
    fn mutate_toggle_agent_round_trips() {
        let mut state = test_state();
        let before = agent_enabled(&state.ui_snapshot(), AgentKey::Codex);
        mutate_toggle_agent(&mut state, AgentKey::Codex);
        mutate_toggle_agent(&mut state, AgentKey::Codex);
        let after = agent_enabled(&state.ui_snapshot(), AgentKey::Codex);
        assert_eq!(before, after);
    }

    #[test]
    fn agent_key_as_str_is_stable() {
        assert_eq!(AgentKey::Claude.as_str(), "agent-claude");
        assert_eq!(AgentKey::Amp.as_str(), "agent-amp");
        assert_eq!(AgentKey::Codex.as_str(), "agent-codex");
    }

    // -- ui_snapshot ----------------------------------------------------------

    #[test]
    fn ui_snapshot_copies_fields() {
        let mut state = test_state();
        state.font_size_pt = 20;
        state.theme = "dracula".to_string();
        state.sidebar_collapsed = true;

        let snap = state.ui_snapshot();
        assert_eq!(snap.font_size_pt, 20);
        assert_eq!(snap.theme, "dracula");
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
        assert_eq!(state.font_size_pt, 13);
        assert!(!state.settings_open);
        assert!(!state.sidebar_collapsed);
        assert!(!state.palette_open);
    }

    // -- mutate_with ----------------------------------------------------------

    #[test]
    fn mutate_with_applies_closure() {
        let shared: SharedState = std::sync::Arc::new(std::sync::Mutex::new(test_state()));
        let result = mutate_with(&shared, |st| {
            st.font_size_pt = 25;
            st.font_size_pt
        });
        assert_eq!(result, 25);
        let guard = shared.lock().unwrap();
        assert_eq!(guard.font_size_pt, 25);
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
        let (cell_w, cell_h) = pre_publish_cell_metrics(1.0, 0.6);
        assert!(cell_w > 0.0, "cell_w must be positive, got {}", cell_w);
        assert!(cell_h > 0.0, "cell_h must be positive, got {}", cell_h);
    }

    #[test]
    fn pre_publish_scales_with_dpi() {
        let ratio = 0.6_f32;
        let (w1, h1) = pre_publish_cell_metrics(1.0, ratio);
        let (w2, h2) = pre_publish_cell_metrics(2.0, ratio);
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
        let (_, cell_h) = pre_publish_cell_metrics(scale, ratio);
        let expected = CSS_BASE_FONT_SIZE * scale * CSS_LINE_HEIGHT;
        assert!(
            (cell_h - expected).abs() < f32::EPSILON,
            "cell_h ({}) must equal font_size * CSS_LINE_HEIGHT ({})",
            cell_h,
            expected,
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
        let ratio = measure_cell_width_ratio_at(12.0, 12.0 * CSS_LINE_HEIGHT);
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
        let ratio = measure_cell_width_ratio_at(12.0, 12.0 * CSS_LINE_HEIGHT);
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
}
