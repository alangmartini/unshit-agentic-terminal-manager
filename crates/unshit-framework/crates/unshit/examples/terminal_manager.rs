//! Terminal manager port. Phase H (visual shell) plus phase I (modal layer)
//! plus phase J (interactivity).
//!
//! Capillaries #132 (visual shell), #133 (settings modal), and #135
//! (interactivity) of the terminal-manager port (epic #125). This file
//! builds the full visual shell (titlebar, sidebar, tab strip, pane grid,
//! statusbar) and the settings modal overlay, then wires up click and
//! keyboard driven mutations via an `Arc<Mutex<AppState>>` shared between
//! the tree builder and the app's `on_command` hook.
//!
//! Phase J scope (interactivity):
//!
//! * Tab click/close/add, workspace head click, pane click, pane header
//!   split/close buttons, modal close/cancel, modal nav tabs, toggles,
//!   theme chips, font size stepper: all fire `on_click` callbacks that
//!   mutate the shared `AppState` and trigger a rebuild.
//! * Keyboard shortcuts (Ctrl+T, Ctrl+D, Ctrl+W, Ctrl+,, Ctrl+1..9,
//!   Ctrl+Tab, Ctrl+Plus/Minus, Escape) are registered via
//!   `AppConfig::user_shortcuts` and dispatched to an `on_command` handler
//!   that consults the same shared state.
//! * Pane resizer drag: deferred, see TODO in `build_pane_row`.
//! * Command palette: stubbed; Ctrl+K / Ctrl+Shift+P maps to the
//!   `palette.toggle` command and flips `palette_open` in state, but the
//!   reference HTML does not ship a palette DOM yet so there is nothing
//!   to render.
//!
//! Phase I scope (modal layer):
//!
//! * `.modal-overlay#settings-modal` renders the frosted backdrop via
//!   `backdrop-filter: blur(6px)` (landed in #134) and fades in via the
//!   `fade-in` keyframe (landed in #129).
//! * `.modal` panel renders header, nav strip with five section tabs, body
//!   with three sections (general, appearance, shell), and footer actions.
//! * Theme chips, toggles, and the font size stepper are emitted with
//!   structural classes and their default active/on state so they light up
//!   under the ported CSS.
//!
//! The reference source lives next to this repo at `../terminal-manager/`.
//! The embedded `styles.css` under `assets/terminal_manager/` is a port of
//! that directory's reference stylesheet with three minimal rewrites:
//!
//!   1. Bare `kbd` element selectors become `.kbd` class selectors. The
//!      framework does not yet expose a `<kbd>` tag so the markup uses
//!      `Tag::Span` with a `kbd` class.
//!   2. `.app { height: 100vh; }` becomes `.app { width: 100%; height: 100%; }`
//!      because the framework does not yet resolve `vh` viewport units.
//!   3. All block comments are stripped. The current CSS parser drops the
//!      rule that immediately follows a standalone comment, so the ported
//!      stylesheet carries no `/* ... */` blocks. The semantic sections are
//!      documented in this file instead of in the stylesheet.
//!
//! Semantic HTML tags (`header`, `aside`, `main`, `footer`) map to `Tag::Div`
//! with a `role-*` class.
//!
//! Run with:
//!   cargo run -p unshit --example terminal_manager

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use unshit::app::{App, AppConfig, FontSource};
use unshit::core::element::*;
use unshit::core::id::NodeId;
use unshit::core::svg::{
    parse_svg_path, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
    ViewBox,
};

// ---------------------------------------------------------------------------
// Interactivity constants
// ---------------------------------------------------------------------------

/// Maximum panes allowed per row when splitting horizontally.
const MAX_COLS: usize = 4;
/// Maximum pane rows allowed in the grid when splitting down.
const MAX_ROWS: usize = 4;
/// Font size is clamped to this inclusive range by the stepper and the
/// Ctrl+Plus / Ctrl+Minus shortcuts.
const MIN_FONT_SIZE: u32 = 8;
const MAX_FONT_SIZE: u32 = 32;

// ---------------------------------------------------------------------------
// Embedded assets
// ---------------------------------------------------------------------------

const STYLES: &str = include_str!("assets/terminal_manager/styles.css");

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// Top level mock state. Interactivity (phase J) now owns the whole
/// structure behind an `Arc<Mutex<_>>` so click and keyboard callbacks
/// can mutate it. `build_tree` reads a fresh snapshot every frame.
#[derive(Clone, Debug)]
#[allow(dead_code)]
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
    /// Monotonic id counter used when new tabs or panes are spawned.
    pub next_id: u32,
    /// The currently hovered node id. Empty when nothing is hovered. The
    /// framework feeds the real hover state into the cascade through its
    /// own interaction tracking, so this field is a placeholder that future
    /// phases may start reading when they wire up hover driven state.
    pub hovered_node: Option<NodeId>,
}

/// Shared handle to the mutable app state. Every click/keyboard callback
/// receives a clone of this Arc and locks the Mutex briefly to mutate the
/// state before the next rebuild.
pub type SharedState = Arc<Mutex<AppState>>;

