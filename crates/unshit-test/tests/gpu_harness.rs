use unshit_core::element::*;
use unshit_test::TestHarness;

fn simple_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root"),
    }
}

#[test]
fn try_with_gpu_returns_bool() {
    let mut h = TestHarness::new(
        ".root { width: 100px; height: 100px; }",
        simple_tree,
        100.0,
        100.0,
    );
    // try_with_gpu must return a bool without panicking, regardless of
    // whether an actual GPU is present.
    let result = h.try_with_gpu();
    assert_eq!(result, h.has_gpu());
}

#[test]
fn has_gpu_false_by_default() {
    let h = TestHarness::new(
        ".root { width: 100px; height: 100px; }",
        simple_tree,
        100.0,
        100.0,
    );
    assert!(!h.has_gpu(), "has_gpu should be false before with_gpu is called");
}

#[test]
#[should_panic(expected = "GPU context required")]
fn require_gpu_panics_when_no_gpu() {
    let h = TestHarness::new(
        ".root { width: 100px; height: 100px; }",
        simple_tree,
        100.0,
        100.0,
    );
    h.require_gpu();
}

#[test]
fn require_gpu_ok_after_try_with_gpu() {
    let mut h = TestHarness::new(
        ".root { width: 100px; height: 100px; }",
        simple_tree,
        100.0,
        100.0,
    );
    if h.try_with_gpu() {
        // Should not panic when GPU was successfully initialized.
        h.require_gpu();
    }
    // If try_with_gpu returned false, we simply skip since no GPU is
    // available in this environment.
}

#[test]
fn backend_env_parse_unknown_does_not_panic() {
    // Setting an unknown backend value should not crash; the harness
    // falls back to auto-detect. Use a scoped guard to ensure cleanup.
    unsafe { std::env::set_var("UNSHIT_TEST_BACKEND", "nonexistent_backend") };

    let mut h = TestHarness::new(
        ".root { width: 100px; height: 100px; }",
        simple_tree,
        100.0,
        100.0,
    );
    // try_with_gpu should still work (returns true or false)
    let _ = h.try_with_gpu();

    unsafe { std::env::remove_var("UNSHIT_TEST_BACKEND") };
}
