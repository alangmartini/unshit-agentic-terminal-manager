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
    pub scale_factor: f32,
    /// Ratio of monospace cell_width to font_size, measured from the actual font.
    pub cell_width_ratio: f32,
    /// Last known physical pixel dimensions of the terminal grid element.
    pub last_grid_width: f32,
    pub last_grid_height: f32,
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
        scale_factor: 1.0,
        cell_width_ratio: 0.6,
        last_grid_width: 0.0,
        last_grid_height: 0.0,
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

    // Use real cell metrics when available; fall back to 80x24.
    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    let (cols, rows) = compute_pty_dimensions(
        state.last_grid_width,
        state.last_grid_height,
        cell_w,
        cell_h,
    );

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

    // Use real cell metrics when available; fall back to 80x24.
    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    let (cols, rows) = compute_pty_dimensions(
        state.last_grid_width,
        state.last_grid_height,
        cell_w,
        cell_h,
    );

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

        // Use real cell metrics when available; fall back to 80x24.
        let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
        let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
        let (cols, rows) = compute_pty_dimensions(
            state.last_grid_width,
            state.last_grid_height,
            cell_w,
            cell_h,
        );

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

/// CSS line-height for `.terminal-content`. Must match `line-height: 1.2` in
/// assets/styles.css.
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
}
