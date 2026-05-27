use unshit::core::cell_grid::{Cell, CellAttrs, CellGrid, ANSI_16};
use unshit::core::style::types::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalPalette {
    pub default_fg: Color,
    pub default_bg: Color,
    pub ansi: [Color; 16],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CustomTheme {
    pub accent: Color,
    pub accent_soft: Color,
    pub background: Color,
    pub surface: Color,
    pub foreground: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CustomThemeSlot {
    Accent,
    AccentSoft,
    Background,
    Surface,
    Foreground,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub meta: &'static str,
    pub swatches: [&'static str; 5],
    pub terminal: TerminalPalette,
}

const SOURCE_DEFAULT_FG: Color = rgb(212, 163, 72);
pub const CUSTOM_THEME_ID: &str = "custom";

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r, g, b, a: 255 }
}

const CATPPUCCIN_ANSI: [Color; 16] = [
    rgb(69, 71, 90),
    rgb(243, 139, 168),
    rgb(166, 227, 161),
    rgb(249, 226, 175),
    rgb(137, 180, 250),
    rgb(245, 194, 231),
    rgb(148, 226, 213),
    rgb(186, 194, 222),
    rgb(88, 91, 112),
    rgb(243, 139, 168),
    rgb(166, 227, 161),
    rgb(249, 226, 175),
    rgb(137, 180, 250),
    rgb(245, 194, 231),
    rgb(148, 226, 213),
    rgb(205, 214, 244),
];

const TOKYO_NIGHT_ANSI: [Color; 16] = [
    rgb(65, 72, 104),
    rgb(247, 118, 142),
    rgb(158, 206, 106),
    rgb(224, 175, 104),
    rgb(122, 162, 247),
    rgb(187, 154, 247),
    rgb(125, 207, 255),
    rgb(169, 177, 214),
    rgb(86, 95, 137),
    rgb(247, 118, 142),
    rgb(158, 206, 106),
    rgb(224, 175, 104),
    rgb(122, 162, 247),
    rgb(187, 154, 247),
    rgb(125, 207, 255),
    rgb(192, 202, 245),
];

const NORD_ANSI: [Color; 16] = [
    rgb(59, 66, 82),
    rgb(191, 97, 106),
    rgb(163, 190, 140),
    rgb(235, 203, 139),
    rgb(129, 161, 193),
    rgb(180, 142, 173),
    rgb(136, 192, 208),
    rgb(229, 233, 240),
    rgb(76, 86, 106),
    rgb(191, 97, 106),
    rgb(163, 190, 140),
    rgb(235, 203, 139),
    rgb(94, 129, 172),
    rgb(180, 142, 173),
    rgb(143, 188, 187),
    rgb(236, 239, 244),
];

const DRACULA_ANSI: [Color; 16] = [
    rgb(33, 34, 44),
    rgb(255, 85, 85),
    rgb(80, 250, 123),
    rgb(241, 250, 140),
    rgb(189, 147, 249),
    rgb(255, 121, 198),
    rgb(139, 233, 253),
    rgb(248, 248, 242),
    rgb(98, 114, 164),
    rgb(255, 110, 110),
    rgb(105, 255, 148),
    rgb(255, 255, 165),
    rgb(214, 172, 255),
    rgb(255, 146, 223),
    rgb(164, 255, 255),
    rgb(255, 255, 255),
];

const EVERFOREST_ANSI: [Color; 16] = [
    rgb(71, 82, 88),
    rgb(230, 126, 128),
    rgb(167, 192, 128),
    rgb(219, 188, 127),
    rgb(127, 187, 179),
    rgb(214, 153, 182),
    rgb(131, 192, 146),
    rgb(211, 198, 170),
    rgb(133, 146, 137),
    rgb(230, 126, 128),
    rgb(167, 192, 128),
    rgb(219, 188, 127),
    rgb(127, 187, 179),
    rgb(214, 153, 182),
    rgb(131, 192, 146),
    rgb(211, 198, 170),
];

const ROSE_PINE_ANSI: [Color; 16] = [
    rgb(57, 53, 82),
    rgb(235, 111, 146),
    rgb(62, 143, 176),
    rgb(246, 193, 119),
    rgb(49, 116, 143),
    rgb(196, 167, 231),
    rgb(156, 207, 216),
    rgb(224, 222, 244),
    rgb(110, 106, 134),
    rgb(235, 111, 146),
    rgb(62, 143, 176),
    rgb(246, 193, 119),
    rgb(156, 207, 216),
    rgb(196, 167, 231),
    rgb(156, 207, 216),
    rgb(224, 222, 244),
];

const GRUVBOX_ANSI: [Color; 16] = [
    rgb(40, 40, 40),
    rgb(204, 36, 29),
    rgb(152, 151, 26),
    rgb(215, 153, 33),
    rgb(69, 133, 136),
    rgb(177, 98, 134),
    rgb(104, 157, 106),
    rgb(168, 153, 132),
    rgb(146, 131, 116),
    rgb(251, 73, 52),
    rgb(184, 187, 38),
    rgb(250, 189, 47),
    rgb(131, 165, 152),
    rgb(211, 134, 155),
    rgb(142, 192, 124),
    rgb(235, 219, 178),
];

const KANAGAWA_ANSI: [Color; 16] = [
    rgb(22, 22, 29),
    rgb(228, 104, 118),
    rgb(152, 187, 108),
    rgb(230, 195, 132),
    rgb(126, 156, 216),
    rgb(149, 127, 184),
    rgb(127, 180, 202),
    rgb(196, 167, 127),
    rgb(114, 113, 105),
    rgb(228, 104, 118),
    rgb(152, 187, 108),
    rgb(230, 195, 132),
    rgb(126, 156, 216),
    rgb(149, 127, 184),
    rgb(127, 180, 202),
    rgb(220, 215, 186),
];

const THEMES: &[ThemeSpec] = &[
    ThemeSpec {
        id: "amber",
        label: "Amber",
        description: "Original warm amber terminal palette.",
        meta: "ember on walnut · default",
        swatches: ["#d4a348", "#8ba85c", "#c9553a", "#6aa2ad", "#a88bb8"],
        terminal: TerminalPalette {
            default_fg: SOURCE_DEFAULT_FG,
            default_bg: Color::TRANSPARENT,
            ansi: ANSI_16,
        },
    },
    ThemeSpec {
        id: "catppuccin",
        label: "Catppuccin",
        description: "Mocha-inspired pastel terminal palette.",
        meta: "mocha · soothing pastels",
        swatches: ["#cba6f7", "#a6e3a1", "#f38ba8", "#89b4fa", "#fab387"],
        terminal: TerminalPalette {
            default_fg: rgb(205, 214, 244),
            default_bg: Color::TRANSPARENT,
            ansi: CATPPUCCIN_ANSI,
        },
    },
    ThemeSpec {
        id: "tokyo-night",
        label: "Tokyo Night",
        description: "Deep navy palette with clean blue and violet accents.",
        meta: "neon-lit nightscape",
        swatches: ["#7aa2f7", "#9ece6a", "#f7768e", "#7dcfff", "#bb9af7"],
        terminal: TerminalPalette {
            default_fg: rgb(192, 202, 245),
            default_bg: Color::TRANSPARENT,
            ansi: TOKYO_NIGHT_ANSI,
        },
    },
    ThemeSpec {
        id: "nord",
        label: "Nord",
        description: "Arctic, low-clutter palette for focused sessions.",
        meta: "arctic · frost & aurora",
        swatches: ["#88c0d0", "#a3be8c", "#bf616a", "#81a1c1", "#b48ead"],
        terminal: TerminalPalette {
            default_fg: rgb(236, 239, 244),
            default_bg: Color::TRANSPARENT,
            ansi: NORD_ANSI,
        },
    },
    ThemeSpec {
        id: "dracula",
        label: "Dracula",
        description: "High-contrast classic with vivid ANSI colors.",
        meta: "high contrast · neon vampire",
        swatches: ["#bd93f9", "#50fa7b", "#ff5555", "#8be9fd", "#ff79c6"],
        terminal: TerminalPalette {
            default_fg: rgb(248, 248, 242),
            default_bg: Color::TRANSPARENT,
            ansi: DRACULA_ANSI,
        },
    },
    ThemeSpec {
        id: "everforest",
        label: "Everforest",
        description: "Muted green and warm neutrals for long-running work.",
        meta: "dark hard · mossy forest",
        swatches: ["#a7c080", "#e67e80", "#7fbbb3", "#dbbc7f", "#d699b6"],
        terminal: TerminalPalette {
            default_fg: rgb(211, 198, 170),
            default_bg: Color::TRANSPARENT,
            ansi: EVERFOREST_ANSI,
        },
    },
    ThemeSpec {
        id: "rose-pine",
        label: "Rosé Pine",
        description: "Moon variant with soft rose, pine, and iris tones.",
        meta: "soho cottage · all natural",
        swatches: ["#ebbcba", "#c4a7e7", "#eb6f92", "#9ccfd8", "#f6c177"],
        terminal: TerminalPalette {
            default_fg: rgb(224, 222, 244),
            default_bg: Color::TRANSPARENT,
            ansi: ROSE_PINE_ANSI,
        },
    },
    ThemeSpec {
        id: "gruvbox",
        label: "Gruvbox",
        description: "Retro warm dark-hard palette.",
        meta: "retro warm · dark hard",
        swatches: ["#fabd2f", "#b8bb26", "#fb4934", "#83a598", "#fe8019"],
        terminal: TerminalPalette {
            default_fg: rgb(235, 219, 178),
            default_bg: Color::TRANSPARENT,
            ansi: GRUVBOX_ANSI,
        },
    },
    ThemeSpec {
        id: "kanagawa",
        label: "Kanagawa",
        description: "Muted ink palette inspired by the great wave.",
        meta: "the great wave · muted ink",
        swatches: ["#7e9cd8", "#98bb6c", "#e46876", "#7fb4ca", "#957fb8"],
        terminal: TerminalPalette {
            default_fg: rgb(220, 215, 186),
            default_bg: Color::TRANSPARENT,
            ansi: KANAGAWA_ANSI,
        },
    },
];

pub fn default_custom_theme() -> CustomTheme {
    CustomTheme {
        accent: rgb(125, 211, 252),
        accent_soft: rgb(165, 243, 252),
        background: rgb(14, 22, 32),
        surface: rgb(22, 32, 44),
        foreground: rgb(230, 237, 243),
    }
}

impl CustomThemeSlot {
    pub fn id(self) -> &'static str {
        match self {
            CustomThemeSlot::Accent => "accent",
            CustomThemeSlot::AccentSoft => "accent-soft",
            CustomThemeSlot::Background => "background",
            CustomThemeSlot::Surface => "surface",
            CustomThemeSlot::Foreground => "foreground",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CustomThemeSlot::Accent => "Accent",
            CustomThemeSlot::AccentSoft => "Accent soft",
            CustomThemeSlot::Background => "Background",
            CustomThemeSlot::Surface => "Surface",
            CustomThemeSlot::Foreground => "Foreground",
        }
    }

    pub fn from_id(raw: &str) -> Option<Self> {
        match raw {
            "accent" => Some(CustomThemeSlot::Accent),
            "accent-soft" | "accent_soft" => Some(CustomThemeSlot::AccentSoft),
            "background" => Some(CustomThemeSlot::Background),
            "surface" => Some(CustomThemeSlot::Surface),
            "foreground" => Some(CustomThemeSlot::Foreground),
            _ => None,
        }
    }
}

