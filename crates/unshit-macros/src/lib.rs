extern crate proc_macro;
use proc_macro::TokenStream;

mod ui_test;
mod view;

#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    view::parse_view(input)
}

/// Attribute macro that wraps a function as a `#[test]` with optional UI test
/// configuration via environment variables.
///
/// # Usage
///
/// ```ignore
/// #[ui_test]
/// fn basic_test() {
///     let app = TestApp::new("", my_tree, 800.0, 600.0);
///     app.query(".btn").unwrap();
/// }
///
/// #[ui_test(headed, slow_mo = 200)]
/// fn debug_test() { /* ... */ }
///
/// #[ui_test(width = 1024, height = 768)]
/// fn wide_layout() { /* ... */ }
/// ```
///
/// Supported attributes: `headed`, `slow_mo`, `width`, `height`, `gpu`, `timeout`.
#[proc_macro_attribute]
pub fn ui_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    ui_test::ui_test_impl(attr, item)
}
