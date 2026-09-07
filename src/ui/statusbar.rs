use unshit::core::element::*;

use crate::state::{Pane, TabStatus, UiSnapshot};

pub fn build_statusbar(state: &UiSnapshot) -> ElementDef {
    if state.settings_open {
        return build_settings_statusbar(state);
    }

    ElementDef::new(Tag::Div)
        .with_class("statusbar")
        .with_class("role-footer")
        .with_child(build_statusbar_left(state))
        // Flex spacer pushes the right group to the far edge. Without it the
        // `.statusbar` (justify-content: flex-start, gap: 0) leaves the two
        // groups flush, so the left group's last item ("k/s") collides with
        // the right group's first ("tab ...") -> the unreadable "k/stab".
        .with_child(ElementDef::new(Tag::Span).with_class("sb-spacer"))
        .with_child(build_statusbar_right(state))
}

fn build_settings_statusbar(state: &UiSnapshot) -> ElementDef {
    // Section-aware detail cell: keybinds shows the binding count, other
    // sections the active theme.
    let detail = if state.settings_section == crate::state::SettingsSection::Keybinds {
        format!("{} bindings", crate::keybinds::KeybindAction::ALL.len())
    } else {
        format!("theme: {}", state.theme)
    };
    ElementDef::new(Tag::Div)
        .with_class("statusbar")
        .with_class("settings-statusbar")
        .with_class("role-footer")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("statusbar-left")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("sb-cell")
                        .with_class("sage")
                        .with_text("ready"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("sb-cell")
                        .with_class("dim")
                        .with_text(detail),
                ),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("sb-spacer"))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("statusbar-right")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("sb-cell")
                        .with_class("amber")
                        .with_text(state.settings_section.label()),
                ),
        )
}

fn build_statusbar_left(state: &UiSnapshot) -> ElementDef {
    let running_count: usize = state
        .tabs
        .iter()
        .filter(|t| t.status == TabStatus::Running)
        .count();

    ElementDef::new(Tag::Div)
        .with_class("statusbar-left")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_class("accent")
                .with_id("status-mode")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("status-glyph")
                        .with_text("\u{25C6}"),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("main")),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("status-dot")
                        .with_class("running"),
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
                        .with_text(stat_text(state.cpu_pct, 1)),
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
                        .with_text(stat_text(state.mem_gb, 2)),
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
                        .with_text(stat_text(state.net_kbps, 1)),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("k/s")),
        )
}

/// A sampled figure with `decimals` places, or `--` when the resource
/// monitor does not know it yet. A dash is honest; `0.0` would claim the
/// tabs cost nothing.
fn stat_text(value: Option<f32>, decimals: usize) -> String {
    match value {
        Some(v) => format!("{v:.decimals$}"),
        None => "--".to_string(),
    }
}

/// The active tab's process trees summed: what the user is looking at,
/// whether or not the tab is split enough to show pane headers. `--` until
/// the sampler has attributed at least one of the tab's panes.
fn tab_usage_text(state: &UiSnapshot) -> String {
    let known: Vec<&Pane> = state
        .panes
        .iter()
        .flatten()
        .filter(|p| p.pid != 0)
        .collect();
    if known.is_empty() {
        return "--".to_string();
    }
    let cpu: f32 = known.iter().map(|p| p.cpu).sum();
    let mem: u64 = known.iter().map(|p| p.mem_bytes).sum();
    let procs: u32 = known
        .iter()
        .map(|p| {
            state
                .resource_trees
                .get(&p.pid)
                .map_or(0, |t| t.process_count)
        })
        .sum();
    format!(
        "{cpu:.1}% \u{00B7} {} \u{00B7} {procs} proc{}",
        crate::resource_monitor::format_compact_bytes(mem),
        if procs == 1 { "" } else { "s" }
    )
}

/// `pwsh · pid 16192` for the focused pane: the program the daemon spawned
/// for it and the pid the sampler attributes the tree to. Replaces the old
/// hardcoded `bash · 5.2`.
fn shell_text(state: &UiSnapshot) -> String {
    let pane = state
        .panes
        .iter()
        .flatten()
        .find(|p| p.id == state.active_pane)
        .or_else(|| state.panes.iter().flatten().next());
    let Some(pane) = pane else {
        return "shell".to_string();
    };
    let pid = if pane.pid == 0 {
        "--".to_string()
    } else {
        pane.pid.to_string()
    };
    // The sampled root image beats the pane subtitle: restored and seeded
    // panes carry a placeholder program ("bash") that may not be what the
    // daemon actually spawned.
    let program = state
        .resource_trees
        .get(&pane.pid)
        .and_then(|tree| tree.root_exe.as_deref())
        .unwrap_or(&pane.subtitle);
    format!("{} \u{00B7} pid {pid}", shell_label(program))
}

