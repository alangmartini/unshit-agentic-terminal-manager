use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use unshit::app::{App, AppConfig};
use unshit::core::element::*;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_hal=error,wgpu_core=error,naga=error"),
    )
    .init();

    // ---- Feature 6: Double-click + Right-click ----
    let click_count = Arc::new(AtomicU32::new(0));
    let right_clicked = Arc::new(AtomicBool::new(false));
    // ---- Feature 8: visibility toggle ----
    let hidden_toggle = Arc::new(AtomicBool::new(false));

    let cc_click = click_count.clone();
    let rc_click = right_clicked.clone();
    let ht_click = hidden_toggle.clone();

    let cc_read = click_count.clone();
    let rc_read = right_clicked.clone();
    let ht_read = hidden_toggle.clone();

    let cc_ctx = click_count.clone();
    let rc_ctx = right_clicked.clone();

    // ---- Feature 5: CSS Custom Properties (variables) ----
    let css = r#"
        :root {
            --bg-dark: rgba(13, 17, 23, 0.95);
            --bg-card: rgba(22, 27, 34, 0.95);
            --bg-card-hover: rgba(30, 37, 46, 0.95);
            --accent: #10b981;
            --accent-light: #34d399;
            --accent-glow: rgba(16, 185, 129, 0.15);
            --accent-glow-strong: rgba(16, 185, 129, 0.25);
            --text-primary: #e6edf3;
            --text-secondary: #8b949e;
            --text-muted: #484f58;
            --warning: #fbbf24;
            --error: #f87171;
            --info: #60a5fa;
            --border-subtle: rgba(255, 255, 255, 0.06);
            --border-accent: rgba(16, 185, 129, 0.2);
            --radius-sm: 6px;
            --radius-md: 12px;
            --radius-lg: 20px;
        }

        /* ---- Layout shell ---- */
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: var(--bg-dark);
            padding: 20px 24px;
            gap: 14px;
        }

        .header {
            display: flex;
            align-items: center;
            gap: 16px;
            padding: 0px 4px;
        }

        .title {
            color: var(--text-primary);
            font-size: 20px;
            font-weight: bold;
        }

        .subtitle {
            color: var(--text-secondary);
            font-size: 14px;
        }

        .version-badge {
            display: flex;
            align-items: center;
            padding: 3px 10px;
            background: var(--accent-glow);
            border-radius: var(--radius-sm);
            color: var(--accent-light);
            font-size: 12px;
            font-weight: bold;
        }

        /* ---- Vertical card list ---- */
        .card-grid {
            display: flex;
            flex-direction: column;
            gap: 12px;
            flex-grow: 1;
            overflow: scroll;
        }

        .card {
            display: flex;
            flex-direction: column;
            width: 100%;
            background: var(--bg-card);
            border-radius: var(--radius-md);
            border-width: 1px;
            border-color: var(--border-subtle);
            padding: 16px;
            gap: 10px;
            cursor: pointer;
        }

        .card:hover {
            background: var(--bg-card-hover);
            border-color: var(--border-accent);
        }

        /* ---- Feature 11: :focus + outline ---- */
        .card:focus {
            outline-color: var(--accent);
            outline-width: 2px;
            outline-offset: 2px;
        }

        .card-title-row {
            display: flex;
            align-items: center;
            gap: 10px;
        }

        .card-number {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 24px;
            height: 24px;
            background: var(--accent-glow);
            border-radius: 12px;
            color: var(--accent-light);
            font-size: 12px;
            font-weight: bold;
        }

        .card-label {
            color: var(--text-primary);
            font-size: 15px;
            font-weight: bold;
        }

        .card-desc {
            color: var(--text-secondary);
            font-size: 12px;
            line-height: 1.4;
        }

        /* ---- Feature 2: :first-child / :last-child / :nth-child ---- */
        .card:first-child {
            border-color: var(--accent-glow-strong);
        }

        .card:last-child {
            border-color: rgba(248, 113, 113, 0.2);
        }

        .card:nth-child(3) {
            border-color: rgba(251, 191, 36, 0.2);
        }

        /* ---- Feature 3: text-decoration ---- */
        .underlined {
            color: var(--accent-light);
            font-size: 13px;
            text-decoration: underline;
        }

        .strikethrough {
            color: var(--error);
            font-size: 13px;
            text-decoration: line-through;
        }

        .overlined {
            color: var(--warning);
            font-size: 13px;
            text-decoration: overline;
        }

        /* ---- Feature 4: cursor styles ---- */
        .cursor-row {
            display: flex;
            flex-wrap: wrap;
            gap: 8px;
            padding: 4px 0px;
        }

        .cursor-chip {
            display: flex;
            align-items: center;
            padding: 4px 10px;
            background: rgba(255, 255, 255, 0.04);
            border-radius: var(--radius-sm);
            color: var(--text-secondary);
            font-size: 12px;
        }

        .cursor-chip:hover {
            background: rgba(255, 255, 255, 0.08);
            color: var(--text-primary);
        }

        .cursor-grab { cursor: grab; }
        .cursor-crosshair { cursor: crosshair; }
        .cursor-not-allowed { cursor: not-allowed; }
        .cursor-move { cursor: move; }
        .cursor-col-resize { cursor: col-resize; }
        .cursor-help { cursor: help; }
        .cursor-wait { cursor: wait; }
        .cursor-text { cursor: text; }

        /* ---- Feature 1: position relative/absolute ---- */
        .pos-container {
            display: flex;
            position: relative;
            width: 100%;
            min-height: 40px;
            background: rgba(255, 255, 255, 0.02);
            border-radius: var(--radius-sm);
            padding: 10px;
        }

        .pos-badge {
            position: absolute;
            top: -8px;
            right: -4px;
            display: flex;
            align-items: center;
            padding: 2px 8px;
            background: var(--accent);
            border-radius: 8px;
            color: #000000;
            font-size: 10px;
            font-weight: bold;
        }

        .pos-offset {
            position: relative;
            top: 4px;
            left: 8px;
            color: var(--info);
            font-size: 12px;
        }

        /* ---- Feature 8: visibility + pointer-events ---- */
        .ghost-text {
            color: var(--text-muted);
            font-size: 13px;
            visibility: hidden;
        }

        .ghost-text-visible {
            color: var(--text-muted);
            font-size: 13px;
        }

        .overlay-passthrough {
            pointer-events: none;
            color: var(--accent-light);
            font-size: 13px;
            opacity: 0.5;
        }

        /* ---- Interactive section ---- */
        .interactive-row {
            display: flex;
            align-items: center;
            gap: 12px;
            padding: 4px 0px;
        }

        .btn {
            display: flex;
            align-items: center;
            padding: 8px 16px;
            background: var(--accent);
            border-radius: var(--radius-sm);
            color: #000000;
            font-size: 13px;
            font-weight: bold;
            cursor: pointer;
        }

        .btn:hover {
            background: var(--accent-light);
        }

        .btn:focus {
            outline-color: var(--accent-light);
            outline-width: 2px;
            outline-offset: 3px;
        }

        .btn-outline {
            display: flex;
            align-items: center;
            padding: 8px 16px;
            background: rgba(0, 0, 0, 0);
            border-width: 1px;
            border-color: var(--accent);
            border-radius: var(--radius-sm);
            color: var(--accent-light);
            font-size: 13px;
            cursor: pointer;
        }

        .btn-outline:hover {
            background: var(--accent-glow);
        }

        .btn-outline:focus {
            outline-color: var(--accent);
            outline-width: 2px;
            outline-offset: 3px;
        }

        .btn-danger {
            display: flex;
            align-items: center;
            padding: 8px 16px;
            background: rgba(248, 113, 113, 0.15);
            border-radius: var(--radius-sm);
            color: var(--error);
            font-size: 13px;
            cursor: not-allowed;
        }

        .btn-danger:focus {
            outline-color: var(--error);
            outline-width: 2px;
            outline-offset: 3px;
        }

        .counter-value {
            color: var(--accent-light);
            font-size: 16px;
            font-weight: bold;
        }

        .status-text {
            color: var(--text-muted);
            font-size: 12px;
        }

        .label {
            color: var(--text-secondary);
            font-size: 13px;
        }

        .section-label {
            color: var(--text-muted);
            font-size: 11px;
            font-weight: bold;
            letter-spacing: 1px;
            padding: 8px 0px 2px 0px;
        }

        .tag-row {
            display: flex;
            flex-wrap: wrap;
            gap: 6px;
        }

        .tag {
            display: flex;
            align-items: center;
            padding: 2px 8px;
            border-radius: 4px;
            font-size: 11px;
        }

        .tag-green {
            background: rgba(16, 185, 129, 0.12);
            color: var(--accent-light);
        }

        .tag-yellow {
            background: rgba(251, 191, 36, 0.12);
            color: var(--warning);
        }

        .tag-blue {
            background: rgba(96, 165, 250, 0.12);
            color: var(--info);
        }

        .tag-red {
            background: rgba(248, 113, 113, 0.12);
            color: var(--error);
        }

        /* ---- Feature 2: :not() selector ---- */
        .tag:not(.tag-green):not(.tag-yellow):not(.tag-blue):not(.tag-red) {
            background: rgba(255, 255, 255, 0.06);
            color: var(--text-secondary);
        }
    "#;

    let app = App::new(
        AppConfig {
            title: "unshit Feature Showcase".to_string(),
            width: 900,
            height: 960,
            css: css.to_string(),
            ..Default::default()
        },
        move || {
            let clicks = cc_read.load(Ordering::Relaxed);
            let was_right_clicked = rc_read.load(Ordering::Relaxed);
            let is_hidden = ht_read.load(Ordering::Relaxed);

            let cc = cc_click.clone();
            let rc = rc_click.clone();
            let ht = ht_click.clone();
            let cc2 = cc_ctx.clone();
            let rc2 = rc_ctx.clone();

            ElementTree {
                root: ElementDef::new(Tag::Div).with_class("root").with_child(header()).with_child(
                    card_grid(AppState {
                        clicks,
                        right_clicked: was_right_clicked,
                        is_hidden,
                        cc,
                        rc,
                        ht,
                        cc_ctx: cc2,
                        rc_ctx: rc2,
                    }),
                ),
            }
        },
    );

    app.run();
}

