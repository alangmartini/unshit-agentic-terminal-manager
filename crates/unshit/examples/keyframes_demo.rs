//! CSS @keyframes and animation: demo (issue #129).
//!
//! Runs the four terminal manager keyframes as synthetic elements so you
//! can eyeball the driver. Every rule exercises a different piece of the
//! pipeline:
//!
//! - `pulse-dot` uses multi selector percentage keyframes and an infinite
//!   iteration count with `ease-in-out` timing.
//! - `cursor-blink` uses a single intermediate keyframe and relies on the
//!   driver to synthesize the missing 0% and 100% endpoints from the base
//!   computed style.
//! - `fade-in` uses the `from`/`to` ident syntax with a cubic bezier
//!   timing function.
//! - `spin-border` drives a non opacity property (border color) to prove
//!   multi property keyframes work, since `transform` is out of scope for
//!   this issue.

use unshit::app::{App, AppConfig};
use unshit::core::element::*;

fn main() {
    env_logger::init();

    let app = App::new(
        AppConfig {
            title: "unshit keyframes demo".to_string(),
            width: 960,
            height: 640,
            css: CSS.to_string(),
            ..Default::default()
        },
        build_tree,
    );

    app.run();
}

fn build_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div).with_class("title").with_text("CSS @keyframes demo"),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("subtitle")
                    .with_text("Four terminal manager animations running on the unshit driver"),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("row")
                    .with_child(card(
                        "pulse-dot",
                        "2s ease-in-out infinite",
                        ElementDef::new(Tag::Div).with_class("pulse-dot"),
                    ))
                    .with_child(card(
                        "cursor-blink",
                        "1.1s linear infinite",
                        ElementDef::new(Tag::Div).with_class("cursor-blink"),
                    ))
                    .with_child(card(
                        "fade-in",
                        "1.6s cubic-bezier infinite alternate",
                        ElementDef::new(Tag::Div).with_class("fade-in"),
                    ))
                    .with_child(card(
                        "spin-border",
                        "2s linear infinite",
                        ElementDef::new(Tag::Div).with_class("spin-border"),
                    )),
            ),
    }
}

fn card(title: &str, description: &str, inner: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_child(ElementDef::new(Tag::Div).with_class("card-stage").with_child(inner))
        .with_child(ElementDef::new(Tag::Span).with_class("card-title").with_text(title))
        .with_child(ElementDef::new(Tag::Span).with_class("card-desc").with_text(description))
}

// Terminal manager keyframes live at styles.css lines 611, 1138, 1326, 1331.
// The `animation:` entries mirror the terminal manager usage sites.
const CSS: &str = r#"
    .root {
        display: flex;
        flex-direction: column;
        width: 100%;
        height: 100%;
        background: #0b1220;
        padding: 48px;
        gap: 24px;
    }

    .title {
        color: #e6edf3;
        font-size: 28px;
        font-weight: bold;
    }

    .subtitle {
        color: #8b949e;
        font-size: 14px;
    }

    .row {
        display: flex;
        flex-direction: row;
        gap: 20px;
        flex-grow: 1;
    }

    .card {
        display: flex;
        flex-direction: column;
        flex-grow: 1;
        background: rgba(13, 17, 23, 0.6);
        border-radius: 16px;
        border-width: 1px;
        border-color: rgba(16, 185, 129, 0.2);
        padding: 20px;
        gap: 12px;
    }

    .card-stage {
        display: flex;
        flex-grow: 1;
        align-items: center;
        justify-content: center;
        min-height: 160px;
        background: rgba(0, 0, 0, 0.35);
        border-radius: 12px;
    }

    .card-title {
        color: #e6edf3;
        font-size: 16px;
        font-weight: bold;
    }

    .card-desc {
        color: #8b949e;
        font-size: 12px;
    }

    /* pulse-dot: terminal manager styles.css line 611. */
    @keyframes pulse-dot {
        0%, 100% { opacity: 1; }
        50% { opacity: 0.4; }
    }
    .pulse-dot {
        width: 64px;
        height: 64px;
        border-radius: 999px;
        background: #10b981;
        opacity: 1;
        animation: pulse-dot 2s ease-in-out infinite;
    }

    /* cursor-blink: terminal manager styles.css line 1138. The 0% and 100%
       entries are synthesized by the driver from the element base style. */
    @keyframes cursor-blink {
        50% { opacity: 0; }
    }
    .cursor-blink {
        width: 18px;
        height: 48px;
        background: #e6edf3;
        opacity: 1;
        animation: cursor-blink 1.1s linear infinite;
    }

    /* fade-in: terminal manager styles.css line 1326. Drives a full ramp
       from 0 to 1, then alternates so the demo loops. */
    @keyframes fade-in {
        from { opacity: 0; }
        to { opacity: 1; }
    }
    .fade-in {
        width: 120px;
        height: 80px;
        background: #38bdf8;
        border-radius: 12px;
        opacity: 0;
        animation: fade-in 1.6s cubic-bezier(0.22, 0.61, 0.36, 1) infinite alternate;
    }

    /* spin-border: non opacity multi property keyframe proof. Border color
       and background cycle through the accent palette. */
    @keyframes spin-border {
        0% { background: #f97316; border-color: #f97316; }
        50% { background: #a855f7; border-color: #a855f7; }
        100% { background: #f97316; border-color: #f97316; }
    }
    .spin-border {
        width: 110px;
        height: 110px;
        border-radius: 999px;
        border-width: 6px;
        border-color: #f97316;
        background: #f97316;
        animation: spin-border 2s linear infinite;
    }
"#;
