use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::terminal::Terminal;

pub const MAX_COLS: usize = 4;
pub const MAX_ROWS: usize = 4;
pub const MIN_FONT_SIZE: u32 = 8;
pub const MAX_FONT_SIZE: u32 = 32;

pub type SharedState = Arc<Mutex<AppState>>;

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
    pub branch: String,
    pub branch_muted: bool,
    pub collapsed: bool,
    pub subtabs: Vec<Subtab>,
}

#[derive(Clone, Debug)]
pub struct Subtab {
    pub label: String,
    pub count: Option<u32>,
    pub pulse: bool,
    pub active: bool,
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
pub struct TerminalTab {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub status: TabStatus,
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
    pub cpu_pct: f32,
    pub mem_gb: f32,
    pub net_kbps: f32,
    pub clock_hhmm: String,
    pub next_id: u32,
    pub pty_manager: crate::pty::PtyManager,
    pub terminals: std::collections::HashMap<u32, Terminal>,
}

impl AppState {
    /// Clone everything except the non-Clone PTY manager and terminals.
    /// UI builders call this to get a snapshot for rendering.
    pub fn ui_snapshot(&self) -> UiSnapshot {
        UiSnapshot {
            workspaces: self.workspaces.clone(),
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
            cpu_pct: self.cpu_pct,
            mem_gb: self.mem_gb,
            net_kbps: self.net_kbps,
            clock_hhmm: self.clock_hhmm.clone(),
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
    pub toggles: BTreeMap<String, bool>,
    pub palette_open: bool,
    pub sidebar_collapsed: bool,
    pub cpu_pct: f32,
    pub mem_gb: f32,
    pub net_kbps: f32,
    pub clock_hhmm: String,
}

pub fn seed_state() -> AppState {
    let workspaces = vec![
        Workspace {
            num: 1,
            name: "main".to_string(),
            branch: "main".to_string(),
            branch_muted: false,
            collapsed: false,
            subtabs: vec![
                Subtab {
                    label: "terminals".to_string(),
                    count: Some(4),
                    pulse: false,
                    active: true,
                    icon: Some(SubtabIcon::Terminal),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "agents".to_string(),
                    count: Some(2),
                    pulse: true,
                    active: false,
                    icon: Some(SubtabIcon::User),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "worktrees".to_string(),
                    count: Some(3),
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::GitBranch),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "sessions".to_string(),
                    count: Some(1),
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::Folder),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "environment".to_string(),
                    count: None,
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::EnvList),
                    tree_glyph: "\u{2514}",
                },
            ],
        },
        Workspace {
            num: 2,
            name: "api".to_string(),
            branch: "fix/pdf-export".to_string(),
            branch_muted: false,
            collapsed: false,
            subtabs: vec![
                Subtab {
                    label: "terminals".to_string(),
                    count: Some(2),
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::Terminal),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "agents".to_string(),
                    count: Some(1),
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::User),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "worktrees".to_string(),
                    count: Some(2),
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::GitBranch),
                    tree_glyph: "\u{2514}",
                },
            ],
        },
        Workspace {
            num: 3,
            name: "infra".to_string(),
            branch: "staging".to_string(),
            branch_muted: false,
            collapsed: true,
            subtabs: vec![
                Subtab {
                    label: "terminals".to_string(),
                    count: Some(1),
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::Terminal),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "sessions".to_string(),
                    count: None,
                    pulse: false,
                    active: false,
                    icon: Some(SubtabIcon::Folder),
                    tree_glyph: "\u{2514}",
                },
            ],
        },
        Workspace {
            num: 4,
            name: "scratch".to_string(),
            branch: "no branch".to_string(),
            branch_muted: true,
            collapsed: true,
            subtabs: vec![Subtab {
                label: "terminals".to_string(),
                count: Some(0),
                pulse: false,
                active: false,
                icon: None,
                tree_glyph: "\u{2514}",
            }],
        },
    ];

    let tabs = vec![TerminalTab {
        id: "t1".to_string(),
        name: "shell".to_string(),
        subtitle: "bash".to_string(),
        status: TabStatus::Running,
    }];

    let default_pane = Pane {
        id: PaneId(1),
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 0,
        cpu: 0.0,
    };
    let panes = vec![vec![default_pane]];

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
        cpu_pct: 0.0,
        mem_gb: 0.0,
        net_kbps: 0.0,
        clock_hhmm: "00:00".to_string(),
        next_id: 2,
        pty_manager: crate::pty::PtyManager::new(),
        terminals: std::collections::HashMap::new(),
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

pub fn mutate_add_tab(state: &mut AppState) {
    let id_num = state.next_id;
    state.next_id += 1;
    let id = format!("t{}", id_num);
    state.tabs.push(TerminalTab {
        id,
        name: "shell".to_string(),
        subtitle: "bash".to_string(),
        status: TabStatus::Running,
    });
    state.active_tab = state.tabs.len() - 1;
}

pub fn mutate_close_tab(state: &mut AppState, index: usize) {
    if index >= state.tabs.len() {
        return;
    }
    state.tabs.remove(index);
    if state.tabs.is_empty() {
        mutate_add_tab(state);
        state.active_tab = 0;
        return;
    }
    if state.active_tab == index {
        state.active_tab = index.min(state.tabs.len() - 1);
    } else if state.active_tab > index {
        state.active_tab -= 1;
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

    // Spawn a PTY and terminal for the new pane.
    let (cols, rows) = (80u16, 24u16);
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn(id_num, cols, rows) {
        Ok(reader) => {
            // Reader will be wired to a subscription by the bridge.
            // Store it temporarily so the bridge can pick it up.
            state.terminals.insert(id_num, terminal);
            // Store the reader in a pending slot.
            crate::bridge::register_reader(id_num, reader);
        }
        Err(e) => {
            log::error!("failed to spawn PTY for pane {}: {}", id_num, e);
            // Still create the terminal so the pane renders.
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

    let (cols, rows) = (80u16, 24u16);
    let mut terminal = Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn(id_num, cols, rows) {
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
    state.active_pane = pane_id;
}

pub fn mutate_close_pane(state: &mut AppState, target: PaneId) {
    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        return;
    };

    // Destroy the PTY and terminal.
    state.pty_manager.destroy(target.0);
    state.terminals.remove(&target.0);

    state.panes[row_idx].remove(col_idx);
    if state.panes[row_idx].is_empty() {
        state.panes.remove(row_idx);
    }
    if state.panes.is_empty() {
        let id_num = state.next_id;
        state.next_id += 1;
        let pane_id = PaneId(id_num);

        let (cols, rows) = (80u16, 24u16);
        let mut terminal = Terminal::new(rows as usize, cols as usize);
        match state.pty_manager.spawn(id_num, cols, rows) {
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
        state.active_pane = pane_id;
        return;
    }
    if state.active_pane == target {
        let new_row = row_idx.min(state.panes.len() - 1);
        let new_col = col_idx.min(state.panes[new_row].len() - 1);
        state.active_pane = state.panes[new_row][new_col].id;
    }
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
            if state.tabs.is_empty() {
                return false;
            }
            state.active_tab = (state.active_tab + 1) % state.tabs.len();
            true
        }
        "tab.prev" => {
            if state.tabs.is_empty() {
                return false;
            }
            state.active_tab = if state.active_tab == 0 {
                state.tabs.len() - 1
            } else {
                state.active_tab - 1
            };
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
                    state.active_tab = index;
                    return true;
                }
            }
            false
        }
        _ => false,
    }
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
