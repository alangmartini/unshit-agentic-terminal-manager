//! Text-presentation fallback for emoji-capable symbols.
//!
//! cosmic-text resolves characters the requested family lacks through a
//! fixed per-platform fallback list. On Windows that list tries
//! "Segoe UI Emoji" *before* "Segoe UI Symbol", so symbols that default to
//! text presentation (✳ U+2733, ⏺ U+23FA, ✔ U+2714, ...) resolve to the
//! color-emoji face whenever the UI or terminal font lacks them. The
//! renderer has no color-glyph pipeline: it keeps only the coverage of a
//! color glyph and tints it with the text color, which turns ✳ (a green
//! square with a white asterisk in Segoe UI Emoji) into a solid box. That
//! is the "box instead of ✳" seen in sidebar labels and tab titles when
//! Claude Code reports its status title.
//!
//! [`set_text_with_symbol_fallback`] shapes text normally and, when a glyph
//! from a known color-emoji face covers a single BMP character that is not
//! followed by U+FE0F (an explicit request for emoji presentation), shapes
//! again with those characters pinned to a monochrome symbol face. Explicit
//! emoji sequences and astral-plane emoji are left alone so a future
//! color-glyph path sees them unchanged, and text without such glyphs pays
//! for exactly one shaping pass.

use std::sync::atomic::{AtomicU64, Ordering};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Shaping};

/// Faces the platform fallback list reaches before any monochrome symbol
/// face, and whose color layers the renderer flattens to coverage.
const COLOR_EMOJI_FAMILIES: &[&str] = &["Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji"];

/// Monochrome symbol face to pin text-presentation symbols to. `None`
/// disables the retry and keeps the plain shaping result.
fn text_symbol_family() -> Option<&'static str> {
    #[cfg(target_os = "windows")]
    {
        Some("Segoe UI Symbol")
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Runs re-shaped onto the symbol face since the last drain. Single writer
/// per text pipeline; the renderer folds it into per-frame metrics so the
/// retry rate is queryable from telemetry without logging any text.
static SYMBOL_FALLBACK_RESHAPES: AtomicU64 = AtomicU64::new(0);

/// Drain the number of runs re-shaped onto the symbol face since the
/// previous call.
pub fn take_symbol_fallback_count() -> u64 {
    SYMBOL_FALLBACK_RESHAPES.swap(0, Ordering::Relaxed)
}

/// Whether `id` is one of the color-emoji faces whose glyphs the renderer
/// cannot draw in color.
pub fn is_color_emoji_face(font_system: &FontSystem, id: cosmic_text::fontdb::ID) -> bool {
    font_system.db().face(id).is_some_and(|face| {
        face.families.iter().any(|(name, _)| COLOR_EMOJI_FAMILIES.iter().any(|color| name == color))
    })
}

/// Byte range within one buffer line to pin to the symbol face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolRange {
    line_i: usize,
    start: usize,
    end: usize,
}

/// True when the glyph covering `line[start..end]` is a lone BMP character
/// with no U+FE0F following it: Unicode's text-presentation default, which
/// a monochrome symbol face renders correctly. Clusters (base + variation
/// selector, ZWJ sequences) and astral-plane emoji are left to the emoji
/// face.
fn prefers_text_presentation(line: &str, start: usize, end: usize) -> bool {
    let Some(slice) = line.get(start..end) else {
        return false;
    };
    let mut chars = slice.chars();
    let Some(ch) = chars.next() else {
        return false;
    };
    if u32::from(ch) >= 0x1_0000 || chars.next().is_some() {
        return false;
    }
    line.get(end..).and_then(|rest| rest.chars().next()).is_none_or(|next| next != '\u{FE0F}')
}

