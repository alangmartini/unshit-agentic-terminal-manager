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

#[derive(Clone, Debug)]
pub struct AgentEntry {
    pub name: String,
    pub path: String,
    pub status: AgentStatus,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentStatus {
    Running,
    Idle,
    Disabled,
}

impl AgentStatus {
    pub fn css_class(self) -> &'static str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::Idle => "idle",
            AgentStatus::Disabled => "disabled",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::Idle => "idle",
            AgentStatus::Disabled => "disabled",
        }
    }
}

pub const KEYBINDS: &[(&str, &[&str])] = &[
    ("New terminal", &["Ctrl", "T"]),
    ("Close tab", &["Ctrl", "W"]),
    ("Split right", &["Ctrl", "D"]),
    ("Split down", &["Ctrl", "Shift", "D"]),
    ("Next tab", &["Ctrl", "Tab"]),
    ("Previous tab", &["Ctrl", "Shift", "Tab"]),
    ("Command palette", &["Ctrl", "K"]),
    ("Toggle sidebar", &["Ctrl", "B"]),
    ("Settings", &["Ctrl", ","]),
    ("Zoom in", &["Ctrl", "="]),
    ("Zoom out", &["Ctrl", "-"]),
    ("Fullscreen", &["F11"]),
];

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
    pub cursor_style: String,
    pub opacity: u32,
    pub line_height_10x: u32,
    pub agent_timeout: u32,
    pub toggles: BTreeMap<String, bool>,
    pub agents: Vec<AgentEntry>,
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
    pub terminals: std::collections::HashMap<u32, Terminal>,
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
}

impl AppState {
    /// Clone everything except the non-Clone PTY manager and terminals.
    /// UI builders call this to get a snapshot for rendering.
    pub fn ui_snapshot(&self) -> UiSnapshot {
        let mut workspaces = self.workspaces.clone();
        // Populate the active workspace's terminal entries from actual panes.
        if let Some(ws) = workspaces.get_mut(self.active_workspace) {
            let entries: Vec<TerminalEntry> = self
                .panes
                .iter()
                .flatten()
                .map(|p| TerminalEntry {
                    name: p.title.clone(),
                    branch: "main".to_string(),
                    branch_muted: false,
                })
                .collect();
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
            cursor_style: self.cursor_style.clone(),
            opacity: self.opacity,
            line_height_10x: self.line_height_10x,
            agent_timeout: self.agent_timeout,
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
        }
    }

    pub fn terminal_grid(&self, pane_id: PaneId) -> Option<&unshit::core::cell_grid::CellGrid> {
        self.terminals.get(&pane_id.0).map(|t| t.grid())
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
    pub cursor_style: String,
    pub opacity: u32,
    pub line_height_10x: u32,
    pub agent_timeout: u32,
    pub toggles: BTreeMap<String, bool>,
    pub agents: Vec<AgentEntry>,
    pub palette_open: bool,
    pub sidebar_collapsed: bool,
    pub sidebar_width: f32,
    pub cpu_pct: f32,
    pub mem_gb: f32,
    pub net_kbps: f32,
    pub clock_hhmm: String,
    pub row_ratios: Vec<f32>,
    pub col_ratios: Vec<Vec<f32>>,
}

fn current_folder_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "workspace".to_string())
}