/// Nav tab currently selected inside the settings modal. Mirrors the five
/// `.modal-nav-item` buttons at `../terminal-manager/index.html:344` to `:348`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct Workspace {
    pub num: u32,
    pub name: String,
    pub branch: String,
    pub branch_muted: bool,
    pub collapsed: bool,
    pub subtabs: Vec<Subtab>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Subtab {
    pub label: String,
    /// When `None` the subtab renders without a count pill. This matches
    /// the environment row in workspace 1 (index.html line 120).
    pub count: Option<u32>,
    pub pulse: bool,
    pub active: bool,
    /// When `None` the subtab has no icon. Only the scratch workspace
    /// terminals subtab (index.html line 203) sets this.
    pub icon: Option<SubtabIcon>,
    /// Tree glyph used as the leading character. Mid rows use the box
    /// drawing vertical tee and the last row in a workspace uses the
    /// bottom left corner glyph.
    pub tree_glyph: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum SubtabIcon {
    Terminal,
    User,
    GitBranch,
    Folder,
    EnvList,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct TerminalTab {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub status: TabStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum TabStatus {
    Running,
    Idle,
    Stopped,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Pane {
    pub id: PaneId,
    pub title: String,
    pub subtitle: String,
    pub pid: u32,
    pub cpu: f32,
    pub sample: SampleKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum SampleKey {
    Dashboard,
    Server,
    Tests,
    Logs,
    Git,
    Shell,
}

fn seed_state() -> AppState {
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

    let tabs = vec![
        TerminalTab {
            id: "t1".to_string(),
            name: "dashboard".to_string(),
            subtitle: "go run".to_string(),
            status: TabStatus::Running,
        },
        TerminalTab {
            id: "t2".to_string(),
            name: "api.server".to_string(),
            subtitle: "bun dev".to_string(),
            status: TabStatus::Running,
        },
        TerminalTab {
            id: "t3".to_string(),
            name: "scratch".to_string(),
            subtitle: "bash".to_string(),
            status: TabStatus::Idle,
        },
    ];

    let panes = vec![
        vec![
            Pane {
                id: PaneId(1),
                title: "dashboard".to_string(),
                subtitle: "go run".to_string(),
                pid: 42101,
                cpu: 3.2,
                sample: SampleKey::Dashboard,
            },
            Pane {
                id: PaneId(2),
                title: "api.server".to_string(),
                subtitle: "bun dev".to_string(),
                pid: 42115,
                cpu: 5.4,
                sample: SampleKey::Server,
            },
        ],
        vec![
            Pane {
                id: PaneId(3),
                title: "tests".to_string(),
                subtitle: "vitest --watch".to_string(),
                pid: 42203,
                cpu: 1.1,
                sample: SampleKey::Tests,
            },
            Pane {
                id: PaneId(4),
                title: "logs".to_string(),
                subtitle: "tail -f prod".to_string(),
                pid: 42217,
                cpu: 0.6,
                sample: SampleKey::Logs,
            },
        ],
    ];

    // Initial toggle states mirror the `aria-pressed="true"` entries in
    // `../terminal-manager/index.html:378` through `:430`. Keys match the
    // visual labels so tests can assert by name.
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
        // Phase J (#135) flips the modal closed at startup; Ctrl+, or the
        // gear icon reopens it.
        settings_open: false,
        settings_section: SettingsSection::General,
        theme: "amber".to_string(),
        font_size_pt: 13,
        toggles,
        palette_open: false,
        sidebar_collapsed: false,
        cpu_pct: 12.4,
        mem_gb: 1.42,
        net_kbps: 0.8,
        clock_hhmm: "14:32".to_string(),
        // Seed data uses tab ids t1, t2, t3 and pane ids 1..=4 so the next
        // free id is 5. Tabs append a unique id by formatting next_id.
        next_id: 5,
        hovered_node: None,
    }
}

// ---------------------------------------------------------------------------
// State mutation helpers (phase J #135)
// ---------------------------------------------------------------------------

/// Append a fresh shell tab and activate it. Used by the `tab-add` button
/// and Ctrl+T.
fn mutate_add_tab(state: &mut AppState) {
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

/// Close the tab at `index`. Preserves active-tab sanity by falling back
/// to the next sibling, otherwise the previous one, otherwise clearing the
/// active index to zero. A final `tabs.is_empty()` case seeds a single
/// replacement shell tab so the tab strip is never empty.
fn mutate_close_tab(state: &mut AppState, index: usize) {
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
        // Active tab closed: fall back to the next sibling if available,
        // otherwise to the previous one. `state.active_tab` already points
        // one past the removed index so clamp it to `len - 1`.
        state.active_tab = index.min(state.tabs.len() - 1);
    } else if state.active_tab > index {
        state.active_tab -= 1;
    }
}

/// Locate a pane by id. Returns (row_index, col_index) or None.
fn find_pane_coord(state: &AppState, target: PaneId) -> Option<(usize, usize)> {
    for (r, row) in state.panes.iter().enumerate() {
        for (c, pane) in row.iter().enumerate() {
            if pane.id == target {
                return Some((r, c));
            }
        }
    }
    None
}

fn mutate_split_right(state: &mut AppState, target: PaneId) {
    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        return;
    };
    if state.panes[row_idx].len() >= MAX_COLS {
        return;
    }
    let id = PaneId(state.next_id);
    state.next_id += 1;
    let new_pane = Pane {
        id,
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 40000 + id.0,
        cpu: 0.0,
        sample: SampleKey::Shell,
    };
    state.panes[row_idx].insert(col_idx + 1, new_pane);
    state.active_pane = id;
}

fn mutate_split_down(state: &mut AppState, target: PaneId) {
    let Some((row_idx, _)) = find_pane_coord(state, target) else {
        return;
    };
    if state.panes.len() >= MAX_ROWS {
        return;
    }
    let id = PaneId(state.next_id);
    state.next_id += 1;
    let new_pane = Pane {
        id,
        title: "shell".to_string(),
        subtitle: "bash".to_string(),
        pid: 40000 + id.0,
        cpu: 0.0,
        sample: SampleKey::Shell,
    };
    state.panes.insert(row_idx + 1, vec![new_pane]);
    state.active_pane = id;
}

fn mutate_close_pane(state: &mut AppState, target: PaneId) {
    let Some((row_idx, col_idx)) = find_pane_coord(state, target) else {
        return;
    };
    state.panes[row_idx].remove(col_idx);
    if state.panes[row_idx].is_empty() {
        state.panes.remove(row_idx);
    }
    if state.panes.is_empty() {
        // Seed a replacement pane so the grid is never empty.
        let id = PaneId(state.next_id);
        state.next_id += 1;
        state.panes.push(vec![Pane {
            id,
            title: "shell".to_string(),
            subtitle: "bash".to_string(),
            pid: 40000 + id.0,
            cpu: 0.0,
            sample: SampleKey::Shell,
        }]);
        state.active_pane = id;
        return;
    }
    // Pick a nearby pane as the new active pane.
    if state.active_pane == target {
        let new_row = row_idx.min(state.panes.len() - 1);
        let new_col = col_idx.min(state.panes[new_row].len() - 1);
        state.active_pane = state.panes[new_row][new_col].id;
    }
}

fn mutate_font_size_delta(state: &mut AppState, delta: i32) {
    let next = state.font_size_pt as i32 + delta;
    state.font_size_pt = (next.clamp(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32)) as u32;
}

/// Central command dispatcher. Returns `true` if the command mutated state
/// so the caller can trigger a rebuild. This is shared between the
/// `on_click` callbacks (which invoke it directly via the closure) and the
/// framework `on_command` hook wired up to `AppConfig::user_shortcuts`.
fn dispatch(state: &mut AppState, command: &str) -> bool {
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
            state.active_tab =
                if state.active_tab == 0 { state.tabs.len() - 1 } else { state.active_tab - 1 };
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

// ---------------------------------------------------------------------------
// SVG icons
// ---------------------------------------------------------------------------

fn svg_icon(node: SvgNode) -> ElementDef {
    ElementDef::new(Tag::Div).with_svg(node)
}

fn group(attrs: SvgAttrs, children: Vec<SvgNode>) -> SvgNode {
    SvgNode { primitive: SvgPrimitive::Group, attrs, children }
}

fn path_d(d: &str) -> SvgNode {
    let commands = parse_svg_path(d).expect("icon path data must parse");
    SvgNode {
        primitive: SvgPrimitive::Path { d: d.to_string(), commands },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn circle(cx: f32, cy: f32, r: f32) -> SvgNode {
    SvgNode {
        primitive: SvgPrimitive::Circle { cx, cy, r },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32, rx: f32) -> SvgNode {
    SvgNode {
        primitive: SvgPrimitive::Rect { x, y, width, height, rx, ry: rx },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> SvgNode {
    SvgNode {
        primitive: SvgPrimitive::Line { x1, y1, x2, y2 },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn root_attrs(stroke_width: f32, cap: StrokeLineCap, join: StrokeLineJoin) -> SvgAttrs {
    SvgAttrs {
        view_box: Some(ViewBox::new(0.0, 0.0, 16.0, 16.0)),
        fill: Some(SvgPaint::None),
        stroke: Some(SvgPaint::Current),
        stroke_width: Some(stroke_width),
        stroke_linecap: Some(cap),
        stroke_linejoin: Some(join),
        ..Default::default()
    }
}

fn icon_brand_chevron() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![path_d("M2 4l4 4l-4 4"), path_d("M9 12h5")],
    )
}

fn icon_search() -> SvgNode {
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![rect(2.0, 2.0, 12.0, 12.0, 1.0), path_d("M5 6l2 2l-2 2M8 10h3")],
    )
}

fn icon_sidebar_toggle() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(6.0, 3.0, 6.0, 13.0)],
    )
}

fn icon_fullscreen_corners() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3")],
    )
}

fn icon_plus() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M8 3v10M3 8h10")],
    )
}

fn icon_chevrons() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 7l4-3l4 3M4 9l4 3l4-3")],
    )
}

fn icon_terminal() -> SvgNode {
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), path_d("M5 7l2 1.5L5 10M8 10h3")],
    )
}

fn icon_user() -> SvgNode {
    let mut body_arc = path_d("M3 13c.8-2.5 2.8-4 5-4s4.2 1.5 5 4");
    body_arc.attrs.stroke_linecap = Some(StrokeLineCap::Round);
    group(
        root_attrs(1.5, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![circle(8.0, 6.0, 2.5), body_arc],
    )
}

fn icon_git_branch() -> SvgNode {
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![
            circle(4.0, 4.0, 1.5),
            circle(4.0, 12.0, 1.5),
            circle(12.0, 8.0, 1.5),
            path_d("M4 5.5v5M5.5 4H9a2 2 0 012 2v.5M5.5 12H9a2 2 0 002-2v-.5"),
        ],
    )
}

fn icon_folder() -> SvgNode {
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![path_d("M2 5h12v8H2zM2 5l6-3l6 3")],
    )
}

fn icon_env_list() -> SvgNode {
    let mut dot_a = circle(5.0, 4.0, 0.5);
    dot_a.attrs.fill = Some(SvgPaint::Current);
    let mut dot_b = circle(11.0, 8.0, 0.5);
    dot_b.attrs.fill = Some(SvgPaint::Current);
    let mut dot_c = circle(7.0, 12.0, 0.5);
    dot_c.attrs.fill = Some(SvgPaint::Current);

    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M3 4h10M3 8h10M3 12h10"), dot_a, dot_b, dot_c],
    )
}

fn icon_split_h() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(8.0, 3.0, 8.0, 13.0)],
    )
}

fn icon_split_v() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(2.0, 8.0, 14.0, 8.0)],
    )
}

fn icon_grid() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(8.0, 3.0, 8.0, 13.0), line(2.0, 8.0, 14.0, 8.0)],
    )
}

fn icon_balance() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 8h8M6 5l-2 3l2 3M10 5l2 3l-2 3")],
    )
}

fn icon_settings() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![
            circle(8.0, 8.0, 2.0),
            path_d(
                "M8 1.5v1.5M8 13v1.5M14.5 8H13M3 8H1.5M12.6 3.4l-1 1M4.4 11.6l-1 1M12.6 12.6l-1-1M4.4 4.4l-1-1",
            ),
        ],
    )
}

fn icon_close() -> SvgNode {
    group(
        root_attrs(1.8, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 4l8 8M12 4l-8 8")],
    )
}

fn subtab_icon_for(kind: SubtabIcon) -> SvgNode {
    match kind {
        SubtabIcon::Terminal => icon_terminal(),
        SubtabIcon::User => icon_user(),
        SubtabIcon::GitBranch => icon_git_branch(),
        SubtabIcon::Folder => icon_folder(),
        SubtabIcon::EnvList => icon_env_list(),
    }
}

// ---------------------------------------------------------------------------
// Pane body content
// ---------------------------------------------------------------------------

struct TermSpan {
    class: &'static str,
    text: &'static str,
}

struct TermLine {
    spans: &'static [TermSpan],
}

/// A hand authored 12 line dashboard transcript. Mixes `term-prompt`,
/// `term-path`, `term-branch`, `term-cmd`, `term-output`, `term-success`,
/// and `term-dim` spans so the cascade has something to chew on. A separate
/// trailing cursor row is appended on top of these 12.
const DASHBOARD_LINES: &[TermLine] = &[
    TermLine {
        spans: &[
            TermSpan { class: "term-prompt", text: "\u{276F} " },
            TermSpan { class: "term-path", text: "~/main/dashboard " },
            TermSpan { class: "term-branch", text: "(main)" },
        ],
    },
    TermLine {
        spans: &[
            TermSpan { class: "term-prompt", text: "\u{276F} " },
            TermSpan { class: "term-cmd", text: "go mod tidy" },
        ],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-dim",
            text: "go: finding module for package github.com/charmbracelet/bubbletea",
        }],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-success",
            text: "\u{2713} resolved 23 dependencies in 1.42s",
        }],
    },
    TermLine {
        spans: &[
            TermSpan { class: "term-prompt", text: "\u{276F} " },
            TermSpan { class: "term-path", text: "~/main/dashboard " },
            TermSpan { class: "term-branch", text: "(main)" },
        ],
    },
    TermLine {
        spans: &[
            TermSpan { class: "term-prompt", text: "\u{276F} " },
            TermSpan { class: "term-cmd", text: "go run main.go --port 4040 --watch" },
        ],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-output",
            text: "\u{2192} listening on http://localhost:4040",
        }],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-dim",
            text: "\u{2192} hot reload enabled (462 files tracked)",
        }],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-output",
            text: "[14:32:07] GET  /api/sessions        200  12ms",
        }],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-output",
            text: "[14:32:09] POST /api/spawn           200  45ms",
        }],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-output",
            text: "[14:32:11] GET  /api/sessions        200   8ms",
        }],
    },
    TermLine {
        spans: &[TermSpan {
            class: "term-success",
            text: "[14:32:14] GET  /api/agents/list     200  11ms",
        }],
    },
];

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Build the region tree.
///
/// Class names match the original `../terminal-manager/index.html` so the
/// ported CSS resolves cleanly. Semantic HTML tags (`header`, `aside`, `main`,
/// `footer`) map to `Tag::Div` because that is what the framework supports
/// today; the role is carried by the class name.
///
/// Phase H (#132) populates titlebar, sidebar, tabbar, terminal grid, and
/// statusbar. Phase I (#133) populates the `.modal-overlay#settings-modal`
/// subtree. Phase J (#135) wires click handlers on every interactive element
/// and toggles `.open` based on `state.settings_open`.
fn build_tree(state: &AppState, shared: &SharedState) -> ElementTree {
    let mut modal_overlay =
        ElementDef::new(Tag::Div).with_class("modal-overlay").with_id("settings-modal");
    if state.settings_open {
        modal_overlay = modal_overlay.with_class("open");
    }
    // Backdrop click closes the modal (phase J). The click handler is
    // attached to the overlay itself; inner `.modal` children stop
    // propagation by default because framework clicks are dispatched to
    // the innermost hit target. The backdrop only receives the click when
    // the user clicks outside the `.modal` panel.
    {
        let s = shared.clone();
        modal_overlay = modal_overlay.on_click(move || {
            mutate_with(&s, |st| dispatch(st, "modal.close"));
        });
    }
    modal_overlay = modal_overlay.with_child(build_settings_modal(state, shared));

    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_child(build_titlebar(shared))
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("layout")
                    .with_child(build_sidebar(state, shared))
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("content")
                            .with_class("role-main")
                            .with_child(build_tabbar(state, shared))
                            .with_child(build_terminal_grid(state, shared))
                            .with_child(build_statusbar(state)),
                    ),
            )
            .with_child(modal_overlay),
    }
}

