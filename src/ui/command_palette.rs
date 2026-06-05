//! Render the command palette overlay.

use unshit::core::element::*;
use unshit::core::event::EventType;

use crate::command_palette::{
    build_palette_results, PaletteEmptyState, PaletteGroupView, PaletteIcon, PaletteItem,
    PaletteItemKind, PaletteMode, PaletteResults,
};
use crate::state::{dispatch, mutate_with, SharedState, UiSnapshot};
use crate::ui::icons::{
    icon_agent, icon_balance, icon_close, icon_folder, icon_fullscreen_corners, icon_grid,
    icon_magnifier, icon_plus, icon_settings, icon_sidebar_toggle, icon_split_h, icon_split_v,
    icon_terminal, svg_icon,
};

#[derive(Clone, Copy)]
struct ModeMeta {
    mode: PaletteMode,
    prefix: &'static str,
    label: &'static str,
    placeholder: &'static str,
    chip_class: &'static str,
}

const MODE_HINTS: &[ModeMeta] = &[
    ModeMeta {
        mode: PaletteMode::Actions,
        prefix: ">",
        label: "actions",
        placeholder: "run a command...",
        chip_class: "m-actions",
    },
    ModeMeta {
        mode: PaletteMode::Agents,
        prefix: "@",
        label: "agents",
        placeholder: "jump to an agent",
        chip_class: "m-agents",
    },
    ModeMeta {
        mode: PaletteMode::Navigation,
        prefix: ":",
        label: "navigate",
        placeholder: "workspace or terminal",
        chip_class: "m-nav",
    },
    ModeMeta {
        mode: PaletteMode::Scrollback,
        prefix: "/",
        label: "scrollback",
        placeholder: "search terminal output",
        chip_class: "m-search",
    },
];

pub fn build_command_palette_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    if !snap.palette_open {
        return ElementDef::new(Tag::Div).with_class("command-palette-hidden cp-hidden");
    }

    let results = build_palette_results(snap, &snap.palette_query);
    let backdrop_shared = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("cp-scrim")
        .with_id("command-palette-overlay")
        .on_click(move || {
            dispatch_shared(&backdrop_shared, "palette.close");
        })
        .with_child(build_palette_card(snap, shared, &results))
}

fn build_palette_card(
    snap: &UiSnapshot,
    shared: &SharedState,
    results: &PaletteResults,
) -> ElementDef {
    let mut card = ElementDef::new(Tag::Div)
        .with_class("cp compact")
        .on_click(|| {
            // Swallow clicks inside the card so backdrop close only handles
            // actual outside clicks.
        })
        .with_child(build_palette_input(snap, results.mode));

    if results.mode == PaletteMode::Unified && results.query.is_empty() {
        card = card.with_child(build_mode_hints(shared));
    }

    card.with_child(build_palette_body(snap, shared, results))
}

fn build_palette_input(snap: &UiSnapshot, mode: PaletteMode) -> ElementDef {
    let meta = mode_meta(mode);
    let query_class = if snap.palette_query.is_empty() {
        "cp-query-input placeholder"
    } else {
        "cp-query-input"
    };
    let query_text = if snap.palette_query.is_empty() {
        meta.placeholder.to_string()
    } else {
        snap.palette_query.clone()
    };
    let mut row = ElementDef::new(Tag::Div).with_class("cp-input").with_child(
        ElementDef::new(Tag::Span)
            .with_class("cp-prompt")
            .with_text("❯".to_string()),
    );

    if mode != PaletteMode::Unified
        && snap
            .palette_query
            .trim_start()
            .starts_with(mode_prefix(mode))
    {
        row = row.with_child(build_mode_chip(meta));
    }

    row.with_child(
        ElementDef::new(Tag::Div)
            .with_class(query_class)
            .with_text(query_text),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("cp-esc")
            .with_text("esc".to_string()),
    )
}

fn build_mode_chip(meta: ModeMeta) -> ElementDef {
    ElementDef::new(Tag::Span)
        .with_class(format!("cp-mode-chip {}", meta.chip_class))
        .with_child(mode_icon(meta.mode))
        .with_child(ElementDef::new(Tag::Span).with_text(meta.prefix.to_string()))
        .with_child(ElementDef::new(Tag::Span).with_text(meta.label.to_string()))
}

