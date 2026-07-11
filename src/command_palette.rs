use crate::keybinds::KeybindAction;
use crate::state::UiSnapshot;

pub const PALETTE_QUERY_MAX_CHARS: usize = 256;

const PALETTE_QUERY_SCAN_MAX_CHARS: usize = 1024;
const PALETTE_LABEL_MAX_CHARS: usize = 80;
const PALETTE_DETAIL_MAX_CHARS: usize = 140;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    Unified,
    Actions,
    Agents,
    Navigation,
    Scrollback,
}

impl PaletteMode {
    pub fn title(self) -> &'static str {
        match self {
            Self::Unified => "all",
            Self::Actions => "commands",
            Self::Agents => "agents",
            Self::Navigation => "navigation",
            Self::Scrollback => "scrollback",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPaletteQuery {
    pub mode: PaletteMode,
    pub query: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyScore {
    pub score: i32,
    pub indices: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteGroup {
    Commands,
    Layout,
    Session,
    App,
    Agents,
    Navigation,
}

impl PaletteGroup {
    pub fn title(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Layout => "layout",
            Self::Session => "session",
            Self::App => "app",
            Self::Agents => "agents",
            Self::Navigation => "navigation",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteIcon {
    Terminal,
    SplitRight,
    SplitDown,
    Fullscreen,
    Balance,
    Grid,
    Plus,
    Close,
    Sidebar,
    Settings,
    Agent,
    Workspace,
    Tab,
    Session,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteAction {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub group: PaletteGroup,
    pub icon: PaletteIcon,
    pub keybind: Option<KeybindAction>,
    pub shortcut_label: Option<&'static str>,
    pub dispatch: &'static str,
    pub keywords: &'static [&'static str],
    pub enabled: bool,
}

pub const SAFE_ACTIONS: &[PaletteAction] = &[
    PaletteAction {
        id: "rename_current_terminal",
        label: "Rename current terminal",
        description: "Rename the focused terminal session.",
        group: PaletteGroup::Session,
        icon: PaletteIcon::Terminal,
        keybind: Some(KeybindAction::RenameSession),
        shortcut_label: None,
        dispatch: "session.rename_active",
        keywords: &["rename", "terminal", "session", "title"],
        enabled: true,
    },
    PaletteAction {
        id: "split_pane_right",
        label: "Split pane right",
        description: "Split the focused pane into a right-hand terminal.",
        group: PaletteGroup::Commands,
        icon: PaletteIcon::SplitRight,
        keybind: Some(KeybindAction::SplitRight),
        shortcut_label: None,
        dispatch: "pane.split_right",
        keywords: &["split", "pane", "right", "vertical", "terminal"],
        enabled: true,
    },
    PaletteAction {
        id: "split_pane_down",
        label: "Split pane down",
        description: "Split the focused pane into a lower terminal.",
        group: PaletteGroup::Commands,
        icon: PaletteIcon::SplitDown,
        keybind: Some(KeybindAction::SplitDown),
        shortcut_label: None,
        dispatch: "pane.split_down",
        keywords: &["split", "pane", "down", "horizontal", "terminal"],
        enabled: true,
    },
    PaletteAction {
        id: "new_terminal",
        label: "New terminal",
        description: "Open a new terminal tab.",
        group: PaletteGroup::Commands,
        icon: PaletteIcon::Plus,
        keybind: Some(KeybindAction::NewTerminal),
        shortcut_label: None,
        dispatch: "tab.new",
        keywords: &["new", "terminal", "tab", "shell"],
        enabled: true,
    },
    PaletteAction {
        id: "close_pane",
        label: "Close pane",
        description: "Close the focused pane using the existing close behavior.",
        group: PaletteGroup::Commands,
        icon: PaletteIcon::Close,
        keybind: Some(KeybindAction::Unsplit),
        shortcut_label: None,
        dispatch: "pane.close",
        keywords: &["close", "pane", "unsplit", "terminal"],
        enabled: true,
    },
    PaletteAction {
        id: "arrange_grid_2x2",
        label: "Arrange grid 2x2",
        description: "Tile panes into a grid when grid arranging is available.",
        group: PaletteGroup::Layout,
        icon: PaletteIcon::Grid,
        keybind: None,
        shortcut_label: None,
        dispatch: "layout",
        keywords: &["arrange", "grid", "2x2", "layout", "tile"],
        enabled: false,
    },
    PaletteAction {
        id: "balance_panes",
        label: "Balance panes",
        description: "Equalize pane sizes when balance support is available.",
        group: PaletteGroup::Layout,
        icon: PaletteIcon::Balance,
        keybind: None,
        shortcut_label: Some("Ctrl+="),
        dispatch: "layout",
        keywords: &["balance", "equalize", "panes", "layout"],
        enabled: false,
    },
    PaletteAction {
        id: "toggle_pane_fullscreen",
        label: "Toggle pane fullscreen",
        description: "Zoom the focused pane when pane fullscreen is available.",
        group: PaletteGroup::Layout,
        icon: PaletteIcon::Fullscreen,
        keybind: None,
        shortcut_label: Some("Ctrl+Enter"),
        dispatch: "layout",
        keywords: &["toggle", "pane", "fullscreen", "zoom", "layout"],
        enabled: false,
    },
    PaletteAction {
        id: "toggle_sidebar",
        label: "Toggle sidebar",
        description: "Show or hide the workspace sidebar.",
        group: PaletteGroup::Layout,
        icon: PaletteIcon::Sidebar,
        keybind: Some(KeybindAction::ToggleSidebar),
        shortcut_label: None,
        dispatch: "sidebar.toggle",
        keywords: &["toggle", "sidebar", "workspace", "nav"],
        enabled: true,
    },
    PaletteAction {
        id: "kill_session",
        label: "Kill session",
        description: "Kill-session commands stay disabled in the palette until scoped safely.",
        group: PaletteGroup::Session,
        icon: PaletteIcon::Terminal,
        keybind: None,
        shortcut_label: Some("Ctrl+C"),
        dispatch: "session",
        keywords: &["kill", "session", "terminal", "danger"],
        enabled: false,
    },
    PaletteAction {
        id: "restart_session",
        label: "Restart session",
        description: "Restart the focused session when restart support is available.",
        group: PaletteGroup::Session,
        icon: PaletteIcon::Terminal,
        keybind: None,
        shortcut_label: None,
        dispatch: "session",
        keywords: &["restart", "session", "respawn", "terminal"],
        enabled: false,
    },
    PaletteAction {
        id: "clear_scrollback",
        label: "Clear scrollback",
        description: "Clear terminal scrollback when a scoped scrollback command is available.",
        group: PaletteGroup::Session,
        icon: PaletteIcon::Terminal,
        keybind: None,
        shortcut_label: Some("Ctrl+L"),
        dispatch: "session",
        keywords: &["clear", "scrollback", "terminal", "session"],
        enabled: false,
    },
    PaletteAction {
        id: "spawn_agent",
        label: "Spawn agent...",
        description: "Open Quick Prompt to spawn an agent-backed terminal.",
        group: PaletteGroup::Session,
        icon: PaletteIcon::Agent,
        keybind: Some(KeybindAction::QuickPromptOpen),
        shortcut_label: None,
        dispatch: "quick_prompt.open",
        keywords: &["spawn", "agent", "quick", "prompt", "codex", "claude"],
        enabled: true,
    },
    PaletteAction {
        id: "new_worktree",
        label: "New worktree...",
        description: "Worktree creation is not wired to a command yet.",
        group: PaletteGroup::Session,
        icon: PaletteIcon::Workspace,
        keybind: None,
        shortcut_label: None,
        dispatch: "session",
        keywords: &["new", "worktree", "git", "session"],
        enabled: false,
    },
    PaletteAction {
        id: "open_settings",
        label: "Open settings",
        description: "Open app settings.",
        group: PaletteGroup::App,
        icon: PaletteIcon::Settings,
        keybind: Some(KeybindAction::OpenSettings),
        shortcut_label: None,
        dispatch: "modal.open",
        keywords: &["open", "settings", "preferences", "config"],
        enabled: true,
    },
    PaletteAction {
        id: "change_theme",
        label: "Change theme...",
        description: "Open settings to change the application theme.",
        group: PaletteGroup::App,
        icon: PaletteIcon::Settings,
        keybind: None,
        shortcut_label: None,
        dispatch: "modal.open",
        keywords: &["change", "theme", "appearance", "color", "app"],
        enabled: true,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteItemKind {
    Action,
    Workspace,
    Tab,
    Terminal,
    Session,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub group: PaletteGroup,
    pub kind: PaletteItemKind,
    pub icon: PaletteIcon,
    pub dispatch: Option<String>,
    pub keybind: Option<KeybindAction>,
    pub shortcut: Option<String>,
    pub keywords: Vec<String>,
    pub enabled: bool,
    pub score: Option<FuzzyScore>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteGroupView {
    pub group: PaletteGroup,
    pub title: String,
    pub items: Vec<PaletteItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteEmptyState {
    pub mode: PaletteMode,
    pub title: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteResults {
    pub mode: PaletteMode,
    pub query: String,
    pub groups: Vec<PaletteGroupView>,
    pub empty_state: Option<PaletteEmptyState>,
}

pub fn sanitize_palette_query(input: &str) -> String {
    let mut output = String::new();
    let mut last_was_space = false;

    for ch in input.chars().take(PALETTE_QUERY_SCAN_MAX_CHARS) {
        if is_bidi_control(ch) {
            continue;
        }

        let Some(ch) = normalize_palette_char(ch) else {
            continue;
        };

        if ch == ' ' {
            if output.is_empty() || last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }

        if output.chars().count() >= PALETTE_QUERY_MAX_CHARS {
            break;
        }
        output.push(ch);
    }

    output
}

pub fn parse_palette_query(input: &str) -> ParsedPaletteQuery {
    let sanitized = sanitize_palette_query(input);
    let trimmed = sanitized.trim_start();
    let (mode, query) = match trimmed.chars().next() {
        Some('>') => (PaletteMode::Actions, &trimmed['>'.len_utf8()..]),
        Some('@') => (PaletteMode::Agents, &trimmed['@'.len_utf8()..]),
        Some(':') => (PaletteMode::Navigation, &trimmed[':'.len_utf8()..]),
        Some('/') => (PaletteMode::Scrollback, &trimmed['/'.len_utf8()..]),
        _ => (PaletteMode::Actions, trimmed),
    };
    ParsedPaletteQuery {
        mode,
        query: query.trim().to_string(),
    }
}

fn normalize_palette_char(ch: char) -> Option<char> {
    if ch.is_whitespace() {
        Some(' ')
    } else if ch.is_control() {
        None
    } else {
        Some(ch)
    }
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn sanitize_palette_text(input: &str, fallback: String, max_chars: usize) -> String {
    let scan_limit = max_chars.saturating_mul(4).max(max_chars + 1);
    let mut output = String::new();
    let mut last_was_space = false;
    let mut accepted = 0usize;
    let mut truncated = false;

    for ch in input.chars().take(scan_limit) {
        if is_bidi_control(ch) {
            continue;
        }

        let Some(ch) = normalize_palette_char(ch) else {
            continue;
        };

        if ch == ' ' {
            if output.is_empty() || last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }

        if accepted >= max_chars {
            truncated = true;
            break;
        }

        output.push(ch);
        accepted += 1;
    }

    let mut output = output.trim().to_string();
    if output.is_empty() {
        return fallback;
    }
    if truncated {
        output.push_str("...");
    }
    output
}

fn sanitize_palette_label(input: &str, fallback: String) -> String {
    sanitize_palette_text(input, fallback, PALETTE_LABEL_MAX_CHARS)
}

fn sanitize_palette_detail(input: &str, fallback: String) -> String {
    sanitize_palette_text(input, fallback, PALETTE_DETAIL_MAX_CHARS)
}

pub fn fuzzy_match(query: &str, text: &str) -> Option<FuzzyScore> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Some(FuzzyScore {
            score: 0,
            indices: Vec::new(),
        });
    }

    let lower_text = text.to_ascii_lowercase();
    let text_bytes = lower_text.as_bytes();
    let original = text.as_bytes();
    let mut search_from = 0usize;
    let mut indices = Vec::with_capacity(query.len());
    let mut score = 0i32;
    let mut previous: Option<usize> = None;

    for needle in query.bytes() {
        let rel = text_bytes[search_from..]
            .iter()
            .position(|candidate| *candidate == needle)?;
        let idx = search_from + rel;
        indices.push(idx);

        score += 100;
        if is_word_boundary(original, idx) {
            score += 25;
        }
        if idx == 0 {
            score += 15;
        }
        if let Some(prev) = previous {
            if idx == prev + 1 {
                score += 40;
            } else {
                let gap = idx.saturating_sub(prev + 1).min(30) as i32;
                score -= gap;
            }
        }

        previous = Some(idx);
        search_from = idx + 1;
    }

    if let Some(first) = indices.first() {
        score -= (*first).min(40) as i32;
    }

    Some(FuzzyScore { score, indices })
}

pub fn build_palette_results(snap: &UiSnapshot, input: &str) -> PaletteResults {
    let parsed = parse_palette_query(input);
    let mut items = match parsed.mode {
        PaletteMode::Unified => {
            let mut items = action_items(snap);
            items.extend(agent_items(snap));
            items.extend(navigation_items(snap));
            items.extend(session_items(snap));
            items
        }
        PaletteMode::Actions => action_items(snap),
        PaletteMode::Agents => agent_items(snap),
        PaletteMode::Navigation => {
            let mut items = navigation_items(snap);
            items.extend(session_items(snap));
            items
        }
        PaletteMode::Scrollback => Vec::new(),
    };

    items = filter_and_rank(items, &parsed.query);
    let groups = group_items(items);
    let empty_state = if groups.is_empty() {
        Some(empty_state(parsed.mode, &parsed.query))
    } else {
        None
    };

    PaletteResults {
        mode: parsed.mode,
        query: parsed.query,
        groups,
        empty_state,
    }
}

fn is_word_boundary(text: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = text[idx - 1];
    !prev.is_ascii_alphanumeric()
}

fn action_items(snap: &UiSnapshot) -> Vec<PaletteItem> {
    SAFE_ACTIONS
        .iter()
        .map(|action| PaletteItem {
            id: action.id.to_string(),
            label: action.label.to_string(),
            description: action.description.to_string(),
            group: action.group,
            kind: PaletteItemKind::Action,
            icon: action.icon,
            dispatch: Some(action.dispatch.to_string()),
            keybind: action.keybind,
            shortcut: action.shortcut_label.map(str::to_string).or_else(|| {
                action
                    .keybind
                    .map(|keybind| snap.keybinds.effective(keybind).to_string())
            }),
            keywords: action
                .keywords
                .iter()
                .map(|keyword| (*keyword).to_string())
                .collect(),
            enabled: action.enabled,
            score: None,
            status: None,
        })
        .collect()
}

fn agent_items(snap: &UiSnapshot) -> Vec<PaletteItem> {
    let mut items = Vec::new();

    if let Some(quick_prompt) = &snap.quick_prompt {
        let agent = quick_prompt.agent.label();
        items.push(PaletteItem {
            id: "agent:quick_prompt".to_string(),
            label: format!("{agent} quick prompt"),
            description: format!("Open real Quick Prompt state with {agent} selected."),
            group: PaletteGroup::Agents,
            kind: PaletteItemKind::Agent,
            icon: PaletteIcon::Agent,
            dispatch: None,
            keybind: None,
            shortcut: None,
            keywords: vec![
                "agent".to_string(),
                "quick prompt".to_string(),
                agent.to_ascii_lowercase(),
            ],
            enabled: false,
            score: None,
            status: Some("active".to_string()),
        });
    }

    for (ws_idx, workspace) in snap.workspaces.iter().enumerate() {
        let workspace_label = workspace_display_label(workspace);
        let active_workspace = ws_idx == snap.active_workspace;
        let tabs = workspace_tabs(snap, ws_idx, workspace);

        for tab in tabs {
            let tab_agent = agent_label_from_program(&tab.subtitle);
            for pane in tab.panes.iter().flatten() {
                let Some(agent) = agent_label_from_program(&pane.subtitle).or(tab_agent) else {
                    continue;
                };
                let pane_title = pane_display_title(pane);
                items.push(PaletteItem {
                    id: format!("agent-terminal:{ws_idx}:{}", pane.id.0),
                    label: format!("{agent}: {pane_title}"),
                    description: format!(
                        "{workspace_label} · pane {} · real agent terminal",
                        pane.id.0
                    ),
                    group: PaletteGroup::Agents,
                    kind: PaletteItemKind::Agent,
                    icon: PaletteIcon::Agent,
                    dispatch: Some(format!("terminal.focus:{ws_idx}:{}", pane.id.0)),
                    keybind: None,
                    shortcut: None,
                    keywords: vec![
                        "agent".to_string(),
                        agent.to_ascii_lowercase(),
                        "terminal".to_string(),
                        "pane".to_string(),
                        pane.id.0.to_string(),
                        workspace_label.clone(),
                    ],
                    enabled: true,
                    score: None,
                    status: (active_workspace && snap.active_pane == pane.id)
                        .then(|| "active".to_string()),
                });
            }
        }
    }

    items
}

fn agent_label_from_program(program: &str) -> Option<&'static str> {
    let executable = program
        .trim()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    let executable = executable
        .strip_suffix(".cmd")
        .or_else(|| executable.strip_suffix(".exe"))
        .unwrap_or(&executable);

    match executable {
        "claude" => Some("Claude"),
        "codex" => Some("Codex"),
        _ => None,
    }
}

fn workspace_display_label(workspace: &crate::state::Workspace) -> String {
    if workspace.name.trim().is_empty() {
        format!("Workspace {}", workspace.num)
    } else {
        sanitize_palette_label(&workspace.name, format!("Workspace {}", workspace.num))
    }
}

fn pane_display_title(pane: &crate::state::Pane) -> String {
    if pane.title.trim().is_empty() {
        format!("pane {}", pane.id.0)
    } else {
        sanitize_palette_label(&pane.title, format!("pane {}", pane.id.0))
    }
}

fn workspace_tabs<'a>(
    snap: &'a UiSnapshot,
    ws_idx: usize,
    workspace: &'a crate::state::Workspace,
) -> &'a [crate::state::TerminalTab] {
    if ws_idx == snap.active_workspace {
        &snap.tabs
    } else {
        &workspace.tabs
    }
}

fn navigation_items(snap: &UiSnapshot) -> Vec<PaletteItem> {
    let mut items = Vec::new();

    for (ws_idx, workspace) in snap.workspaces.iter().enumerate() {
        let workspace_label = workspace_display_label(workspace);
        let active_workspace = ws_idx == snap.active_workspace;
        items.push(PaletteItem {
            id: format!("workspace:{ws_idx}"),
            label: workspace_label.clone(),
            description: if active_workspace {
                "Active workspace".to_string()
            } else {
                format!("Workspace {}", workspace.num)
            },
            group: PaletteGroup::Navigation,
            kind: PaletteItemKind::Workspace,
            icon: PaletteIcon::Workspace,
            dispatch: Some(format!("workspace.switch:{ws_idx}")),
            keybind: None,
            shortcut: None,
            keywords: vec!["workspace".to_string(), workspace.num.to_string()],
            enabled: true,
            score: None,
            status: active_workspace.then(|| "active".to_string()),
        });

        let tabs = workspace_tabs(snap, ws_idx, workspace);
        for (tab_idx, tab) in tabs.iter().enumerate() {
            let dispatch = tab_focus_dispatch(ws_idx, tab);
            items.push(PaletteItem {
                id: format!("tab:{ws_idx}:{tab_idx}"),
                label: sanitize_palette_label(&tab.name, format!("tab {}", tab_idx + 1)),
                description: format!("{workspace_label} · tab {}", tab_idx + 1),
                group: PaletteGroup::Navigation,
                kind: PaletteItemKind::Tab,
                icon: PaletteIcon::Tab,
                dispatch: dispatch.clone(),
                keybind: None,
                shortcut: None,
                keywords: vec![
                    "tab".to_string(),
                    "terminal".to_string(),
                    workspace_label.clone(),
                ],
                enabled: dispatch.is_some(),
                score: None,
                status: (active_workspace && tab_idx == snap.active_tab)
                    .then(|| "active".to_string()),
            });
        }

        let entries = if workspace.terminal_entries.is_empty() {
            terminal_entries_from_tabs(tabs)
        } else {
            workspace
                .terminal_entries
                .iter()
                .map(|entry| {
                    let pane_id = entry.pane_id.0;
                    (
                        pane_id,
                        sanitize_palette_label(&entry.name, format!("pane {pane_id}")),
                        if entry.branch_error {
                            "no git".to_string()
                        } else {
                            sanitize_palette_detail(&entry.branch, "branch unavailable".to_string())
                        },
                    )
                })
                .collect()
        };
        for (pane_id, name, branch) in entries {
            items.push(PaletteItem {
                id: format!("terminal:{ws_idx}:{pane_id}"),
                label: name,
                description: format!("{workspace_label} · pane {pane_id} · {branch}"),
                group: PaletteGroup::Navigation,
                kind: PaletteItemKind::Terminal,
                icon: PaletteIcon::Terminal,
                dispatch: Some(format!("terminal.focus:{ws_idx}:{pane_id}")),
                keybind: None,
                shortcut: None,
                keywords: vec![
                    "terminal".to_string(),
                    "pane".to_string(),
                    pane_id.to_string(),
                    workspace_label.clone(),
                ],
                enabled: true,
                score: None,
                status: (active_workspace && snap.active_pane.0 == pane_id)
                    .then(|| "active".to_string()),
            });
        }
    }

    items
}

fn tab_focus_dispatch(ws_idx: usize, tab: &crate::state::TerminalTab) -> Option<String> {
    let pane_id = tab
        .panes
        .iter()
        .flatten()
        .find(|pane| pane.id == tab.active_pane)
        .or_else(|| tab.panes.iter().flatten().next())
        .map(|pane| pane.id)?;
    Some(format!("terminal.focus:{ws_idx}:{}", pane_id.0))
}

fn terminal_entries_from_tabs(tabs: &[crate::state::TerminalTab]) -> Vec<(u32, String, String)> {
    tabs.iter()
        .flat_map(|tab| {
            tab.panes.iter().flat_map(|row| {
                row.iter().map(|pane| {
                    (
                        pane.id.0,
                        pane_display_title(pane),
                        sanitize_palette_detail(&tab.subtitle, "shell".to_string()),
                    )
                })
            })
        })
        .collect()
}

fn session_items(snap: &UiSnapshot) -> Vec<PaletteItem> {
    snap.sessions
        .iter()
        .map(|session| {
            let ws_idx = snap
                .workspaces
                .iter()
                .position(|workspace| workspace.num == session.workspace_id);
            let label = session
                .name
                .clone()
                .unwrap_or_else(|| format!("session {}", session.session_id));
            let label = sanitize_palette_label(&label, format!("session {}", session.session_id));
            PaletteItem {
                id: format!("session:{}", session.session_id),
                label,
                description: format!(
                    "Workspace {} · pane {}",
                    session.workspace_id, session.pane_id
                ),
                group: PaletteGroup::Session,
                kind: PaletteItemKind::Session,
                icon: PaletteIcon::Session,
                dispatch: ws_idx.map(|idx| format!("terminal.focus:{idx}:{}", session.pane_id)),
                keybind: None,
                shortcut: None,
                keywords: vec![
                    "session".to_string(),
                    "terminal".to_string(),
                    session.session_id.to_string(),
                    session.pane_id.to_string(),
                ],
                enabled: session.alive && ws_idx.is_some(),
                score: None,
                status: Some(if session.alive { "alive" } else { "stopped" }.to_string()),
            }
        })
        .collect()
}

fn filter_and_rank(items: Vec<PaletteItem>, query: &str) -> Vec<PaletteItem> {
    if query.trim().is_empty() {
        return items;
    }

    let mut scored: Vec<PaletteItem> = items
        .into_iter()
        .filter_map(|mut item| {
            score_item(&item, query).map(|score| {
                item.score = Some(score);
                item
            })
        })
        .collect();

    scored.sort_by(|a, b| {
        let a_score = a
            .score
            .as_ref()
            .map(|score| score.score)
            .unwrap_or_default();
        let b_score = b
            .score
            .as_ref()
            .map(|score| score.score)
            .unwrap_or_default();
        b_score
            .cmp(&a_score)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });

    scored
}

fn score_item(item: &PaletteItem, query: &str) -> Option<FuzzyScore> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() {
        return Some(FuzzyScore {
            score: 0,
            indices: Vec::new(),
        });
    }

    let fields = searchable_fields(item);
    let mut total = 0;
    let mut indices = Vec::new();
    for term in terms {
        let best = fields
            .iter()
            .filter_map(|(field, boost)| {
                fuzzy_match(term, field).map(|mut score| {
                    score.score += boost;
                    score
                })
            })
            .max_by_key(|score| score.score)?;
        total += best.score;
        indices.extend(best.indices);
    }
    total += exact_phrase_bonus(item, query);

    Some(FuzzyScore {
        score: total,
        indices,
    })
}

fn exact_phrase_bonus(item: &PaletteItem, query: &str) -> i32 {
    let phrase = query.trim().to_ascii_lowercase();
    if phrase.is_empty() {
        return 0;
    }

    let id_words = item.id.replace('_', " ").to_ascii_lowercase();
    if item.label.to_ascii_lowercase().contains(&phrase) || id_words.contains(&phrase) {
        return 1_000_000;
    }

    let keyword_words = item.keywords.join(" ").to_ascii_lowercase();
    if keyword_words.contains(&phrase) {
        return 750_000;
    }

    if item.description.to_ascii_lowercase().contains(&phrase) {
        return 250_000;
    }

    0
}

fn searchable_fields(item: &PaletteItem) -> Vec<(String, i32)> {
    let mut fields = vec![
        (item.label.clone(), 120),
        (item.description.clone(), 60),
        (item.id.clone(), 40),
    ];
    if let Some(dispatch) = &item.dispatch {
        fields.push((dispatch.clone(), 30));
    }
    if let Some(keybind) = item.keybind {
        fields.push((keybind.label().to_string(), 50));
    }
    if let Some(shortcut) = &item.shortcut {
        fields.push((shortcut.clone(), 50));
    }
    for keyword in &item.keywords {
        fields.push((keyword.clone(), 80));
    }
    fields
}

fn group_items(items: Vec<PaletteItem>) -> Vec<PaletteGroupView> {
    let mut groups: Vec<PaletteGroupView> = GROUP_ORDER
        .iter()
        .filter_map(|group| {
            let grouped: Vec<PaletteItem> = items
                .iter()
                .filter(|item| item.group == *group)
                .cloned()
                .collect();
            (!grouped.is_empty()).then(|| PaletteGroupView {
                group: *group,
                title: group.title().to_string(),
                items: grouped,
            })
        })
        .collect();

    if groups
        .iter()
        .any(|group| group.items.iter().any(|item| item.score.is_some()))
    {
        groups.sort_by(|a, b| {
            group_best_score(b)
                .cmp(&group_best_score(a))
                .then_with(|| group_rank(a.group).cmp(&group_rank(b.group)))
        });
    }

    groups
}

const GROUP_ORDER: &[PaletteGroup] = &[
    PaletteGroup::Commands,
    PaletteGroup::Layout,
    PaletteGroup::Session,
    PaletteGroup::App,
    PaletteGroup::Agents,
    PaletteGroup::Navigation,
];

fn group_rank(group: PaletteGroup) -> usize {
    GROUP_ORDER
        .iter()
        .position(|candidate| *candidate == group)
        .unwrap_or(GROUP_ORDER.len())
}

fn group_best_score(group: &PaletteGroupView) -> i32 {
    group
        .items
        .iter()
        .filter_map(|item| item.score.as_ref().map(|score| score.score))
        .max()
        .unwrap_or_default()
}

fn empty_state(mode: PaletteMode, query: &str) -> PaletteEmptyState {
    let (title, message) = match mode {
        PaletteMode::Unified | PaletteMode::Actions if !query.is_empty() => {
            ("No commands found", "Try another command name or prefix.")
        }
        PaletteMode::Unified => (
            "No results",
            "No commands or navigation rows are available.",
        ),
        PaletteMode::Actions => ("No commands", "No executable commands are available."),
        PaletteMode::Navigation => (
            "No navigation results",
            "No matching real workspace, tab, terminal, or session exists.",
        ),
        PaletteMode::Agents => (
            "No agents available",
            "No real agent metadata is connected to terminal state yet.",
        ),
        PaletteMode::Scrollback => (
            "No searchable scrollback",
            "No read-only terminal scrollback source is available yet.",
        ),
    };

    PaletteEmptyState {
        mode,
        title: title.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, Pane, PaneId, SessionSnapshot, TabStatus, TerminalTab};

    fn flatten(results: &PaletteResults) -> Vec<&PaletteItem> {
        results
            .groups
            .iter()
            .flat_map(|group| group.items.iter())
            .collect()
    }

    fn flattened_ids(results: &PaletteResults) -> Vec<String> {
        flatten(results)
            .into_iter()
            .map(|item| item.id.clone())
            .collect()
    }

    fn assert_display_clean(text: &str) {
        assert!(!text.chars().any(char::is_control));
        assert!(!text.contains('\u{202e}'));
        assert!(!text.contains('\u{2066}'));
    }

    #[test]
    fn parses_typed_modes_and_strips_prefix() {
        assert_eq!(parse_palette_query("rename").mode, PaletteMode::Actions);

        let actions = parse_palette_query("> split");
        assert_eq!(actions.mode, PaletteMode::Actions);
        assert_eq!(actions.query, "split");

        let agents = parse_palette_query("@ codex");
        assert_eq!(agents.mode, PaletteMode::Agents);
        assert_eq!(agents.query, "codex");

        let navigation = parse_palette_query(": api");
        assert_eq!(navigation.mode, PaletteMode::Navigation);
        assert_eq!(navigation.query, "api");

        let scrollback = parse_palette_query("/ error");
        assert_eq!(scrollback.mode, PaletteMode::Scrollback);
        assert_eq!(scrollback.query, "error");
    }

    #[test]
    fn palette_query_is_bounded_and_sanitized() {
        let input = format!(">\trename\n{}\u{202e}", "x".repeat(400));
        let sanitized = sanitize_palette_query(&input);
        let parsed = parse_palette_query(&input);

        assert!(sanitized.chars().count() <= PALETTE_QUERY_MAX_CHARS);
        assert!(!sanitized.chars().any(char::is_control));
        assert!(!sanitized.contains('\u{202e}'));
        assert_eq!(parsed.mode, PaletteMode::Actions);
        assert!(parsed.query.starts_with("rename"));
        assert!(parsed.query.chars().count() < PALETTE_QUERY_MAX_CHARS);
    }

    #[test]
    fn real_navigation_labels_are_sanitized_for_display() {
        let mut state = seed_state();
        state.workspaces[0].name = "Ops\n\u{202e}Workspace".to_string();
        state.tabs[0].name = format!("release\n{}", "x".repeat(120));
        state.panes[0][0].title = "build\r\u{2066}pane".to_string();
        state.workspaces[0].terminal_entries.clear();
        state.workspaces[0].tabs = state.tabs.clone();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, ":");
        let items = flatten(&results);
        for id in ["workspace:0", "tab:0:0", "terminal:0:1"] {
            let item = items
                .iter()
                .find(|item| item.id == id)
                .unwrap_or_else(|| panic!("missing {id}"));
            assert_display_clean(&item.label);
            assert_display_clean(&item.description);
            assert!(item.label.chars().count() <= PALETTE_LABEL_MAX_CHARS + 3);
        }
    }

    #[test]
    fn fuzzy_match_prefers_contiguous_and_word_boundary_hits() {
        let contiguous = fuzzy_match("term", "terminal").expect("contiguous match");
        let gapped = fuzzy_match("term", "t e r m").expect("gapped match");
        assert!(contiguous.score > gapped.score);

        let boundary = fuzzy_match("term", "open terminal").expect("boundary match");
        let middle = fuzzy_match("term", "preterminal").expect("middle match");
        assert!(boundary.score > middle.score);
    }

    #[test]
    fn action_catalog_matches_reference_groups_and_safety() {
        let ids: Vec<&str> = SAFE_ACTIONS.iter().map(|action| action.id).collect();
        assert_eq!(
            ids,
            vec![
                "rename_current_terminal",
                "split_pane_right",
                "split_pane_down",
                "new_terminal",
                "close_pane",
                "arrange_grid_2x2",
                "balance_panes",
                "toggle_pane_fullscreen",
                "toggle_sidebar",
                "kill_session",
                "restart_session",
                "clear_scrollback",
                "spawn_agent",
                "new_worktree",
                "open_settings",
                "change_theme",
            ]
        );
        assert!(SAFE_ACTIONS.iter().all(|action| {
            action.enabled
                || matches!(
                    action.id,
                    "arrange_grid_2x2"
                        | "balance_panes"
                        | "toggle_pane_fullscreen"
                        | "kill_session"
                        | "restart_session"
                        | "clear_scrollback"
                        | "new_worktree"
                )
        }));
        assert!(SAFE_ACTIONS
            .iter()
            .filter(|action| action.enabled)
            .all(|action| !action.id.contains("kill") && !action.dispatch.contains("kill")));
    }

    #[test]
    fn result_builder_uses_real_snapshot_rows_without_prototype_data() {
        let mut state = seed_state();
        let pane = Pane {
            id: PaneId(7),
            title: "shell-7".to_string(),
            subtitle: "bash".to_string(),
            pid: 0,
            cpu: 0.0,
        };
        state.workspaces[1].tabs = vec![TerminalTab {
            id: "t7".to_string(),
            name: "api-tab".to_string(),
            subtitle: "bash".to_string(),
            status: TabStatus::Running,
            panes: vec![vec![pane]],
            active_pane: PaneId(7),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        }];
        let snap = state.ui_snapshot();

        let nav = build_palette_results(&snap, ":shell-7");
        let items = flatten(&nav);
        assert!(items.iter().any(|item| {
            item.kind == PaletteItemKind::Terminal
                && item.label == "shell-7"
                && item.dispatch.as_deref() == Some("terminal.focus:1:7")
        }));

        let all_text = items
            .iter()
            .map(|item| format!("{} {}", item.id, item.label))
            .collect::<Vec<_>>()
            .join("\n");
        for fake in [
            "dashboard-dev",
            "api.server",
            "claude - refactor-userlist",
            "design-system",
        ] {
            assert!(!all_text.contains(fake), "prototype row leaked: {fake}");
        }
    }

    #[test]
    fn unavailable_modes_return_honest_empty_states() {
        let state = seed_state();
        let snap = state.ui_snapshot();

        let agents = build_palette_results(&snap, "@");
        assert!(agents.groups.is_empty());
        assert_eq!(
            agents.empty_state.as_ref().map(|empty| empty.mode),
            Some(PaletteMode::Agents)
        );

        let scrollback = build_palette_results(&snap, "/");
        assert!(scrollback.groups.is_empty());
        assert_eq!(
            scrollback.empty_state.as_ref().map(|empty| empty.mode),
            Some(PaletteMode::Scrollback)
        );
    }

    #[test]
    fn actions_mode_contains_reference_action_rows() {
        let state = seed_state();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, ">");
        let items = flatten(&results);
        let mut expected_action_ids: Vec<String> = SAFE_ACTIONS
            .iter()
            .map(|action| action.id.to_string())
            .collect();
        let mut actual_ids = flattened_ids(&results);
        expected_action_ids.sort();
        actual_ids.sort();

        assert_eq!(results.mode, PaletteMode::Actions);
        assert_eq!(actual_ids, expected_action_ids);
        assert!(items
            .iter()
            .all(|item| item.kind == PaletteItemKind::Action));
        assert!(items.iter().filter(|item| item.enabled).all(|item| item
            .dispatch
            .as_deref()
            .is_some_and(|dispatch| {
                matches!(
                    dispatch,
                    "session.rename_active"
                        | "pane.split_right"
                        | "pane.split_down"
                        | "tab.new"
                        | "pane.close"
                        | "sidebar.toggle"
                        | "modal.open"
                        | "quick_prompt.open"
                )
            })));
        assert!(items
            .iter()
            .filter(|item| !item.enabled)
            .all(|item| item.dispatch.is_some()));
    }

    #[test]
    fn actions_mode_groups_safe_commands_by_scope() {
        let state = seed_state();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, ">");
        let grouped_ids: Vec<(PaletteGroup, Vec<&str>)> = results
            .groups
            .iter()
            .map(|group| {
                (
                    group.group,
                    group.items.iter().map(|item| item.id.as_str()).collect(),
                )
            })
            .collect();

        assert_eq!(
            grouped_ids,
            vec![
                (
                    PaletteGroup::Commands,
                    vec![
                        "split_pane_right",
                        "split_pane_down",
                        "new_terminal",
                        "close_pane",
                    ],
                ),
                (
                    PaletteGroup::Layout,
                    vec![
                        "arrange_grid_2x2",
                        "balance_panes",
                        "toggle_pane_fullscreen",
                        "toggle_sidebar",
                    ],
                ),
                (
                    PaletteGroup::Session,
                    vec![
                        "rename_current_terminal",
                        "kill_session",
                        "restart_session",
                        "clear_scrollback",
                        "spawn_agent",
                        "new_worktree",
                    ],
                ),
                (PaletteGroup::App, vec!["open_settings", "change_theme"]),
            ]
        );
    }

    #[test]
    fn navigation_mode_contains_real_workspace_tab_and_terminal_rows() {
        let state = seed_state();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, ":");
        let ids = flattened_ids(&results);

        assert_eq!(results.mode, PaletteMode::Navigation);
        assert!(ids.contains(&"workspace:0".to_string()));
        assert!(ids.contains(&"tab:0:0".to_string()));
        assert!(ids.contains(&"terminal:0:1".to_string()));
        assert!(flatten(&results)
            .iter()
            .all(|item| item.kind != PaletteItemKind::Action));
    }

    #[test]
    fn navigation_tab_rows_focus_their_real_active_pane() {
        let mut state = seed_state();
        let pane = Pane {
            id: PaneId(9),
            title: "ops-shell".to_string(),
            subtitle: "bash".to_string(),
            pid: 0,
            cpu: 0.0,
        };
        state.tabs.push(TerminalTab {
            id: "t9".to_string(),
            name: "ops-tab".to_string(),
            subtitle: "bash".to_string(),
            status: TabStatus::Running,
            panes: vec![vec![pane]],
            active_pane: PaneId(9),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        });
        state.workspaces[0].tabs = state.tabs.clone();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, ": ops-tab");
        let tab = flatten(&results)
            .into_iter()
            .find(|item| item.id == "tab:0:1")
            .expect("real tab row");

        assert_eq!(tab.kind, PaletteItemKind::Tab);
        assert_eq!(tab.label, "ops-tab");
        assert_eq!(tab.dispatch.as_deref(), Some("terminal.focus:0:9"));
    }

    #[test]
    fn navigation_mode_includes_real_tabs_and_sessions_where_present() {
        let mut state = seed_state();
        state.tabs[0].name = "main-tab".to_string();
        state.workspaces[0].tabs = state.tabs.clone();
        state.sessions = vec![SessionSnapshot {
            session_id: 42,
            pane_id: 1,
            workspace_id: 1,
            name: Some("main-session".to_string()),
            pid: Some(1234),
            memory_rss_bytes: Some(64 * 1024 * 1024),
            alive: true,
        }];
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, ":");
        let ids = flattened_ids(&results);
        let session = flatten(&results)
            .into_iter()
            .find(|item| item.id == "session:42")
            .expect("real daemon session row");

        assert!(ids.contains(&"tab:0:0".to_string()));
        assert!(ids.contains(&"session:42".to_string()));
        assert_eq!(session.label, "main-session");
        assert_eq!(session.dispatch.as_deref(), Some("terminal.focus:0:1"));
        assert!(session.enabled);
    }