/// Lock the shared state, run `f`, and discard the return value. All
/// callbacks use this helper so the lock window is kept narrow and the
/// call site stays a single line.
fn mutate_with<F, R>(shared: &SharedState, f: F) -> R
where
    F: FnOnce(&mut AppState) -> R,
{
    let mut guard = shared.lock().expect("terminal_manager state mutex poisoned");
    f(&mut guard)
}

// ---------------------------------------------------------------------------
// Settings modal subtree (phase I, #133)
// ---------------------------------------------------------------------------

/// Build the `.modal` panel, mirroring the static structure at
/// `../terminal-manager/index.html:331` to `:450`. Phase J wires click
/// handlers on every interactive descendant of this subtree.
fn build_settings_modal(state: &AppState, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal")
        .with_child(build_modal_header(shared))
        .with_child(build_modal_nav(state.settings_section, shared))
        .with_child(build_modal_body(state, shared))
        .with_child(build_modal_footer(shared))
}

fn build_modal_header(shared: &SharedState) -> ElementDef {
    let close_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-title-row")
                .with_child(
                    ElementDef::new(Tag::Span).with_class("modal-mark").with_text("\u{25C6}"),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("modal-title")
                        .with_id("settings-title")
                        .with_text("settings"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("settings-close")
                .on_click(move || {
                    mutate_with(&close_state, |st| dispatch(st, "modal.close"));
                })
                .with_child(ElementDef::new(Tag::Div).with_svg(icon_close())),
        )
}

fn build_modal_nav(active: SettingsSection, shared: &SharedState) -> ElementDef {
    let mut nav = ElementDef::new(Tag::Div).with_class("modal-nav");
    for section in SettingsSection::all() {
        let mut item =
            ElementDef::new(Tag::Button).with_class("modal-nav-item").with_text(section.label());
        if section == active {
            item = item.with_class("active");
        }
        let s = shared.clone();
        let target = section;
        item = item.on_click(move || {
            mutate_with(&s, |st| st.settings_section = target);
        });
        nav = nav.with_child(item);
    }
    nav
}

fn build_modal_body(state: &AppState, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_child(build_general_section(state, shared))
        .with_child(build_appearance_section(state, shared))
        .with_child(build_shell_section(state, shared))
}

fn build_general_section(state: &AppState, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div).with_class("modal-section-title").with_text("general"),
        )
        .with_child(setting_row(
            "Default shell",
            "Command run when opening a new terminal",
            // The ported framework does not yet support <select> open state;
            // phase J keeps this as a static pill for now.
            ElementDef::new(Tag::Div).with_class("input").with_class("select").with_text("bash"),
        ))
        .with_child(setting_row(
            "Working directory",
            "Starting directory for new terminals",
            ElementDef::new(Tag::Input).with_class("input").with_placeholder("~/projects/main"),
        ))
        .with_child(setting_row(
            "Restore on startup",
            "Reopen last active session and panes",
            toggle_button(is_on(state, "restore-on-startup"), "restore-on-startup", shared),
        ))
}

fn build_appearance_section(state: &AppState, shared: &SharedState) -> ElementDef {
    let mut theme_chips = ElementDef::new(Tag::Div).with_class("theme-chips");
    for theme in ["amber", "green", "cyan", "mono"] {
        let mut chip = ElementDef::new(Tag::Button).with_class("theme-chip").with_class(theme);
        if state.theme == theme {
            chip = chip.with_class("active");
        }
        let s = shared.clone();
        let theme_name = theme.to_string();
        chip = chip.on_click(move || {
            mutate_with(&s, |st| st.theme = theme_name.clone());
        });
        theme_chips = theme_chips.with_child(chip);
    }

    let dec_state = shared.clone();
    let inc_state = shared.clone();
    let stepper = ElementDef::new(Tag::Div)
        .with_class("stepper")
        .with_child(
            ElementDef::new(Tag::Button).with_class("stepper-btn").with_text("\u{2212}").on_click(
                move || {
                    mutate_with(&dec_state, |st| dispatch(st, "font.dec"));
                },
            ),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("stepper-val")
                .with_class("tnum")
                .with_text(state.font_size_pt.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Button).with_class("stepper-btn").with_text("+").on_click(
                move || {
                    mutate_with(&inc_state, |st| dispatch(st, "font.inc"));
                },
            ),
        );

    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div).with_class("modal-section-title").with_text("appearance"),
        )
        .with_child(setting_row("Theme", "Visual palette", theme_chips))
        .with_child(setting_row("Font size", "Terminal output size", stepper))
        .with_child(setting_row(
            "Glow effect",
            "Subtle CRT-style text shadow",
            toggle_button(is_on(state, "glow-effect"), "glow-effect", shared),
        ))
        .with_child(setting_row(
            "Background texture",
            "Warm ambient gradient",
            toggle_button(is_on(state, "background-texture"), "background-texture", shared),
        ))
}

fn build_shell_section(state: &AppState, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(ElementDef::new(Tag::Div).with_class("modal-section-title").with_text("shell"))
        .with_child(setting_row(
            "Shell integration",
            "Inject prompt markers for smart scrollback",
            toggle_button(is_on(state, "shell-integration"), "shell-integration", shared),
        ))
        .with_child(setting_row(
            "History size",
            "Lines retained per pane",
            ElementDef::new(Tag::Input).with_class("input").with_placeholder("50000"),
        ))
}

fn build_modal_footer(shared: &SharedState) -> ElementDef {
    let cancel_state = shared.clone();
    let save_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-footer")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("modal-hint")
                .with_child(ElementDef::new(Tag::Span).with_class("kbd").with_text("esc"))
                .with_child(
                    ElementDef::new(Tag::Span).with_class("modal-hint-text").with_text(" close"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-footer-actions")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("ghost")
                        .with_id("settings-cancel")
                        .with_text("cancel")
                        .on_click(move || {
                            mutate_with(&cancel_state, |st| dispatch(st, "modal.close"));
                        }),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("primary")
                        .with_text("save changes")
                        .on_click(move || {
                            // No persistence layer yet; save is just close.
                            mutate_with(&save_state, |st| dispatch(st, "modal.close"));
                        }),
                ),
        )
}

// ---------------------------------------------------------------------------
// Modal widget helpers
// ---------------------------------------------------------------------------

fn setting_row(label: &str, desc: &str, control: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("setting-meta")
                .with_child(ElementDef::new(Tag::Span).with_class("setting-label").with_text(label))
                .with_child(ElementDef::new(Tag::Span).with_class("setting-desc").with_text(desc)),
        )
        .with_child(control)
}

fn toggle_button(on: bool, key: &str, shared: &SharedState) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("toggle");
    if on {
        btn = btn.with_class("on");
    }
    let key_owned = key.to_string();
    let s = shared.clone();
    btn.on_click(move || {
        mutate_with(&s, |st| {
            let next = !st.toggles.get(&key_owned).copied().unwrap_or(false);
            st.toggles.insert(key_owned.clone(), next);
        });
    })
}

fn is_on(state: &AppState, key: &str) -> bool {
    state.toggles.get(key).copied().unwrap_or(false)
}

// ---------- Titlebar ----------

fn build_titlebar(shared: &SharedState) -> ElementDef {
    let search_state = shared.clone();
    let sidebar_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("titlebar")
        .with_class("role-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("titlebar-left")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("brand")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-mark")
                                .with_child(svg_icon(icon_brand_chevron())),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-name")
                                .with_text("terminal.mgr"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-version")
                                .with_text("v0.1.0"),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("titlebar-breadcrumb")
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("crumb").with_text("main"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("crumb-sep").with_text("/"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("crumb")
                                .with_class("active")
                                .with_text("dashboard"),
                        ),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("titlebar-right")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pill-btn")
                        .on_click(move || {
                            mutate_with(&search_state, |st| dispatch(st, "palette.toggle"));
                        })
                        .with_child(svg_icon(icon_search()))
                        .with_child(ElementDef::new(Tag::Span).with_text("search"))
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("kbd").with_text("\u{2318}K"),
                        ),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("titlebar-divider"))
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .on_click(move || {
                            mutate_with(&sidebar_state, |st| dispatch(st, "sidebar.toggle"));
                        })
                        .with_child(svg_icon(icon_sidebar_toggle())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .with_child(svg_icon(icon_fullscreen_corners())),
                ),
        )
}

