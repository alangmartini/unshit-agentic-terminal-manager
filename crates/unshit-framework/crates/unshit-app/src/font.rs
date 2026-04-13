//! Custom font registration at app startup.
//!
//! Consumers can ship bundled typefaces or reference CSS `@font-face` entries
//! through [`AppConfig::fonts`](crate::AppConfig). All loading happens exactly
//! once inside `App::can_create_surfaces`, so there is no per-frame cost.
//!
//! Unresolvable entries (missing files, corrupt bytes) are logged at warn level
//! and skipped. A single bad entry never aborts startup.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cosmic_text::{fontdb, FontSystem};
use unshit_core::style::parse::{CompiledStylesheet, FontFaceRule, FontFaceSrc};

/// Declarative source of a custom font registered at app startup.
///
/// Every variant is loaded exactly once during `can_create_surfaces`, just
/// after [`cosmic_text::FontSystem::new`] and before the first layout pass.
///
/// # Ordering
///
/// Programmatic entries in `AppConfig::fonts` load before any CSS
/// `@font-face` rules. fontdb assigns each parsed face a fresh `ID`, so
/// duplicate family names coexist (cosmic-text resolves by face attributes,
/// not the family string alone).
#[derive(Clone)]
pub enum FontSource {
    /// Raw font bytes owned by an `Arc<[u8]>`. Zero copy, shared with fontdb.
    Bytes(Arc<[u8]>),
    /// File path to a font file. Relative paths resolve against the current
    /// working directory at load time.
    Path(PathBuf),
    /// Name of an already installed system family. Recorded for completeness
    /// and for the future fallback chain wiring. No-op at load time.
    System(String),
}

impl std::fmt::Debug for FontSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bytes(b) => {
                f.debug_tuple("Bytes").field(&format_args!("{} bytes", b.len())).finish()
            }
            Self::Path(p) => f.debug_tuple("Path").field(p).finish(),
            Self::System(name) => f.debug_tuple("System").field(name).finish(),
        }
    }
}

/// Adapter wrapping an `Arc<[u8]>` so it can flow into fontdb's
/// `Source::Binary`, which expects `Arc<dyn AsRef<[u8]> + Sync + Send>`.
///
/// This is a zero cost conversion: no bytes are copied, we only widen the
/// trait object type on the `Arc`.
struct ArcSliceFontData(Arc<[u8]>);

impl AsRef<[u8]> for ArcSliceFontData {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Ordered list of font family names for glyph fallback.
/// When the primary font cannot render a glyph, the framework
/// tries each family in order until one provides the glyph.
#[derive(Clone, Debug, Default)]
pub struct FallbackChain {
    pub families: Vec<String>,
}

impl FallbackChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sensible default chain: system emoji, CJK, and monospace fallbacks.
    pub fn default_chain() -> Self {
        Self {
            families: vec![
                "Noto Color Emoji".into(),
                "Segoe UI Emoji".into(),
                "Apple Color Emoji".into(),
                "Noto Sans CJK SC".into(),
                "Noto Sans".into(),
            ],
        }
    }

    pub fn with_family(mut self, family: impl Into<String>) -> Self {
        self.families.push(family.into());
        self
    }
}

/// Check which families from the fallback chain are available in the font
/// database. Logs a warning for any family not found among the loaded faces.
pub fn check_fallback_chain(font_system: &FontSystem, chain: &FallbackChain) {
    let db = font_system.db();
    for family in &chain.families {
        let found = db
            .faces()
            .any(|face| face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)));
        if !found {
            log::debug!(
                "FallbackChain: family {:?} not found in fontdb (emoji/CJK glyphs may not render)",
                family
            );
        }
    }
}

/// Track how many faces were loaded, for test assertions and debug logs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FontLoadReport {
    /// Number of faces registered from `AppConfig::fonts`.
    pub config_faces: usize,
    /// Number of faces registered from `@font-face` CSS rules.
    pub css_faces: usize,
    /// Number of `AppConfig::fonts` entries that failed to load.
    pub config_errors: usize,
    /// Number of `@font-face` entries that failed to load.
    pub css_errors: usize,
}

