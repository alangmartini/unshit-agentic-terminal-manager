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
