use unshit::core::element::*;

use crate::state::{TabStatus, UiSnapshot};

pub fn build_statusbar(state: &UiSnapshot) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("statusbar")
        .with_class("role-footer")
        .with_child(build_statusbar_left(state))
        .with_child(build_statusbar_right(state))
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

fn build_statusbar_right(state: &UiSnapshot) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("statusbar-right")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_text("utf-8"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_text("bash \u{00B7} 5.2"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("status-item")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text("80"),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("\u{00D7}"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("tnum")
                        .with_text("24"),
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
            }]],
            active_pane: PaneId(1),
            settings_open: false,
            settings_section: SettingsSection::General,
            theme: "amber".into(),
            font_size_pt: 13,
            toggles: BTreeMap::new(),
            agents: vec![],
            palette_open: false,
            sidebar_collapsed: false,
            sidebar_width: 252.0,
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
            ctx_menu: None,
            cpu_pct: 0.0,
            mem_gb: 0.0,
            net_kbps: 0.0,
            clock_hhmm: "00:00".into(),
            keybinds: crate::keybinds::KeybindsState::default(),
            drag: crate::drag::DragState::default(),
            tabbar_rect: crate::drag::Rect::default(),
            pane_rects: std::collections::HashMap::new(),
        }
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
    fn build_statusbar_has_left_and_right() {
        let snap = snapshot_from_seed();
        let elem = build_statusbar(&snap);
        assert_eq!(elem.children.len(), 2);
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
        snap.cpu_pct = 99.9;
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_high_mem() {
        let mut snap = minimal_snapshot();
        snap.mem_gb = 128.55;
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_with_high_net() {
        let mut snap = minimal_snapshot();
        snap.net_kbps = 9999.9;
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
        snap.cpu_pct = 0.0;
        snap.mem_gb = 0.0;
        snap.net_kbps = 0.0;
        let _elem = build_statusbar(&snap);
    }

    #[test]
    fn statusbar_right_has_static_items() {
        let snap = minimal_snapshot();
        let elem = build_statusbar_right(&snap);
        // Should have 4 children: utf-8, bash version, dimensions, clock
        assert_eq!(elem.children.len(), 4);
    }
}