/// Load every custom font source into the given [`FontSystem`].
///
/// Programmatic entries from `config_fonts` run first, followed by
/// `@font-face` rules from the compiled stylesheet. Failures are logged and
/// skipped: this function never panics.
pub fn load_custom_fonts(
    font_system: &mut FontSystem,
    config_fonts: &[FontSource],
    stylesheet: &CompiledStylesheet,
) -> FontLoadReport {
    let mut report = FontLoadReport::default();
    let db = font_system.db_mut();

    for source in config_fonts {
        match source {
            FontSource::Bytes(bytes) => {
                if bytes.is_empty() {
                    log::warn!("FontSource::Bytes is empty, skipping");
                    report.config_errors += 1;
                    continue;
                }
                let adapter: Arc<dyn AsRef<[u8]> + Sync + Send> =
                    Arc::new(ArcSliceFontData(Arc::clone(bytes)));
                let ids = db.load_font_source(fontdb::Source::Binary(adapter));
                if ids.is_empty() {
                    log::warn!(
                        "FontSource::Bytes: fontdb could not parse any face from {} bytes",
                        bytes.len()
                    );
                    report.config_errors += 1;
                } else {
                    report.config_faces += ids.len();
                }
            }
            FontSource::Path(path) => {
                let ids = load_path_into_db(db, path);
                if ids == 0 {
                    report.config_errors += 1;
                } else {
                    report.config_faces += ids;
                }
            }
            FontSource::System(name) => {
                log::debug!("FontSource::System({:?}) recorded, no-op at load time", name);
            }
        }
    }

    for rule in &stylesheet.font_faces {
        match load_font_face_rule(db, rule) {
            Ok(n) if n > 0 => report.css_faces += n,
            Ok(_) => report.css_errors += 1,
            Err(msg) => {
                log::warn!("@font-face {{ font-family: {:?}; ... }}: {}", rule.family, msg);
                report.css_errors += 1;
            }
        }
    }

    report
}

fn load_path_into_db(db: &mut fontdb::Database, path: &Path) -> usize {
    match std::fs::read(path) {
        Ok(data) => {
            let adapter: Arc<dyn AsRef<[u8]> + Sync + Send> = Arc::new(data);
            let ids = db.load_font_source(fontdb::Source::Binary(adapter));
            if ids.is_empty() {
                log::warn!("FontSource::Path({}): fontdb could not parse any face", path.display());
                0
            } else {
                ids.len()
            }
        }
        Err(e) => {
            log::warn!("FontSource::Path({}): failed to read file: {}", path.display(), e);
            0
        }
    }
}