pub fn custom_theme_slots() -> &'static [CustomThemeSlot] {
    &[
        CustomThemeSlot::Accent,
        CustomThemeSlot::AccentSoft,
        CustomThemeSlot::Background,
        CustomThemeSlot::Surface,
        CustomThemeSlot::Foreground,
    ]
}

pub fn custom_theme_color(theme: &CustomTheme, slot: CustomThemeSlot) -> Color {
    match slot {
        CustomThemeSlot::Accent => theme.accent,
        CustomThemeSlot::AccentSoft => theme.accent_soft,
        CustomThemeSlot::Background => theme.background,
        CustomThemeSlot::Surface => theme.surface,
        CustomThemeSlot::Foreground => theme.foreground,
    }
}

pub fn set_custom_theme_color(theme: &mut CustomTheme, slot: CustomThemeSlot, color: Color) {
    match slot {
        CustomThemeSlot::Accent => theme.accent = color,
        CustomThemeSlot::AccentSoft => theme.accent_soft = color,
        CustomThemeSlot::Background => theme.background = color,
        CustomThemeSlot::Surface => theme.surface = color,
        CustomThemeSlot::Foreground => theme.foreground = color,
    }
}

fn normalize_theme_id(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub fn themes() -> &'static [ThemeSpec] {
    THEMES
}