fn build_mode_hints(shared: &SharedState) -> ElementDef {
    let mut hints = ElementDef::new(Tag::Div).with_class("cp-modes");
    for meta in MODE_HINTS {
        let pill_shared = shared.clone();
        let command = format!("palette.query:{}", meta.prefix);
        hints = hints.with_child(
            ElementDef::new(Tag::Button)
                .with_class("cp-mode-pill")
                .on_click(move || {
                    dispatch_shared(&pill_shared, &command);
                })
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("pfx")
                        .with_text(meta.prefix.to_string()),
                )
                .with_child(ElementDef::new(Tag::Span).with_text(meta.label.to_string())),
        );
    }
    hints
}

fn build_palette_body(
    snap: &UiSnapshot,
    shared: &SharedState,
    results: &PaletteResults,
) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("cp-body")
        .with_child(build_result_list(snap, shared, results))
}

fn build_result_list(
    snap: &UiSnapshot,
    shared: &SharedState,
    results: &PaletteResults,
) -> ElementDef {
    let mut list = ElementDef::new(Tag::Div).with_class("cp-list");
    if let Some(empty) = &results.empty_state {
        return list.with_child(build_empty_state(empty));
    }

    let mut flat_idx = 0usize;
    for group in &results.groups {
        let group_start = flat_idx;
        list = list.with_child(build_group(group, snap, shared, &mut flat_idx));
        debug_assert_eq!(group_start + group.items.len(), flat_idx);
    }
    list
}

fn build_group(
    group: &PaletteGroupView,
    snap: &UiSnapshot,
    shared: &SharedState,
    flat_idx: &mut usize,
) -> ElementDef {
    let mut el = ElementDef::new(Tag::Div).with_class("cp-group").with_child(
        ElementDef::new(Tag::Div)
            .with_class("cp-group-title")
            .with_child(ElementDef::new(Tag::Span).with_text(group.title.clone()))
            .with_child(ElementDef::new(Tag::Span).with_class("rule"))
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("gcount")
                    .with_text(group.items.len().to_string()),
            ),
    );

    for item in &group.items {
        let idx = *flat_idx;
        el = el.with_child(build_row(item, idx, idx == snap.palette_active, shared));
        *flat_idx += 1;
    }
    el
}

fn build_row(item: &PaletteItem, index: usize, active: bool, shared: &SharedState) -> ElementDef {
    let row_shared = shared.clone();
    let hover_shared = shared.clone();
    let command = format!("palette.execute:{}", item.id);
    let hover_command = format!("palette.hover:{index}");
    let mut row = ElementDef::new(Tag::Div)
        .with_key(item.id.clone())
        .with_class("cp-item")
        .on(EventType::MouseMove, move |_| {
            dispatch_shared(&hover_shared, &hover_command);
            None
        })
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class(icon_class(item))
                .with_child(icon_for(item.icon)),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("cp-main")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("cp-label")
                        .with_text(item.label.clone()),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("cp-sub")
                        .with_text(item.description.clone()),
                ),
        )
        .with_child(build_right_cell(item));
    if active {
        row = row.with_class("active");
    }
    if item.enabled {
        row = row.on_click(move || {
            dispatch_shared(&row_shared, &command);
        });
    } else {
        row = row.with_class("disabled");
    }
    row
}

fn build_right_cell(item: &PaletteItem) -> ElementDef {
    let mut right = ElementDef::new(Tag::Div).with_class("cp-right");
    if let Some(status) = &item.status {
        return right.with_child(
            ElementDef::new(Tag::Span)
                .with_class(format!("cp-tag {}", status_class(status)))
                .with_text(status.clone()),
        );
    }
    if let Some(shortcut) = &item.shortcut {
        right = right.with_child(build_kbd_combo(shortcut));
    } else if let Some(dispatch) = &item.dispatch {
        right = right.with_child(
            ElementDef::new(Tag::Span)
                .with_class("cp-meta")
                .with_text(dispatch.clone()),
        );
    }
    right
}

