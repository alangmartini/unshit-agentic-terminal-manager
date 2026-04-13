//! Demonstrates App::spawn(): run async tasks on the background tokio runtime.
//!
//! A spawned async task fetches a "result" after a delay, then pushes it
//! to the UI via EventSink. The UI rebuilds to show the loaded value.
//!
//! Run with: cargo run -p unshit --features async --example async_spawn

use std::sync::{Arc, Mutex};
use std::time::Duration;

use unshit::app::{App, AppConfig, ExternalEvent};
use unshit::core::element::*;

fn main() {
    env_logger::init();

    let status: Arc<Mutex<String>> = Arc::new(Mutex::new("Loading...".to_string()));

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
        .value {
            color: #10b981;
            font-size: 48px;
            font-weight: bold;
        }
        .hint {
            color: #484f58;
            font-size: 13px;
        }
    "#;

    let status_for_tree = Arc::clone(&status);
    let app = App::new(
        AppConfig {
            title: "async spawn".to_string(),
            width: 500,
            height: 300,
            css: css.to_string(),
            ..Default::default()
        },
        move || {
            let text = status_for_tree.lock().unwrap().clone();
            ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("root")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("label")
                            .with_text("App::spawn() demo"),
                    )
                    .with_child(ElementDef::new(Tag::Span).with_class("value").with_text(&text))
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("hint")
                            .with_text("Async task completes after 2 seconds"),
                    ),
            }
        },
    );

    let sink = app.event_sink();
    let status_for_task = Arc::clone(&status);

    // Spawn an async task on the background tokio runtime.
    app.spawn(async move {
        // Simulate an async operation (network fetch, file I/O, etc.)
        tokio::time::sleep(Duration::from_secs(2)).await;

        *status_for_task.lock().unwrap() = "Data loaded!".to_string();
        let _ = sink.send(ExternalEvent::RequestRebuild);

        // Show a sequence of updates.
        for i in 1..=5 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            *status_for_task.lock().unwrap() = format!("Update #{}", i);
            let _ = sink.send(ExternalEvent::RequestRebuild);
        }

        *status_for_task.lock().unwrap() = "All done!".to_string();
        let _ = sink.send(ExternalEvent::RequestRebuild);
    });

    app.run();
}