pub fn default_theme_id() -> &'static str {
    THEMES[0].id
}

pub fn resolve_theme_id(raw: &str) -> &'static str {
    let requested = normalize_theme_id(raw);
    if requested == CUSTOM_THEME_ID {
        return CUSTOM_THEME_ID;
    }
    THEMES
        .iter()
        .find(|theme| theme.id == requested)
        .map_or(default_theme_id(), |theme| theme.id)
}

pub fn theme_spec(raw: &str) -> &'static ThemeSpec {
    let id = resolve_theme_id(raw);
    THEMES
        .iter()
        .find(|theme| theme.id == id)
        .unwrap_or(&THEMES[0])
}

pub fn terminal_palette(raw: &str) -> &'static TerminalPalette {
    &theme_spec(raw).terminal
}

pub fn terminal_palette_for(raw: &str, custom: &CustomTheme) -> TerminalPalette {
    if resolve_theme_id(raw) == CUSTOM_THEME_ID {
        custom_terminal_palette(custom)
    } else {
        *terminal_palette(raw)
    }
}

pub fn theme_class_name(raw: &str) -> String {
    format!("theme-{}", resolve_theme_id(raw))
}

pub fn color_to_hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

pub fn parse_hex_color(raw: &str) -> Option<Color> {
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(rgb(r, g, b))
}

