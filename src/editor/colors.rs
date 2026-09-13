//! The editor's cell palette, derived from the active theme.
//!
//! Editor grids are published to the renderer raw — unlike terminal
//! grids they never pass through [`crate::theme::apply_terminal_palette_to_grid`],
//! which only remaps exact matches of the 16 ANSI colours and the amber
//! source default. So the editor has to resolve real colours at paint
//! time instead of painting sentinel values and having them rewritten.
//!
//! Roles follow the conventions the stylesheet already uses (there are no
//! named success/error tokens in this codebase): `ansi[1]`/`--rust` is
//! red — errors and removed lines; `ansi[2]`/`--sage` is green — success
//! and added lines; `ansi[6]`/`--azure` is blue — info and modified;
//! `ansi[3]` is yellow — search matches; `ansi[8]` is dim grey — gutters
//! and comments. `src/ui/settings.rs`'s appearance preview derives its
//! swatches the same way, so the editor and the settings preview agree.

use unshit::core::style::types::Color;

use crate::syntax::TokenKind;
use crate::theme::TerminalPalette;

/// Every colour the editor and diff panes paint into cells.
///
/// Backgrounds are stored with real alpha rather than pre-blended against
/// the pane background: the renderer skips fully transparent cell
/// backgrounds and alpha-blends the rest, so a tint reads as a tint over
/// whatever the theme puts behind the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorColors {
    pub text: Color,
    pub gutter: Color,
    pub selection_bg: Color,
    /// Highlight for the line holding the cursor, painted only when
    /// nothing is selected so it never fights the selection.
    pub current_line_bg: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub punct: Color,
    pub find_bg: Color,
    pub find_current_bg: Color,
    pub diff_added_bg: Color,
    pub diff_removed_bg: Color,
    /// Colour of the `+` / `-` marker column and of a file header's
    /// status letter.
    pub diff_added_fg: Color,
    pub diff_removed_fg: Color,
    pub diff_hunk_fg: Color,
    pub diff_hunk_bg: Color,
    pub diff_file_fg: Color,
    pub diff_file_bg: Color,
    pub diff_meta_fg: Color,
}

/// Selection background (VS Code dark `#264f78`), the same constant the
/// terminal uses (`crate::state::SELECTION_BG`). Deliberately not themed:
/// a selection has to stay legible under every palette, and users read it
/// as "the editor selection colour" across apps.
pub const SELECTION_BG: Color = Color {
    r: 0x26,
    g: 0x4f,
    b: 0x78,
    a: 0xff,
};

/// `base` with its alpha replaced, for tints painted over the pane
/// background.
const fn alpha(base: Color, a: u8) -> Color {
    Color {
        r: base.r,
        g: base.g,
        b: base.b,
        a,
    }
}

/// Mix `a` and `b` in sRGB by `weight`/255 of `b`. Good enough for
/// deriving a dimmer punctuation colour; the renderer blends in sRGB too
/// (`Color::to_linear_f32`), so this matches what the eye sees there.
fn mix(a: Color, b: Color, weight: u8) -> Color {
    let w = weight as u16;
    let inv = 255 - w;
    let chan = |x: u8, y: u8| (((x as u16) * inv + (y as u16) * w) / 255) as u8;
    Color {
        r: chan(a.r, b.r),
        g: chan(a.g, b.g),
        b: chan(a.b, b.b),
        a: a.a,
    }
}

impl EditorColors {
    /// Derive the editor palette from a theme's terminal palette.
    pub fn from_palette(palette: &TerminalPalette) -> Self {
        let fg = palette.default_fg;
        let bg = palette.default_bg;
        let red = palette.ansi[1];
        let green = palette.ansi[2];
        let yellow = palette.ansi[3];
        let blue = palette.ansi[6];
        let dim = palette.ansi[8];
        Self {
            text: fg,
            gutter: dim,
            selection_bg: SELECTION_BG,
            // Faint enough to survive under a syntax-coloured line; the
            // cursor row is a locator, not a highlight.
            current_line_bg: alpha(fg, 16),
            keyword: red,
            string: green,
            number: blue,
            comment: dim,
            // Punctuation recedes behind identifiers without becoming
            // comment-coloured, which would read as "this is inert".
            punct: mix(fg, bg, 64),
            find_bg: alpha(yellow, 72),
            find_current_bg: alpha(yellow, 140),
            diff_added_bg: alpha(green, 40),
            diff_removed_bg: alpha(red, 40),
            diff_added_fg: green,
            diff_removed_fg: red,
            diff_hunk_fg: blue,
            diff_hunk_bg: alpha(fg, 13),
            diff_file_fg: fg,
            diff_file_bg: alpha(fg, 20),
            diff_meta_fg: dim,
        }
    }