fn build_kbd_combo(shortcut: &str) -> ElementDef {
    let mut combo = ElementDef::new(Tag::Span).with_class("cp-kbd");
    for part in shortcut.split('+') {
        let label = match part.trim() {
            "Shift" => "⇧",
            "Enter" => "↵",
            other => other,
        };
        combo = combo.with_child(
            ElementDef::new(Tag::Span)
                .with_class("kbd")
                .with_text(label.to_string()),
        );
    }
    combo
}

fn build_empty_state(empty: &PaletteEmptyState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("cp-empty")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("big")
                .with_text(empty.title.clone()),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("dim")
                .with_text(empty.message.clone()),
        )
}

fn dispatch_shared(shared: &SharedState, command: &str) {
    mutate_with(shared, |st| {
        dispatch(st, command);
    });
}

fn mode_meta(mode: PaletteMode) -> ModeMeta {
    MODE_HINTS
        .iter()
        .copied()
        .find(|meta| meta.mode == mode)
        .unwrap_or(ModeMeta {
            mode: PaletteMode::Unified,
            prefix: "",
            label: "everything",
            placeholder: "search sessions, terminals, and commands",
            chip_class: "",
        })
}

fn mode_prefix(mode: PaletteMode) -> &'static str {
    mode_meta(mode).prefix
}

fn mode_icon(mode: PaletteMode) -> ElementDef {
    match mode {
        PaletteMode::Unified => svg_icon(icon_magnifier()),
        PaletteMode::Actions => svg_icon(icon_settings()),
        PaletteMode::Agents => svg_icon(icon_agent()),
        PaletteMode::Navigation => svg_icon(icon_folder()),
        PaletteMode::Scrollback => svg_icon(icon_terminal()),
    }
}

fn icon_for(icon: PaletteIcon) -> ElementDef {
    match icon {
        PaletteIcon::Terminal | PaletteIcon::Session => svg_icon(icon_terminal()),
        PaletteIcon::SplitRight => svg_icon(icon_split_h()),
        PaletteIcon::SplitDown => svg_icon(icon_split_v()),
        PaletteIcon::Fullscreen => svg_icon(icon_fullscreen_corners()),
        PaletteIcon::Balance => svg_icon(icon_balance()),
        PaletteIcon::Grid => svg_icon(icon_grid()),
        PaletteIcon::Plus => svg_icon(icon_plus()),
        PaletteIcon::Close => svg_icon(icon_close()),
        PaletteIcon::Sidebar => svg_icon(icon_sidebar_toggle()),
        PaletteIcon::Settings => svg_icon(icon_settings()),
        PaletteIcon::Agent => svg_icon(icon_agent()),
        PaletteIcon::Workspace => svg_icon(icon_folder()),
        PaletteIcon::Tab => svg_icon(icon_grid()),
    }
}

fn icon_class(item: &PaletteItem) -> &'static str {
    if matches!(
        item.kind,
        PaletteItemKind::Terminal | PaletteItemKind::Session | PaletteItemKind::Agent
    ) {
        "cp-ic run"
    } else {
        "cp-ic"
    }
}