fn custom_terminal_palette(custom: &CustomTheme) -> TerminalPalette {
    let bg = custom.background;
    let surface = custom.surface;
    let accent = custom.accent;
    let accent_soft = custom.accent_soft;
    let fg = custom.foreground;
    TerminalPalette {
        default_fg: fg,
        default_bg: Color::TRANSPARENT,
        ansi: [
            bg,
            rgb(228, 104, 118),
            rgb(152, 187, 108),
            accent_soft,
            accent,
            rgb(149, 127, 184),
            accent_soft,
            fg,
            surface,
            rgb(255, 120, 130),
            rgb(170, 210, 120),
            accent_soft,
            accent,
            rgb(185, 150, 230),
            accent_soft,
            fg,
        ],
    }
}

fn map_terminal_fg(color: Color, palette: &TerminalPalette) -> Color {
    if color == SOURCE_DEFAULT_FG {
        return palette.default_fg;
    }

    for (index, source) in ANSI_16.iter().enumerate() {
        if color == *source {
            return palette.ansi[index];
        }
    }

    color
}

fn map_terminal_bg(color: Color, palette: &TerminalPalette) -> Color {
    if color.a == 0 {
        return color;
    }

    for (index, source) in ANSI_16.iter().enumerate() {
        if color == *source {
            return palette.ansi[index];
        }
    }

    color
}

fn map_terminal_cell(cell: Cell, palette: &TerminalPalette) -> Cell {
    let mut themed = cell;
    let inverse = cell.attrs.contains(CellAttrs::INVERSE);
    if !cell.is_empty() || inverse {
        themed.fg = map_terminal_fg(cell.fg, palette);
    }
    if cell.bg.a != 0 || inverse {
        themed.bg = map_terminal_bg(cell.bg, palette);
    }
    themed
}