    /// Resolve the palette for a theme id plus the user's custom slots,
    /// exactly as terminal panes do.
    pub fn for_theme(theme: &str, custom: &crate::theme::CustomTheme) -> Self {
        Self::from_palette(&crate::theme::terminal_palette_for(theme, custom))
    }

    /// Foreground for a syntax token. Identifiers and whitespace take the
    /// plain text colour so an unknown language costs nothing.
    pub fn token(&self, kind: TokenKind) -> Color {
        match kind {
            TokenKind::Keyword => self.keyword,
            TokenKind::Str => self.string,
            TokenKind::Number => self.number,
            TokenKind::Comment => self.comment,
            TokenKind::Punct => self.punct,
            TokenKind::Ident | TokenKind::Ws => self.text,
        }
    }
}

impl Default for EditorColors {
    /// The default (amber) theme's palette. Panes are recoloured when the
    /// theme changes, so this only decides what a pane looks like before
    /// its first theme sync — and in tests.
    fn default() -> Self {
        Self::from_palette(crate::theme::terminal_palette("amber"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every role must be filled from the theme, not left at a hardcoded
    /// white: the editor rendered white-on-theme-background before this
    /// existed, which looked wrong under every non-amber theme.
    #[test]
    fn colors_follow_the_theme_palette() {
        let amber = EditorColors::from_palette(crate::theme::terminal_palette("amber"));
        let nord = EditorColors::from_palette(crate::theme::terminal_palette("nord"));
        assert_ne!(
            amber.text, nord.text,
            "text colour must differ between themes"
        );
        assert_ne!(amber.keyword, nord.keyword);
        assert_eq!(
            amber.selection_bg, SELECTION_BG,
            "selection stays fixed across themes"
        );
    }

    /// Tints are painted over the pane background, so they must carry
    /// partial alpha — an opaque diff row would hide the theme and an
    /// alpha of 0 would be skipped by the renderer entirely.
    #[test]
    fn row_tints_are_translucent_but_visible() {
        let c = EditorColors::default();
        for (name, color) in [
            ("current_line", c.current_line_bg),
            ("find", c.find_bg),
            ("find_current", c.find_current_bg),
            ("diff_added", c.diff_added_bg),
            ("diff_removed", c.diff_removed_bg),
            ("diff_hunk", c.diff_hunk_bg),
            ("diff_file", c.diff_file_bg),
        ] {
            assert!(
                color.a > 0 && color.a < 255,
                "{name} background must be a translucent tint, got alpha {}",
                color.a
            );
        }
        assert!(
            c.find_current_bg.a > c.find_bg.a,
            "the current match must stand out from the other matches"
        );
    }

    /// Token colours must be distinguishable, or highlighting is noise.
    #[test]
    fn token_roles_map_to_distinct_colors() {
        let c = EditorColors::default();
        assert_eq!(c.token(TokenKind::Ident), c.text);
        assert_eq!(c.token(TokenKind::Ws), c.text);
        assert_ne!(c.token(TokenKind::Keyword), c.text);
        assert_ne!(c.token(TokenKind::Str), c.token(TokenKind::Keyword));
        assert_ne!(c.token(TokenKind::Number), c.token(TokenKind::Str));
        assert_ne!(c.token(TokenKind::Comment), c.text);
        assert_ne!(
            c.token(TokenKind::Punct),
            c.token(TokenKind::Comment),
            "punctuation must not read as a comment"
        );
    }

    #[test]
    fn mix_interpolates_towards_the_second_color() {
        let white = Color {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        };
        let black = Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        assert_eq!(mix(white, black, 0), white);
        let half = mix(white, black, 128);
        assert!(half.r > 100 && half.r < 155, "got {}", half.r);
        assert_eq!(half.a, 255, "alpha comes from the first colour");
    }
}