fn header() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("header")
        .with_child(ElementDef::new(Tag::Span).with_class("title").with_text("unshit"))
        .with_child(ElementDef::new(Tag::Span).with_class("version-badge").with_text("v0.2"))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("subtitle")
                .with_text("Feature Showcase: 11 new capabilities"),
        )
}

struct AppState {
    clicks: u32,
    right_clicked: bool,
    is_hidden: bool,
    cc: Arc<AtomicU32>,
    rc: Arc<AtomicBool>,
    ht: Arc<AtomicBool>,
    cc_ctx: Arc<AtomicU32>,
    rc_ctx: Arc<AtomicBool>,
}

fn card_grid(s: AppState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card-grid")
        .with_child(card_css_variables())
        .with_child(card_selectors())
        .with_child(card_text_decoration())
        .with_child(card_cursors())
        .with_child(card_position())
        .with_child(card_flex_wrap())
        .with_child(card_visibility(s.is_hidden, s.ht))
        .with_child(card_focus())
        .with_child(card_events(s.clicks, s.right_clicked, s.cc, s.rc, s.cc_ctx, s.rc_ctx))
}

// ---- Card 1: CSS Variables ----
fn card_css_variables() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(1)
        .with_child(card_title("1", "CSS Custom Properties"))
        .with_child(desc(
            "All colors and sizes in this demo use var() references to :root variables. \
             This card's green border comes from --accent-glow-strong.",
        ))
        .with_child(
            ElementDef::new(Tag::Div).with_class("section-label").with_text("ACTIVE VARIABLES"),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("--accent", "tag-green"))
                .with_child(tag("--warning", "tag-yellow"))
                .with_child(tag("--info", "tag-blue"))
                .with_child(tag("--error", "tag-red"))
                .with_child(tag("--bg-card", "")),
        )
}