pub fn seed_state() -> AppState {
    let workspaces = vec![
        Workspace {
            num: 1,
            name: current_folder_name(),
            path: std::env::current_dir().ok(),
            collapsed: false,
            terminals_expanded: true,
            terminal_entries: vec![
                TerminalEntry {
                    name: "shell".to_string(),
                    branch: "main".to_string(),
                    branch_muted: false,
                },
                TerminalEntry {
                    name: "build".to_string(),
                    branch: "main".to_string(),
                    branch_muted: false,
                },
                TerminalEntry {
                    name: "dev-server".to_string(),
                    branch: "main".to_string(),
                    branch_muted: false,
                },
                TerminalEntry {
                    name: "logs".to_string(),
                    branch: "main".to_string(),
                    branch_muted: false,
                },
            ],
            subtabs: vec![
                Subtab {
                    label: "terminals".to_string(),
                    count: Some(4),
                    pulse: false,
                    active: true,
                    disabled: false,
                    icon: Some(SubtabIcon::Terminal),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "agents".to_string(),
                    count: Some(2),
                    pulse: true,
                    active: false,
                    disabled: true,
                    icon: Some(SubtabIcon::User),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "worktrees".to_string(),
                    count: Some(3),
                    pulse: false,
                    active: false,
                    disabled: true,
                    icon: Some(SubtabIcon::GitBranch),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "sessions".to_string(),
                    count: Some(1),
                    pulse: false,
                    active: false,
                    disabled: true,
                    icon: Some(SubtabIcon::Folder),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "environment".to_string(),
                    count: None,
                    pulse: false,
                    active: false,
                    disabled: true,
                    icon: Some(SubtabIcon::EnvList),
                    tree_glyph: "\u{2514}",
                },
            ],
        },
        Workspace {
            num: 2,
            name: "api".to_string(),
            path: None,
            collapsed: false,
            terminals_expanded: false,
            terminal_entries: vec![
                TerminalEntry {
                    name: "shell".to_string(),
                    branch: "fix/pdf-export".to_string(),
                    branch_muted: false,
                },
                TerminalEntry {
                    name: "tests".to_string(),
                    branch: "fix/pdf-export".to_string(),
                    branch_muted: false,
                },
            ],
            subtabs: vec![
                Subtab {
                    label: "terminals".to_string(),
                    count: Some(2),
                    pulse: false,
                    active: false,
                    disabled: false,
                    icon: Some(SubtabIcon::Terminal),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "agents".to_string(),
                    count: Some(1),
                    pulse: false,
                    active: false,
                    disabled: true,
                    icon: Some(SubtabIcon::User),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "worktrees".to_string(),
                    count: Some(2),
                    pulse: false,
                    active: false,
                    disabled: true,
                    icon: Some(SubtabIcon::GitBranch),
                    tree_glyph: "\u{2514}",
                },
            ],
        },
        Workspace {
            num: 3,
            name: "infra".to_string(),
            path: None,
            collapsed: true,
            terminals_expanded: false,
            terminal_entries: vec![TerminalEntry {
                name: "shell".to_string(),
                branch: "staging".to_string(),
                branch_muted: false,
            }],
            subtabs: vec![
                Subtab {
                    label: "terminals".to_string(),
                    count: Some(1),
                    pulse: false,
                    active: false,
                    disabled: false,
                    icon: Some(SubtabIcon::Terminal),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "sessions".to_string(),
                    count: None,
                    pulse: false,
                    active: false,
                    disabled: true,
                    icon: Some(SubtabIcon::Folder),
                    tree_glyph: "\u{2514}",
                },
            ],
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
    toggles.insert("confirm-before-closing".to_string(), true);
    toggles.insert("check-for-updates".to_string(), true);
    toggles.insert("start-minimized".to_string(), false);
    toggles.insert("scroll-on-output".to_string(), true);
    toggles.insert("bell-notification".to_string(), false);
    toggles.insert("font-ligatures".to_string(), true);
    toggles.insert("auto-discovery".to_string(), true);

    let agents = vec![
        AgentEntry {
            name: "claude".to_string(),
            path: "~/.local/bin/claude".to_string(),
            status: AgentStatus::Running,
            enabled: true,
        },
        AgentEntry {
            name: "amp".to_string(),
            path: "~/.local/bin/amp".to_string(),
            status: AgentStatus::Idle,
            enabled: true,
        },
        AgentEntry {
            name: "codex".to_string(),
            path: "~/.local/bin/codex".to_string(),
            status: AgentStatus::Disabled,
            enabled: false,
        },
    ];

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
        cursor_style: "block".to_string(),
        opacity: 100,
        line_height_10x: 14,
        agent_timeout: 300,
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
        pty_manager: crate::pty::PtyManager::new(),
        terminals: std::collections::HashMap::new(),
        scale_factor: 1.0,
        cell_width_ratio: 0.6,
        last_grid_width: 0.0,
        last_grid_height: 0.0,
        row_ratios: vec![1.0],
        col_ratios: vec![vec![1.0]],
        resize_drag: None,
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
            state.terminals.insert(id_num, terminal);
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!("failed to spawn PTY for new tab pane {}: {}", id_num, e);
            terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
            state.terminals.insert(id_num, terminal);
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
    state.workspaces.push(Workspace {
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
    });
    state.active_workspace = state.workspaces.len() - 1;
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
            state.terminals.insert(id_num, terminal);
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!("failed to spawn PTY for pane {}: {}", id_num, e);
            terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
            state.terminals.insert(id_num, terminal);
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
            state.terminals.insert(id_num, terminal);
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!("failed to spawn PTY for pane {}: {}", id_num, e);
            terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
            state.terminals.insert(id_num, terminal);
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
                state.terminals.insert(id_num, terminal);
                crate::bridge::register_reader(id_num, reader);
            }
            Err(e) => {
                log::error!("failed to spawn PTY for pane {}: {}", id_num, e);
                terminal.process_bytes(format!("error: {}\r\n", e).as_bytes());
                state.terminals.insert(id_num, terminal);
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
            if state.settings_open {
                state.settings_open = false;
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
        "line_height.inc" => {
            if state.line_height_10x < 25 {
                state.line_height_10x += 1;
                true
            } else {
                false
            }
        }
        "line_height.dec" => {
            if state.line_height_10x > 10 {
                state.line_height_10x -= 1;
                true
            } else {
                false
            }
        }
        "agent_timeout.inc" => {
            if state.agent_timeout < 600 {
                state.agent_timeout = (state.agent_timeout + 30).min(600);
                true
            } else {
                false
            }
        }
        "agent_timeout.dec" => {
            if state.agent_timeout > 30 {
                state.agent_timeout = state.agent_timeout.saturating_sub(30).max(30);
                true
            } else {
                false
            }
        }
        other if other.starts_with("cursor.set:") => {
            let style = &other["cursor.set:".len()..];
            if matches!(style, "block" | "line" | "bar") && state.cursor_style != style {
                state.cursor_style = style.to_string();
                true
            } else {
                false
            }
        }
        other if other.starts_with("opacity.set:") => {
            if let Ok(val) = other["opacity.set:".len()..].parse::<u32>() {
                let clamped = val.clamp(50, 100);
                if state.opacity != clamped {
                    state.opacity = clamped;
                    return true;
                }
            }
            false
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
        if let Some(terminal) = state.terminals.get_mut(&id) {
            terminal.resize(rows as usize, cols as usize);
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
            cursor_style: "block".to_string(),
            opacity: 100,
            line_height_10x: 14,
            agent_timeout: 300,
            toggles: BTreeMap::new(),
            agents: vec![],
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
        state.terminals.insert(1, Terminal::new(24, 80));
        state.terminals.insert(2, Terminal::new(24, 80));

        resize_all_terminals(&mut state, 120, 40);

        for (_, term) in &state.terminals {
            let grid = term.grid();
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

    // -- new settings state fields --------------------------------------------

    #[test]
    fn seed_state_defaults_cursor_style_block() {
        let state = seed_state();
        assert_eq!(state.cursor_style, "block");
    }

    #[test]
    fn seed_state_defaults_opacity_100() {
        let state = seed_state();
        assert_eq!(state.opacity, 100);
    }

    #[test]
    fn seed_state_defaults_line_height_14() {
        let state = seed_state();
        assert_eq!(state.line_height_10x, 14);
    }

    #[test]
    fn seed_state_defaults_agent_timeout_300() {
        let state = seed_state();
        assert_eq!(state.agent_timeout, 300);
    }

    #[test]
    fn seed_state_has_eleven_toggles() {
        let state = seed_state();
        assert_eq!(state.toggles.len(), 11);
    }

    #[test]
    fn seed_state_has_three_agents() {
        let state = seed_state();
        assert_eq!(state.agents.len(), 3);
        assert_eq!(state.agents[0].name, "claude");
        assert_eq!(state.agents[1].name, "amp");
        assert_eq!(state.agents[2].name, "codex");
    }

    #[test]
    fn keybinds_has_twelve_entries() {
        assert_eq!(KEYBINDS.len(), 12);
    }

    #[test]
    fn agent_status_css_class() {
        assert_eq!(AgentStatus::Running.css_class(), "running");
        assert_eq!(AgentStatus::Idle.css_class(), "idle");
        assert_eq!(AgentStatus::Disabled.css_class(), "disabled");
    }

    #[test]
    fn ui_snapshot_copies_new_fields() {
        let state = seed_state();
        let snap = state.ui_snapshot();
        assert_eq!(snap.cursor_style, "block");
        assert_eq!(snap.opacity, 100);
        assert_eq!(snap.line_height_10x, 14);
        assert_eq!(snap.agent_timeout, 300);
        assert_eq!(snap.agents.len(), 3);
    }

    // -- dispatch: cursor.set -------------------------------------------------

    #[test]
    fn dispatch_cursor_set_line() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "cursor.set:line"));
        assert_eq!(state.cursor_style, "line");
    }

    #[test]
    fn dispatch_cursor_set_bar() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "cursor.set:bar"));
        assert_eq!(state.cursor_style, "bar");
    }

    #[test]
    fn dispatch_cursor_set_same_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "cursor.set:block")); // already block
    }

    #[test]
    fn dispatch_cursor_set_invalid_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "cursor.set:unknown"));
    }

    // -- dispatch: opacity.set ------------------------------------------------

    #[test]
    fn dispatch_opacity_set() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "opacity.set:75"));
        assert_eq!(state.opacity, 75);
    }

    #[test]
    fn dispatch_opacity_set_clamps_low() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "opacity.set:10"));
        assert_eq!(state.opacity, 50);
    }

    #[test]
    fn dispatch_opacity_set_clamps_high() {
        let mut state = test_state();
        state.opacity = 90;
        assert!(dispatch(&mut state, "opacity.set:200"));
        assert_eq!(state.opacity, 100);
    }

    #[test]
    fn dispatch_opacity_set_same_returns_false() {
        let mut state = test_state();
        assert!(!dispatch(&mut state, "opacity.set:100")); // already 100
    }

    // -- dispatch: line_height.inc/dec ----------------------------------------

    #[test]
    fn dispatch_line_height_inc() {
        let mut state = test_state();
        assert_eq!(state.line_height_10x, 14);
        assert!(dispatch(&mut state, "line_height.inc"));
        assert_eq!(state.line_height_10x, 15);
    }

    #[test]
    fn dispatch_line_height_dec() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "line_height.dec"));
        assert_eq!(state.line_height_10x, 13);
    }

    #[test]
    fn dispatch_line_height_inc_clamps_at_25() {
        let mut state = test_state();
        state.line_height_10x = 25;
        assert!(!dispatch(&mut state, "line_height.inc"));
    }

    #[test]
    fn dispatch_line_height_dec_clamps_at_10() {
        let mut state = test_state();
        state.line_height_10x = 10;
        assert!(!dispatch(&mut state, "line_height.dec"));
    }

    // -- dispatch: agent_timeout.inc/dec --------------------------------------

    #[test]
    fn dispatch_agent_timeout_inc() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "agent_timeout.inc"));
        assert_eq!(state.agent_timeout, 330);
    }

    #[test]
    fn dispatch_agent_timeout_dec() {
        let mut state = test_state();
        assert!(dispatch(&mut state, "agent_timeout.dec"));
        assert_eq!(state.agent_timeout, 270);
    }

    #[test]
    fn dispatch_agent_timeout_inc_clamps_at_600() {
        let mut state = test_state();
        state.agent_timeout = 600;
        assert!(!dispatch(&mut state, "agent_timeout.inc"));
    }

    #[test]
    fn dispatch_agent_timeout_dec_clamps_at_30() {
        let mut state = test_state();
        state.agent_timeout = 30;
        assert!(!dispatch(&mut state, "agent_timeout.dec"));
    }
}
