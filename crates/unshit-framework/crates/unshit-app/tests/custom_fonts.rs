//! Integration tests for custom font loading via `AppConfig::fonts` and
//! CSS `@font-face` rules.
//!
//! These tests drive `load_custom_fonts` directly against a real
//! `cosmic_text::FontSystem`, exactly the way `App::can_create_surfaces`
//! does at startup. They avoid winit / wgpu because the loader itself is
//! independent of both.

use std::path::PathBuf;
use std::sync::Arc;

use cosmic_text::FontSystem;
use unshit_app::{load_custom_fonts, FontSource};
use unshit_core::style::parse::CompiledStylesheet;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn fixture_family(name: &str) -> &'static str {
    // FiraMono-Medium.ttf registers the family "Fira Mono".
    match name {
        "FiraMono-Medium.ttf" => "Fira Mono",
        other => panic!("unknown fixture family for {}", other),
    }
}

fn db_has_family(fs: &FontSystem, family: &str) -> bool {
    fs.db().faces().any(|f| f.families.iter().any(|(fam, _)| fam.eq_ignore_ascii_case(family)))
}

fn baseline_count(fs: &FontSystem) -> usize {
    fs.db().len()
}

#[test]
fn config_fonts_bytes_land_in_font_system() {
    let bytes = std::fs::read(fixture_path("FiraMono-Medium.ttf")).unwrap();
    let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());

    let mut fs = FontSystem::new();
    let baseline = baseline_count(&fs);
    let sheet = CompiledStylesheet::parse("");

    let report = load_custom_fonts(&mut fs, &[FontSource::Bytes(arc)], &sheet);

    assert_eq!(report.config_errors, 0);
    assert!(report.config_faces >= 1);
    assert!(fs.db().len() > baseline);
    assert!(db_has_family(&fs, fixture_family("FiraMono-Medium.ttf")));
}

#[test]
fn config_fonts_path_lands_in_font_system() {
    let mut fs = FontSystem::new();
    let baseline = baseline_count(&fs);
    let sheet = CompiledStylesheet::parse("");

    let report = load_custom_fonts(
        &mut fs,
        &[FontSource::Path(fixture_path("FiraMono-Medium.ttf"))],
        &sheet,
    );

    assert_eq!(report.config_errors, 0);
    assert!(report.config_faces >= 1);
    assert!(fs.db().len() > baseline);
    assert!(db_has_family(&fs, fixture_family("FiraMono-Medium.ttf")));
}

#[test]
fn css_font_face_rule_lands_in_font_system() {
    let path = fixture_path("FiraMono-Medium.ttf");
    // Forward slashes keep the url path CSS safe on every platform.
    let path_str = path.to_string_lossy().replace('\\', "/");
    let css = format!("@font-face {{ font-family: \"Fira Mono\"; src: url(\"{}\"); }}", path_str);
    let sheet = CompiledStylesheet::parse(&css);
    assert_eq!(sheet.font_faces.len(), 1);

    let mut fs = FontSystem::new();
    let baseline = baseline_count(&fs);
    let report = load_custom_fonts(&mut fs, &[], &sheet);

    assert_eq!(report.css_errors, 0, "CSS loaded without errors");
    assert!(report.css_faces >= 1);
    assert!(fs.db().len() > baseline);
    assert!(db_has_family(&fs, fixture_family("FiraMono-Medium.ttf")));
}

#[test]
fn css_font_face_with_missing_file_is_not_fatal() {
    let css = "@font-face { font-family: \"Phantom\"; src: url(\"does/not/exist.ttf\"); }";
    let sheet = CompiledStylesheet::parse(css);
    assert_eq!(sheet.font_faces.len(), 1);

    let mut fs = FontSystem::new();
    let baseline = baseline_count(&fs);
    let report = load_custom_fonts(&mut fs, &[], &sheet);

    assert_eq!(report.css_faces, 0);
    assert_eq!(report.css_errors, 1);
    assert_eq!(fs.db().len(), baseline);
}

#[test]
fn css_font_face_with_data_uri_is_rejected_cleanly() {
    let css = "@font-face { font-family: \"Embedded\"; src: url(\"data:font/ttf;base64,AAAA\"); }";
    let sheet = CompiledStylesheet::parse(css);
    assert_eq!(sheet.font_faces.len(), 1);

    let mut fs = FontSystem::new();
    let baseline = baseline_count(&fs);
    let report = load_custom_fonts(&mut fs, &[], &sheet);

    assert_eq!(report.css_faces, 0);
    assert_eq!(report.css_errors, 1);
    assert_eq!(fs.db().len(), baseline);
}

#[test]
fn config_fonts_and_css_combined_both_register() {
    let bytes = std::fs::read(fixture_path("FiraMono-Medium.ttf")).unwrap();
    let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());

    let path = fixture_path("FiraMono-Medium.ttf");
    let path_str = path.to_string_lossy().replace('\\', "/");
    let css = format!("@font-face {{ font-family: \"Fira Mono\"; src: url(\"{}\"); }}", path_str);
    let sheet = CompiledStylesheet::parse(&css);

    let mut fs = FontSystem::new();
    let baseline = baseline_count(&fs);
    let report = load_custom_fonts(&mut fs, &[FontSource::Bytes(arc)], &sheet);

    assert_eq!(report.config_errors, 0);
    assert_eq!(report.css_errors, 0);
    assert!(report.config_faces >= 1);
    assert!(report.css_faces >= 1);
    // Both registrations should add faces. fontdb assigns each a fresh ID,
    // so the family count is independent of duplicate names.
    assert!(fs.db().len() >= baseline + 2);
}

#[test]
fn duplicate_family_names_all_register() {
    let bytes = std::fs::read(fixture_path("FiraMono-Medium.ttf")).unwrap();
    let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());

    let sources = vec![
        FontSource::Bytes(Arc::clone(&arc)),
        FontSource::Bytes(Arc::clone(&arc)),
        FontSource::Bytes(Arc::clone(&arc)),
    ];

    let mut fs = FontSystem::new();
    let baseline = baseline_count(&fs);
    let sheet = CompiledStylesheet::parse("");
    let report = load_custom_fonts(&mut fs, &sources, &sheet);

    assert_eq!(report.config_errors, 0);
    assert!(report.config_faces >= 3);
    assert!(fs.db().len() >= baseline + 3);
}

#[test]
fn appconfig_default_has_empty_fonts() {
    let cfg = unshit_app::AppConfig::default();
    assert!(cfg.fonts.is_empty());
}