// ---- Card 2: Structural Selectors ----
fn card_selectors() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(2)
        .with_child(card_title("2", "Structural Selectors"))
        .with_child(desc(
            "This grid uses :first-child (green border on card 1), :last-child \
             (red border on last card), :nth-child(3) (yellow border on card 3), \
             and :not() for default tag colors above.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag(":first-child", "tag-green"))
                .with_child(tag(":last-child", "tag-red"))
                .with_child(tag(":nth-child(n)", "tag-yellow"))
                .with_child(tag(":not()", "tag-blue")),
        )
}

// ---- Card 3: Text Decoration ----
fn card_text_decoration() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(3)
        .with_child(card_title("3", "Text Decoration"))
        .with_child(desc("GPU-rendered underline, line-through, and overline decorations."))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("underlined")
                .with_text("This text has an underline decoration"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("strikethrough")
                .with_text("This text has a line-through decoration"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("overlined")
                .with_text("This text has an overline decoration"),
        )
}

// ---- Card 4: Cursor Styles ----
fn card_cursors() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(4)
        .with_child(card_title("4", "Cursor Styles + Outline"))
        .with_child(desc(
            "Hover each chip to see a different cursor. Tab to this card to see the focus outline.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("cursor-row")
                .with_child(cursor_chip("grab", "cursor-grab"))
                .with_child(cursor_chip("crosshair", "cursor-crosshair"))
                .with_child(cursor_chip("not-allowed", "cursor-not-allowed"))
                .with_child(cursor_chip("move", "cursor-move"))
                .with_child(cursor_chip("col-resize", "cursor-col-resize"))
                .with_child(cursor_chip("help", "cursor-help"))
                .with_child(cursor_chip("wait", "cursor-wait"))
                .with_child(cursor_chip("text", "cursor-text")),
        )
}

