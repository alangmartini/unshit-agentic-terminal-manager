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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal AppState for testing tab/dispatch logic.
    /// Avoids PTY spawning by providing empty panes and terminals directly.
    fn test_state() -> AppState {
        let tabs = vec![TerminalTab {
            id: "t1".to_string(),
            name: "shell".to_string(),
            subtitle: "bash".to_string(),
            status: TabStatus::Running,
        }];
        let pane = Pane {
            id: PaneId(1),
            title: "shell".to_string(),
            subtitle: "bash".to_string(),
            pid: 0,
            cpu: 0.0,
        };
        AppState {
            workspaces: vec![],
            active_workspace: 0,
            tabs,
            active_tab: 0,
            panes: vec![vec![pane]],
            active_pane: PaneId(1),
            settings_open: false,
            settings_section: SettingsSection::General,
            theme: "amber".to_string(),
            font_size_pt: 13,
            toggles: BTreeMap::new(),
            palette_open: false,
            sidebar_collapsed: false,
            cpu_pct: 0.0,
            mem_gb: 0.0,
            net_kbps: 0.0,
            clock_hhmm: "12:00".to_string(),
            next_id: 2,
            pty_manager: crate::pty::PtyManager::new(),
            terminals: std::collections::HashMap::new(),
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
}
