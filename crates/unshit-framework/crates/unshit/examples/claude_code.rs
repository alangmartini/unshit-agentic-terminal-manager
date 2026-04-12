use unshit::app::{App, AppConfig};
use unshit::core::element::*;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_hal=error,wgpu_core=error,naga=error"),
    )
    .init();

    let css = r#"
        .root {
            display: flex;
            flex-direction: row;
            width: 100%;
            height: 100%;
            background: rgba(13, 17, 23, 0.95);
        }

        .sidebar {
            display: flex;
            flex-direction: column;
            width: 220px;
            flex-shrink: 0;
            background: rgba(11, 15, 20, 0.85);
        }

        .sidebar-header {
            display: flex;
            align-items: center;
            padding: 14px 16px;
            gap: 8px;
        }

        .sidebar-title {
            color: #e6edf3;
            font-size: 14px;
            font-weight: bold;
        }

        .sidebar-count {
            color: #484f58;
            font-size: 13px;
        }

        .sidebar-bolt {
            color: #fbbf24;
            font-size: 13px;
        }

        .session-item {
            display: flex;
            flex-direction: column;
            padding: 10px 16px;
            gap: 2px;
        }

        .session-active {
            display: flex;
            flex-direction: column;
            padding: 10px 16px;
            gap: 2px;
            background: rgba(16, 185, 129, 0.12);
        }

        .session-row {
            display: flex;
            align-items: center;
            gap: 10px;
        }

        .session-number {
            color: #484f58;
            font-size: 13px;
        }

        .session-name {
            color: #8b949e;
            font-size: 14px;
        }

        .session-name-hl {
            color: #10b981;
            font-size: 14px;
            font-weight: bold;
        }

        .session-branch {
            color: #484f58;
            font-size: 12px;
            padding: 0px 0px 0px 24px;
        }

        .session-branch-hl {
            color: #34d399;
            font-size: 12px;
            padding: 0px 0px 0px 24px;
        }

        .divider {
            width: 1px;
            background: rgba(16, 185, 129, 0.15);
            flex-shrink: 0;
        }

        .main {
            display: flex;
            flex-direction: column;
            flex-grow: 1;
            padding: 20px 28px;
            gap: 4px;
        }

        .text-body {
            color: #e6edf3;
            font-size: 14px;
            line-height: 1.5;
        }

        .text-indent1 {
            color: #e6edf3;
            font-size: 14px;
            line-height: 1.5;
            padding: 0px 0px 0px 20px;
        }

        .text-indent2 {
            color: #e6edf3;
            font-size: 14px;
            line-height: 1.5;
            padding: 0px 0px 0px 44px;
        }

        .heading {
            color: #fbbf24;
            font-size: 16px;
            font-weight: bold;
            padding: 10px 0px 4px 0px;
        }

        .inline-row {
            display: flex;
            align-items: center;
            padding: 0px 0px 0px 20px;
        }

        .code-badge {
            display: flex;
            align-items: center;
            padding: 1px 6px;
            background: rgba(16, 185, 129, 0.1);
            border-radius: 4px;
            color: #34d399;
            font-size: 14px;
        }

        .link {
            color: #58a6ff;
            font-size: 14px;
        }

        .blockquote-row {
            display: flex;
            padding: 4px 0px 4px 20px;
        }

        .bq-bar-green {
            width: 3px;
            flex-shrink: 0;
            background: #34d399;
            border-radius: 2px;
        }

        .bq-bar-yellow {
            width: 3px;
            flex-shrink: 0;
            background: #fbbf24;
            border-radius: 2px;
        }

        .bq-text-green {
            color: #34d399;
            font-size: 14px;
            padding: 4px 12px;
            background: rgba(52, 211, 153, 0.08);
        }

        .bq-text-yellow {
            color: #fbbf24;
            font-size: 14px;
            padding: 4px 12px;
            background: rgba(251, 191, 36, 0.06);
        }

        .thoughts-row {
            display: flex;
            align-items: center;
            gap: 6px;
            padding: 8px 0px 2px 0px;
        }

        .check-text {
            color: #34d399;
            font-size: 14px;
        }

        .muted {
            color: #484f58;
            font-size: 13px;
        }

        .thought-item {
            display: flex;
            align-items: center;
            gap: 6px;
            padding: 1px 0px 1px 28px;
        }

        .thought-dot {
            width: 4px;
            height: 4px;
            border-radius: 2px;
            background: #484f58;
            flex-shrink: 0;
        }

        .cmd-row {
            display: flex;
            align-items: center;
            gap: 6px;
            padding: 4px 0px;
        }

        .cmd-dollar {
            color: #484f58;
            font-size: 14px;
        }

        .cmd-text {
            color: #8b949e;
            font-size: 14px;
        }

        .numbered-row {
            display: flex;
            gap: 6px;
            padding: 2px 0px 2px 20px;
        }

        .num-label {
            color: #8b949e;
            font-size: 14px;
            flex-shrink: 0;
        }

        .session-item {
            cursor: pointer;
        }
        .session-item:hover {
            background: rgba(16, 185, 129, 0.06);
        }

        .link {
            cursor: pointer;
        }
        .link:hover {
            color: #79b8ff;
        }

        .code-badge:hover {
            background: rgba(16, 185, 129, 0.18);
            color: #6ee7b7;
        }
    "#;

    let app = App::new(
        AppConfig {
            title: "Claude Code UI Demo".to_string(),
            width: 1100,
            height: 750,
            css: css.to_string(),
            ..Default::default()
        },
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(sidebar())
                .with_child(ElementDef::new(Tag::Div).with_class("divider"))
                .with_child(main_content()),
        },
    );

    app.run();
}

