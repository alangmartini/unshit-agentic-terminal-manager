use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::terminal::Terminal;

pub const MAX_COLS: usize = 4;
pub const MAX_ROWS: usize = 4;
pub const MIN_FONT_SIZE: u32 = 8;
pub const MAX_FONT_SIZE: u32 = 32;

pub type SharedState = Arc<Mutex<AppState>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsSection { General, Appearance, Shell, Keybinds, Agents }
impl SettingsSection {
    pub fn label(self) -> &'static str { match self { Self::General => "general", Self::Appearance => "appearance", Self::Shell => "shell", Self::Keybinds => "keybinds", Self::Agents => "agents" } }
    pub fn all() -> [Self; 5] { [Self::General, Self::Appearance, Self::Shell, Self::Keybinds, Self::Agents] }
}

#[derive(Clone, Debug)]
pub struct Workspace { pub num: u32, pub name: String, pub branch: String, pub branch_muted: bool, pub collapsed: bool, pub subtabs: Vec<Subtab> }
#[derive(Clone, Debug)]
pub struct Subtab { pub label: String, pub count: Option<u32>, pub pulse: bool, pub active: bool, pub icon: Option<SubtabIcon>, pub tree_glyph: &'static str }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubtabIcon { Terminal, User, GitBranch, Folder, EnvList }
#[derive(Clone, Debug)]
pub struct TerminalTab { pub id: String, pub name: String, pub subtitle: String, pub status: TabStatus }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabStatus { Running, Idle, Stopped }
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PaneId(pub u32);
#[derive(Clone, Debug)]
pub struct Pane { pub id: PaneId, pub title: String, pub subtitle: String, pub pid: u32, pub cpu: f32 }

pub struct AppState {
    pub workspaces: Vec<Workspace>, pub active_workspace: usize, pub tabs: Vec<TerminalTab>,
    pub active_tab: usize, pub panes: Vec<Vec<Pane>>, pub active_pane: PaneId,
    pub settings_open: bool, pub settings_section: SettingsSection, pub theme: String,
    pub font_size_pt: u32, pub toggles: BTreeMap<String, bool>, pub palette_open: bool,
    pub sidebar_collapsed: bool, pub cpu_pct: f32, pub mem_gb: f32, pub net_kbps: f32,
    pub clock_hhmm: String, pub next_id: u32, pub pty_manager: crate::pty::PtyManager,
    pub terminals: std::collections::HashMap<u32, Terminal>,
    pub last_grid_width: f32, pub last_grid_height: f32,
}

impl AppState {
    pub fn ui_snapshot(&self) -> UiSnapshot {
        UiSnapshot { workspaces: self.workspaces.clone(), active_workspace: self.active_workspace,
            tabs: self.tabs.clone(), active_tab: self.active_tab, panes: self.panes.clone(),
            active_pane: self.active_pane, settings_open: self.settings_open,
            settings_section: self.settings_section, theme: self.theme.clone(),
            font_size_pt: self.font_size_pt, toggles: self.toggles.clone(),
            palette_open: self.palette_open, sidebar_collapsed: self.sidebar_collapsed,
            cpu_pct: self.cpu_pct, mem_gb: self.mem_gb, net_kbps: self.net_kbps,
            clock_hhmm: self.clock_hhmm.clone() }
    }
    pub fn terminal_grid(&self, pane_id: PaneId) -> Option<&unshit::core::cell_grid::CellGrid> {
        self.terminals.get(&pane_id.0).map(|t| t.grid())
    }
}

#[derive(Clone, Debug)]
pub struct UiSnapshot {
    pub workspaces: Vec<Workspace>, pub active_workspace: usize, pub tabs: Vec<TerminalTab>,
    pub active_tab: usize, pub panes: Vec<Vec<Pane>>, pub active_pane: PaneId,
    pub settings_open: bool, pub settings_section: SettingsSection, pub theme: String,
    pub font_size_pt: u32, pub toggles: BTreeMap<String, bool>, pub palette_open: bool,
    pub sidebar_collapsed: bool, pub cpu_pct: f32, pub mem_gb: f32, pub net_kbps: f32,
    pub clock_hhmm: String,
}

pub fn seed_state() -> AppState {
    let workspaces = vec![
        Workspace { num: 1, name: "main".into(), branch: "main".into(), branch_muted: false, collapsed: false, subtabs: vec![
            Subtab { label: "terminals".into(), count: Some(4), pulse: false, active: true, icon: Some(SubtabIcon::Terminal), tree_glyph: "\u{251C}" },
            Subtab { label: "agents".into(), count: Some(2), pulse: true, active: false, icon: Some(SubtabIcon::User), tree_glyph: "\u{251C}" },
            Subtab { label: "worktrees".into(), count: Some(3), pulse: false, active: false, icon: Some(SubtabIcon::GitBranch), tree_glyph: "\u{251C}" },
            Subtab { label: "sessions".into(), count: Some(1), pulse: false, active: false, icon: Some(SubtabIcon::Folder), tree_glyph: "\u{251C}" },
            Subtab { label: "environment".into(), count: None, pulse: false, active: false, icon: Some(SubtabIcon::EnvList), tree_glyph: "\u{2514}" },
        ]},
        Workspace { num: 2, name: "api".into(), branch: "fix/pdf-export".into(), branch_muted: false, collapsed: false, subtabs: vec![
            Subtab { label: "terminals".into(), count: Some(2), pulse: false, active: false, icon: Some(SubtabIcon::Terminal), tree_glyph: "\u{251C}" },
            Subtab { label: "agents".into(), count: Some(1), pulse: false, active: false, icon: Some(SubtabIcon::User), tree_glyph: "\u{251C}" },
            Subtab { label: "worktrees".into(), count: Some(2), pulse: false, active: false, icon: Some(SubtabIcon::GitBranch), tree_glyph: "\u{2514}" },
        ]},
        Workspace { num: 3, name: "infra".into(), branch: "staging".into(), branch_muted: false, collapsed: true, subtabs: vec![
            Subtab { label: "terminals".into(), count: Some(1), pulse: false, active: false, icon: Some(SubtabIcon::Terminal), tree_glyph: "\u{251C}" },
            Subtab { label: "sessions".into(), count: None, pulse: false, active: false, icon: Some(SubtabIcon::Folder), tree_glyph: "\u{2514}" },
        ]},
        Workspace { num: 4, name: "scratch".into(), branch: "no branch".into(), branch_muted: true, collapsed: true, subtabs: vec![
            Subtab { label: "terminals".into(), count: Some(0), pulse: false, active: false, icon: None, tree_glyph: "\u{2514}" },
        ]},
    ];
    let tabs = vec![TerminalTab { id: "t1".into(), name: "shell".into(), subtitle: "bash".into(), status: TabStatus::Running }];
    let panes = vec![vec![Pane { id: PaneId(1), title: "shell".into(), subtitle: "bash".into(), pid: 0, cpu: 0.0 }]];
    let mut toggles = BTreeMap::new();
    toggles.insert("restore-on-startup".into(), true);
    toggles.insert("glow-effect".into(), true);
    toggles.insert("background-texture".into(), true);
    toggles.insert("shell-integration".into(), true);
    AppState { workspaces, active_workspace: 0, tabs, active_tab: 0, panes, active_pane: PaneId(1),
        settings_open: false, settings_section: SettingsSection::General, theme: "amber".into(),
        font_size_pt: 13, toggles, palette_open: false, sidebar_collapsed: false,
        cpu_pct: 0.0, mem_gb: 0.0, net_kbps: 0.0, clock_hhmm: "00:00".into(), next_id: 2,
        pty_manager: crate::pty::PtyManager::new(), terminals: std::collections::HashMap::new(),
        last_grid_width: 0.0, last_grid_height: 0.0 }
}

pub fn mutate_with<F, R>(shared: &SharedState, f: F) -> R where F: FnOnce(&mut AppState) -> R {
    let mut guard = shared.lock().expect("state mutex poisoned"); f(&mut guard)
}
pub fn mutate_add_tab(state: &mut AppState) {
    let id_num = state.next_id; state.next_id += 1;
    state.tabs.push(TerminalTab { id: format!("t{}", id_num), name: "shell".into(), subtitle: "bash".into(), status: TabStatus::Running });
    state.active_tab = state.tabs.len() - 1;
}
pub fn mutate_close_tab(state: &mut AppState, index: usize) {
    if index >= state.tabs.len() { return; } state.tabs.remove(index);
    if state.tabs.is_empty() { mutate_add_tab(state); state.active_tab = 0; return; }
    if state.active_tab == index { state.active_tab = index.min(state.tabs.len() - 1); }
    else if state.active_tab > index { state.active_tab -= 1; }
}
pub fn find_pane_coord(state: &AppState, target: PaneId) -> Option<(usize, usize)> {
    for (r, row) in state.panes.iter().enumerate() { for (c, p) in row.iter().enumerate() { if p.id == target { return Some((r, c)); } } } None
}
fn pty_dims(state: &AppState) -> (u16, u16) {
    let cw = unshit::core::cell_grid::CellGrid::global_cell_w();
    let ch = unshit::core::cell_grid::CellGrid::global_cell_h();
    compute_pty_dimensions(state.last_grid_width, state.last_grid_height, cw, ch)
}
pub fn mutate_split_right(state: &mut AppState, target: PaneId) {
    let Some((ri, ci)) = find_pane_coord(state, target) else { return; };
    if state.panes[ri].len() >= MAX_COLS { return; }
    let id = state.next_id; state.next_id += 1; let pid = PaneId(id);
    let (cols, rows) = pty_dims(state);
    let mut t = Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn(id, cols, rows) {
        Ok(r) => { state.terminals.insert(id, t); crate::bridge::register_reader(id, r); }
        Err(e) => { log::error!("spawn PTY {}: {}", id, e); t.process_bytes(format!("error: {}\r\n", e).as_bytes()); state.terminals.insert(id, t); }
    }
    state.panes[ri].insert(ci + 1, Pane { id: pid, title: "shell".into(), subtitle: "bash".into(), pid: 0, cpu: 0.0 });
    state.active_pane = pid;
}
pub fn mutate_split_down(state: &mut AppState, target: PaneId) {
    let Some((ri, _)) = find_pane_coord(state, target) else { return; };
    if state.panes.len() >= MAX_ROWS { return; }
    let id = state.next_id; state.next_id += 1; let pid = PaneId(id);
    let (cols, rows) = pty_dims(state);
    let mut t = Terminal::new(rows as usize, cols as usize);
    match state.pty_manager.spawn(id, cols, rows) {
        Ok(r) => { state.terminals.insert(id, t); crate::bridge::register_reader(id, r); }
        Err(e) => { log::error!("spawn PTY {}: {}", id, e); t.process_bytes(format!("error: {}\r\n", e).as_bytes()); state.terminals.insert(id, t); }
    }
    state.panes.insert(ri + 1, vec![Pane { id: pid, title: "shell".into(), subtitle: "bash".into(), pid: 0, cpu: 0.0 }]);
    state.active_pane = pid;
}
pub fn mutate_close_pane(state: &mut AppState, target: PaneId) {
    let Some((ri, ci)) = find_pane_coord(state, target) else { return; };
    state.pty_manager.destroy(target.0); state.terminals.remove(&target.0);
    state.panes[ri].remove(ci);
    if state.panes[ri].is_empty() { state.panes.remove(ri); }
    if state.panes.is_empty() {
        let id = state.next_id; state.next_id += 1; let pid = PaneId(id);
        let (cols, rows) = pty_dims(state);
        let mut t = Terminal::new(rows as usize, cols as usize);
        match state.pty_manager.spawn(id, cols, rows) {
            Ok(r) => { state.terminals.insert(id, t); crate::bridge::register_reader(id, r); }
            Err(e) => { log::error!("spawn PTY {}: {}", id, e); t.process_bytes(format!("error: {}\r\n", e).as_bytes()); state.terminals.insert(id, t); }
        }
        state.panes.push(vec![Pane { id: pid, title: "shell".into(), subtitle: "bash".into(), pid: 0, cpu: 0.0 }]);
        state.active_pane = pid; return;
    }
    if state.active_pane == target {
        let nr = ri.min(state.panes.len() - 1); let nc = ci.min(state.panes[nr].len() - 1);
        state.active_pane = state.panes[nr][nc].id;
    }
}
pub fn mutate_font_size_delta(state: &mut AppState, delta: i32) {
    state.font_size_pt = ((state.font_size_pt as i32 + delta).clamp(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32)) as u32;
}
pub fn dispatch(state: &mut AppState, command: &str) -> bool {
    match command {
        "modal.close" => { if state.settings_open { state.settings_open = false; true } else { false } }
        "modal.open" => { if !state.settings_open { state.settings_open = true; true } else { false } }
        "tab.new" => { mutate_add_tab(state); true }
        "tab.close.active" => { let i = state.active_tab; mutate_close_tab(state, i); true }
        "tab.next" => { if state.tabs.is_empty() { return false; } state.active_tab = (state.active_tab + 1) % state.tabs.len(); true }
        "tab.prev" => { if state.tabs.is_empty() { return false; } state.active_tab = if state.active_tab == 0 { state.tabs.len() - 1 } else { state.active_tab - 1 }; true }
        "pane.split_right" => { mutate_split_right(state, state.active_pane); true }
        "pane.split_down" => { mutate_split_down(state, state.active_pane); true }
        "pane.close" => { mutate_close_pane(state, state.active_pane); true }
        "sidebar.toggle" => { state.sidebar_collapsed = !state.sidebar_collapsed; true }
        "font.inc" => { let o = state.font_size_pt; mutate_font_size_delta(state, 1); o != state.font_size_pt }
        "font.dec" => { let o = state.font_size_pt; mutate_font_size_delta(state, -1); o != state.font_size_pt }
        "palette.toggle" => { state.palette_open = !state.palette_open; true }
        other if other.starts_with("tab.switch:") => {
            if let Ok(i) = other["tab.switch:".len()..].parse::<usize>() { if i < state.tabs.len() && state.active_tab != i { state.active_tab = i; return true; } } false
        }
        _ => false,
    }
}
pub fn find_active_pane(state: &UiSnapshot) -> &Pane {
    for row in &state.panes { for p in row { if p.id == state.active_pane { return p; } } } &state.panes[0][0]
}
pub fn is_on(state: &UiSnapshot, key: &str) -> bool { state.toggles.get(key).copied().unwrap_or(false) }

/// Compute PTY dimensions from cell metrics. Falls back to (80, 24).
pub fn compute_pty_dimensions(gw: f32, gh: f32, cw: f32, ch: f32) -> (u16, u16) {
    if cw > 0.0 && ch > 0.0 && gw > 0.0 { ((gw / cw).max(1.0) as u16, (gh / ch).max(1.0) as u16) } else { (80, 24) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn seed_state_has_empty_terminals() { assert!(seed_state().terminals.is_empty()); }
    #[test] fn seed_state_has_default_pane() { let s = seed_state(); assert_eq!(s.panes[0][0].id, PaneId(1)); }
    #[test] fn dims_valid() { assert_eq!(compute_pty_dimensions(800.0, 600.0, 8.0, 16.0), (100, 37)); }
    #[test] fn dims_no_metrics() { assert_eq!(compute_pty_dimensions(800.0, 600.0, 0.0, 0.0), (80, 24)); }
    #[test] fn dims_no_grid() { assert_eq!(compute_pty_dimensions(0.0, 0.0, 8.0, 16.0), (80, 24)); }
    #[test] fn dims_partial() { assert_eq!(compute_pty_dimensions(800.0, 600.0, 8.0, 0.0), (80, 24)); }
    #[test] fn dims_min() { assert_eq!(compute_pty_dimensions(1.0, 1.0, 8.0, 16.0), (1, 1)); }
}
