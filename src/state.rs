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

#[derive(Clone, Debug)]
pub struct CtxMenu {
    pub x: f32,
    pub y: f32,
    pub workspace_idx: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsSection {
    General,
    Appearance,
    Shell,
    Keybinds,
    Agents,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::General => "general",
            SettingsSection::Appearance => "appearance",
            SettingsSection::Shell => "shell",
            SettingsSection::Keybinds => "keybinds",
            SettingsSection::Agents => "agents",
        }
    }

    pub fn all() -> [SettingsSection; 5] {
        [
            SettingsSection::General,
            SettingsSection::Appearance,
            SettingsSection::Shell,
            SettingsSection::Keybinds,
            SettingsSection::Agents,
        ]
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
    pub toggles: BTreeMap<String, bool>,
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
    pub pty_manager: crate::pty::PtyManager,
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
            if idx == active_idx {
                let entries: Vec<TerminalEntry> = self
                    .panes
                    .iter()
                    .flatten()
                    .map(|p| TerminalEntry {
                        name: p.title.clone(),
                        branch: branch_text.clone(),
                        branch_muted: false,
                        branch_error,
                        pane_id: p.id,
                    })
                    .collect();
                for sub in &mut ws.subtabs {
                    if sub.label == "terminals" {
                        sub.count = Some(entries.len() as u32);
                    }
                }
                ws.terminal_entries = entries;
            } else {
                let entries: Vec<TerminalEntry> = ws
                    .tabs
                    .get(ws.active_tab)
                    .map(|tab| tab.panes.iter().flatten().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| TerminalEntry {
                        name: p.title.clone(),
                        branch: branch_text.clone(),
                        branch_muted: false,
                        branch_error,
                        pane_id: p.id,
                    })
                    .collect();
                for sub in &mut ws.subtabs {
                    if sub.label == "terminals" {
                        sub.count = Some(entries.len() as u32);
                    }
                }
                ws.terminal_entries = entries;
            }
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
        }
    }

    /// Clone the cell grid for a given pane. Returns `None` if no terminal
    /// exists for the pane. The returned grid is a snapshot; further writes
    /// to the live terminal won't affect it.
    pub fn terminal_grid(&self, pane_id: PaneId) -> Option<unshit::core::cell_grid::CellGrid> {
        self.terminals
            .get(&pane_id.0)
            .map(|t| t.lock().expect("terminal mutex poisoned").grid().clone())
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
    pub toggles: BTreeMap<String, bool>,
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
    toggles.insert("restore-on-startup".to_string(), true);
    toggles.insert("glow-effect".to_string(), true);
    toggles.insert("background-texture".to_string(), true);
    toggles.insert("shell-integration".to_string(), true);

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
        palette_open: false,
        sidebar_collapsed: false,
        sidebar_width: 252.0,
        sidebar_drag_start: None,
        cpu_pct: 0.0,
        mem_gb: 0.0,
        net_kbps: 0.0,
        clock_hhmm: "00:00".to_string(),
        next_id: 2,
        pty_manager: crate::pty::PtyManager::new(),
        terminals: std::collections::HashMap::new(),
        scale_factor: 1.0,
        cell_width_ratio: 0.6,
        last_grid_width: 0.0,
        last_grid_height: 0.0,
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
        resize_drag: None,
        ctx_menu: None,
    }
}

// ---------------------------------------------------------------------------
// State mutation helpers
// ---------------------------------------------------------------------------

pub fn mutate_with<F, R>(shared: &SharedState, f: F) -> R
where
    F: FnOnce(&mut AppState) -> R,
{
    let mut guard = shared.lock().expect("state mutex poisoned");
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
    let mut terminal = crate::terminal::Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn(id_num, cols, rows) {
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
        // mutate_add_tab handles creating a fresh tab with PTY + pane.
        mutate_add_tab(state);
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
    state.active_workspace = state.workspaces.len() - 1;
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
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state
        .pty_manager
        .spawn_in(id_num, cols, rows, cwd.as_deref())
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
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state
        .pty_manager
        .spawn_in(id_num, cols, rows, cwd.as_deref())
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
        let mut terminal = Terminal::new(rows as usize, cols as usize);
        match state
            .pty_manager
            .spawn_in(id_num, cols, rows, cwd.as_deref())
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

        state.panes.push(vec![Pane {
            id: pane_id,
            title: "shell".to_string(),
            subtitle: "bash".to_string(),
            pid: 0,
            cpu: 0.0,
        }]);
        state.row_ratios = vec![1.0];
        state.col_ratios = vec![vec![1.0]];
        state.active_pane = pane_id;
        return;
    }
    if state.active_pane == target {
        let new_row = row_idx.min(state.panes.len() - 1);
        let new_col = col_idx.min(state.panes[new_row].len() - 1);
        state.active_pane = state.panes[new_row][new_col].id;
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
            changed
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
        _ => false,
    }
}

/// Return the working directory for the active workspace, falling back to home.
pub fn active_workspace_cwd(state: &AppState) -> Option<PathBuf> {
    state
        .workspaces
        .get(state.active_workspace)
        .and_then(|ws| ws.path.clone())
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

pub fn is_on(state: &UiSnapshot, key: &str) -> bool {
    state.toggles.get(key).copied().unwrap_or(false)
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
            palette_open: false,
            sidebar_collapsed: false,
            sidebar_width: 252.0,
            sidebar_drag_start: None,
            cpu_pct: 0.0,
            mem_gb: 0.0,
            net_kbps: 0.0,
            clock_hhmm: "12:00".to_string(),
            next_id: 2,
            pty_manager: crate::pty::PtyManager::new(),
            terminals: std::collections::HashMap::new(),
            scale_factor: 1.0,
            cell_width_ratio: 0.6,
            last_grid_width: 0.0,
            last_grid_height: 0.0,
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
            resize_drag: None,
            ctx_menu: None,
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
    }

    #[test]
    fn settings_section_all_returns_five() {
        let all = SettingsSection::all();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0], SettingsSection::General);
        assert_eq!(all[4], SettingsSection::Agents);
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
    fn close_last_tab_creates_new_one() {
        let mut state = test_state();
        // only one tab
        mutate_close_tab(&mut state, 0);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.active_tab, 0);
        assert_eq!(state.tabs[0].id, "t2"); // new tab was created
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
            workspace_idx: 0,
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
            workspace_idx: 0,
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
            workspace_idx: 0,
        });
        assert!(dispatch(&mut state, "workspace.remove:1"));
        assert!(state.ctx_menu.is_none());
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
        state.toggles.insert("glow".to_string(), true);
        state.toggles.insert("dim".to_string(), false);
        let snap = state.ui_snapshot();

        assert!(is_on(&snap, "glow"));
        assert!(!is_on(&snap, "dim"));
        assert!(!is_on(&snap, "nonexistent"));
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
    fn close_last_pane_auto_creates_new_one() {
        let mut state = seed_state();
        let original_pane = state.active_pane;

        mutate_close_pane(&mut state, original_pane);

        // Should still have exactly one pane
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 1);
        // The new pane should have a different id
        assert_ne!(state.active_pane, original_pane);
        // Old terminal removed
        assert!(!state.terminals.contains_key(&original_pane.0));
        // New terminal created
        assert!(state.terminals.contains_key(&state.active_pane.0));
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
}