fn load_font_face_rule(db: &mut fontdb::Database, rule: &FontFaceRule) -> Result<usize, String> {
    match &rule.src {
        FontFaceSrc::Url(url) => {
            if url.starts_with("data:") {
                return Err("data: URIs are not supported in @font-face src".into());
            }
            let path = PathBuf::from(url);
            let count = load_path_into_db(db, &path);
            Ok(count)
        }
        FontFaceSrc::Local(name) => {
            log::debug!(
                "@font-face src: local({:?}) recorded for family {:?}, no-op at load time",
                name,
                rule.family
            );
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
    }

    /// Build a FontSystem with no system font scan (we still get whatever the
    /// default constructor discovers), and return the initial face count so
    /// tests can assert against deltas.
    fn system_with_baseline() -> (FontSystem, usize) {
        let fs = FontSystem::new();
        let n = fs.db().len();
        (fs, n)
    }

    #[test]
    fn font_source_bytes_loads_faces() {
        let bytes = std::fs::read(fixture_path("FiraMono-Medium.ttf"))
            .expect("fixture FiraMono-Medium.ttf must exist");
        let src = FontSource::Bytes(Arc::from(bytes.into_boxed_slice()));

        let (mut fs, baseline) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, std::slice::from_ref(&src), &stylesheet);

        assert_eq!(report.config_errors, 0, "bytes load should not fail");
        assert!(report.config_faces >= 1, "at least one face should load from FiraMono");
        assert!(fs.db().len() > baseline, "fontdb should grow after loading bytes");
    }

    #[test]
    fn font_source_path_loads_faces() {
        let src = FontSource::Path(fixture_path("FiraMono-Medium.ttf"));
        let (mut fs, baseline) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, std::slice::from_ref(&src), &stylesheet);

        assert_eq!(report.config_errors, 0);
        assert!(report.config_faces >= 1);
        assert!(fs.db().len() > baseline);
    }

    #[test]
    fn font_source_missing_path_warns_does_not_panic() {
        let src =
            FontSource::Path(PathBuf::from("this/path/definitely/does/not/exist/phantom.ttf"));
        let (mut fs, baseline) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, std::slice::from_ref(&src), &stylesheet);

        assert_eq!(report.config_errors, 1);
        assert_eq!(report.config_faces, 0);
        assert_eq!(fs.db().len(), baseline, "missing file must not affect fontdb");
    }

    #[test]
    fn font_source_invalid_bytes_warns_does_not_panic() {
        let garbage: Arc<[u8]> =
            Arc::from(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01].into_boxed_slice());
        let src = FontSource::Bytes(garbage);
        let (mut fs, baseline) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, std::slice::from_ref(&src), &stylesheet);

        assert_eq!(report.config_errors, 1);
        assert_eq!(report.config_faces, 0);
        assert_eq!(fs.db().len(), baseline, "garbage bytes must not affect fontdb");
    }

    #[test]
    fn font_source_empty_bytes_warns() {
        let empty: Arc<[u8]> = Arc::from(Vec::new().into_boxed_slice());
        let src = FontSource::Bytes(empty);
        let (mut fs, _) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, std::slice::from_ref(&src), &stylesheet);

        assert_eq!(report.config_errors, 1);
        assert_eq!(report.config_faces, 0);
    }

    #[test]
    fn font_source_system_is_noop_at_load_time() {
        let src = FontSource::System("Helvetica".to_string());
        let (mut fs, baseline) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, std::slice::from_ref(&src), &stylesheet);

        assert_eq!(report.config_errors, 0);
        assert_eq!(report.config_faces, 0);
        assert_eq!(fs.db().len(), baseline);
    }

    #[test]
    fn duplicate_font_sources_both_register() {
        let bytes = std::fs::read(fixture_path("FiraMono-Medium.ttf")).unwrap();
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());

        let sources =
            vec![FontSource::Bytes(Arc::clone(&arc)), FontSource::Bytes(Arc::clone(&arc))];

        let (mut fs, baseline) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, &sources, &stylesheet);

        assert_eq!(report.config_errors, 0);
        assert!(report.config_faces >= 2, "both registrations should land");
        assert!(fs.db().len() >= baseline + 2);
    }

    #[test]
    fn empty_fonts_vec_is_noop() {
        let (mut fs, baseline) = system_with_baseline();
        let stylesheet = CompiledStylesheet::parse("");
        let report = load_custom_fonts(&mut fs, &[], &stylesheet);

        assert_eq!(report, FontLoadReport::default());
        assert_eq!(fs.db().len(), baseline);
    }

    #[test]
    fn fallback_chain_new_is_empty() {
        let chain = FallbackChain::new();
        assert!(chain.families.is_empty(), "FallbackChain::new() must have no families");
    }

    #[test]
    fn fallback_chain_default_chain_is_nonempty() {
        let chain = FallbackChain::default_chain();
        assert!(
            !chain.families.is_empty(),
            "FallbackChain::default_chain() must have at least one family"
        );
    }

    #[test]
    fn fallback_chain_with_family_appends() {
        let chain =
            FallbackChain::new().with_family("Noto Color Emoji").with_family("Noto Sans CJK SC");
        assert_eq!(chain.families.len(), 2);
        assert_eq!(chain.families[0], "Noto Color Emoji");
        assert_eq!(chain.families[1], "Noto Sans CJK SC");
    }
}
