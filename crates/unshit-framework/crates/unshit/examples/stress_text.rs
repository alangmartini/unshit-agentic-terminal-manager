use unshit::app::{App, AppConfig};
use unshit::core::element::*;

fn main() {
    env_logger::init();

    let css = r#"
        .terminal {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: rgba(13, 17, 23, 0.9);
            padding: 8px;
        }
        .line {
            color: #e6edf3;
            font-size: 14px;
        }
        .prompt {
            color: #10b981;
            font-size: 14px;
        }
        .output {
            color: #8b949e;
            font-size: 14px;
        }
        .line:hover {
            background: rgba(255, 255, 255, 0.04);
        }
        .prompt:hover {
            color: #34d399;
            background: rgba(16, 185, 129, 0.06);
        }
        .output:hover {
            color: #b1bac4;
            background: rgba(255, 255, 255, 0.03);
        }
    "#;

    let app = App::new(
        AppConfig {
            title: "unshit - Stress Test (1000 lines)".to_string(),
            width: 1200,
            height: 800,
            css: css.to_string(),
            ..Default::default()
        },
        || {
            let mut children = Vec::new();

            // Generate 1000 lines of terminal-like output
            for i in 0..1000 {
                let text = if i % 5 == 0 {
                    format!("$ cargo build --release [line {}]", i)
                } else if i % 5 == 1 {
                    format!("   Compiling unshit v0.1.0 ({}/1000)", i)
                } else if i % 5 == 2 {
                    format!("   Compiling unshit-core v0.1.0 ({}/1000)", i)
                } else if i % 5 == 3 {
                    format!("   Compiling unshit-renderer v0.1.0 ({}/1000)", i)
                } else {
                    format!("    Finished release [optimized] target(s) in 0.{}s", i)
                };

                let class = if i % 5 == 0 {
                    "prompt"
                } else if i % 5 == 4 {
                    "output"
                } else {
                    "line"
                };

                children.push(ElementDef::new(Tag::Span).with_class(class).with_text(text));
            }

            ElementTree {
                root: ElementDef::new(Tag::Div).with_class("terminal").with_children(children),
            }
        },
    );

    app.run();
}
