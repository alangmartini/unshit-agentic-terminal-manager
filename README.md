# unshit

A GPU-accelerated UI framework for Rust. CSS-styled, flexbox-layouted, wgpu-rendered.

## Features

- **GPU-first rendering** via wgpu
- **CSS styling** with familiar properties (flexbox, border-radius, box-shadow, hover/active states, keyframe animations)
- **Flexbox layout** powered by taffy
- **Text rendering** via cosmic-text
- **Canvas API** for custom drawing with lyon
- **Async support** with tokio-based subscriptions and event streams
- **Clipboard** integration

## Quick start

```rust
use unshit::app::{App, AppConfig};
use unshit::core::element::*;

fn main() {
    let app = App::new(
        AppConfig {
            title: "hello".to_string(),
            width: 800,
            height: 600,
            css: r#"
                .root { display: flex; background: #0d1117; padding: 32px; }
                .title { color: #e6edf3; font-size: 32px; font-weight: bold; }
            "#.to_string(),
            ..Default::default()
        },
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("title")
                        .with_text("Hello, unshit!"),
                ),
        },
    );

    app.run();
}
```

## Crates

| Crate | Description |
|-------|-------------|
| `unshit` | Top-level re-export crate (use this) |
| `unshit-core` | CSS parsing, style cascade, layout, element tree |
| `unshit-renderer` | wgpu rendering, text shaping, canvas |
| `unshit-macros` | Proc macros (view! macro) |
| `unshit-app` | Windowing, event loop, app lifecycle |

## Examples

```sh
cargo run --example hello
cargo run --example canvas
cargo run --example keyframes_demo
cargo run --example kitchen_sink --features async
cargo run --example terminal_manager
```

## License

MIT