// ---- Card 5: Position ----
fn card_position() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(5)
        .with_child(card_title("5", "CSS Position"))
        .with_child(desc(
            "Support for position: relative and position: absolute with top/right/bottom/left insets.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("pos-container")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("label")
                        .with_text("Relative container with absolute badge -->"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("pos-badge")
                        .with_text("ABS"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("pos-offset")
                        .with_text("(offset via relative)"),
                ),
        )
}

// ---- Card 6: Flex Wrap ----
fn card_flex_wrap() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(6)
        .with_child(card_title("6", "Flex Wrap + Align Content"))
        .with_child(desc(
            "Children wrap to new lines when they overflow. This row uses flex-wrap: wrap.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("tag-row")
                .with_child(tag("wrap", "tag-green"))
                .with_child(tag("nowrap", "tag-yellow"))
                .with_child(tag("wrap-reverse", "tag-blue"))
                .with_child(tag("align-content", "tag-red"))
                .with_child(tag("start", "tag-green"))
                .with_child(tag("center", "tag-yellow"))
                .with_child(tag("space-between", "tag-blue"))
                .with_child(tag("space-around", "tag-red"))
                .with_child(tag("stretch", "")),
        )
}

// ---- Card 7: Visibility + Pointer Events ----
fn card_visibility(is_hidden: bool, toggle: Arc<AtomicBool>) -> ElementDef {
    let hidden_class = if is_hidden { "ghost-text" } else { "ghost-text-visible" };
    let btn_label = if is_hidden { "Show Text" } else { "Hide Text" };

    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(7)
        .with_child(card_title("7", "Visibility + Pointer Events"))
        .with_child(desc(
            "visibility:hidden hides an element but preserves its layout space. \
             pointer-events:none lets clicks pass through. Click the button to toggle.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("interactive-row")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn-outline")
                        .with_text(btn_label)
                        .on_click(move || {
                            let prev = toggle.load(Ordering::Relaxed);
                            toggle.store(!prev, Ordering::Relaxed);
                        }),
                )
                .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("["))
                .with_child(ElementDef::new(Tag::Span).with_class(hidden_class).with_text("HIDDEN"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("label")
                        .with_text("] <-- space preserved"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div).with_class("interactive-row").with_child(
                ElementDef::new(Tag::Span)
                    .with_class("overlay-passthrough")
                    .with_text("This text has pointer-events: none (clicks pass through)"),
            ),
        )
}

// ---- Card 8: Focus + Tab ----
fn card_focus() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(8)
        .with_child(card_title("8", "Focus + Tab Navigation"))
        .with_child(desc(
            "Press Tab to cycle focus through all cards (they have tab_index). \
             The focused card gets an outline via :focus CSS. \
             Shift+Tab goes backward.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("interactive-row")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_text("Focusable Button A")
                        .with_tab_index(10),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn-outline")
                        .with_text("Focusable Button B")
                        .with_tab_index(11),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn-danger")
                        .with_text("Disabled Look")
                        .with_tab_index(12),
                ),
        )
}

// ---- Card 9: Events ----
fn card_events(
    clicks: u32,
    right_clicked: bool,
    cc: Arc<AtomicU32>,
    rc: Arc<AtomicBool>,
    cc_ctx: Arc<AtomicU32>,
    rc_ctx: Arc<AtomicBool>,
) -> ElementDef {
    let status = if right_clicked {
        "Right-click detected!"
    } else if clicks > 0 {
        "Double-click to select words"
    } else {
        "Click the button or right-click this card"
    };

    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_tab_index(9)
        .on_context_menu(move |_x, _y| {
            rc_ctx.store(true, Ordering::Relaxed);
            cc_ctx.fetch_add(1, Ordering::Relaxed);
        })
        .with_child(card_title("9", "Double-Click + Right-Click"))
        .with_child(desc(
            "Click the button to increment. Right-click this card to fire on_context_menu. \
             Double-click text to select words.",
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("interactive-row")
                .with_child(
                    ElementDef::new(Tag::Button).with_class("btn").with_text("Click me!").on_click(
                        move || {
                            cc.fetch_add(1, Ordering::Relaxed);
                            rc.store(false, Ordering::Relaxed);
                        },
                    ),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("counter-value")
                        .with_text(format!("{clicks}")),
                )
                .with_child(ElementDef::new(Tag::Span).with_class("status-text").with_text(status)),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("label").with_text(
            "Try double-clicking this sentence to select individual words. \
                     Right-click anywhere on this card for context menu event.",
        ))
}

// ---- Helpers ----

fn card_title(num: &str, title: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card-title-row")
        .with_child(ElementDef::new(Tag::Span).with_class("card-number").with_text(num))
        .with_child(ElementDef::new(Tag::Span).with_class("card-label").with_text(title))
}

fn desc(s: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("card-desc").with_text(s)
}

fn tag(label: &str, extra_class: &str) -> ElementDef {
    let mut el = ElementDef::new(Tag::Span).with_class("tag").with_text(label);
    if !extra_class.is_empty() {
        el = el.with_class(extra_class);
    }
    el
}

fn cursor_chip(label: &str, cursor_class: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("cursor-chip").with_class(cursor_class).with_text(label)
}