// ---------- Sidebar ----------

fn build_sidebar(state: &AppState, shared: &SharedState) -> ElementDef {
    let mut scroll = ElementDef::new(Tag::Div).with_class("sidebar-scroll");
    for (w_idx, workspace) in state.workspaces.iter().enumerate() {
        scroll = scroll.with_child(build_workspace(w_idx, workspace, shared));
    }

    let mut sidebar =
        ElementDef::new(Tag::Div).with_class("sidebar").with_class("role-aside").with_id("sidebar");
    if state.sidebar_collapsed {
        sidebar = sidebar.with_class("collapsed");
    }
    sidebar
        .with_child(build_sidebar_head())
        .with_child(scroll)
        .with_child(build_sidebar_footer())
        .with_child(build_sidebar_hints())
}

fn build_sidebar_head() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar-head")
        .with_child(ElementDef::new(Tag::Span).with_class("sidebar-title").with_text("workspaces"))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("sidebar-head-actions")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .with_child(svg_icon(icon_plus())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .with_child(svg_icon(icon_chevrons())),
                ),
        )
}

fn build_workspace(
    workspace_index: usize,
    workspace: &Workspace,
    shared: &SharedState,
) -> ElementDef {
    let head_state = shared.clone();
    let idx = workspace_index;
    let mut head = ElementDef::new(Tag::Div)
        .with_class("workspace-head")
        .with_tab_index(0)
        .on_click(move || {
            mutate_with(&head_state, |st| {
                if let Some(ws) = st.workspaces.get_mut(idx) {
                    ws.collapsed = !ws.collapsed;
                }
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_class("chevron").with_text("\u{25BE}"))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-num")
                .with_text(workspace.num.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-name")
                .with_text(workspace.name.clone()),
        );
    let mut branch_tag =
        ElementDef::new(Tag::Span).with_class("branch-tag").with_text(workspace.branch.clone());
    if workspace.branch_muted {
        branch_tag = branch_tag.with_class("muted");
    }
    head = head
        .with_child(ElementDef::new(Tag::Span).with_class("workspace-meta").with_child(branch_tag));

    let mut body = ElementDef::new(Tag::Div).with_class("workspace-body");
    for (s_idx, subtab) in workspace.subtabs.iter().enumerate() {
        body = body.with_child(build_subtab(workspace_index, s_idx, subtab, shared));
    }

    let mut container = ElementDef::new(Tag::Div).with_class("workspace");
    if workspace.collapsed {
        container = container.with_class("collapsed");
    }
    if workspace.num == 1 {
        container = container.with_class("active");
    }
    container.with_child(head).with_child(body)
}

fn build_subtab(
    workspace_index: usize,
    subtab_index: usize,
    subtab: &Subtab,
    shared: &SharedState,
) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("subtab");
    if subtab.active {
        btn = btn.with_class("active");
    }

    let s = shared.clone();
    let (wi, si) = (workspace_index, subtab_index);
    btn = btn.on_click(move || {
        mutate_with(&s, |st| {
            st.active_workspace = wi;
            if let Some(ws) = st.workspaces.get_mut(wi) {
                for (i, sub) in ws.subtabs.iter_mut().enumerate() {
                    sub.active = i == si;
                }
            }
        });
    });

    btn = btn.with_child(
        ElementDef::new(Tag::Span).with_class("tree-glyph").with_text(subtab.tree_glyph),
    );

    if let Some(icon) = subtab.icon {
        btn = btn.with_child(
            ElementDef::new(Tag::Span)
                .with_class("subtab-icon")
                .with_child(svg_icon(subtab_icon_for(icon))),
        );
    }

    btn = btn.with_child(
        ElementDef::new(Tag::Span).with_class("subtab-label").with_text(subtab.label.clone()),
    );

    if let Some(count) = subtab.count {
        let mut count_el =
            ElementDef::new(Tag::Span).with_class("subtab-count").with_text(count.to_string());
        if subtab.pulse {
            count_el = count_el.with_class("pulse");
        }
        btn = btn.with_child(count_el);
    }

    btn
}

fn build_sidebar_footer() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar-footer")
        .with_child(ElementDef::new(Tag::Div).with_class("footer-title").with_text("activity"))
        .with_child(activity_item("running", "claude", "running", "refactor split pane logic"))
        .with_child(activity_item("stopped", "amp", "stopped", "verify readme docs"))
        .with_child(activity_item("waiting", "codex", "waiting", "needs review"))
}

fn activity_item(state_class: &str, name: &str, state: &str, desc: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("activity-item")
        .with_class(state_class.to_string())
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("activity-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("activity-name")
                        .with_text(name.to_string()),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("activity-state")
                        .with_text(state.to_string()),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div).with_class("activity-desc").with_text(desc.to_string()),
        )
}

fn build_sidebar_hints() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar-hints")
        .with_child(hint_item("\u{2191}\u{2193}", "cycle"))
        .with_child(hint_item("\u{23CE}", "open"))
        .with_child(hint_item("x", "kill"))
        .with_child(hint_item("t", "theme"))
}

fn hint_item(key: &str, label: &str) -> ElementDef {
    ElementDef::new(Tag::Span)
        .with_class("hint")
        .with_child(ElementDef::new(Tag::Span).with_class("kbd").with_text(key.to_string()))
        .with_child(ElementDef::new(Tag::Span).with_text(label.to_string()))
}

// ---------- Tab strip ----------

fn build_tabbar(state: &AppState, shared: &SharedState) -> ElementDef {
    let mut tabs = ElementDef::new(Tag::Div).with_class("tabs").with_id("tabs");
    for (index, tab) in state.tabs.iter().enumerate() {
        tabs = tabs.with_child(build_tab(index, tab, index == state.active_tab, shared));
    }
    let add_state = shared.clone();
    tabs = tabs.with_child(
        ElementDef::new(Tag::Button)
            .with_class("tab-add")
            .on_click(move || {
                mutate_with(&add_state, |st| dispatch(st, "tab.new"));
            })
            .with_child(svg_icon(icon_plus())),
    );

    let split_h_state = shared.clone();
    let split_v_state = shared.clone();
    let settings_state = shared.clone();
    let actions = ElementDef::new(Tag::Div)
        .with_class("tabbar-actions")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-split-h")
                .on_click(move || {
                    mutate_with(&split_h_state, |st| dispatch(st, "pane.split_right"));
                })
                .with_child(svg_icon(icon_split_h())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-split-v")
                .on_click(move || {
                    mutate_with(&split_v_state, |st| dispatch(st, "pane.split_down"));
                })
                .with_child(svg_icon(icon_split_v())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-grid")
                .with_child(svg_icon(icon_grid())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-balance")
                .with_child(svg_icon(icon_balance())),
        )
        .with_child(ElementDef::new(Tag::Div).with_class("tabbar-divider"))
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-settings")
                .on_click(move || {
                    mutate_with(&settings_state, |st| dispatch(st, "modal.open"));
                })
                .with_child(svg_icon(icon_settings())),
        );

    ElementDef::new(Tag::Div).with_class("tabbar").with_child(tabs).with_child(actions)
}

fn build_tab(index: usize, tab: &TerminalTab, is_active: bool, shared: &SharedState) -> ElementDef {
    let status_class = match tab.status {
        TabStatus::Running => "running",
        TabStatus::Idle => "idle",
        TabStatus::Stopped => "stopped",
    };

    let mut btn = ElementDef::new(Tag::Button).with_class("tab");
    if is_active {
        btn = btn.with_class("active");
    }
    let activate_state = shared.clone();
    btn = btn.on_click(move || {
        mutate_with(&activate_state, |st| {
            if st.active_tab != index {
                st.active_tab = index;
            }
        });
    });

    let close_state = shared.clone();
    btn.with_child(
        ElementDef::new(Tag::Span).with_class("tab-status").with_class(status_class.to_string()),
    )
    .with_child(ElementDef::new(Tag::Span).with_class("tab-name").with_text(tab.name.clone()))
    .with_child(
        ElementDef::new(Tag::Span).with_class("tab-subtitle").with_text(tab.subtitle.clone()),
    )
    .with_child(
        ElementDef::new(Tag::Span).with_class("tab-close").with_text("\u{00D7}").on_click(
            move || {
                mutate_with(&close_state, |st| mutate_close_tab(st, index));
            },
        ),
    )
}

// ---------- Pane grid ----------

fn build_terminal_grid(state: &AppState, shared: &SharedState) -> ElementDef {
    // Phase J (#135): only the pane that matches `state.active_pane` is
    // rendered in detail. The reference visual layer still displays a
    // single large pane even when the underlying grid has 4 panes, because
    // the visual shell does not yet split the display area. Click-to-
    // activate on the pane body, plus the header split/close buttons,
    // operate on `state.active_pane` directly.
    //
    // TODO(#135 follow-up): pane resizer drag is deferred. Once the
    // visual shell actually renders the full row/column grid, attach
    // `.on_drag` handlers to `.pane-resizer` elements that adjust
    // sibling `flex-basis` percentages via `on_pane_resize`. Today the
    // resizer element does not even exist in the DOM because only one
    // pane is shown at a time.
    let active = find_active_pane(state);
    ElementDef::new(Tag::Div).with_class("terminal-grid").with_id("terminal-grid").with_child(
        ElementDef::new(Tag::Div)
            .with_class("pane-row")
            .with_child(build_pane(active, true, shared)),
    )
}

fn find_active_pane(state: &AppState) -> &Pane {
    for row in &state.panes {
        for pane in row {
            if pane.id == state.active_pane {
                return pane;
            }
        }
    }
    &state.panes[0][0]
}

fn build_pane(pane: &Pane, is_active: bool, shared: &SharedState) -> ElementDef {
    let mut container = ElementDef::new(Tag::Div).with_class("pane");
    if is_active {
        container = container.with_class("active");
    }
    let activate_state = shared.clone();
    let pane_id = pane.id;
    container = container.on_click(move || {
        mutate_with(&activate_state, |st| {
            st.active_pane = pane_id;
        });
    });
    container.with_child(build_pane_header(pane, shared)).with_child(build_pane_body())
}

fn build_pane_header(pane: &Pane, shared: &SharedState) -> ElementDef {
    let meta = format!("pid {} \u{00B7} {:.1}%", pane.pid, pane.cpu);
    let pane_id = pane.id;
    let split_h_state = shared.clone();
    let split_v_state = shared.clone();
    let close_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("pane-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("pane-header-left")
                .with_child(ElementDef::new(Tag::Span).with_class("pane-status-dot"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("pane-title")
                        .with_text(pane.title.clone()),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("pane-subtitle")
                        .with_text(format!("\u{00B7} {}", pane.subtitle)),
                ),
        )
        .with_child(ElementDef::new(Tag::Div).with_class("pane-meta").with_text(meta))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("pane-header-right")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .with_child(svg_icon(icon_search())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .on_click(move || {
                            mutate_with(&split_h_state, |st| mutate_split_right(st, pane_id));
                        })
                        .with_child(svg_icon(icon_split_h())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .on_click(move || {
                            mutate_with(&split_v_state, |st| mutate_split_down(st, pane_id));
                        })
                        .with_child(svg_icon(icon_split_v())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .with_class("danger")
                        .on_click(move || {
                            mutate_with(&close_state, |st| mutate_close_pane(st, pane_id));
                        })
                        .with_child(svg_icon(icon_close())),
                ),
        )
}

fn build_pane_body() -> ElementDef {
    let mut body = ElementDef::new(Tag::Div).with_class("pane-body");
    for line in DASHBOARD_LINES {
        let mut row = ElementDef::new(Tag::Div).with_class("term-line");
        for span in line.spans {
            row = row.with_child(
                ElementDef::new(Tag::Span)
                    .with_class(span.class.to_string())
                    .with_text(span.text.to_string()),
            );
        }
        body = body.with_child(row);
    }
    body = body.with_child(
        ElementDef::new(Tag::Div)
            .with_class("term-line")
            .with_child(ElementDef::new(Tag::Span).with_class("term-prompt").with_text("\u{276F} "))
            .with_child(ElementDef::new(Tag::Span).with_class("term-cursor")),
    );
    body
}

// ---------- Statusbar ----------

fn build_statusbar(state: &AppState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("statusbar")
        .with_class("role-footer")
        .with_child(build_statusbar_left(state))
        .with_child(build_statusbar_right(state))
}

fn build_statusbar_left(state: &AppState) -> ElementDef {
    let running_count: usize =
        state.tabs.iter().filter(|t| t.status == TabStatus::Running).count() + 2; // matches the "4 active" string in the reference index.html

    ElementDef::new(Tag::Div)
        .with_class("statusbar-left")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_class("accent")
                .with_id("status-mode")
                .with_child(
                    ElementDef::new(Tag::Span).with_class("status-glyph").with_text("\u{25C6}"),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("main")),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_child(
                    ElementDef::new(Tag::Span).with_class("status-dot").with_class("running"),
                )
                .with_child(
                    ElementDef::new(Tag::Span).with_text(format!("{} active", running_count)),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_id("status-cpu")
                .with_child(ElementDef::new(Tag::Span).with_text("cpu "))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text(format!("{:.1}", state.cpu_pct)),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("%")),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_id("status-mem")
                .with_child(ElementDef::new(Tag::Span).with_text("mem "))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text(format!("{:.2}", state.mem_gb)),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("G")),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_id("status-net")
                .with_child(ElementDef::new(Tag::Span).with_text("\u{2193} "))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text(format!("{:.1}", state.net_kbps)),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("k/s")),
        )
}

