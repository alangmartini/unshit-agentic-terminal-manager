//! Demonstrates the EventSink API: a background thread sends
//! `ExternalEvent::RequestRebuild` once per second, causing the
//! displayed counter to increment without any user interaction.
//!
//! Run with: cargo run -p unshit --example async_counter

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use unshit::app::{App, AppConfig, ExternalEvent};
use unshit::core::element::*;

fn main() {
    env_logger::init();

    let counter = Arc::new(AtomicU64::new(0));

    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: rgba(13, 17, 23, 0.95);
            align-items: center;
            justify-content: center;
            gap: 24px;
        }
        .label {
            color: #8b949e;
            font-size: 16px;
        }
        .counter {
            color: #10b981;
            font-size: 72px;
            font-weight: bold;
        }
        .hint {
            color: #484f58;
            font-size: 13px;
        }
    "#;

    let counter_for_tree = Arc::clone(&counter);
    let app = App::new(
        AppConfig {
            title: "async counter".to_string(),
            width: 400,
            height: 300,
            css: css.to_string(),
            ..Default::default()
        },
        move || {
            let value = counter_for_tree.load(Ordering::Relaxed);
            ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("root")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("label")
                            .with_text("External event counter"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("counter")
                            .with_text(&value.to_string()),
                    )
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("hint")
                            .with_text("Incremented by a background thread via EventSink"),
                    ),
            }
        },
    );

    let sink = app.event_sink();
    let counter_for_thread = Arc::clone(&counter);

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        counter_for_thread.fetch_add(1, Ordering::Relaxed);
        if sink.send(ExternalEvent::RequestRebuild).is_err() {
            break; // Event loop shut down.
        }
    });

    app.run();
}
