//! End to end test for `@font-face` support.
//!
//! Exercises the full pipeline a real app goes through on startup:
//! 1. Parse CSS into `CompiledStylesheet`, recording `font_faces`.
//! 2. Call `load_custom_fonts` to register each `@font-face` src against
//!    the harness `FontSystem`.
//! 3. Verify the family lands in the cosmic-text font database.

use std::path::{Path, PathBuf};

use unshit_app::{load_custom_fonts, FontSource};
use unshit_core::element::*;
use unshit_core::style::parse::{CompiledStylesheet, FontFaceSrc};
use unshit_test::TestHarness;

fn fixture_path(name: &str) -> PathBuf {
    // unshit-test depends on unshit-app at workspace scope, so we can
    // reach its fixtures from here via CARGO_MANIFEST_DIR of unshit-test.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace parent")
        .join("unshit-app")
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn css_safe_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn db_has_family(fs: &cosmic_text::FontSystem, family: &str) -> bool {
    fs.db().faces().any(|f| f.families.iter().any(|(fam, _)| fam.eq_ignore_ascii_case(family)))
}

#[test]
fn at_font_face_parsed_into_stylesheet() {
    let css = r#"
        @font-face {
            font-family: "Inter";
            src: url("assets/Inter.ttf");
        }
        body { font-family: "Inter"; }
    "#;
    let h = TestHarness::new(css, || ElementTree { root: ElementDef::new(Tag::Div) }, 800.0, 600.0);

    let sheet = h.stylesheet();
    assert_eq!(sheet.font_faces.len(), 1);
    assert_eq!(sheet.font_faces[0].family, "Inter");
    assert_eq!(sheet.font_faces[0].src, FontFaceSrc::Url("assets/Inter.ttf".to_string()));
    // The normal selector rule should still be populated.
    assert!(!sheet.rules.is_empty());
}

#[test]
fn at_font_face_then_load_via_harness_font_system() {
    let path = fixture_path("FiraMono-Medium.ttf");
    assert!(path.exists(), "test fixture FiraMono-Medium.ttf must exist at {:?}", path);
    let css = format!(
        "@font-face {{ font-family: \"Fira Mono\"; src: url(\"{}\"); }} \
         body {{ font-family: \"Fira Mono\"; }}",
        css_safe_path(&path),
    );
    let h =
        TestHarness::new(&css, || ElementTree { root: ElementDef::new(Tag::Div) }, 800.0, 600.0);

    // Stylesheet should carry the @font-face rule straight through parsing.
    assert_eq!(h.stylesheet().font_faces.len(), 1);

    // TestHarness mirrors App startup by loading @font-face rules before the
    // first layout and render pass.
    assert!(db_has_family(h.font_system(), "Fira Mono"));
}

#[test]
fn at_font_face_coexists_with_other_selector_rules() {
    let css = r#"
        body { color: red; }
        @font-face { font-family: "A"; src: url("a.ttf"); }
        .card { width: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("card") },
        800.0,
        600.0,
    );

    let sheet = h.stylesheet();
    assert_eq!(sheet.font_faces.len(), 1);
    assert_eq!(sheet.font_faces[0].family, "A");
    // The .card and body rules should still be present.
    assert!(sheet.rules.len() >= 2);
}

#[test]
fn config_fonts_through_harness_font_system() {
    let mut h =
        TestHarness::new("", || ElementTree { root: ElementDef::new(Tag::Div) }, 800.0, 600.0);
    let baseline = h.font_system().db().len();

    let sheet = CompiledStylesheet::parse("");
    let report = load_custom_fonts(
        h.font_system_mut(),
        &[FontSource::Path(fixture_path("FiraMono-Medium.ttf"))],
        &sheet,
    );

    assert_eq!(report.config_errors, 0);
    assert!(report.config_faces >= 1);
    assert!(h.font_system().db().len() > baseline);
    assert!(db_has_family(h.font_system(), "Fira Mono"));
}