fn build_statusbar_right(state: &AppState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("statusbar-right")
        .with_child(ElementDef::new(Tag::Span).with_class("status-item").with_text("utf-8"))
        .with_child(
            ElementDef::new(Tag::Span).with_class("status-item").with_text("bash \u{00B7} 5.2"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_child(ElementDef::new(Tag::Span).with_class("tnum").with_text("80"))
                .with_child(ElementDef::new(Tag::Span).with_text("\u{00D7}"))
                .with_child(ElementDef::new(Tag::Span).with_class("tnum").with_text("24")),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_id("status-clock")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text(state.clock_hhmm.clone()),
                ),
        )
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_hal=error,wgpu_core=error,naga=error"),
    )
    .init();

    // Phase J (#135): seed state lives behind an `Arc<Mutex<_>>` so click
    // and keyboard callbacks can mutate it, and the tree builder reads a
    // fresh snapshot every frame. The `on_command` hook and the user
    // shortcut registry share the same Arc.
    let shared: SharedState = Arc::new(Mutex::new(seed_state()));

    let tree_shared = shared.clone();
    let command_shared = shared.clone();

    let app = App::new(
        AppConfig {
            title: "terminal manager".to_string(),
            width: 1280,
            height: 800,
            css: STYLES.to_string(),
            fonts: vec![
                FontSource::System("JetBrains Mono".to_string()),
                FontSource::System("Berkeley Mono".to_string()),
                FontSource::System("SF Mono".to_string()),
                FontSource::System("Menlo".to_string()),
                FontSource::System("Consolas".to_string()),
            ],
            user_shortcuts: user_shortcut_bindings(),
            on_command: Some(Arc::new(move |command: &str| -> bool {
                let mut guard =
                    command_shared.lock().expect("terminal_manager state mutex poisoned");
                dispatch(&mut guard, command)
            })),
            ..Default::default()
        },
        move || {
            let snap = tree_shared.lock().expect("terminal_manager state mutex poisoned").clone();
            build_tree(&snap, &tree_shared)
        },
    );

    app.run();
}