fn status_class(status: &str) -> &str {
    match status {
        "active" | "alive" | "running" => "running",
        "stopped" => "stopped",
        "error" => "error",
        "waiting" => "waiting",
        _ => "idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, ConfirmDialog};
    use std::sync::{Arc, Mutex};

    fn shared_with(state: crate::state::AppState) -> SharedState {
        Arc::new(Mutex::new(state))
    }

    #[test]
    fn hidden_when_palette_closed() {
        let state = seed_state();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);

        assert!(has_class(&el, "command-palette-hidden"));
        assert!(has_class(&el, "cp-hidden"));
    }

    #[test]
    fn renders_input_and_rename_command_when_open() {
        let mut state = seed_state();
        state.palette_open = true;
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);

        assert!(has_class(&el, "cp-scrim"));
        assert!(find_by_class(&el, "cp-query-input").is_some());
        assert!(text_anywhere(&el).contains("Rename current terminal"));
    }

    #[test]
    fn renders_handoff_shell_structure_when_open() {
        let mut state = seed_state();
        state.palette_open = true;
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);

        for class in [
            "cp-scrim", "cp", "compact", "cp-input", "cp-body", "cp-list",
        ] {
            assert!(find_by_class(&el, class).is_some(), "missing .{class}");
        }
        assert!(find_by_class(&el, "cp-preview").is_none());
        assert!(find_by_class(&el, "cp-foot").is_none());
        assert!(find_by_class(&el, "cp-mode-pill").is_none());
    }

    #[test]
    fn renders_group_titles_with_counts() {
        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = ">".to_string();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let text = text_anywhere(&el);

        assert!(find_by_class(&el, "cp-group-title").is_some());
        assert!(find_by_class(&el, "gcount").is_some());
        assert!(text.contains("commands"));
        assert!(text.contains("layout"));
    }

    #[test]
    fn active_row_class_follows_snapshot_active_index() {
        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = ">".to_string();
        state.palette_active = 2;
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let rows = all_by_class(&el, "cp-item");

        assert!(rows.len() > 2, "expected multiple palette rows");
        assert!(!has_class(rows[0], "active"));
        assert!(has_class(rows[2], "active"));
        assert_eq!(all_by_class(&el, "active").len(), 1);
    }

    #[test]
    fn row_mouse_move_updates_active_selection() {
        use unshit::core::event::{Event, Modifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = ">".to_string();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let rows = all_by_class(&el, "cp-item");
        let row = rows.get(3).expect("fourth palette row");
        let (_, handler) = row
            .handlers
            .iter()
            .find(|(event_type, _)| *event_type == EventType::MouseMove)
            .expect("row mouse move handler");

        handler(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Move,
            x: 0.0,
            y: 0.0,
            button: MouseButton::None,
            modifiers: Modifiers::empty(),
        }));

        assert_eq!(shared.lock().unwrap().palette_active, 3);
    }

    #[test]
    fn typed_mode_chip_and_honest_empty_state_render() {
        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = "@".to_string();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let text = text_anywhere(&el);

        assert!(find_by_class(&el, "cp-mode-chip").is_some());
        assert!(text.contains("@"));
        assert!(text.contains("agents"));
        assert!(text.contains("No agents available"));
        assert!(text.contains("No real agent metadata"));
    }

    #[test]
    fn query_display_reflects_snapshot_query() {
        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = "rename".to_string();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let query = find_by_class(&el, "cp-query-input").expect("palette query");

        assert_eq!(text_anywhere(query), "rename");
        assert!(!has_class(query, "placeholder"));
    }

    #[test]
    fn empty_query_display_uses_mode_placeholder() {
        let mut state = seed_state();
        state.palette_open = true;
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let query = find_by_class(&el, "cp-query-input").expect("palette query");

        assert!(has_class(query, "placeholder"));
        assert!(text_anywhere(query).contains("run a command"));
    }

    #[test]
    fn row_click_executes_rename_command() {
        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = "rename".to_string();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let row = find_by_class(&el, "cp-item").expect("palette row");

        (row.on_click.as_ref().unwrap())();

        let guard = shared.lock().unwrap();
        assert!(!guard.palette_open);
        assert!(matches!(
            guard.confirm_dialog,
            Some(ConfirmDialog::RenameSession { pane_id: 1, .. })
        ));
    }

    #[test]
    fn row_click_executes_row_via_dispatch() {
        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = "> open settings".to_string();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let row = find_by_class(&el, "cp-item").expect("palette row");

        (row.on_click.as_ref().unwrap())();

        let guard = shared.lock().unwrap();
        assert!(!guard.palette_open);
        assert!(guard.settings_open);
    }

    #[test]
    fn disabled_reference_rows_are_not_clickable() {
        let mut state = seed_state();
        state.palette_open = true;
        state.palette_query = "> balance".to_string();
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_command_palette_overlay(&snap, &shared);
        let row = find_by_class(&el, "cp-item").expect("metadata row");

        assert!(has_class(row, "disabled"));
        assert!(row.on_click.is_none());
    }

    #[test]
    fn handoff_css_locks_palette_layout_contract() {
        let styles = include_str!("../../assets/styles.css");
        let scrim = css_rule(styles, ".cp-scrim");
        let panel = css_rule(styles, ".cp");
        let active = css_rule(styles, ".cp-item.active");

        for required in [
            "position: fixed",
            "inset: 0",
            "align-items: flex-start",
            "justify-content: center",
            "padding-top: 12vh",
            "background: rgba(10, 8, 6, 0.62)",
            "backdrop-filter: blur(5px)",
        ] {
            assert!(scrim.contains(required), ".cp-scrim missing `{required}`");
        }

        for required in [
            "width: 760px",
            "max-width: calc(100vw - 48px)",
            "max-height: 72vh",
            "border-radius: var(--r-xl)",
        ] {
            assert!(panel.contains(required), ".cp missing `{required}`");
        }

        for required in [
            "background: var(--cp-accent-soft)",
            "border-left-color: var(--cp-accent)",
        ] {
            assert!(
                active.contains(required),
                ".cp-item.active missing `{required}`"
            );
        }
    }

    #[test]
    fn handoff_css_covers_palette_subsurfaces_and_narrow_viewports() {
        let styles = include_str!("../../assets/styles.css");

        for selector in [
            ".cp-mode-chip",
            ".cp-mode-chip.m-actions",
            ".cp-mode-chip.m-agents",
            ".cp-mode-chip.m-nav",
            ".cp-mode-chip.m-search",
            ".cp-mode-pill",
            ".cp-empty",
            ".cp-item.disabled",
            ".cp-preview",
            ".cp-pv-head",
            ".cp-pv-row",
            ".cp-pv-run",
            ".cp-pv-run.disabled",
            ".cp-foot",
            ".cp-foot .fh",
        ] {
            assert!(
                styles.contains(&format!("{selector} {{")),
                "missing dedicated CSS selector `{selector}`"
            );
        }

        for stale_selector in [
            ".command-palette-card",
            ".command-palette-input-row",
            ".command-palette-row",
            ".command-palette-empty",
        ] {
            assert!(
                !styles.contains(stale_selector),
                "stale pre-redesign selector `{stale_selector}` should not style palette UI"
            );
        }

        let preview = css_rule(styles, ".cp-preview");
        assert!(
            preview.contains("radial-gradient"),
            ".cp-preview should keep the handoff subtle top glow"
        );

        assert!(styles.contains("@media (max-width: 720px)"));
        assert!(styles.contains("width: calc(100vw - 24px)"));
        assert!(styles.contains("max-width: calc(100vw - 24px)"));
        assert!(styles.contains("flex-basis: 100%"));
        assert!(styles.contains("display: none"));
    }

    fn find_by_class<'a>(el: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if has_class(el, class) {
            return Some(el);
        }
        el.children.iter().find_map(|c| find_by_class(c, class))
    }

    fn has_class(el: &ElementDef, class: &str) -> bool {
        el.classes.iter().any(|c| c == class)
    }

    fn text_anywhere(el: &ElementDef) -> String {
        let mut out = String::new();
        if let ElementContent::Text(text) = &el.content {
            out.push_str(text);
        }
        for child in &el.children {
            out.push_str(&text_anywhere(child));
        }
        out
    }

    fn all_by_class<'a>(el: &'a ElementDef, class: &str) -> Vec<&'a ElementDef> {
        let mut out = Vec::new();
        collect_by_class(el, class, &mut out);
        out
    }

    fn collect_by_class<'a>(el: &'a ElementDef, class: &str, out: &mut Vec<&'a ElementDef>) {
        if has_class(el, class) {
            out.push(el);
        }
        for child in &el.children {
            collect_by_class(child, class, out);
        }
    }

    fn css_rule<'a>(styles: &'a str, selector: &str) -> &'a str {
        let needle = format!("{selector} {{");
        let start = styles
            .find(&needle)
            .unwrap_or_else(|| panic!("missing selector `{selector}`"));
        let body_start = start + needle.len();
        let body_end = styles[body_start..]
            .find('}')
            .map(|offset| body_start + offset)
            .expect("unterminated css rule");
        &styles[body_start..body_end]
    }
}
