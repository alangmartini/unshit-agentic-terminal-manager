use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use unshit_core::element::*;
use unshit_macros::view;

#[test]
fn view_macro_with_on_click_compiles() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let tree = view! {
        div.root {
            button.btn [on_click = move || { c.fetch_add(1, Ordering::SeqCst); }] {
                "Click me"
            }
        }
    };
    assert_eq!(tree.root.tag, Tag::Div);
    // Verify the child button has an on_click handler
    assert_eq!(tree.root.children.len(), 1);
    assert!(tree.root.children[0].on_click.is_some());
}

#[test]
fn view_macro_on_click_handler_is_callable() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let tree = view! {
        button.btn [on_click = move || { c.fetch_add(1, Ordering::SeqCst); }] {
            "Click me"
        }
    };
    // Call the handler and verify side effect
    (tree.root.on_click.as_ref().unwrap())();
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn view_macro_on_click_with_id() {
    let tree = view! {
        button.btn [id = "submit", on_click = || { }] {
            "Submit"
        }
    };
    assert_eq!(tree.root.id.as_deref(), Some("submit"));
    assert!(tree.root.on_click.is_some());
}

#[test]
fn view_macro_without_on_click() {
    let tree = view! {
        div.root {
            "Hello"
        }
    };
    assert!(tree.root.on_click.is_none());
}

#[test]
fn view_macro_key_string_literal() {
    let tree = view! {
        div [key = "my-key"] {}
    };
    assert_eq!(tree.root.key.as_deref(), Some("my-key"));
}

#[test]
fn view_macro_key_dynamic_expression() {
    let idx: usize = 42;
    let tree = view! {
        div [key = idx.to_string()] {}
    };
    assert_eq!(tree.root.key.as_deref(), Some("42"));
}

#[test]
fn view_macro_key_and_id_coexist() {
    let tree = view! {
        div [id = "css-id", key = "recon-key"] {}
    };
    assert_eq!(tree.root.id.as_deref(), Some("css-id"));
    assert_eq!(tree.root.key.as_deref(), Some("recon-key"));
}

#[test]
fn view_macro_key_on_child() {
    let items = vec!["alpha", "beta", "gamma"];
    let tree = view! {
        div.list {
            span [key = items[0]] {}
            span [key = items[1]] {}
            span [key = items[2]] {}
        }
    };
    assert_eq!(tree.root.children.len(), 3);
    assert_eq!(tree.root.children[0].key.as_deref(), Some("alpha"));
    assert_eq!(tree.root.children[1].key.as_deref(), Some("beta"));
    assert_eq!(tree.root.children[2].key.as_deref(), Some("gamma"));
}