fn sidebar() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("sidebar-header")
                .with_child(
                    ElementDef::new(Tag::Span).with_class("sidebar-title").with_text("Sessions"),
                )
                .with_child(ElementDef::new(Tag::Span).with_class("sidebar-count").with_text("3"))
                .with_child(ElementDef::new(Tag::Span).with_class("sidebar-bolt").with_text("*"))
                .with_child(ElementDef::new(Tag::Span).with_class("sidebar-count").with_text("1")),
        )
        .with_child(session("1", "plane", "fix/pdf-export...", false))
        .with_child(session("2", "opensessions", "main", true))
        .with_child(session("3", "quiver", "main", false))
}

fn session(num: &str, name: &str, branch: &str, active: bool) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class(if active { "session-active" } else { "session-item" })
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("session-row")
                .with_child(ElementDef::new(Tag::Span).with_class("session-number").with_text(num))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class(if active { "session-name-hl" } else { "session-name" })
                        .with_text(name),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class(if active { "session-branch-hl" } else { "session-branch" })
                .with_text(branch),
        )
}

fn main_content() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("main")
        .with_child(text_indent1(
            "* Tightened the support story so the README stays clearly tmux-first while still acknowledging the experimental zellij code in-repo.",
        ))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("heading")
                .with_text("Verification Notes"),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("inline-row")
                .with_child(text("* The rewritten "))
                .with_child(code("README.md"))
                .with_child(text(" now reflects the actual sidebar surface:")),
        )
        .with_child(text_indent2("* session list shows branch and agent status"))
        .with_child(text_indent2(
            "* detail panel shows working directory, ports, agent rows, and thread names",
        ))
        .with_child(text_indent2(
            "* users can hide/restore/kill/reorder sessions and switch themes",
        ))
        .with_child(text_indent2("* detected localhost ports open from the UI"))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("inline-row")
                .with_child(text("* I also verified the smoke-test path directly: "))
                .with_child(code("bun test"))
                .with_child(text(" from the repo root")),
        )
        .with_child(text_indent2("passes right now with 167 passing tests."))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("heading")
                .with_text("One Residual Note"),
        )
        .with_child(text_body(
            "I only changed the README. While auditing, I noticed some secondary docs still have minor drift from the current UI behavior, especially the feature/keybinding docs.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("numbered-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("num-label")
                        .with_text("1."),
                )
                .with_child(text("If you want, I can do the same line-by-line cleanup pass on")),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("inline-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("link")
                        .with_text("docs/reference/features-and-keybindings.md"),
                )
                .with_child(text(" and the rest of "))
                .with_child(code("docs/"))
                .with_child(text(" next.")),
        )
        .with_child(blockquote("also make it a bit more fun! (interrupted)", true))
        .with_child(blockquote("and compact!", false))
        .with_child(thoughts_section(
            &["Editing README for clarity", "Streamlining README structure"],
        ))
        .with_child(text_body(
            "I'm tightening the README again, this time for tone and density: shorter sections, less repetition, and a little more personality without getting cute enough to obscure what the project does.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("cmd-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("cmd-dollar")
                        .with_text("$"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("cmd-text")
                        .with_text("rtk proxy nl -ba README.md | sed -n '1,220p'"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("muted")
                        .with_text(">"),
                ),
        )
        .with_child(thoughts_section(
            &["Condensing the file", "Simplifying the README"],
        ))
        .with_child(text_body(
            "I'm collapsing the README structure now: fewer headings, less /verification",
        ))
}

fn text(s: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("text-body").with_text(s)
}

fn text_body(s: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("text-body").with_text(s)
}

fn text_indent1(s: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("text-indent1").with_text(s)
}

fn text_indent2(s: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("text-indent2").with_text(s)
}

fn code(s: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("code-badge").with_text(s)
}

fn blockquote(s: &str, green: bool) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("blockquote-row")
        .with_child(ElementDef::new(Tag::Div).with_class(if green {
            "bq-bar-green"
        } else {
            "bq-bar-yellow"
        }))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class(if green { "bq-text-green" } else { "bq-text-yellow" })
                .with_text(s),
        )
}

fn thoughts_section(items: &[&str]) -> ElementDef {
    let mut container = ElementDef::new(Tag::Div).with_class("main").with_child(
        ElementDef::new(Tag::Div)
            .with_class("thoughts-row")
            .with_child(ElementDef::new(Tag::Span).with_class("check-text").with_text("v"))
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("muted")
                    .with_text(format!("{} thoughts", items.len())),
            )
            .with_child(ElementDef::new(Tag::Span).with_class("muted").with_text("v")),
    );

    for item in items {
        container = container.with_child(
            ElementDef::new(Tag::Div)
                .with_class("thought-item")
                .with_child(ElementDef::new(Tag::Div).with_class("thought-dot"))
                .with_child(ElementDef::new(Tag::Span).with_class("muted").with_text(*item))
                .with_child(ElementDef::new(Tag::Span).with_class("muted").with_text(">")),
        );
    }

    container
}