    #[test]
    fn agents_mode_projects_real_quick_prompt_state_when_present() {
        let mut state = seed_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_with_agent(
            crate::quick_prompt::Agent::Codex,
        ));
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, "@ codex");
        let items = flatten(&results);

        assert_eq!(results.mode, PaletteMode::Agents);
        assert!(results.empty_state.is_none());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "agent:quick_prompt");
        assert_eq!(items[0].label, "Codex quick prompt");
        assert!(items[0].description.contains("real Quick Prompt state"));
        assert_eq!(items[0].status.as_deref(), Some("active"));
        assert!(!items[0].enabled);
        assert!(items[0].dispatch.is_none());
    }

    #[test]
    fn agents_mode_projects_real_agent_terminal_rows_when_detectable() {
        let mut state = seed_state();
        state.tabs[0].subtitle = "codex.cmd".to_string();
        state.panes[0][0].subtitle = "codex.cmd".to_string();
        state.workspaces[0].tabs = state.tabs.clone();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, "@ codex");
        let items = flatten(&results);

        assert_eq!(results.mode, PaletteMode::Agents);
        assert!(items.iter().any(|item| {
            item.id == "agent-terminal:0:1"
                && item.label == "Codex: shell"
                && item.dispatch.as_deref() == Some("terminal.focus:0:1")
                && item.enabled
        }));
    }

    #[test]
    fn scrollback_mode_does_not_show_fake_or_unbacked_rows() {
        let state = seed_state();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, "/ panic");
        let all_text = flatten(&results)
            .iter()
            .map(|item| format!("{} {} {}", item.id, item.label, item.description))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(results.groups.is_empty());
        assert!(results.empty_state.is_some());
        assert!(!all_text.contains("panic"));
        assert_eq!(
            results
                .empty_state
                .as_ref()
                .map(|empty| empty.title.as_str()),
            Some("No searchable scrollback")
        );
    }

    #[test]
    fn default_command_mode_keeps_navigation_and_agent_rows_out_without_fake_data() {
        let mut state = seed_state();
        state.tabs[0].subtitle = "claude".to_string();
        state.panes[0][0].subtitle = "claude".to_string();
        state.workspaces[0].tabs = state.tabs.clone();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, "");
        let ids = flattened_ids(&results);
        let all_text = flatten(&results)
            .iter()
            .map(|item| format!("{} {} {}", item.id, item.label, item.description))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(results.mode, PaletteMode::Actions);
        assert!(ids.contains(&"rename_current_terminal".to_string()));
        assert!(!ids.contains(&"workspace:0".to_string()));
        assert!(!ids.contains(&"terminal:0:1".to_string()));
        assert!(!ids.contains(&"agent-terminal:0:1".to_string()));
        for fake in ["dashboard-dev", "api.server", "refactor-userlist"] {
            assert!(!all_text.contains(fake), "prototype row leaked: {fake}");
        }
    }

    #[test]
    fn exact_phrase_query_ranks_matching_label_first() {
        let state = seed_state();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, "> open settings");
        let items = flatten(&results);

        assert_eq!(
            items.first().map(|item| item.id.as_str()),
            Some("open_settings")
        );
    }

    #[test]
    fn action_rows_use_effective_keybind_overrides_for_search_metadata() {
        let mut state = seed_state();
        state
            .keybinds
            .set(
                KeybindAction::OpenSettings,
                unshit::core::shortcut::KeyCombo::parse("Alt+O").unwrap(),
            )
            .unwrap();
        let snap = state.ui_snapshot();

        let results = build_palette_results(&snap, "> Alt+O");
        let items = flatten(&results);

        assert_eq!(
            items.first().map(|item| item.id.as_str()),
            Some("open_settings")
        );
        assert_eq!(
            items.first().and_then(|item| item.shortcut.as_deref()),
            Some("Alt+O")
        );
    }
}
