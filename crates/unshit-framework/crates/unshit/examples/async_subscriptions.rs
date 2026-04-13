//! Demonstrates the Subscription API: declarative, identity-tracked async
//! event sources that the framework manages automatically.
//!
//! Two subscriptions run concurrently:
//! - "seconds": ticks every second, incrementing a counter
//! - "fast": ticks every 250ms, incrementing a separate counter
//!
//! Run with: cargo run -p unshit --features async --example async_subscriptions

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use unshit::app::{App, AppConfig, ExternalEvent, Subscription};
use unshit::core::element::*;

fn main() {
    env_logger::init();

    let seconds = Arc::new(AtomicU64::new(0));
    let fast_ticks = Arc::new(AtomicU64::new(0));

    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: rgba(13, 17, 23, 0.95);
            align-items: center;
            justify-content: center;
            gap: 32px;
        }
        .title {
            color: #8b949e;
            font-size: 16px;
        }
        .row {
            display: flex;
            gap: 40px;
            align-items: center;
        }
        .card {
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 8px;
            padding: 24px 32px;
            background: rgba(255, 255, 255, 0.03);
            border-radius: 16px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.15);
        }
        .card-label {
            color: #484f58;
            font-size: 12px;
            font-weight: bold;
            letter-spacing: 2px;
        }
        .card-value {
            color: #10b981;
            font-size: 48px;
            font-weight: bold;
        }
        .card-hint {
            color: #6b7280;
            font-size: 12px;
        }
        .hint {
            color: #484f58;
            font-size: 13px;
        }
    "#;

    let sec_for_tree = Arc::clone(&seconds);
    let fast_for_tree = Arc::clone(&fast_ticks);

    let mut app = App::new(
        AppConfig {
            title: "subscriptions".to_string(),
            width: 600,
            height: 350,
            css: css.to_string(),
            ..Default::default()
        },
        move || {
            let sec = sec_for_tree.load(Ordering::Relaxed);
            let fast = fast_for_tree.load(Ordering::Relaxed);
            ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("root")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("title")
                            .with_text("Subscription API demo"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("row")
                            .with_child(counter_card("SECONDS", &sec.to_string(), "1s interval"))
                            .with_child(counter_card("FAST", &fast.to_string(), "250ms interval")),
                    )
                    .with_child(
                        ElementDef::new(Tag::Span).with_class("hint").with_text(
                            "Two identity-tracked subscriptions managed by the framework",
                        ),
                    ),
            }
        },
    );

    let sec_for_sub = Arc::clone(&seconds);
    let fast_for_sub = Arc::clone(&fast_ticks);

    app.set_subscriptions(move || {
        let sec = Arc::clone(&sec_for_sub);
        let fast = Arc::clone(&fast_for_sub);
        vec![
            Subscription::new("seconds", move |_sink| {
                let sec = Arc::clone(&sec);
                Box::pin(async_stream::stream! {
                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        sec.fetch_add(1, Ordering::Relaxed);
                        yield ExternalEvent::RequestRebuild;
                    }
                })
            }),
            Subscription::new("fast", move |_sink| {
                let fast = Arc::clone(&fast);
                Box::pin(async_stream::stream! {
                    loop {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        fast.fetch_add(1, Ordering::Relaxed);
                        yield ExternalEvent::RequestRebuild;
                    }
                })
            }),
        ]
    });

    app.run();
}

fn counter_card(label: &str, value: &str, hint: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_child(ElementDef::new(Tag::Span).with_class("card-label").with_text(label))
        .with_child(ElementDef::new(Tag::Span).with_class("card-value").with_text(value))
        .with_child(ElementDef::new(Tag::Span).with_class("card-hint").with_text(hint))
}