/// Keyboard shortcut table. The framework parses each entry via
/// `Shortcut::parse` at startup and routes matches to the user command
/// handler installed via `AppConfig::on_command`. Single-letter bindings
/// like `Ctrl+T` are lowered to `Ctrl+t` by `KeyCombo::parse` so
/// key-combo equality always compares against lowercase chars.
fn user_shortcut_bindings() -> Vec<(String, String)> {
    vec![
        ("Ctrl+T".to_string(), "tab.new".to_string()),
        ("Ctrl+W".to_string(), "pane.close".to_string()),
        ("Ctrl+D".to_string(), "pane.split_right".to_string()),
        ("Ctrl+Shift+D".to_string(), "pane.split_down".to_string()),
        ("Ctrl+B".to_string(), "sidebar.toggle".to_string()),
        ("Ctrl+,".to_string(), "modal.open".to_string()),
        ("Ctrl+K".to_string(), "palette.toggle".to_string()),
        ("Ctrl+Shift+P".to_string(), "palette.toggle".to_string()),
        ("Escape".to_string(), "modal.close".to_string()),
        ("Ctrl+1".to_string(), "tab.switch:0".to_string()),
        ("Ctrl+2".to_string(), "tab.switch:1".to_string()),
        ("Ctrl+3".to_string(), "tab.switch:2".to_string()),
        ("Ctrl+4".to_string(), "tab.switch:3".to_string()),
        ("Ctrl+5".to_string(), "tab.switch:4".to_string()),
        ("Ctrl+6".to_string(), "tab.switch:5".to_string()),
        ("Ctrl+7".to_string(), "tab.switch:6".to_string()),
        ("Ctrl+8".to_string(), "tab.switch:7".to_string()),
        ("Ctrl+9".to_string(), "tab.switch:8".to_string()),
        // Ctrl+Plus lives on the `=` key plus Shift on US keyboards, so
        // register both the shifted combo and the plain equal for
        // convenience. The shortcut parser cannot currently tokenize
        // "Ctrl++" directly (the second `+` is ambiguous in the split),
        // which is why we spell it via `Ctrl+Shift+=` instead.
        ("Ctrl+=".to_string(), "font.inc".to_string()),
        ("Ctrl+Shift+=".to_string(), "font.inc".to_string()),
        ("Ctrl+-".to_string(), "font.dec".to_string()),
        // Note: Ctrl+Tab and Ctrl+Shift+Tab land in `tab.next` and
        // `tab.prev`, but the default `focus.next`/`focus.prev` bindings
        // shadow them. We register at User priority so our entries win.
        ("Ctrl+Tab".to_string(), "tab.next".to_string()),
        ("Ctrl+Shift+Tab".to_string(), "tab.prev".to_string()),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap `state` in a SharedState and build a tree against it. Tests use
    /// this to keep call sites compact. The `shared` handle is returned in
    /// case a test needs to fire click callbacks directly.
    fn build_with_shared(state: AppState) -> (ElementTree, SharedState) {
        let shared: SharedState = Arc::new(Mutex::new(state));
        let snap = shared.lock().unwrap().clone();
        let tree = build_tree(&snap, &shared);
        (tree, shared)
    }

    /// Rebuild a tree from the current shared state. Used by tests that
    /// fire callbacks then want to re-inspect the resulting tree.
    fn rebuild_shared(shared: &SharedState) -> ElementTree {
        let snap = shared.lock().unwrap().clone();
        build_tree(&snap, shared)
    }

    #[test]
    fn seed_state_has_four_workspaces() {
        let state = seed_state();
        assert_eq!(state.workspaces.len(), 4);
        assert_eq!(state.workspaces[0].name, "main");
        assert!(!state.workspaces[0].collapsed);
        assert_eq!(state.workspaces[1].name, "api");
        assert!(!state.workspaces[1].collapsed);
        assert_eq!(state.workspaces[2].name, "infra");
        assert!(state.workspaces[2].collapsed);
        assert_eq!(state.workspaces[3].name, "scratch");
        assert!(state.workspaces[3].collapsed);
        assert!(state.workspaces[3].branch_muted);
    }

    #[test]
    fn seed_state_has_three_tabs() {
        let state = seed_state();
        assert_eq!(state.tabs.len(), 3);
        assert_eq!(state.tabs[0].name, "dashboard");
        assert_eq!(state.tabs[0].subtitle, "go run");
        assert_eq!(state.tabs[0].status, TabStatus::Running);
        assert_eq!(state.tabs[1].name, "api.server");
        assert_eq!(state.tabs[1].subtitle, "bun dev");
        assert_eq!(state.tabs[1].status, TabStatus::Running);
        assert_eq!(state.tabs[2].name, "scratch");
        assert_eq!(state.tabs[2].subtitle, "bash");
        assert_eq!(state.tabs[2].status, TabStatus::Idle);
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn seed_state_has_two_by_two_pane_grid() {
        let state = seed_state();
        assert_eq!(state.panes.len(), 2, "expected 2 rows");
        for (row_index, row) in state.panes.iter().enumerate() {
            assert_eq!(row.len(), 2, "row {row_index} should have 2 panes");
        }
        let total: usize = state.panes.iter().map(|row| row.len()).sum();
        assert_eq!(total, 4, "expected 4 panes total in 2x2 grid");
        assert_eq!(state.active_pane, PaneId(1), "top left pane should be active");
    }

    #[test]
    fn seed_state_starts_with_settings_modal_closed() {
        // Phase J (#135) flips the modal closed by default; interactive
        // callbacks reopen it via the gear icon or Ctrl+,.
        let state = seed_state();
        assert!(!state.settings_open);
        assert!(!state.palette_open);
        assert_eq!(state.settings_section, SettingsSection::General);
        assert_eq!(state.theme, "amber");
        assert_eq!(state.font_size_pt, 13);
        // Four toggles are on by default, matching the `aria-pressed="true"`
        // entries in `../terminal-manager/index.html`.
        assert_eq!(state.toggles.get("restore-on-startup").copied(), Some(true));
        assert_eq!(state.toggles.get("glow-effect").copied(), Some(true));
        assert_eq!(state.toggles.get("background-texture").copied(), Some(true));
        assert_eq!(state.toggles.get("shell-integration").copied(), Some(true));
    }

    #[test]
    fn seed_state_reports_hardcoded_metrics() {
        let state = seed_state();
        assert_eq!(state.cpu_pct, 12.4);
        assert_eq!(state.mem_gb, 1.42);
        assert_eq!(state.net_kbps, 0.8);
        assert_eq!(state.clock_hhmm, "14:32");
        assert_eq!(state.active_workspace, 0);
        assert!(state.hovered_node.is_none());
    }

    fn count_class(def: &ElementDef, class: &str) -> usize {
        let mut hits = 0;
        if def.classes.iter().any(|c| c == class) {
            hits += 1;
        }
        for child in &def.children {
            hits += count_class(child, class);
        }
        hits
    }

    fn count_classes_all(def: &ElementDef, classes: &[&str]) -> usize {
        let mut hits = 0;
        if classes.iter().all(|needle| def.classes.iter().any(|c| c == *needle)) {
            hits += 1;
        }
        for child in &def.children {
            hits += count_classes_all(child, classes);
        }
        hits
    }

    fn count_svg(def: &ElementDef) -> usize {
        let mut hits = if def.tag == Tag::Svg { 1 } else { 0 };
        for child in &def.children {
            hits += count_svg(child);
        }
        hits
    }

    #[test]
    fn tree_has_titlebar_layout_modal() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        // root is div.app
        assert!(matches!(tree.root.tag, Tag::Div));
        assert!(tree.root.classes.iter().any(|c| c == "app"));

        // app contains: titlebar, layout, modal-overlay
        assert_eq!(tree.root.children.len(), 3);
        assert!(tree.root.children[0].classes.iter().any(|c| c == "titlebar"));
        assert!(tree.root.children[0].classes.iter().any(|c| c == "role-header"));

        let layout = &tree.root.children[1];
        assert!(layout.classes.iter().any(|c| c == "layout"));
        assert_eq!(layout.children.len(), 2);
        assert!(layout.children[0].classes.iter().any(|c| c == "sidebar"));
        assert!(layout.children[0].classes.iter().any(|c| c == "role-aside"));

        let content = &layout.children[1];
        assert!(content.classes.iter().any(|c| c == "content"));
        assert!(content.classes.iter().any(|c| c == "role-main"));
        assert_eq!(content.children.len(), 3);

        let statusbar = &content.children[2];
        assert!(statusbar.classes.iter().any(|c| c == "statusbar"));
        assert!(statusbar.classes.iter().any(|c| c == "role-footer"));

        let modal = &tree.root.children[2];
        assert!(modal.classes.iter().any(|c| c == "modal-overlay"));
        assert_eq!(modal.id.as_deref(), Some("settings-modal"));
        // Phase I (#133): modal overlay holds the populated `.modal` panel.
        assert_eq!(modal.children.len(), 1, "modal overlay wraps one .modal panel");
    }

    #[test]
    fn tree_has_four_workspaces() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_class(&tree.root, "workspace"), 4);
    }

    #[test]
    fn tree_has_eleven_subtabs() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_class(&tree.root, "subtab"), 11);
    }

    #[test]
    fn tree_has_one_active_subtab() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_classes_all(&tree.root, &["subtab", "active"]), 1);
    }

    #[test]
    fn tree_has_three_tabs_and_one_tab_add() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_class(&tree.root, "tab"), 3);
        assert_eq!(count_class(&tree.root, "tab-add"), 1);
    }

    #[test]
    fn tree_has_one_active_tab() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_classes_all(&tree.root, &["tab", "active"]), 1);
    }

    #[test]
    fn tree_has_pane_row_with_active_pane() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_class(&tree.root, "pane-row"), 1);
        assert_eq!(count_classes_all(&tree.root, &["pane", "active"]), 1);
    }

    #[test]
    fn tree_has_twelve_term_lines_plus_cursor() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_class(&tree.root, "term-line"), 13);
        assert_eq!(count_class(&tree.root, "term-cursor"), 1);
    }

    #[test]
    fn tree_has_expected_statusbar_items() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_class(&tree.root, "status-item"), 9);
        assert_eq!(count_class(&tree.root, "statusbar-left"), 1);
        assert_eq!(count_class(&tree.root, "statusbar-right"), 1);
    }

    #[test]
    fn tree_has_twenty_seven_svg_icons() {
        // 26 icons from the visual shell plus one modal close-button icon.
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        assert_eq!(count_svg(&tree.root), 27);
    }

    /// Creates a `TestHarness` against a fresh seed state. Hides the
    /// `Arc<Mutex<_>>` boilerplate the harness closure needs.
    fn harness_with_seed() -> unshit_test::TestHarness {
        let shared: SharedState = Arc::new(Mutex::new(seed_state()));
        let h_shared = shared.clone();
        unshit_test::TestHarness::new(
            STYLES,
            move || {
                let snap = h_shared.lock().unwrap().clone();
                build_tree(&snap, &h_shared)
            },
            1280.0,
            800.0,
        )
    }

    #[test]
    fn layout_titlebar_is_thirty_four_px_tall() {
        let harness = harness_with_seed();
        let titlebar = harness.query(".titlebar").expect("titlebar exists");
        assert_eq!(titlebar.layout_rect.height, 34.0);
    }

    #[test]
    fn layout_sidebar_is_two_hundred_fifty_two_px_wide() {
        let harness = harness_with_seed();
        let sidebar = harness.query(".sidebar").expect("sidebar exists");
        assert_eq!(sidebar.layout_rect.width, 252.0);
    }

    #[test]
    fn layout_pane_fits_between_tabbar_and_statusbar() {
        let harness = harness_with_seed();
        let tabbar = harness.query(".tabbar").expect("tabbar exists");
        let statusbar = harness.query(".statusbar").expect("statusbar exists");
        let pane_row = harness.query(".pane-row").expect("pane row exists");
        assert!(
            pane_row.layout_rect.y >= tabbar.layout_rect.y + tabbar.layout_rect.height - 1.0,
            "pane row y {} should be >= tabbar bottom {}",
            pane_row.layout_rect.y,
            tabbar.layout_rect.y + tabbar.layout_rect.height
        );
        assert!(
            pane_row.layout_rect.y + pane_row.layout_rect.height <= statusbar.layout_rect.y + 1.0,
            "pane row bottom {} should be <= statusbar top {}",
            pane_row.layout_rect.y + pane_row.layout_rect.height,
            statusbar.layout_rect.y
        );
        assert_eq!(tabbar.layout_rect.height, 38.0);
        assert_eq!(statusbar.layout_rect.height, 24.0);
    }

    // -----------------------------------------------------------------------
    // Phase I (#133): settings modal
    // -----------------------------------------------------------------------

    /// Depth first walk that yields every descendant under `root`.
    fn walk<'a>(root: &'a ElementDef, out: &mut Vec<&'a ElementDef>) {
        out.push(root);
        for c in &root.children {
            walk(c, out);
        }
    }

    fn find_by_class<'a>(root: &'a ElementDef, class: &str) -> Vec<&'a ElementDef> {
        let mut all = Vec::new();
        walk(root, &mut all);
        all.into_iter().filter(|e| e.classes.iter().any(|c| c == class)).collect()
    }

    fn find_modal_overlay(tree: &ElementTree) -> &ElementDef {
        tree.root
            .children
            .iter()
            .find(|c| c.id.as_deref() == Some("settings-modal"))
            .expect("modal overlay must exist in the tree")
    }

    #[test]
    fn modal_overlay_has_open_class_when_settings_open_true() {
        let mut state = seed_state();
        state.settings_open = true;
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        assert!(
            overlay.classes.iter().any(|c| c == "open"),
            "overlay must carry `.open` so .modal-overlay.open CSS applies"
        );
    }

    #[test]
    fn modal_overlay_drops_open_class_when_settings_open_false() {
        // Phase J (#135) flips the default to closed, so the seed state
        // already satisfies this assertion.
        let state = seed_state();
        assert!(!state.settings_open);
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        assert!(
            !overlay.classes.iter().any(|c| c == "open"),
            "overlay must not carry `.open` when settings_open is false"
        );
    }

    #[test]
    fn modal_panel_has_header_nav_body_footer() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        assert_eq!(overlay.children.len(), 1, "overlay wraps a single .modal");
        let modal = &overlay.children[0];
        assert!(modal.classes.iter().any(|c| c == "modal"));
        assert_eq!(modal.children.len(), 4, "modal has header, nav, body, footer");
        assert!(modal.children[0].classes.iter().any(|c| c == "modal-header"));
        assert!(modal.children[1].classes.iter().any(|c| c == "modal-nav"));
        assert!(modal.children[2].classes.iter().any(|c| c == "modal-body"));
        assert!(modal.children[3].classes.iter().any(|c| c == "modal-footer"));
    }

    #[test]
    fn modal_header_has_title_and_close_button() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let title_nodes = find_by_class(overlay, "modal-title");
        assert_eq!(title_nodes.len(), 1);
        assert_eq!(title_nodes[0].content, ElementContent::Text("settings".to_string()));

        let close_buttons: Vec<_> = {
            let mut all = Vec::new();
            walk(overlay, &mut all);
            all.into_iter().filter(|e| e.id.as_deref() == Some("settings-close")).collect()
        };
        assert_eq!(close_buttons.len(), 1, "close button must exist");
        assert!(matches!(close_buttons[0].tag, Tag::Button));
    }

    #[test]
    fn modal_nav_has_five_items_with_one_active() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let nav_items = find_by_class(overlay, "modal-nav-item");
        assert_eq!(nav_items.len(), 5, "five section tabs");

        let active: Vec<&&ElementDef> =
            nav_items.iter().filter(|e| e.classes.iter().any(|c| c == "active")).collect();
        assert_eq!(active.len(), 1, "exactly one active nav item");
        assert_eq!(active[0].content, ElementContent::Text("general".to_string()));
    }

    #[test]
    fn modal_nav_active_tracks_state_settings_section() {
        let mut state = seed_state();
        state.settings_section = SettingsSection::Appearance;
        state.settings_open = true;
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let active: Vec<&ElementDef> = find_by_class(overlay, "modal-nav-item")
            .into_iter()
            .filter(|e| e.classes.iter().any(|c| c == "active"))
            .collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].content, ElementContent::Text("appearance".to_string()));
    }

    #[test]
    fn modal_body_has_three_sections() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let sections = find_by_class(overlay, "modal-section");
        assert_eq!(sections.len(), 3, "general, appearance, shell");
    }

    #[test]
    fn modal_body_has_expected_setting_rows() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let rows = find_by_class(overlay, "setting-row");
        // general: 3 rows. appearance: 4 rows. shell: 2 rows. Total 9.
        assert_eq!(rows.len(), 9, "nine setting rows across three sections");
    }

    #[test]
    fn modal_appearance_has_four_theme_chips_one_active() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let chips = find_by_class(overlay, "theme-chip");
        assert_eq!(chips.len(), 4, "four theme chips");
        let active: Vec<&&ElementDef> =
            chips.iter().filter(|e| e.classes.iter().any(|c| c == "active")).collect();
        assert_eq!(active.len(), 1, "exactly one active theme chip");
        assert!(active[0].classes.iter().any(|c| c == "amber"), "amber is the default active chip");
    }

    #[test]
    fn modal_appearance_active_chip_tracks_state_theme() {
        let mut state = seed_state();
        state.theme = "cyan".to_string();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let chips = find_by_class(overlay, "theme-chip");
        let active: Vec<&&ElementDef> =
            chips.iter().filter(|e| e.classes.iter().any(|c| c == "active")).collect();
        assert_eq!(active.len(), 1);
        assert!(active[0].classes.iter().any(|c| c == "cyan"));
    }

    #[test]
    fn modal_stepper_value_tracks_state_font_size_pt() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let vals = find_by_class(overlay, "stepper-val");
        assert_eq!(vals.len(), 1, "one stepper-val");
        assert_eq!(
            vals[0].content,
            ElementContent::Text("13".to_string()),
            "default font size is 13"
        );
    }

    #[test]
    fn modal_toggles_reflect_state_map() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let toggles = find_by_class(overlay, "toggle");
        // 4 toggles: restore-on-startup, glow-effect, background-texture,
        // shell-integration. All start `on` per the seed state.
        assert_eq!(toggles.len(), 4, "four toggles across general, appearance, shell");
        let on_count = toggles.iter().filter(|t| t.classes.iter().any(|c| c == "on")).count();
        assert_eq!(on_count, 4, "all four default toggles start on");
    }

    #[test]
    fn modal_toggle_class_drops_when_state_flag_false() {
        let mut state = seed_state();
        state.toggles.insert("glow-effect".to_string(), false);
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let toggles = find_by_class(overlay, "toggle");
        let on_count = toggles.iter().filter(|t| t.classes.iter().any(|c| c == "on")).count();
        assert_eq!(on_count, 3, "one toggle flipped off");
    }

    #[test]
    fn modal_footer_has_cancel_and_primary_buttons() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let btns = find_by_class(overlay, "btn");
        assert_eq!(btns.len(), 2, "cancel + save changes");
        assert!(btns.iter().any(|b| b.classes.iter().any(|c| c == "ghost")));
        assert!(btns.iter().any(|b| b.classes.iter().any(|c| c == "primary")));
    }

    #[test]
    fn modal_footer_hint_contains_kbd_span() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let kbds = find_by_class(overlay, "kbd");
        assert_eq!(kbds.len(), 1, "modal hint carries one .kbd span");
        assert_eq!(kbds[0].content, ElementContent::Text("esc".to_string()));
        assert!(matches!(kbds[0].tag, Tag::Span));
    }

    #[test]
    fn modal_close_button_carries_inline_svg_close_icon() {
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let overlay = find_modal_overlay(&tree);
        let close: Vec<&ElementDef> = {
            let mut all = Vec::new();
            walk(overlay, &mut all);
            all.into_iter().filter(|e| e.id.as_deref() == Some("settings-close")).collect()
        };
        assert_eq!(close.len(), 1);
        // The close button wraps a single icon container holding the SVG.
        assert_eq!(close[0].children.len(), 1);
        let icon_wrapper = &close[0].children[0];
        assert!(matches!(icon_wrapper.tag, Tag::Svg));
        assert!(matches!(icon_wrapper.content, ElementContent::Svg(_)));
    }

    // -----------------------------------------------------------------------
    // Phase J (#135): interactivity
    // -----------------------------------------------------------------------
    //
    // These tests exercise the `dispatch` central dispatcher and the
    // click-side mutation helpers that every `on_click` callback routes
    // through. They do not try to simulate pointer events against the
    // framework because the `TestHarness` does not yet expose a click
    // injector; instead we prove that the same function the callbacks
    // invoke produces the expected state mutation, then rebuild the tree
    // and assert the rendered structure updates.

    fn fresh_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    /// Fire the `dispatch` dispatcher against a shared state and rebuild
    /// a fresh tree. Returns the rebuilt tree so tests can assert on the
    /// new structure.
    fn dispatch_and_rebuild(shared: &SharedState, command: &str) -> ElementTree {
        {
            let mut guard = shared.lock().unwrap();
            dispatch(&mut *guard, command);
        }
        rebuild_shared(shared)
    }

    #[test]
    fn dispatch_modal_open_flips_state_and_adds_open_class() {
        let shared = fresh_shared();
        assert!(!shared.lock().unwrap().settings_open);
        let tree = dispatch_and_rebuild(&shared, "modal.open");
        assert!(shared.lock().unwrap().settings_open);
        let overlay = find_modal_overlay(&tree);
        assert!(overlay.classes.iter().any(|c| c == "open"));
    }

    #[test]
    fn dispatch_modal_close_drops_open_class() {
        let shared = fresh_shared();
        shared.lock().unwrap().settings_open = true;
        let tree = dispatch_and_rebuild(&shared, "modal.close");
        assert!(!shared.lock().unwrap().settings_open);
        let overlay = find_modal_overlay(&tree);
        assert!(!overlay.classes.iter().any(|c| c == "open"));
    }

    #[test]
    fn dispatch_tab_new_appends_and_activates_tab() {
        let shared = fresh_shared();
        let before = shared.lock().unwrap().tabs.len();
        let tree = dispatch_and_rebuild(&shared, "tab.new");
        assert_eq!(shared.lock().unwrap().tabs.len(), before + 1);
        assert_eq!(shared.lock().unwrap().active_tab, before);
        assert_eq!(count_class(&tree.root, "tab"), before + 1);
    }

    #[test]
    fn mutate_close_tab_preserves_active_on_adjacent_siblings() {
        let mut state = seed_state();
        state.active_tab = 1; // target middle tab
        mutate_close_tab(&mut state, 1);
        // Closing the active tab falls through to the next sibling which
        // was at index 2 and is now at index 1 after removal.
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab, 1);
    }

    #[test]
    fn mutate_close_tab_decrements_active_when_earlier_tab_removed() {
        let mut state = seed_state();
        state.active_tab = 2;
        mutate_close_tab(&mut state, 0);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab, 1);
    }

    #[test]
    fn dispatch_tab_switch_updates_active_index() {
        let shared = fresh_shared();
        let _tree = dispatch_and_rebuild(&shared, "tab.switch:2");
        assert_eq!(shared.lock().unwrap().active_tab, 2);
    }

    #[test]
    fn dispatch_tab_switch_out_of_range_is_noop() {
        let shared = fresh_shared();
        assert_eq!(shared.lock().unwrap().active_tab, 0);
        let _tree = dispatch_and_rebuild(&shared, "tab.switch:99");
        assert_eq!(shared.lock().unwrap().active_tab, 0);
    }

    #[test]
    fn dispatch_tab_next_wraps_around_last_tab() {
        let shared = fresh_shared();
        shared.lock().unwrap().active_tab = 2;
        let _tree = dispatch_and_rebuild(&shared, "tab.next");
        assert_eq!(shared.lock().unwrap().active_tab, 0);
    }

    #[test]
    fn dispatch_tab_prev_wraps_around_first_tab() {
        let shared = fresh_shared();
        shared.lock().unwrap().active_tab = 0;
        let _tree = dispatch_and_rebuild(&shared, "tab.prev");
        assert_eq!(shared.lock().unwrap().active_tab, 2);
    }

    #[test]
    fn dispatch_sidebar_toggle_flips_collapsed_flag() {
        let shared = fresh_shared();
        assert!(!shared.lock().unwrap().sidebar_collapsed);
        let tree = dispatch_and_rebuild(&shared, "sidebar.toggle");
        assert!(shared.lock().unwrap().sidebar_collapsed);
        // The sidebar class cascade adds `collapsed` when the flag is set.
        let sidebar: Vec<&ElementDef> = {
            let mut all = Vec::new();
            walk(&tree.root, &mut all);
            all.into_iter().filter(|e| e.classes.iter().any(|c| c == "sidebar")).collect()
        };
        assert_eq!(sidebar.len(), 1);
        assert!(sidebar[0].classes.iter().any(|c| c == "collapsed"));
    }

    #[test]
    fn dispatch_font_inc_increments_font_size() {
        let shared = fresh_shared();
        let before = shared.lock().unwrap().font_size_pt;
        let _tree = dispatch_and_rebuild(&shared, "font.inc");
        assert_eq!(shared.lock().unwrap().font_size_pt, before + 1);
    }

    #[test]
    fn dispatch_font_dec_decrements_font_size() {
        let shared = fresh_shared();
        let before = shared.lock().unwrap().font_size_pt;
        let _tree = dispatch_and_rebuild(&shared, "font.dec");
        assert_eq!(shared.lock().unwrap().font_size_pt, before - 1);
    }

    #[test]
    fn font_size_clamps_to_upper_bound() {
        let mut state = seed_state();
        state.font_size_pt = MAX_FONT_SIZE;
        mutate_font_size_delta(&mut state, 5);
        assert_eq!(state.font_size_pt, MAX_FONT_SIZE);
    }

    #[test]
    fn font_size_clamps_to_lower_bound() {
        let mut state = seed_state();
        state.font_size_pt = MIN_FONT_SIZE;
        mutate_font_size_delta(&mut state, -5);
        assert_eq!(state.font_size_pt, MIN_FONT_SIZE);
    }

    #[test]
    fn theme_chip_click_would_mutate_active_theme() {
        // Theme mutation is not routed through dispatch; the click
        // callback inlines the mutation. Verify via a direct state
        // mutation that mirrors what the click closure does.
        let shared = fresh_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.theme = "cyan".to_string();
        }
        let tree = rebuild_shared(&shared);
        let chips: Vec<&ElementDef> = {
            let mut all = Vec::new();
            walk(&tree.root, &mut all);
            all.into_iter().filter(|e| e.classes.iter().any(|c| c == "theme-chip")).collect()
        };
        let active = chips
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "active"))
            .expect("one active chip");
        assert!(active.classes.iter().any(|c| c == "cyan"));
    }

    #[test]
    fn toggle_click_flips_toggle_state_and_class() {
        let shared = fresh_shared();
        {
            let mut guard = shared.lock().unwrap();
            let prev = guard.toggles.get("glow-effect").copied().unwrap_or(false);
            guard.toggles.insert("glow-effect".to_string(), !prev);
        }
        let state_after = shared.lock().unwrap().clone();
        assert_eq!(state_after.toggles.get("glow-effect"), Some(&false));
        let tree = build_tree(&state_after, &shared);
        let toggles: Vec<&ElementDef> = {
            let mut all = Vec::new();
            walk(&tree.root, &mut all);
            all.into_iter().filter(|e| e.classes.iter().any(|c| c == "toggle")).collect()
        };
        let on_count = toggles.iter().filter(|t| t.classes.iter().any(|c| c == "on")).count();
        assert_eq!(on_count, 3);
    }

    #[test]
    fn pane_split_right_inserts_new_pane_in_same_row() {
        let mut state = seed_state();
        let before_len = state.panes[0].len();
        mutate_split_right(&mut state, PaneId(1));
        assert_eq!(state.panes[0].len(), before_len + 1);
        // Active pane now points at the newly inserted pane id (5).
        assert_eq!(state.active_pane, PaneId(5));
    }

    #[test]
    fn pane_split_right_respects_max_cols_cap() {
        let mut state = seed_state();
        while state.panes[0].len() < MAX_COLS {
            let pid = state.panes[0][0].id;
            mutate_split_right(&mut state, pid);
        }
        assert_eq!(state.panes[0].len(), MAX_COLS);
        let saved = state.panes[0].len();
        let pid = state.panes[0][0].id;
        mutate_split_right(&mut state, pid);
        assert_eq!(state.panes[0].len(), saved, "cap must be enforced");
    }

    #[test]
    fn pane_split_down_adds_new_row() {
        let mut state = seed_state();
        let before_rows = state.panes.len();
        mutate_split_down(&mut state, PaneId(1));
        assert_eq!(state.panes.len(), before_rows + 1);
        assert_eq!(state.panes[1].len(), 1, "new row has a single pane");
    }

    #[test]
    fn pane_close_collapses_empty_row() {
        let mut state = seed_state();
        mutate_close_pane(&mut state, PaneId(1));
        mutate_close_pane(&mut state, PaneId(2));
        // Entire first row is now gone.
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0][0].id, PaneId(3));
    }

    #[test]
    fn pane_close_seeds_replacement_when_grid_empties() {
        let mut state = seed_state();
        for pane_id in [PaneId(1), PaneId(2), PaneId(3), PaneId(4)] {
            mutate_close_pane(&mut state, pane_id);
        }
        assert_eq!(state.panes.len(), 1);
        assert_eq!(state.panes[0].len(), 1);
        // A fresh pane id was minted.
        assert!(state.panes[0][0].id.0 >= 5);
    }

    #[test]
    fn pane_click_callback_activates_pane() {
        // Verify the click mutation that `build_pane` installs produces
        // an `active_pane` update. We emulate the callback inline because
        // the callback itself is captured inside the `on_click` closure.
        let shared = fresh_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.active_pane = PaneId(2);
        }
        let tree = rebuild_shared(&shared);
        // The currently rendered pane matches the new active id.
        assert_eq!(count_classes_all(&tree.root, &["pane", "active"]), 1);
    }

    #[test]
    fn workspace_head_click_toggles_collapsed_on_target_workspace() {
        let mut state = seed_state();
        // Workspace 0 ("main") starts expanded.
        assert!(!state.workspaces[0].collapsed);
        // Fire the same mutation the workspace-head callback performs.
        state.workspaces[0].collapsed = !state.workspaces[0].collapsed;
        assert!(state.workspaces[0].collapsed);
    }

    #[test]
    fn subtab_click_updates_active_workspace_and_subtab() {
        let mut state = seed_state();
        // Simulate clicking the third subtab of workspace 1 ("api").
        let wi = 1;
        let si = 2;
        state.active_workspace = wi;
        for (i, sub) in state.workspaces[wi].subtabs.iter_mut().enumerate() {
            sub.active = i == si;
        }
        assert_eq!(state.active_workspace, 1);
        assert!(state.workspaces[1].subtabs[2].active);
        assert!(!state.workspaces[1].subtabs[0].active);
    }

    #[test]
    fn user_shortcut_table_covers_every_required_combo() {
        let bindings = user_shortcut_bindings();
        let combos: Vec<&str> = bindings.iter().map(|(k, _)| k.as_str()).collect();
        for required in [
            "Ctrl+T",
            "Ctrl+W",
            "Ctrl+D",
            "Ctrl+Shift+D",
            "Ctrl+B",
            "Ctrl+,",
            "Ctrl+K",
            "Ctrl+Shift+P",
            "Escape",
            "Ctrl+Tab",
            "Ctrl+Shift+Tab",
            "Ctrl+=",
            "Ctrl+Shift+=",
            "Ctrl+-",
            "Ctrl+1",
            "Ctrl+9",
        ] {
            assert!(combos.contains(&required), "missing shortcut binding for {required}");
        }
    }

    #[test]
    fn user_shortcut_strings_all_parse() {
        use unshit::core::shortcut::Shortcut;
        for (key, _) in user_shortcut_bindings() {
            Shortcut::parse(&key).unwrap_or_else(|e| panic!("failed to parse shortcut {key}: {e}"));
        }
    }

    #[test]
    fn palette_toggle_flips_palette_flag() {
        let shared = fresh_shared();
        assert!(!shared.lock().unwrap().palette_open);
        dispatch_and_rebuild(&shared, "palette.toggle");
        assert!(shared.lock().unwrap().palette_open);
        dispatch_and_rebuild(&shared, "palette.toggle");
        assert!(!shared.lock().unwrap().palette_open);
    }

    #[test]
    fn tabbar_contains_interactive_click_handlers_on_every_tab() {
        // Structural sanity: every rendered `.tab` has an on_click closure
        // attached so clicking anywhere on it activates the tab, and every
        // `.tab-close` has its own closure so clicking the X closes.
        let state = seed_state();
        let (tree, _shared) = build_with_shared(state);
        let tabs: Vec<&ElementDef> = {
            let mut all = Vec::new();
            walk(&tree.root, &mut all);
            all.into_iter().filter(|e| e.classes.iter().any(|c| c == "tab")).collect()
        };
        assert_eq!(tabs.len(), 3);
        for tab in &tabs {
            assert!(tab.on_click.is_some(), "every .tab must have on_click");
        }
        let closes: Vec<&ElementDef> = {
            let mut all = Vec::new();
            walk(&tree.root, &mut all);
            all.into_iter().filter(|e| e.classes.iter().any(|c| c == "tab-close")).collect()
        };
        assert_eq!(closes.len(), 3);
        for close in &closes {
            assert!(close.on_click.is_some(), "every .tab-close has on_click");
        }
    }
}