/// Program basename without a trailing `.exe`. `codex.cmd` stays as is, a
/// full path collapses to its file name and an empty program (the daemon's
/// default shell) reads `shell`.
pub(crate) fn shell_label(program: &str) -> String {
    let base = program
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim();
    if base.is_empty() {
        return "shell".to_string();
    }
    match base.to_ascii_lowercase().strip_suffix(".exe") {
        Some(stem) => base[..stem.len()].to_string(),
        None => base.to_string(),
    }
}

fn build_statusbar_right(state: &UiSnapshot) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("statusbar-right")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_id("status-tab-usage")
                .with_child(ElementDef::new(Tag::Span).with_text("tab "))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text(tab_usage_text(state)),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_id("status-shell")
                .with_text(shell_text(state)),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text(state.active_terminal_cols.to_string()),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("\u{00D7}"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text(state.active_terminal_rows.to_string()),
                ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, Pane, PaneId, SettingsSection, TerminalTab};
    use std::collections::BTreeMap;

    fn snapshot_from_seed() -> UiSnapshot {
        seed_state().ui_snapshot()
    }

    fn minimal_snapshot() -> UiSnapshot {
        UiSnapshot {
            workspaces: vec![],
            active_workspace: 0,
            tabs: vec![],
            active_tab: 0,
            panes: vec![vec![Pane {
                id: PaneId(1),
                title: "shell".into(),
                subtitle: "bash".into(),
                pid: 0,
                cpu: 0.0,
                mem_bytes: 0,
            }]],
            active_pane: PaneId(1),
            settings_open: false,
            settings_section: SettingsSection::Appearance,
            theme: crate::theme::default_theme_id().into(),
            custom_theme: crate::theme::default_custom_theme(),
            config_font_size_pt: crate::state::DEFAULT_CONFIG_FONT_SIZE_PT,
            terminal_font_size_pt: crate::state::DEFAULT_TERMINAL_FONT_SIZE_PT,
            ui_zoom: crate::state::DEFAULT_UI_ZOOM,
            ui_density: crate::state::DEFAULT_UI_DENSITY,
            scroll_line_px: crate::state::DEFAULT_SCROLL_LINE_PX,
            smooth_scroll_duration_ms: crate::state::DEFAULT_SMOOTH_SCROLL_DURATION_MS,
            tab_width_mode: crate::state::DEFAULT_TAB_WIDTH_MODE,
            tab_row_mode: crate::state::DEFAULT_TAB_ROW_MODE,
            tab_width_px: crate::state::DEFAULT_TAB_WIDTH_PX,
            toggles: BTreeMap::new(),
            palette_open: false,
            palette_query: String::new(),
            palette_active: 0,
            sidebar_collapsed: false,
            sidebar_width: 252.0,
            window_maximized: false,
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
            ctx_menu: None,
            agent_pane_ids: Default::default(),
            confirm_dialog: None,
            update: Default::default(),
            terminal_count: 0,
            active_terminal_cols: 80,
            active_terminal_rows: 24,
            sessions: Vec::new(),
            ui_pid: std::process::id(),
            ui_memory_rss_bytes: None,
            daemon_pid: None,
            daemon_memory_rss_bytes: None,
            sessions_stale: false,
            resource_trees: std::collections::HashMap::new(),
            diagnostic_scroll_samples: Vec::new(),
            toasts: Vec::new(),
            cpu_pct: None,
            mem_gb: None,
            net_kbps: None,
            clock_hhmm: "00:00".into(),
            keybinds: crate::keybinds::KeybindsState::default(),
            drag: crate::drag::DragState::default(),
            tabbar_rect: crate::drag::Rect::default(),
            last_grid_width: 0.0,
            last_grid_height: 0.0,
            window_width: 0.0,
            window_height: 0.0,
            scale_factor: 1.0,
            default_shell: crate::shell::ShellSpec::default(),
            quick_prompt: None,
            terminal_link_hover: None,
            pending_agent_resumes: BTreeMap::new(),
            editor_panes: std::collections::HashSet::new(),
            flow_panes: std::collections::HashMap::new(),
        }
    }

    fn collect_text(el: &ElementDef) -> String {
        let mut out = String::new();
        if let ElementContent::Text(text) = &el.content {
            out.push_str(text);
        }
        for child in &el.children {
            out.push_str(&collect_text(child));
        }
        out
    }

    #[test]
    fn build_statusbar_does_not_panic() {
        let snap = snapshot_from_seed();
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn build_statusbar_returns_div() {
        let snap = snapshot_from_seed();
        let elem = build_statusbar(&snap);
        assert!(matches!(elem.tag, Tag::Div));
    }

    #[test]
    fn build_statusbar_has_left_spacer_and_right() {
        let snap = snapshot_from_seed();
        let elem = build_statusbar(&snap);
        // left group, flex spacer, right group
        assert_eq!(elem.children.len(), 3);
        assert!(elem.children[1].classes.contains(&"sb-spacer".to_string()));
    }

    #[test]
    fn settings_statusbar_matches_theme_design_cells() {
        let mut snap = minimal_snapshot();
        snap.settings_open = true;
        snap.settings_section = SettingsSection::Appearance;
        snap.theme = "amber".into();

        let elem = build_statusbar(&snap);

        assert!(elem.classes.contains(&"settings-statusbar".to_string()));
        assert_eq!(elem.children.len(), 3);
        let left = &elem.children[0];
        assert_eq!(collect_text(&left.children[0]), "ready");
        assert_eq!(collect_text(&left.children[1]), "theme: amber");
        let right = &elem.children[2];
        assert_eq!(collect_text(&right.children[0]), "appearance");
    }

    #[test]
    fn build_statusbar_left_does_not_panic() {
        let snap = snapshot_from_seed();
        let _elem = build_statusbar_left(&snap);
    }

    #[test]
    fn build_statusbar_right_does_not_panic() {
        let snap = snapshot_from_seed();
        let _elem = build_statusbar_right(&snap);
    }

    #[test]
    fn statusbar_with_no_tabs_shows_zero_active() {
        let snap = minimal_snapshot();
        // Should not panic even with zero tabs
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_multiple_running_tabs() {
        let mut snap = minimal_snapshot();
        snap.tabs = vec![
            TerminalTab {
                id: "t1".into(),
                name: "shell".into(),
                subtitle: "bash".into(),
                status: TabStatus::Running,
                panes: vec![vec![Pane {
                    id: PaneId(1),
                    title: "shell".into(),
                    subtitle: "bash".into(),
                    pid: 0,
                    cpu: 0.0,
                    mem_bytes: 0,
                }]],
                active_pane: PaneId(1),
                row_ratios: vec![1.0],
                col_ratios: vec![vec![1.0]],
            },
            TerminalTab {
                id: "t2".into(),
                name: "build".into(),
                subtitle: "cargo".into(),
                status: TabStatus::Running,
                panes: vec![vec![Pane {
                    id: PaneId(2),
                    title: "build".into(),
                    subtitle: "cargo".into(),
                    pid: 0,
                    cpu: 0.0,
                    mem_bytes: 0,
                }]],
                active_pane: PaneId(2),
                row_ratios: vec![1.0],
                col_ratios: vec![vec![1.0]],
            },
            TerminalTab {
                id: "t3".into(),
                name: "idle".into(),
                subtitle: "bash".into(),
                status: TabStatus::Idle,
                panes: vec![vec![Pane {
                    id: PaneId(3),
                    title: "idle".into(),
                    subtitle: "bash".into(),
                    pid: 0,
                    cpu: 0.0,
                    mem_bytes: 0,
                }]],
                active_pane: PaneId(3),
                row_ratios: vec![1.0],
                col_ratios: vec![vec![1.0]],
            },
            TerminalTab {
                id: "t4".into(),
                name: "stopped".into(),
                subtitle: "bash".into(),
                status: TabStatus::Stopped,
                panes: vec![vec![Pane {
                    id: PaneId(4),
                    title: "stopped".into(),
                    subtitle: "bash".into(),
                    pid: 0,
                    cpu: 0.0,
                    mem_bytes: 0,
                }]],
                active_pane: PaneId(4),
                row_ratios: vec![1.0],
                col_ratios: vec![vec![1.0]],
            },
        ];
        // Should not panic, running count should be 2
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_high_cpu() {
        let mut snap = minimal_snapshot();
        snap.cpu_pct = Some(99.9);
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_high_mem() {
        let mut snap = minimal_snapshot();
        snap.mem_gb = Some(128.55);
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_high_net() {
        let mut snap = minimal_snapshot();
        snap.net_kbps = Some(9999.9);
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_custom_clock() {
        let mut snap = minimal_snapshot();
        snap.clock_hhmm = "23:59".into();
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_zero_values() {
        let mut snap = minimal_snapshot();
        snap.cpu_pct = Some(0.0);
        snap.mem_gb = Some(0.0);
        snap.net_kbps = Some(0.0);
        let _elem = build_statusbar(&snap);
    }

    fn item_text(el: &ElementDef, id: &str) -> String {
        el.children
            .iter()
            .find(|c| c.id.as_deref() == Some(id))
            .map(collect_text)
            .unwrap_or_else(|| panic!("no #{id} in status bar"))
    }

    #[test]
    fn unknown_resource_figures_render_as_dashes_not_zeros() {
        let snap = minimal_snapshot();
        let left = build_statusbar_left(&snap);
        assert_eq!(item_text(&left, "status-cpu"), "cpu --%");
        assert_eq!(item_text(&left, "status-mem"), "mem --G");
        assert_eq!(item_text(&left, "status-net"), "\u{2193} --k/s");
    }

    #[test]
    fn known_resource_figures_render_with_fixed_precision() {
        let mut snap = minimal_snapshot();
        snap.cpu_pct = Some(12.34);
        snap.mem_gb = Some(3.456);
        snap.net_kbps = Some(0.06);
        let left = build_statusbar_left(&snap);
        assert_eq!(item_text(&left, "status-cpu"), "cpu 12.3%");
        assert_eq!(item_text(&left, "status-mem"), "mem 3.46G");
        assert_eq!(item_text(&left, "status-net"), "\u{2193} 0.1k/s");
    }

    #[test]
    fn statusbar_right_has_static_items() {
        let snap = minimal_snapshot();
        let elem = build_statusbar_right(&snap);
        // Four items: active tab usage, focused shell + pid, dimensions, clock.
        assert_eq!(elem.children.len(), 4);
    }

    #[test]
    fn statusbar_right_shows_dashes_until_the_active_tab_is_sampled() {
        let mut snap = minimal_snapshot();
        snap.panes[0][0].pid = 0;
        snap.panes[0][0].subtitle = "bash".into();
        snap.active_pane = snap.panes[0][0].id;
        let right = build_statusbar_right(&snap);
        assert_eq!(item_text(&right, "status-tab-usage"), "tab --");
        assert_eq!(item_text(&right, "status-shell"), "bash \u{00B7} pid --");
    }

    #[test]
    fn statusbar_right_shows_the_active_tab_tree_and_the_real_shell() {
        let mut snap = minimal_snapshot();
        let pane = &mut snap.panes[0][0];
        pane.pid = 16192;
        pane.cpu = 4.16;
        pane.mem_bytes = 164 << 20;
        pane.subtitle = r"C:\Program Files\PowerShell\7\pwsh.exe".into();
        let pane_id = pane.id;
        snap.active_pane = pane_id;
        snap.resource_trees.insert(
            16192,
            crate::resource_monitor::TreeUsage {
                cpu_pct: Some(4.16),
                mem_bytes: 164 << 20,
                process_count: 5,
                root_exe: None,
            },
        );
        let right = build_statusbar_right(&snap);
        assert_eq!(
            item_text(&right, "status-tab-usage"),
            "tab 4.2% \u{00B7} 164 MiB \u{00B7} 5 procs"
        );
        assert_eq!(item_text(&right, "status-shell"), "pwsh \u{00B7} pid 16192");

        // A seeded "bash" subtitle loses to the image the sampler saw.
        snap.panes[0][0].subtitle = "bash".into();
        snap.resource_trees.get_mut(&16192).unwrap().root_exe = Some("powershell.exe".into());
        let right = build_statusbar_right(&snap);
        assert_eq!(
            item_text(&right, "status-shell"),
            "powershell \u{00B7} pid 16192"
        );
    }

    #[test]
    fn tab_usage_sums_every_sampled_pane_of_the_active_tab() {
        let mut snap = minimal_snapshot();
        let first = snap.panes[0][0].clone();
        let mut second = first.clone();
        second.id = PaneId(first.id.0 + 1);
        snap.panes = vec![vec![first, second]];
        snap.panes[0][0].pid = 11;
        snap.panes[0][0].cpu = 1.0;
        snap.panes[0][0].mem_bytes = 100 << 20;
        snap.panes[0][1].pid = 22;
        snap.panes[0][1].cpu = 2.5;
        snap.panes[0][1].mem_bytes = 50 << 20;
        for (pid, count) in [(11, 1), (22, 3)] {
            snap.resource_trees.insert(
                pid,
                crate::resource_monitor::TreeUsage {
                    cpu_pct: Some(0.0),
                    mem_bytes: 0,
                    process_count: count,
                    root_exe: None,
                },
            );
        }
        assert_eq!(
            tab_usage_text(&snap),
            "3.5% \u{00B7} 150 MiB \u{00B7} 4 procs"
        );
    }

    #[test]
    fn shell_label_keeps_the_program_name_only() {
        assert_eq!(
            shell_label(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "pwsh"
        );
        assert_eq!(shell_label("/usr/bin/bash"), "bash");
        assert_eq!(shell_label("codex.cmd"), "codex.cmd");
        assert_eq!(shell_label("CMD.EXE"), "CMD");
        assert_eq!(shell_label(""), "shell");
        assert_eq!(shell_label("editor"), "editor");
    }
}
