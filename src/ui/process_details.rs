//! Live RAM breakdown for the active tab, using background sampler data.
use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{CssPosition, Dimension};

use crate::resource_monitor::{format_compact_bytes, ProcessUsage};
use crate::state::{dispatch, mutate_with, SharedState, UiSnapshot};

fn tab_processes(snap: &UiSnapshot) -> Vec<&ProcessUsage> {
    let mut rows: Vec<_> = snap
        .panes
        .iter()
        .flatten()
        .filter(|pane| pane.pid != 0)
        .filter_map(|pane| snap.resource_trees.get(&pane.pid))
        .flat_map(|tree| tree.processes.iter())
        .collect();
    rows.sort_unstable_by_key(|p| p.pid);
    rows.dedup_by_key(|p| p.pid);
    rows.sort_unstable_by_key(|p| (std::cmp::Reverse(p.mem_bytes), p.pid));
    rows
}

fn cell(text: impl Into<String>, class: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class(class).with_text(text)
}

pub fn build(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    if !snap.process_details_open {
        return ElementDef::new(Tag::Div).with_class("confirm-dialog-hidden");
    }
    let rows = tab_processes(snap);
    let total: u64 = rows.iter().map(|p| p.mem_bytes).sum();
    let mut list = ElementDef::new(Tag::Div).with_class("process-list");
    if rows.is_empty() {
        list = list.with_child(cell(
            "Process data unavailable. Waiting for a sample...",
            "tm-dialog-body",
        ));
    } else {
        for process in &rows {
            let share = if total == 0 {
                0.0
            } else {
                process.mem_bytes as f64 * 100.0 / total as f64
            };
            list = list.with_child(
                ElementDef::new(Tag::Div)
                    .with_class("process-row")
                    .with_key(format!("process-{}", process.pid))
                    .with_child(cell(&process.name, "process-name"))
                    .with_child(cell(process.pid.to_string(), "process-pid"))
                    .with_child(cell(format_compact_bytes(process.mem_bytes), "process-ram"))
                    .with_child(cell(format!("{share:.1}%"), "process-share")),
            );
        }
    }
    let close_shared = shared.clone();
    let card = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-simple-card")
        .with_class("process-dialog")
        .on_click(|| {})
        .with_child(cell("Tab processes", "confirm-dialog-title"))
        .with_child(cell(
            if rows.is_empty() {
                "RAM: --".into()
            } else {
                format!(
                    "{} processes · {} RAM",
                    rows.len(),
                    format_compact_bytes(total)
                )
            },
            "tm-dialog-body",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("process-row")
                .with_class("process-head")
                .with_child(cell("Name", "process-name"))
                .with_child(cell("PID", "process-pid"))
                .with_child(cell("RAM", "process-ram"))
                .with_child(cell("Share", "process-share")),
        )
        .with_child(list)
        .with_child(cell(
            "Working set · largest first · updates every second",
            "process-note",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-buttons")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("confirm-dialog-button")
                        .with_id("process-details-close")
                        .with_autofocus(true)
                        .with_text("Close")
                        .on_click(move || {
                            mutate_with(&close_shared, |state| {
                                dispatch(state, "processes.close");
                            });
                        }),
                ),
        );
    let shared = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-overlay")
        .with_id("process-details-overlay")
        .with_style(StyleDeclaration::Position(CssPosition::Fixed))
        .with_style(StyleDeclaration::Top(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Right(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Bottom(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Left(Dimension::Px(0.0)))
        .on_click(move || {
            mutate_with(&shared, |state| {
                dispatch(state, "processes.close");
            });
        })
        .with_child(card)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_monitor::TreeUsage;
    use crate::state::{seed_state, PaneId};

    #[test]
    fn breakdown_includes_split_panes_excludes_other_tabs_and_sorts_by_ram() {
        let mut state = seed_state();
        let mut pane = state.panes[0][0].clone();
        pane.pid = 10;
        state.panes = vec![vec![pane.clone()]];
        pane.id = PaneId(999);
        pane.pid = 20;
        state.panes[0].push(pane.clone());
        state.panes[0].push(pane); // Never double count a shared root.
        for (root, members) in [
            (10, vec![(10, 20), (11, 80)]),
            (20, vec![(20, 50)]),
            (30, vec![(30, 100)]),
        ] {
            state.resource_trees.insert(
                root,
                TreeUsage {
                    processes: members
                        .into_iter()
                        .map(|(pid, mem_bytes)| ProcessUsage {
                            pid,
                            name: format!("process-{pid}"),
                            mem_bytes,
                        })
                        .collect(),
                    ..Default::default()
                },
            );
        }
        let snap = state.ui_snapshot();
        let rows = tab_processes(&snap);
        assert_eq!(rows.iter().map(|p| p.pid).collect::<Vec<_>>(), [11, 20, 10]);
        assert_eq!(rows.iter().map(|p| p.mem_bytes).sum::<u64>(), 150);
        assert_eq!(rows[0].name, "process-11");
    }
}