pub fn apply_terminal_theme_to_grid(grid: &mut CellGrid, theme_id: &str) {
    let palette = terminal_palette(theme_id);
    apply_terminal_palette_to_grid(grid, palette);
}

pub fn apply_terminal_palette_to_grid(grid: &mut CellGrid, palette: &TerminalPalette) {
    for row in 0..grid.rows() {
        for col in 0..grid.cols() {
            let Some(cell) = grid.get_cell(row, col).copied() else {
                continue;
            };
            let themed = map_terminal_cell(cell, palette);
            if themed != cell {
                grid.set_cell(row, col, themed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<&'static str> {
        themes().iter().map(|theme| theme.id).collect()
    }

    #[test]
    fn catalog_includes_requested_theme_examples() {
        let ids = ids();

        assert!(
            ids.contains(&"amber"),
            "catalog should preserve the original Amber theme"
        );
        assert!(
            ids.contains(&"catppuccin"),
            "catalog should include Catppuccin"
        );
        assert!(
            ids.contains(&"tokyo-night"),
            "catalog should include Tokyo Night"
        );
        assert!(ids.contains(&"dracula"), "catalog should include Dracula");
        assert!(ids.contains(&"gruvbox"), "catalog should include Gruvbox");
        assert!(ids.contains(&"kanagawa"), "catalog should include Kanagawa");
    }

    #[test]
    fn unknown_theme_resolves_to_default() {
        assert_eq!(resolve_theme_id("does not exist"), default_theme_id());
        assert_eq!(theme_class_name("does not exist"), "theme-amber");
        assert_eq!(resolve_theme_id("custom"), CUSTOM_THEME_ID);
        assert_eq!(theme_class_name("custom"), "theme-custom");
    }

    #[test]
    fn terminal_palette_maps_default_and_ansi_colors() {
        let palette = terminal_palette("dracula");
        let default_cell = Cell {
            ch: 'D',
            fg: SOURCE_DEFAULT_FG,
            ..Cell::default()
        };
        let red_cell = Cell {
            ch: 'R',
            fg: ANSI_16[1],
            ..Cell::default()
        };
        let bright_white_cell = Cell {
            ch: 'W',
            fg: ANSI_16[15],
            ..Cell::default()
        };
        let truecolor_cell = Cell {
            ch: 'T',
            fg: rgb(1, 2, 3),
            ..Cell::default()
        };

        assert_eq!(
            map_terminal_cell(default_cell, palette).fg,
            palette.default_fg
        );
        assert_eq!(map_terminal_cell(red_cell, palette).fg, palette.ansi[1]);
        assert_eq!(
            map_terminal_cell(bright_white_cell, palette).fg,
            palette.ansi[15]
        );
        assert_eq!(map_terminal_cell(truecolor_cell, palette).fg, rgb(1, 2, 3));
    }

    #[test]
    fn custom_terminal_palette_uses_user_foreground() {
        let mut custom = default_custom_theme();
        custom.foreground = rgb(18, 52, 86);
        let palette = terminal_palette_for(CUSTOM_THEME_ID, &custom);
        assert_eq!(palette.default_fg, rgb(18, 52, 86));
    }

    #[test]
    fn themes_json_matches_static_catalog_metadata() {
        #[derive(serde::Deserialize)]
        struct JsonTheme {
            id: String,
            label: String,
            description: String,
        }

        let parsed: Vec<JsonTheme> =
            serde_json::from_str(include_str!("../assets/themes.json")).unwrap();
        let metadata: Vec<(&str, &str, &str)> = parsed
            .iter()
            .map(|theme| {
                (
                    theme.id.as_str(),
                    theme.label.as_str(),
                    theme.description.as_str(),
                )
            })
            .collect();
        let catalog: Vec<(&str, &str, &str)> = themes()
            .iter()
            .map(|theme| (theme.id, theme.label, theme.description))
            .collect();

        assert_eq!(metadata, catalog);
    }
}