/// Shape `text` into `buffer` (replacing [`Buffer::set_text`] followed by
/// [`Buffer::shape_until_scroll`]), retrying text-presentation symbols on a
/// monochrome symbol face when the platform fallback resolved them to a
/// color-emoji face. Returns whether the retry happened.
pub fn set_text_with_symbol_fallback(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    text: &str,
    attrs: Attrs<'_>,
    shaping: Shaping,
) -> bool {
    buffer.set_text(font_system, text, attrs, shaping);
    buffer.shape_until_scroll(font_system, false);

    let Some(symbol_family) = text_symbol_family() else {
        return false;
    };

    let mut ranges: Vec<SymbolRange> = Vec::new();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            if is_color_emoji_face(font_system, glyph.font_id)
                && prefers_text_presentation(run.text, glyph.start, glyph.end)
            {
                ranges.push(SymbolRange { line_i: run.line_i, start: glyph.start, end: glyph.end });
            }
        }
    }
    if ranges.is_empty() {
        return false;
    }
    // Layout runs come in visual order; span building needs source order.
    ranges.sort_unstable();
    ranges.dedup();

    // Rebuild exactly the text cosmic-text split into lines (line text plus
    // its ending) so the retry lays out identically apart from the face.
    let lines: Vec<(String, &'static str)> =
        buffer.lines.iter().map(|line| (line.text().to_string(), line.ending().as_str())).collect();
    let symbol_attrs = attrs.family(Family::Name(symbol_family));
    let mut spans: Vec<(&str, Attrs<'_>)> = Vec::with_capacity(lines.len() + ranges.len() * 2);
    let mut pending = ranges.iter().peekable();
    for (line_i, (line_text, ending)) in lines.iter().enumerate() {
        let mut cursor = 0usize;
        while let Some(range) = pending.next_if(|range| range.line_i == line_i) {
            if range.start < cursor || range.end > line_text.len() {
                continue;
            }
            if range.start > cursor {
                spans.push((&line_text[cursor..range.start], attrs));
            }
            spans.push((&line_text[range.start..range.end], symbol_attrs));
            cursor = range.end;
        }
        if cursor < line_text.len() {
            spans.push((&line_text[cursor..], attrs));
        }
        if !ending.is_empty() {
            spans.push((ending, attrs));
        }
    }

    buffer.set_rich_text(font_system, spans, attrs, shaping);
    buffer.shape_until_scroll(font_system, false);
    SYMBOL_FALLBACK_RESHAPES.fetch_add(1, Ordering::Relaxed);
    log::debug!(
        "{{\"event\":\"text.symbol_fallback\",\"level\":\"debug\",\"symbol_family\":{symbol_family:?},\"glyphs\":{}}}",
        ranges.len()
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_text_presentation_accepts_lone_bmp_symbol() {
        let line = "\u{2733} Workspace";
        assert!(prefers_text_presentation(line, 0, 3));
    }

    #[test]
    fn prefers_text_presentation_rejects_variation_selector_cluster() {
        // VS16 merged into the base glyph's cluster...
        let clustered = "\u{2733}\u{FE0F}";
        assert!(!prefers_text_presentation(clustered, 0, clustered.len()));
        // ...or shaped as its own (zero-width) glyph right after it.
        assert!(!prefers_text_presentation(clustered, 0, 3));
    }

    #[test]
    fn prefers_text_presentation_rejects_astral_and_multichar_glyphs() {
        let astral = "\u{1F7E8}";
        assert!(!prefers_text_presentation(astral, 0, astral.len()));
        let ligature = "fi";
        assert!(!prefers_text_presentation(ligature, 0, 2));
        assert!(!prefers_text_presentation("abc", 5, 9), "out-of-range slice is never retried");
    }

    #[test]
    fn take_symbol_fallback_count_drains() {
        let _ = take_symbol_fallback_count();
        SYMBOL_FALLBACK_RESHAPES.fetch_add(3, Ordering::Relaxed);
        assert!(take_symbol_fallback_count() >= 3);
    }

    #[cfg(target_os = "windows")]
    mod windows {
        use super::*;
        use cosmic_text::Metrics;

        const FONT_SIZE: f32 = 14.0;

        fn shape(font_system: &mut FontSystem, text: &str) -> (Buffer, bool) {
            let mut buffer = Buffer::new(font_system, Metrics::new(FONT_SIZE, FONT_SIZE * 1.4));
            buffer.set_size(font_system, Some(1000.0), None);
            let retried = set_text_with_symbol_fallback(
                &mut buffer,
                font_system,
                text,
                Attrs::new().family(Family::Name("Segoe UI")),
                Shaping::Advanced,
            );
            (buffer, retried)
        }

        /// `(source slice, family of the face that shaped it)` per glyph.
        fn glyph_faces(font_system: &FontSystem, buffer: &Buffer) -> Vec<(String, String)> {
            buffer
                .layout_runs()
                .flat_map(|run| {
                    run.glyphs
                        .iter()
                        .map(|glyph| {
                            let family = font_system
                                .db()
                                .face(glyph.font_id)
                                .and_then(|face| face.families.first().map(|(n, _)| n.clone()))
                                .unwrap_or_default();
                            (run.text[glyph.start..glyph.end].to_string(), family)
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        }

        fn face_for(faces: &[(String, String)], slice: &str) -> String {
            faces
                .iter()
                .find(|(text, _)| text == slice)
                .map(|(_, family)| family.clone())
                .unwrap_or_else(|| panic!("no glyph shaped from {slice:?} in {faces:?}"))
        }

        #[test]
        fn eight_spoked_asterisk_moves_from_emoji_face_to_symbol_face() {
            let mut fs = FontSystem::new();
            let (buffer, retried) = shape(&mut fs, "\u{2733}");
            let faces = glyph_faces(&fs, &buffer);
            assert!(retried, "the color-emoji fallback must trigger the symbol retry");
            assert_eq!(face_for(&faces, "\u{2733}"), "Segoe UI Symbol");
        }

        #[test]
        fn mixed_label_only_moves_the_symbol() {
            let mut fs = FontSystem::new();
            let (buffer, retried) = shape(&mut fs, "\u{2733} Workspace");
            let faces = glyph_faces(&fs, &buffer);
            assert!(retried);
            assert_eq!(face_for(&faces, "\u{2733}"), "Segoe UI Symbol");
            assert_eq!(face_for(&faces, "W"), "Segoe UI", "primary-face glyphs stay put");
        }

        #[test]
        fn explicit_emoji_presentation_keeps_the_emoji_face() {
            let mut fs = FontSystem::new();
            let (buffer, retried) = shape(&mut fs, "\u{2733}\u{FE0F}");
            let faces = glyph_faces(&fs, &buffer);
            assert!(!retried, "VS16 is an explicit request for the emoji face");
            assert!(
                faces.iter().any(|(text, family)| {
                    text.starts_with('\u{2733}') && family == "Segoe UI Emoji"
                }),
                "expected the emoji face for the VS16 sequence, got {faces:?}"
            );
        }

        #[test]
        fn plain_text_and_astral_emoji_shape_once() {
            let mut fs = FontSystem::new();
            let (_, retried) = shape(&mut fs, "abc");
            assert!(!retried);
            let (buffer, retried) = shape(&mut fs, "\u{1F7E8}");
            assert!(!retried, "astral emoji stay on the emoji face for a future color path");
            let faces = glyph_faces(&fs, &buffer);
            assert_eq!(face_for(&faces, "\u{1F7E8}"), "Segoe UI Emoji");
        }

        #[test]
        fn monochrome_glyph_from_the_emoji_face_is_retried_harmlessly() {
            // ◐ has no color layer in Segoe UI Emoji, so it rendered before;
            // the face-level check still moves it, and the symbol face has it.
            let mut fs = FontSystem::new();
            let (buffer, retried) = shape(&mut fs, "\u{25D0} Create PR");
            let faces = glyph_faces(&fs, &buffer);
            assert!(retried);
            assert_eq!(face_for(&faces, "\u{25D0}"), "Segoe UI Symbol");
            assert_eq!(face_for(&faces, "C"), "Segoe UI");
        }

        #[test]
        fn multi_line_text_keeps_line_structure_on_retry() {
            let mut fs = FontSystem::new();
            let (buffer, retried) = shape(&mut fs, "one\n\u{2733} two\nthree");
            assert!(retried);
            assert_eq!(buffer.lines.len(), 3, "line endings must be rebuilt verbatim");
            assert_eq!(buffer.lines[1].text(), "\u{2733} two");
            let faces = glyph_faces(&fs, &buffer);
            assert_eq!(face_for(&faces, "\u{2733}"), "Segoe UI Symbol");
            assert_eq!(face_for(&faces, "t"), "Segoe UI");
        }
    }
}
