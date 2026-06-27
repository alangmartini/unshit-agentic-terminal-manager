use smallvec::smallvec;
use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{
    Background, Color, Dimension, Display, FlexDirection, GradientStop, GradientStopPosition,
    LinearGradient, Overflow, TextAlign,
};
use unshit::prelude::SvgNode;

use unshit::core::event::Modifiers;
use unshit::core::shortcut::KeyCombo;

use crate::keybinds::{KeybindAction, KeybindError, KeybindErrorKind};
use crate::state::{
    dispatch, is_on, mutate_with, SettingsSection, SharedState, ToggleKey, UiDensity, UiSnapshot,
    DEFAULT_CONFIG_FONT_SIZE_PT,
};
use crate::theme;
use crate::ui::icons::*;

pub fn build_settings_modal(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal")
        .with_class("set-modal")
        .with_style(StyleDeclaration::Display(Display::Grid))
        .with_style(StyleDeclaration::Width(Dimension::Px(860.0)))
        .with_style(StyleDeclaration::Height(Dimension::Percent(76.0)))
        .with_style(StyleDeclaration::MaxHeight(Dimension::Px(760.0)))
        .with_child(build_modal_header(shared))
        .with_child(build_modal_content(state, shared))
        .with_child(build_modal_footer(shared))
}

pub fn build_settings_page(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("settings-page")
        .with_id("settings-page")
        .with_child(build_settings_page_rail(state, shared))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-content")
                .with_child(build_settings_page_header(state.settings_section))
                .with_child(build_settings_page_body(state, shared))
                .with_child(build_settings_page_savebar(state.settings_section, shared)),
        )
}

fn build_settings_page_rail(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let active = state.settings_section;
    let session_label = format!("ptyd up · session {:02}", state.active_tab + 1);
    ElementDef::new(Tag::Div)
        .with_class("set-page-rail")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-rail-head")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("title")
                        .with_text("settings"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("sub")
                        .with_text(format!("v{}", env!("CARGO_PKG_VERSION"))),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-search")
                .with_child(svg_icon(icon_magnifier()))
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("set-page-search-input")
                        .with_placeholder("find a setting..."),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("kbd")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("kbd-command")
                                .with_text("\u{2318}"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("kbd-key")
                                .with_text("F"),
                        ),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-nav")
                .with_child(settings_nav_group("workspace"))
                .with_child(settings_nav_item(
                    SettingsSection::Appearance,
                    active,
                    shared,
                ))
                .with_child(settings_nav_item(SettingsSection::Shell, active, shared))
                .with_child(settings_nav_item(SettingsSection::Sessions, active, shared))
                .with_child(settings_nav_group("automation"))
                .with_child(settings_nav_item(SettingsSection::Keybinds, active, shared))
                .with_child(settings_nav_item(
                    SettingsSection::Notifications,
                    active,
                    shared,
                ))
                .with_child(settings_nav_item(
                    SettingsSection::DangerZone,
                    active,
                    shared,
                )),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-foot")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("dot")
                        .with_class("status-running"),
                )
                .with_child(ElementDef::new(Tag::Span).with_text(session_label)),
        )
}

fn settings_nav_group(label: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("group")
        .with_text(label.to_string())
}

fn settings_section_title(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Appearance => "Appearance",
        SettingsSection::Shell => "Shell",
        SettingsSection::Keybinds => "Keybinds",
        SettingsSection::Sessions => "Sessions",
        SettingsSection::Notifications => "Notifications",
        SettingsSection::DangerZone => "Danger Zone",
    }
}

fn settings_nav_item(
    section: SettingsSection,
    active: SettingsSection,
    shared: &SharedState,
) -> ElementDef {
    let s = shared.clone();
    let mut item = ElementDef::new(Tag::Button)
        .with_class("set-page-nav-item")
        .with_class(settings_nav_class(section))
        .with_child(svg_icon(settings_nav_icon(section)))
        .with_child(ElementDef::new(Tag::Span).with_text(settings_section_title(section)));
    if section == active {
        item = item.with_class("active");
    }
    item.on_click(move || {
        mutate_with(&s, |st| {
            st.settings_section = section;
            if section == SettingsSection::Sessions {
                crate::state::refresh_sessions(st);
            }
        });
    })
}

fn settings_nav_class(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Appearance => "nav-appearance",
        SettingsSection::Shell => "nav-shell",
        SettingsSection::Sessions => "nav-sessions",
        SettingsSection::Keybinds => "nav-keybinds",
        SettingsSection::Notifications => "nav-notifications",
        SettingsSection::DangerZone => "nav-danger-zone",
    }
}

fn settings_nav_icon(section: SettingsSection) -> SvgNode {
    match section {
        SettingsSection::Appearance => icon_settings_nav_grid(),
        SettingsSection::Shell => icon_terminal(),
        SettingsSection::Keybinds => icon_chevrons(),
        SettingsSection::Sessions => icon_folder(),
        SettingsSection::Notifications => icon_bell(),
        SettingsSection::DangerZone => icon_settings_nav_close(),
    }
}

fn build_settings_page_header(active: SettingsSection) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-page-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("crumb")
                .with_text(format!("settings · {}", active.label())),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("page-title")
                .with_text(settings_section_title(active)),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("blurb")
                .with_text(settings_section_desc(active)),
        )
}

fn settings_section_desc(active: SettingsSection) -> &'static str {
    match active {
        SettingsSection::Appearance => {
            "Themes, density, and the visual feel of the terminal. Changes apply immediately."
        }
        SettingsSection::Shell => "Default shell, font, scrollback.",
        SettingsSection::Keybinds => {
            "Click any shortcut to rebind it. Press a new combination, or Esc to cancel."
        }
        SettingsSection::Sessions => "Daemon sessions and workspace attachment.",
        SettingsSection::Notifications => "Desktop notifications and focused panes.",
        SettingsSection::DangerZone => "Destructive session and close behavior.",
    }
}

fn build_settings_page_body(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut body = ElementDef::new(Tag::Div).with_class("set-page-body");
    if let Some(font_size) = scaled_config_font_px(12.0, state.config_font_size_pt) {
        body = body.with_style(StyleDeclaration::FontSize(font_size));
    }
    body = match state.settings_section {
        SettingsSection::Appearance => {
            body.with_child(build_appearance_page_section(state, shared))
        }
        SettingsSection::Shell => body.with_child(build_shell_section(state, shared)),
        SettingsSection::Keybinds => body.with_child(build_keybinds_section(state, shared)),
        SettingsSection::Sessions => body.with_child(build_sessions_section(state, shared)),
        SettingsSection::Notifications => body.with_child(build_notifications_section(shared)),
        SettingsSection::DangerZone => body.with_child(build_danger_zone_section(state, shared)),
    };
    body
}

fn build_appearance_page_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let active_theme = theme::resolve_theme_id(&state.theme);
    let active_spec = theme::theme_spec(active_theme);
    let theme_meta = if active_theme == theme::CUSTOM_THEME_ID {
        "custom · your palette".to_string()
    } else {
        format!(
            "{} · {}",
            active_spec.label.to_lowercase(),
            active_spec.meta
        )
    };
    let mut theme_card = set_card("theme", Some(theme_meta.as_str())).with_child(
        settings_page_field(
            "Color theme",
            Some("Sets surface, text, accent, and syntax tints across the whole app."),
            build_theme_picker(state, shared),
            state.config_font_size_pt,
        )
        .with_class("theme-field"),
    );
    if active_theme == theme::CUSTOM_THEME_ID {
        theme_card = theme_card.with_child(build_custom_theme_editor(state, shared));
    }

    ElementDef::new(Tag::Div)
        .with_class("set-page-section")
        .with_child(theme_card)
        .with_child(
            set_card("interface", None)
                .with_child(settings_page_field(
                    "Config font size",
                    Some("Settings and app chrome text size in points."),
                    font_stepper(
                        state.config_font_size_pt,
                        "config_font.dec",
                        "config_font.inc",
                        shared,
                    ),
                    state.config_font_size_pt,
                ))
                .with_child(settings_page_field(
                    "Density",
                    Some("Vertical padding inside lists and panes."),
                    density_segmented(state.ui_density, shared),
                    state.config_font_size_pt,
                ))
                .with_child(settings_page_field(
                    "Wheel scroll step",
                    Some("Pixels moved per wheel notch."),
                    command_stepper(
                        state.scroll_line_px.to_string(),
                        "scroll.line_px.dec",
                        "scroll.line_px.inc",
                        shared,
                    ),
                    state.config_font_size_pt,
                ))
                .with_child(settings_page_field(
                    "Smooth scroll duration",
                    Some("Animation time after wheel input."),
                    command_stepper(
                        format!("{} ms", state.smooth_scroll_duration_ms),
                        "scroll.duration.dec",
                        "scroll.duration.inc",
                        shared,
                    ),
                    state.config_font_size_pt,
                )),
        )
        .with_child(build_tabs_card(state, shared))
        .with_child(
            set_card("terminal", None)
                .with_child(settings_page_field(
                    "Terminal font size",
                    Some("Terminal output size in points."),
                    font_stepper(
                        state.terminal_font_size_pt,
                        "terminal_font.dec",
                        "terminal_font.inc",
                        shared,
                    ),
                    state.config_font_size_pt,
                ))
                .with_child(settings_page_field(
                    "Sidebar width",
                    Some("Width of the workspace sidebar."),
                    readout_with_unit(&format!("{:.0}", state.sidebar_width), "px"),
                    state.config_font_size_pt,
                )),
        )
        .with_child(set_card("preview", None).with_child(build_appearance_preview(state)))
}

/// The "tabs" card on the Appearance page: how horizontal terminal tabs
/// are sized and whether the strip stays one scrolling row or wraps onto
/// two or three rows. The width stepper only appears in `Fixed` mode (in
/// `FitContent` mode there is no width to tune).
fn build_tabs_card(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut card = set_card("tabs", None).with_child(settings_page_field(
        "Tab sizing",
        Some("Fixed width per tab, or shrink-wrap each tab to its label."),
        tab_width_mode_segmented(state.tab_width_mode, shared),
        state.config_font_size_pt,
    ));

    if state.tab_width_mode == crate::state::TabWidthMode::Fixed {
        card = card.with_child(settings_page_field(
            "Tab width",
            Some("Width of each tab when sizing is fixed."),
            command_stepper(
                format!("{} px", state.tab_width_px),
                "tabs.width.dec",
                "tabs.width.inc",
                shared,
            ),
            state.config_font_size_pt,
        ));
    }

    card.with_child(settings_page_field(
        "Tab rows",
        Some("Single scrolling row, or wrap tabs onto two or three rows."),
        tab_row_mode_segmented(state.tab_row_mode, shared),
        state.config_font_size_pt,
    ))
}

fn tab_width_mode_segmented(
    active: crate::state::TabWidthMode,
    shared: &SharedState,
) -> ElementDef {
    let mut segmented = ElementDef::new(Tag::Div).with_class("input-segmented");
    for mode in crate::state::TabWidthMode::all() {
        let s = shared.clone();
        let command = format!("tabs.width_mode:{}", mode.id());
        let mut button = ElementDef::new(Tag::Button)
            .with_class("seg-btn")
            .with_text(mode.label())
            .on_click(move || {
                let command = command.clone();
                mutate_with(&s, move |st| dispatch(st, &command));
            });
        if mode == active {
            button = button.with_class("active");
        }
        segmented = segmented.with_child(button);
    }
    segmented
}

fn tab_row_mode_segmented(active: crate::state::TabRowMode, shared: &SharedState) -> ElementDef {
    let mut segmented = ElementDef::new(Tag::Div).with_class("input-segmented");
    for mode in crate::state::TabRowMode::all() {
        let s = shared.clone();
        let command = format!("tabs.row_mode:{}", mode.id());
        let mut button = ElementDef::new(Tag::Button)
            .with_class("seg-btn")
            .with_text(mode.label())
            .on_click(move || {
                let command = command.clone();
                mutate_with(&s, move |st| dispatch(st, &command));
            });
        if mode == active {
            button = button.with_class("active");
        }
        segmented = segmented.with_child(button);
    }
    segmented
}

fn build_theme_picker(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let active = theme::resolve_theme_id(&state.theme);
    let mut picker = ElementDef::new(Tag::Div).with_class("theme-picker");
    for spec in theme::themes() {
        let swatches = spec.swatches.map(str::to_string);
        let mut chip = build_theme_chip(
            spec.id,
            spec.label,
            spec.meta,
            &swatches,
            spec.id == active,
            &state.custom_theme,
        );
        let s = shared.clone();
        let id = spec.id;
        chip = chip.on_click(move || {
            let id = id.to_string();
            mutate_with(&s, move |st| {
                crate::state::mutate_theme(st, &id);
            });
        });
        picker = picker.with_child(chip);
    }
    let custom_swatches = custom_theme_swatches(&state.custom_theme);
    let mut custom_chip = build_theme_chip(
        theme::CUSTOM_THEME_ID,
        "Custom",
        "pick your own",
        &custom_swatches,
        active == theme::CUSTOM_THEME_ID,
        &state.custom_theme,
    );
    let s = shared.clone();
    custom_chip = custom_chip.on_click(move || {
        mutate_with(&s, |st| {
            crate::state::mutate_theme(st, theme::CUSTOM_THEME_ID);
        });
    });
    picker = picker.with_child(custom_chip);
    picker
}

fn build_theme_chip(
    id: &str,
    label: &str,
    meta: &str,
    swatches: &[String],
    active: bool,
    custom_theme: &theme::CustomTheme,
) -> ElementDef {
    let palette = theme_chip_palette(id, custom_theme);
    let mut body = ElementDef::new(Tag::Div)
        .with_class("theme-chip-main")
        .with_class("theme-chip-screen")
        .with_style(StyleDeclaration::Background(palette.preview.clone()))
        .with_style(StyleDeclaration::BorderColor(palette.divider));
    if id == theme::CUSTOM_THEME_ID {
        body = body.with_child(ElementDef::new(Tag::Span).with_class("theme-chip-pattern"));
    }
    body = body
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("tcs-glyph")
                .with_style(StyleDeclaration::Color(palette.glyph))
                .with_text(if id == theme::CUSTOM_THEME_ID {
                    "+"
                } else {
                    "\u{276F}"
                }),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("theme-chip-copy")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("tcs-name")
                        .with_style(StyleDeclaration::Color(palette.text))
                        .with_text(label),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("tcs-sub")
                        .with_style(StyleDeclaration::Color(palette.text_dim))
                        .with_text(meta),
                ),
        );
    let pin = ElementDef::new(Tag::Span)
        .with_class("theme-chip-pin")
        .with_style(StyleDeclaration::Background(Background::Color(
            palette.pin_background,
        )))
        .with_style(StyleDeclaration::Color(palette.pin_foreground))
        .with_text("✓");

    let mut chip = ElementDef::new(Tag::Button)
        .with_class("theme-chip")
        .with_class(id)
        .with_style(StyleDeclaration::TextAlign(TextAlign::Left))
        .with_style(StyleDeclaration::OverflowX(Overflow::Hidden))
        .with_style(StyleDeclaration::OverflowY(Overflow::Hidden))
        .with_child(pin)
        .with_child(body)
        .with_child(build_theme_swatch_strip(swatches));
    if !active {
        chip = chip.with_child(ElementDef::new(Tag::Div).with_class("theme-chip-top-hairline"));
    }
    if active {
        chip = chip.with_class("active");
    }
    chip
}

fn build_theme_swatch_strip(swatches: &[String]) -> ElementDef {
    let mut strip = ElementDef::new(Tag::Div).with_class("theme-chip-foot");
    for color in swatches {
        let mut swatch = ElementDef::new(Tag::Span);
        if let Some(color) = theme::parse_hex_color(color) {
            swatch = swatch.with_style(StyleDeclaration::Background(Background::Color(color)));
        }
        strip = strip.with_child(swatch);
    }
    strip
}

#[derive(Clone, Debug)]
struct ThemeChipPalette {
    preview: Background,
    divider: Color,
    text: Color,
    text_dim: Color,
    glyph: Color,
    pin_background: Color,
    pin_foreground: Color,
}

fn theme_chip_palette(id: &str, custom: &theme::CustomTheme) -> ThemeChipPalette {
    let solid = |preview, divider, text, text_dim, accent, accent_on| ThemeChipPalette {
        preview: Background::Color(preview),
        divider,
        text,
        text_dim,
        glyph: accent,
        pin_background: accent,
        pin_foreground: accent_on,
    };

    match id {
        "catppuccin" => solid(
            Color::rgb(0x1e, 0x1e, 0x2e),
            Color::rgb(0x11, 0x11, 0x1b),
            Color::rgb(0xcd, 0xd6, 0xf4),
            Color::rgb(0x66, 0x6f, 0x86),
            Color::rgb(0xcb, 0xa6, 0xf7),
            Color::rgb(0x1e, 0x1e, 0x2e),
        ),
        "tokyo-night" => solid(
            Color::rgb(0x1a, 0x1b, 0x26),
            Color::rgb(0x16, 0x16, 0x1e),
            Color::rgb(0xc0, 0xca, 0xf5),
            Color::rgb(0x48, 0x51, 0x6c),
            Color::rgb(0x7a, 0xa2, 0xf7),
            Color::rgb(0x1a, 0x1b, 0x26),
        ),
        "nord" => solid(
            Color::rgb(0x2e, 0x34, 0x40),
            Color::rgb(0x24, 0x29, 0x33),
            Color::rgb(0xec, 0xef, 0xf4),
            Color::rgb(0x6c, 0x75, 0x87),
            Color::rgb(0x88, 0xc0, 0xd0),
            Color::rgb(0x2e, 0x34, 0x40),
        ),
        "dracula" => solid(
            Color::rgb(0x28, 0x2a, 0x36),
            Color::rgb(0x21, 0x22, 0x2c),
            Color::rgb(0xf8, 0xf8, 0xf2),
            Color::rgb(0x62, 0x72, 0xa4),
            Color::rgb(0xbd, 0x93, 0xf9),
            Color::rgb(0x28, 0x2a, 0x36),
        ),
        "everforest" => solid(
            Color::rgb(0x27, 0x2e, 0x33),
            Color::rgb(0x1e, 0x23, 0x26),
            Color::rgb(0xd3, 0xc6, 0xaa),
            Color::rgb(0x85, 0x92, 0x89),
            Color::rgb(0xa7, 0xc0, 0x80),
            Color::rgb(0x2d, 0x35, 0x3b),
        ),
        "rose-pine" => solid(
            Color::rgb(0x1f, 0x1d, 0x2e),
            Color::rgb(0x19, 0x17, 0x24),
            Color::rgb(0xe0, 0xde, 0xf4),
            Color::rgb(0x90, 0x8c, 0xaa),
            Color::rgb(0xeb, 0xbc, 0xba),
            Color::rgb(0x23, 0x21, 0x36),
        ),
        "gruvbox" => solid(
            Color::rgb(0x28, 0x28, 0x28),
            Color::rgb(0x1d, 0x20, 0x21),
            Color::rgb(0xeb, 0xdb, 0xb2),
            Color::rgb(0xa8, 0x99, 0x84),
            Color::rgb(0xfa, 0xbd, 0x2f),
            Color::rgb(0x28, 0x28, 0x28),
        ),
        "kanagawa" => solid(
            Color::rgb(0x1f, 0x1f, 0x28),
            Color::rgb(0x16, 0x16, 0x1d),
            Color::rgb(0xdc, 0xd7, 0xba),
            Color::rgb(0x72, 0x71, 0x69),
            Color::rgb(0x7e, 0x9c, 0xd8),
            Color::rgb(0x1f, 0x1f, 0x28),
        ),
        theme::CUSTOM_THEME_ID => ThemeChipPalette {
            preview: Background::LinearGradient(LinearGradient {
                angle_deg: 135.0,
                stops: smallvec![
                    GradientStop {
                        color: custom.accent,
                        position: GradientStopPosition::Percent(0.0),
                    },
                    GradientStop {
                        color: custom.accent_soft,
                        position: GradientStopPosition::Percent(1.0),
                    },
                ],
                repeating: false,
            }),
            divider: custom.background,
            text: custom.background,
            text_dim: Color::rgba(
                custom.background.r,
                custom.background.g,
                custom.background.b,
                179,
            ),
            glyph: custom.background,
            pin_background: custom.accent,
            pin_foreground: custom.background,
        },
        _ => solid(
            Color::rgb(0x1c, 0x18, 0x12),
            Color::rgb(0x14, 0x11, 0x0c),
            Color::rgb(0xeb, 0xdc, 0xb6),
            Color::rgb(0x6f, 0x5a, 0x33),
            Color::rgb(0xd4, 0xa3, 0x48),
            Color::rgb(0x1c, 0x18, 0x12),
        ),
    }
}

fn custom_theme_swatches(custom: &theme::CustomTheme) -> [String; 5] {
    [
        theme::color_to_hex(custom.accent),
        theme::color_to_hex(custom.accent_soft),
        theme::color_to_hex(custom.background),
        theme::color_to_hex(custom.foreground),
        theme::color_to_hex(custom.surface),
    ]
}

#[derive(Clone, Copy, Debug)]
struct AppearancePreviewPalette {
    background: Color,
    chrome: Color,
    border: Color,
    text: Color,
    dim: Color,
    accent: Color,
    command: Color,
    azure: Color,
    sage: Color,
    rust: Color,
    violet: Color,
    number: Color,
    badge_background: Color,
    cursor: Color,
}

fn appearance_preview_palette(state: &UiSnapshot) -> AppearancePreviewPalette {
    let active = theme::resolve_theme_id(&state.theme);
    let terminal = theme::terminal_palette_for(active, &state.custom_theme);

    if active == theme::CUSTOM_THEME_ID {
        let violet = terminal.ansi[5];
        return AppearancePreviewPalette {
            background: state.custom_theme.background,
            chrome: state.custom_theme.surface,
            border: state.custom_theme.surface,
            text: state.custom_theme.foreground,
            dim: Color::rgba(
                state.custom_theme.foreground.r,
                state.custom_theme.foreground.g,
                state.custom_theme.foreground.b,
                178,
            ),
            accent: state.custom_theme.accent,
            command: state.custom_theme.foreground,
            azure: terminal.ansi[6],
            sage: terminal.ansi[2],
            rust: terminal.ansi[1],
            violet,
            number: terminal.ansi[3],
            badge_background: Color::rgba(violet.r, violet.g, violet.b, 46),
            cursor: state.custom_theme.accent_soft,
        };
    }

    let chip = theme_chip_palette(active, &state.custom_theme);
    let background = match chip.preview {
        Background::Color(color) => color,
        _ => terminal.default_fg,
    };

    AppearancePreviewPalette {
        background,
        chrome: chip.divider,
        border: chip.divider,
        text: terminal.default_fg,
        dim: chip.text_dim,
        accent: chip.glyph,
        command: chip.text,
        azure: terminal.ansi[6],
        sage: terminal.ansi[2],
        rust: terminal.ansi[1],
        violet: terminal.ansi[5],
        number: terminal.ansi[3],
        badge_background: Color::rgba(
            terminal.ansi[5].r,
            terminal.ansi[5].g,
            terminal.ansi[5].b,
            46,
        ),
        cursor: terminal.ansi[3],
    }
}

fn build_custom_theme_editor(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut grid = ElementDef::new(Tag::Div).with_class("custom-editor-grid");
    for slot in theme::custom_theme_slots() {
        grid = grid.with_child(custom_color_field(*slot, state, shared));
    }

    let reset_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("custom-editor")
        .with_class("open")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("custom-editor-head")
                .with_child(ElementDef::new(Tag::Span).with_text("Custom palette"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("name-meta")
                        .with_text("hex edits apply live"),
                ),
        )
        .with_child(grid)
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("custom-editor-actions")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_text("Five colors define the look; syntax colors stay practical."),
                )
                .with_child(ElementDef::new(Tag::Span).with_class("spacer"))
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("ghost")
                        .with_text("reset custom")
                        .on_click(move || {
                            mutate_with(&reset_state, |st| {
                                crate::state::reset_custom_theme(st);
                            });
                        }),
                ),
        )
}

fn custom_color_field(
    slot: theme::CustomThemeSlot,
    state: &UiSnapshot,
    shared: &SharedState,
) -> ElementDef {
    let color = theme::custom_theme_color(&state.custom_theme, slot);
    let hex = theme::color_to_hex(color);
    let edit_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("color-field")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("color-swatch")
                .with_style(StyleDeclaration::Background(Background::Color(color))),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("color-field-meta")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("color-field-label")
                        .with_text(slot.label()),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("color-field-value")
                        .with_text(hex.clone()),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Input)
                .with_class("input-text")
                .with_class("color-input")
                .with_placeholder(hex.clone())
                .on_change(move |value| {
                    mutate_with(&edit_state, |st| {
                        crate::state::mutate_custom_theme_color(st, slot, value);
                    });
                }),
        )
}

fn set_card(name: &str, meta: Option<&str>) -> ElementDef {
    let mut head = ElementDef::new(Tag::Div)
        .with_class("set-card-head")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("name")
                .with_text(name),
        );
    if let Some(meta) = meta {
        head = head.with_child(
            ElementDef::new(Tag::Span)
                .with_class("name-meta")
                .with_text(meta),
        );
    }
    ElementDef::new(Tag::Div)
        .with_class("set-card")
        .with_style(StyleDeclaration::Width(Dimension::Percent(100.0)))
        .with_child(head)
}

fn scaled_config_font_px(base_px: f32, config_font_size_pt: u32) -> Option<f32> {
    if config_font_size_pt == DEFAULT_CONFIG_FONT_SIZE_PT {
        None
    } else {
        Some(base_px * config_font_size_pt as f32 / DEFAULT_CONFIG_FONT_SIZE_PT as f32)
    }
}

fn settings_page_field(
    label: &str,
    desc: Option<&str>,
    control: ElementDef,
    config_font_size_pt: u32,
) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_class("set-field")
        .with_child(setting_meta_with_config_font(
            label,
            desc,
            config_font_size_pt,
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-control")
                .with_child(control),
        )
}

fn readout_with_unit(value: &str, unit: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-inline-control")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("input-text")
                .with_class("input-num")
                .with_text(value),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("set-unit")
                .with_text(unit),
        )
}

fn build_appearance_preview(state: &UiSnapshot) -> ElementDef {
    let palette = appearance_preview_palette(state);
    ElementDef::new(Tag::Div)
        .with_class("preview-tile")
        .with_style(StyleDeclaration::Background(Background::Color(
            palette.background,
        )))
        .with_style(StyleDeclaration::BorderColor(palette.border))
        .with_style(StyleDeclaration::Color(palette.text))
        .with_style(StyleDeclaration::FontSize(
            state.terminal_font_size_pt as f32,
        ))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("preview-head")
                .with_style(StyleDeclaration::Background(Background::Color(
                    palette.chrome,
                )))
                .with_style(StyleDeclaration::BorderColor(palette.border))
                .with_style(StyleDeclaration::Color(palette.dim))
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("tm-traffic")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("tl-dot")
                                .with_class("tl-close"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("tl-dot")
                                .with_class("tl-min"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("tl-dot")
                                .with_class("tl-zoom"),
                        ),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("~/code/main/dashboard — zsh")),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("preview-body")
                .with_child(preview_line(vec![
                    preview_span(&palette, "prompt", "\u{276F} "),
                    preview_span(&palette, "path", "~/code/main/dashboard "),
                    preview_span(&palette, "branch", "(main)"),
                ]))
                .with_child(preview_line(vec![
                    preview_span(&palette, "prompt", "\u{276F} "),
                    preview_span(&palette, "cmd", "npm run dev"),
                ]))
                .with_child(preview_line(vec![
                    preview_span(&palette, "azure", "\u{2192} vite v5.4.0  ready in "),
                    preview_span(&palette, "num", "312"),
                    preview_span(&palette, "azure", " ms"),
                ]))
                .with_child(preview_line(vec![
                    preview_span(&palette, "muted", "  \u{279C}  local:   "),
                    preview_span(&palette, "azure", "http://localhost:4040/"),
                ]))
                .with_child(preview_line(vec![
                    preview_span(&palette, "sage", "\u{2713} recompiled in "),
                    preview_span(&palette, "num", "84"),
                    preview_span(&palette, "sage", "ms "),
                    preview_span(&palette, "muted", "\u{2014} 4 modules"),
                ]))
                .with_child(preview_line(vec![preview_span(
                    &palette,
                    "rust",
                    "\u{2717} src/lib/format.test.ts (2)",
                )]))
                .with_child(preview_line(vec![
                    preview_span(&palette, "muted", "    expected "),
                    preview_span(&palette, "num", "42"),
                    preview_span(&palette, "muted", " to be "),
                    preview_span(&palette, "num", "41"),
                ]))
                .with_child(preview_line(vec![
                    preview_span(&palette, "agent-tag", "claude"),
                    preview_span(&palette, "violet", "patching format.ts..."),
                ]))
                .with_child(preview_line(vec![
                    preview_span(&palette, "prompt", "\u{276F} "),
                    ElementDef::new(Tag::Span).with_class("cur").with_style(
                        StyleDeclaration::Background(Background::Color(palette.cursor)),
                    ),
                ])),
        )
}

fn preview_line(parts: Vec<ElementDef>) -> ElementDef {
    let mut line = ElementDef::new(Tag::Div).with_class("preview-line");
    for part in parts {
        line = line.with_child(part);
    }
    line
}

fn preview_span(palette: &AppearancePreviewPalette, class: &str, text: &str) -> ElementDef {
    let color = match class {
        "prompt" => palette.accent,
        "path" | "azure" => palette.azure,
        "branch" | "sage" => palette.sage,
        "cmd" => palette.command,
        "rust" => palette.rust,
        "violet" => palette.violet,
        "muted" => palette.dim,
        "num" => palette.number,
        "agent-tag" => palette.violet,
        _ => palette.text,
    };
    let mut span = ElementDef::new(Tag::Span)
        .with_class(class)
        .with_style(StyleDeclaration::Color(color))
        .with_text(text);
    if class == "agent-tag" {
        span = span.with_style(StyleDeclaration::Background(Background::Color(
            palette.badge_background,
        )));
    }
    span
}

fn build_settings_page_savebar(section: SettingsSection, shared: &SharedState) -> ElementDef {
    let close_state = shared.clone();
    let reset_state = shared.clone();
    // Per-section reset affordance: keybinds restore their defaults; other
    // sections reset appearance settings.
    let (reset_label, reset_cmd) = match section {
        SettingsSection::Keybinds => ("restore defaults", "keybind.reset_all"),
        _ => ("reset", "appearance.reset"),
    };
    ElementDef::new(Tag::Div)
        .with_class("set-page-savebar")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("saved")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("saved-dot")
                        .with_class("status-running"),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("changes apply immediately")),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("spacer"))
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .with_class("ghost")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("btn-label")
                        .with_text(reset_label),
                )
                .on_click(move || {
                    mutate_with(&reset_state, |st| dispatch(st, reset_cmd));
                }),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .with_class("primary")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("btn-label")
                        .with_text("done"),
                )
                .on_click(move || {
                    mutate_with(&close_state, |st| dispatch(st, "modal.close"));
                }),
        )
}

fn build_modal_header(shared: &SharedState) -> ElementDef {
    let close_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-header")
        .with_class("set-head")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-title-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("modal-mark")
                        .with_class("set-mark")
                        .with_text("\u{25C6}"),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("modal-title")
                        .with_class("set-title")
                        .with_id("settings-title")
                        .with_text("settings"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("settings-close")
                .on_click(move || {
                    mutate_with(&close_state, |st| dispatch(st, "modal.close"));
                })
                .with_child(svg_icon(icon_close())),
        )
}

fn build_modal_nav(active: SettingsSection, shared: &SharedState) -> ElementDef {
    let mut nav = ElementDef::new(Tag::Div)
        .with_class("modal-nav")
        .with_class("set-nav-rail");
    for section in SettingsSection::all() {
        let mut item = ElementDef::new(Tag::Button)
            .with_class("modal-nav-item")
            .with_class("set-nav")
            .with_text(section.label());
        if section == active {
            item = item.with_class("active");
        }
        let s = shared.clone();
        let target = section;
        item = item.on_click(move || {
            mutate_with(&s, |st| {
                st.settings_section = target;
                if target == SettingsSection::Sessions {
                    crate::state::refresh_sessions(st);
                }
            });
        });
        nav = nav.with_child(item);
    }
    nav
}

fn build_modal_body(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let section = match state.settings_section {
        SettingsSection::Appearance => build_appearance_section(state, shared),
        SettingsSection::Shell => build_shell_section(state, shared),
        SettingsSection::Keybinds => build_keybinds_section(state, shared),
        SettingsSection::Sessions => build_sessions_section(state, shared),
        SettingsSection::Notifications => build_notifications_section(shared),
        SettingsSection::DangerZone => build_danger_zone_section(state, shared),
    };
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_class("set-content")
        .with_style(StyleDeclaration::Display(Display::Flex))
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_style(StyleDeclaration::FlexGrow(1.0))
        .with_style(StyleDeclaration::FlexBasis(Dimension::Auto))
        .with_style(StyleDeclaration::OverflowX(Overflow::Scroll))
        .with_style(StyleDeclaration::OverflowY(Overflow::Scroll))
        .with_style(StyleDeclaration::MinHeight(Dimension::Px(0.0)))
        .with_child(section)
}

fn build_modal_content(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-body")
        .with_child(build_modal_nav(state.settings_section, shared))
        .with_child(build_modal_body(state, shared))
}

// -- section builders -------------------------------------------------------

fn build_appearance_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    section_shell("appearance")
        .with_child(setting_row(
            "Config font size",
            "Settings and app chrome text size in points",
            font_stepper(
                state.config_font_size_pt,
                "config_font.dec",
                "config_font.inc",
                shared,
            ),
        ))
        .with_child(setting_row(
            "Terminal font size",
            "Terminal output size in points",
            font_stepper(
                state.terminal_font_size_pt,
                "terminal_font.dec",
                "terminal_font.inc",
                shared,
            ),
        ))
}

fn build_shell_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let installed = crate::shell::discover_installed();
    let mut section = section_shell("shell").with_child(shell_scope_block(
        ShellScope::AppDefault,
        "App default",
        "Shell launched for new panes when no workspace overrides it",
        &state.default_shell,
        &installed,
        shared,
    ));

    if !state.workspaces.is_empty() {
        let mut overrides = ElementDef::new(Tag::Div)
            .with_class("workspace-overrides")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("modal-section-title")
                    .with_text("workspace overrides"),
            );
        for (idx, ws) in state.workspaces.iter().enumerate() {
            overrides = overrides.with_child(shell_scope_block(
                ShellScope::Workspace(idx),
                &ws.name,
                "Override the app default for this workspace only",
                &ws.shell,
                &installed,
                shared,
            ));
        }
        section = section.with_child(overrides);
    }

    section
}

/// Which shell scope a picker mutates: the app wide default, or a
/// specific workspace override (carries the workspace index used in
/// the dispatch command).
#[derive(Clone, Copy)]
enum ShellScope {
    AppDefault,
    Workspace(usize),
}

impl ShellScope {
    fn set_cmd_prefix(&self) -> String {
        match self {
            ShellScope::AppDefault => "shell.set_default:".to_string(),
            ShellScope::Workspace(idx) => format!("shell.set_workspace:{idx}:"),
        }
    }

    fn clear_cmd(&self) -> String {
        match self {
            ShellScope::AppDefault => "shell.clear_default".to_string(),
            ShellScope::Workspace(idx) => format!("shell.clear_workspace:{idx}"),
        }
    }
}

/// One editable scope in the Shell tab. Bundles label + description,
/// the chip picker (one chip per discovered shell, plus a "Use
/// default" chip for workspace scopes), a custom path input for
/// shells that aren't on PATH, and an args input.
fn shell_scope_block(
    scope: ShellScope,
    label: &str,
    desc: &str,
    current: &crate::shell::ShellSpec,
    installed: &[std::path::PathBuf],
    shared: &SharedState,
) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("shell-scope-block")
        .with_child(setting_meta(label, Some(desc)))
        .with_child(shell_picker(scope, current, installed, shared))
        .with_child(shell_custom_program_input(scope, current, shared))
        .with_child(shell_args_input(scope, current, shared))
}

/// Chip group of every discovered shell. The chip whose path matches
/// `current.program` is marked active. Workspace pickers also get a
/// "Use default" chip that dispatches the matching `shell.clear_*`.
fn shell_picker(
    scope: ShellScope,
    current: &crate::shell::ShellSpec,
    installed: &[std::path::PathBuf],
    shared: &SharedState,
) -> ElementDef {
    let mut picker = ElementDef::new(Tag::Div).with_class("shell-picker");

    if let ShellScope::Workspace(_) = scope {
        let mut chip = ElementDef::new(Tag::Button)
            .with_class("shell-chip")
            .with_class("clear")
            .with_text("use default");
        if current.is_empty() {
            chip = chip.with_class("active");
        }
        let s = shared.clone();
        let cmd = scope.clear_cmd();
        chip = chip.on_click(move || {
            mutate_with(&s, |st| dispatch(st, &cmd));
        });
        picker = picker.with_child(chip);
    }

    let labels = crate::shell::label_installed_shells(installed);
    for (path, label) in installed.iter().zip(labels.iter()) {
        let program = path.display().to_string();
        let active = !current.program.is_empty() && current.program == program;
        let mut chip = ElementDef::new(Tag::Button)
            .with_class("shell-chip")
            .with_text(label.as_str());
        if active {
            chip = chip.with_class("active");
        }
        let s = shared.clone();
        let prefix = scope.set_cmd_prefix();
        let prog = program.clone();
        let args = current.args.clone();
        chip = chip.on_click(move || {
            let spec = crate::shell::ShellSpec {
                program: prog.clone(),
                args: args.clone(),
            };
            let json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into());
            mutate_with(&s, |st| {
                dispatch(st, &format!("{prefix}{json}"));
            });
        });
        picker = picker.with_child(chip);
    }

    picker
}

/// Text input that reads as the current `program` (via placeholder)
/// and on submit dispatches a fresh `shell.set_*` with the typed path
/// and the existing args. Lets users pick a shell that isn't on the
/// PATH probe (e.g. portable installs, custom toolchains).
fn shell_custom_program_input(
    scope: ShellScope,
    current: &crate::shell::ShellSpec,
    shared: &SharedState,
) -> ElementDef {
    let placeholder = if current.program.is_empty() {
        "custom shell path (press enter to apply)".to_string()
    } else {
        current.program.clone()
    };
    let s = shared.clone();
    let prefix = scope.set_cmd_prefix();
    let args = current.args.clone();
    ElementDef::new(Tag::Input)
        .with_class("input")
        .with_class("shell-custom-input")
        .with_placeholder(placeholder)
        .on_submit(move |text| {
            let typed = text.trim().to_string();
            if typed.is_empty() {
                return;
            }
            let spec = crate::shell::ShellSpec {
                program: typed,
                args: args.clone(),
            };
            let json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into());
            mutate_with(&s, |st| {
                dispatch(st, &format!("{prefix}{json}"));
            });
        })
}

/// Always visible args text input. Placeholder shows the current
/// args (space joined) so the user can see what's set without
/// pre-population (the framework's input doesn't seed initial value).
/// On submit, splits on whitespace and dispatches a fresh
/// `shell.set_*` with the existing program.
fn shell_args_input(
    scope: ShellScope,
    current: &crate::shell::ShellSpec,
    shared: &SharedState,
) -> ElementDef {
    let placeholder = if current.args.is_empty() {
        "optional args, space separated".to_string()
    } else {
        current.args.join(" ")
    };
    let s = shared.clone();
    let prefix = scope.set_cmd_prefix();
    let program = current.program.clone();
    ElementDef::new(Tag::Input)
        .with_class("input")
        .with_class("shell-args-input")
        .with_placeholder(placeholder)
        .on_submit(move |text| {
            let args: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();
            let spec = crate::shell::ShellSpec {
                program: program.clone(),
                args,
            };
            let json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into());
            mutate_with(&s, |st| {
                dispatch(st, &format!("{prefix}{json}"));
            });
        })
}

fn build_keybinds_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    use crate::keybinds::KeybindGroup;

    // No set-card shell: keybind groups sit directly on the page body, as
    // in the design mockup.
    let mut section = ElementDef::new(Tag::Div)
        .with_class("kb-page")
        .with_child(keybind_error_banner(state.keybinds.error.as_ref()));

    let filter = state.keybinds.filter.trim().to_lowercase();
    let total = KeybindAction::ALL.len();
    let mut visible_total = 0usize;
    let mut groups: Vec<ElementDef> = Vec::new();

    for group in KeybindGroup::ALL {
        let actions: Vec<KeybindAction> = KeybindAction::ALL
            .iter()
            .copied()
            .filter(|a| a.group() == *group)
            .filter(|a| keybind_matches_filter(*a, state, &filter))
            .collect();
        if actions.is_empty() {
            continue;
        }
        visible_total += actions.len();
        groups.push(keybind_group(*group, &actions, state, shared));
    }

    section = section.with_child(keybind_toolbar(state, shared, visible_total, total));

    if visible_total == 0 {
        section = section.with_child(
            ElementDef::new(Tag::Div)
                .with_class("kb-empty")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("big")
                        .with_text(format!("No commands match \u{201c}{}\u{201d}", filter)),
                )
                .with_child(ElementDef::new(Tag::Div).with_text("Try a different command or key.")),
        );
    } else {
        for g in groups {
            section = section.with_child(g);
        }
    }

    section
}

/// Case-insensitive match against label, description, and key names.
fn keybind_matches_filter(action: KeybindAction, state: &UiSnapshot, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let combo = state.keybinds.effective(action);
    let hay = format!(
        "{} {} {}",
        action.label(),
        action.description(),
        combo_parts(combo).join(" ")
    )
    .to_lowercase();
    hay.contains(filter)
}

fn keybind_toolbar(
    state: &UiSnapshot,
    shared: &SharedState,
    visible: usize,
    total: usize,
) -> ElementDef {
    let s = shared.clone();
    let count = if state.keybinds.filter.trim().is_empty() {
        format!("{total} commands")
    } else {
        format!("{visible} of {total}")
    };
    ElementDef::new(Tag::Div)
        .with_class("kb-toolbar")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("kb-filter")
                .with_child(svg_icon(icon_magnifier()))
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_id("kb-search")
                        .with_class("kb-filter-input")
                        .with_placeholder("filter commands or keys...")
                        .on_change(move |value| {
                            let value = value.to_string();
                            mutate_with(&s, |st| {
                                st.keybinds.filter = value.clone();
                            });
                        }),
                )
                .with_child(ElementDef::new(Tag::Span).with_class("kbd").with_text("/")),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("kb-count")
                .with_text(count),
        )
}

fn keybind_group(
    group: crate::keybinds::KeybindGroup,
    actions: &[KeybindAction],
    state: &UiSnapshot,
    shared: &SharedState,
) -> ElementDef {
    let head = ElementDef::new(Tag::Div)
        .with_class("kb-group-head")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("kb-group-title")
                .with_text(group.title()),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("kb-group-rule"))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("kb-group-count")
                .with_text(actions.len().to_string()),
        );

    let mut list = ElementDef::new(Tag::Div).with_class("kb-list");
    for action in actions {
        list = list.with_child(keybind_row(*action, state, shared));
    }

    ElementDef::new(Tag::Div)
        .with_class("kb-group")
        .with_child(head)
        .with_child(list)
}

fn keybind_group_icon(group: crate::keybinds::KeybindGroup) -> SvgNode {
    use crate::keybinds::KeybindGroup;
    match group {
        KeybindGroup::Panes => icon_split_panes(),
        KeybindGroup::Tabs => icon_tab_folder(),
        KeybindGroup::Navigation => icon_chevrons(),
        KeybindGroup::Application => icon_app_target(),
    }
}

fn build_notifications_section(shared: &SharedState) -> ElementDef {
    let test_notification_shared = shared.clone();
    let test_notification = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_id("settings-test-notification")
        .with_text("send test")
        .on_click(move || {
            mutate_with(&test_notification_shared, |st| {
                dispatch(st, "notifications.test");
            });
        });

    section_shell("notifications").with_child(setting_row(
        "test notification",
        "sends a notification targeted at the active workspace and terminal",
        test_notification,
    ))
}

fn keybind_error_banner(err: Option<&KeybindError>) -> ElementDef {
    let mut banner = ElementDef::new(Tag::Div).with_class("keybind-banner-error");
    match err {
        None => banner.with_class("hidden"),
        Some(e) => {
            let msg = match &e.kind {
                KeybindErrorKind::Conflict { other, combo } => {
                    format!(
                        "{} is already bound to \"{}\"; pick another combo.",
                        combo,
                        other.label()
                    )
                }
                KeybindErrorKind::InvalidCombo { combo, message } => {
                    format!("\"{}\" is not a valid combo: {}", combo, message)
                }
            };
            banner = banner.with_text(msg.as_str());
            banner
        }
    }
}

fn keybind_row(action: KeybindAction, state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let is_recording = state.keybinds.recording == Some(action);
    let is_overridden = state.keybinds.overrides.contains_key(&action);
    let has_error = state
        .keybinds
        .error
        .as_ref()
        .map(|e| e.action == action)
        .unwrap_or(false);
    let combo = state.keybinds.effective(action);

    let meta = ElementDef::new(Tag::Div)
        .with_class("kb-row-meta")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("kb-row-name")
                .with_text(action.label()),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("kb-row-desc")
                .with_text(action.description()),
        );

    let mut row = ElementDef::new(Tag::Div)
        .with_class("kb-row")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("kb-row-icon")
                .with_svg(keybind_group_icon(action.group())),
        )
        .with_child(meta)
        .with_child(keybind_binding(
            action,
            combo,
            is_recording,
            has_error,
            shared,
        ));
    if is_recording {
        row = row.with_class("recording");
    }

    if is_overridden {
        row = row.with_child(reset_row_button(action, shared));
    }

    row
}

/// The clickable binding button: keycaps joined by "+", with an edit pencil
/// that fades in on row hover; flips to a recording label while capturing.
fn keybind_binding(
    action: KeybindAction,
    combo: KeyCombo,
    is_recording: bool,
    has_error: bool,
    shared: &SharedState,
) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("kb-binding");
    if is_recording {
        btn = btn.with_class("recording");
    }
    if has_error {
        btn = btn.with_class("conflict");
    }

    if is_recording {
        btn = btn.with_child(
            ElementDef::new(Tag::Span)
                .with_class("rec-label")
                .with_child(ElementDef::new(Tag::Span).with_class("rec-dot"))
                .with_child(ElementDef::new(Tag::Span).with_text("press keys... (esc to cancel)")),
        );
    } else {
        // The "+" separators are real elements rather than `::before` pseudo
        // content. The framework now measures text+pseudo hosts correctly
        // (via anonymous text boxes), so this is a stylistic choice: real
        // spans keep the keycaps plain childless text leaves and the combo
        // structure explicit in one place.
        let mut keys = ElementDef::new(Tag::Span).with_class("keys");
        for (i, part) in combo_parts(combo).into_iter().enumerate() {
            if i > 0 {
                keys =
                    keys.with_child(ElementDef::new(Tag::Span).with_class("plus").with_text("+"));
            }
            keys = keys.with_child(pill("keycap", None, &part));
        }
        btn = btn.with_child(keys).with_child(
            ElementDef::new(Tag::Span)
                .with_class("edit-pencil")
                .with_svg(icon_pencil()),
        );
    }

    let s = shared.clone();
    let command = if is_recording {
        "keybind.cancel_record".to_string()
    } else {
        format!("keybind.record:{}", action.id())
    };
    btn.on_click(move || {
        mutate_with(&s, |st| dispatch(st, &command));
    })
}

fn reset_row_button(action: KeybindAction, shared: &SharedState) -> ElementDef {
    let s = shared.clone();
    let cmd = format!("keybind.reset:{}", action.id());
    ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_class("keybind-reset")
        .with_text("reset")
        .on_click(move || {
            mutate_with(&s, |st| dispatch(st, &cmd));
        })
}

/// Split a combo into the parts shown as individual key pills. Modifiers
/// are pushed in the canonical Ctrl, Shift, Alt, Meta order; then the key
/// name comes last.
fn combo_parts(combo: KeyCombo) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if combo.modifiers.contains(Modifiers::CTRL) {
        parts.push("Ctrl".to_string());
    }
    if combo.modifiers.contains(Modifiers::SHIFT) {
        parts.push("Shift".to_string());
    }
    if combo.modifiers.contains(Modifiers::ALT) {
        parts.push("Alt".to_string());
    }
    if combo.modifiers.contains(Modifiers::META) {
        parts.push("Meta".to_string());
    }
    parts.push(combo.key.to_string());
    parts
}

fn build_sessions_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let refresh_shared = shared.clone();
    let refresh = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_id("settings-sessions-refresh")
        .with_text("refresh")
        .on_click(move || {
            mutate_with(&refresh_shared, |st| {
                dispatch(st, "sessions.refresh");
            });
        });

    let mut control = ElementDef::new(Tag::Div)
        .with_class("sessions-refresh-control")
        .with_child(refresh);
    if state.sessions_stale {
        control = control.with_child(
            ElementDef::new(Tag::Span)
                .with_class("sessions-refresh-stale")
                .with_text("stale"),
        );
    }

    let mut section = section_shell("sessions").with_child(setting_row(
        "daemon sessions",
        "sessions currently tracked by the session daemon; refresh to re-poll",
        control,
    ));

    if state.sessions.is_empty() {
        section = section.with_child(
            ElementDef::new(Tag::Div)
                .with_class("sessions-empty")
                .with_text("no sessions; press refresh to poll the daemon"),
        );
        return section;
    }

    for s in &state.sessions {
        section = section.with_child(session_row(s, shared));
    }
    section
}

fn session_row(s: &crate::state::SessionSnapshot, shared: &SharedState) -> ElementDef {
    let label = s.name.clone().unwrap_or_else(|| match s.pid {
        Some(p) => format!("shell ({p})"),
        None => format!("shell (session {})", s.session_id),
    });
    let meta = ElementDef::new(Tag::Span)
        .with_class("setting-desc")
        .with_child(ElementDef::new(Tag::Span).with_text(format!(
            "workspace {} · pane {} · ",
            s.workspace_id, s.pane_id
        )))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class(if s.alive {
                    "session-status-alive"
                } else {
                    "session-status-dead"
                })
                .with_text(if s.alive { "alive" } else { "dead" }),
        );

    let kill_shared = shared.clone();
    let session_id = s.session_id;
    let kill = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("danger")
        .with_text("kill")
        .on_click(move || {
            mutate_with(&kill_shared, |st| {
                dispatch(st, &format!("session.kill:{session_id}"));
            });
        });

    let rename_shared = shared.clone();
    let pane_id = s.pane_id;
    let rename = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_text("rename")
        .on_click(move || {
            mutate_with(&rename_shared, |st| {
                dispatch(st, "modal.close");
                dispatch(st, &format!("tab.request_rename:{pane_id}"));
            });
        });

    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("setting-meta")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("setting-label")
                        .with_text(label),
                )
                .with_child(meta),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("session-row-actions")
                .with_child(rename)
                .with_child(kill),
        )
}

fn build_danger_zone_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let live_count = state.terminal_count;
    let button_shared = shared.clone();
    let kill_all = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("danger")
        .with_id("settings-kill-all-terminals")
        .on_click(move || {
            mutate_with(&button_shared, |st| {
                dispatch(st, "modal.close");
                dispatch(st, "app.request_kill_all_terminals");
            });
        })
        .with_text(if live_count == 0 {
            "kill all terminals".to_string()
        } else if live_count == 1 {
            "kill 1 terminal".to_string()
        } else {
            format!("kill {live_count} terminals")
        });

    let mut section = section_shell("danger zone").with_child(setting_row(
        "kill all terminals",
        "Destroys every running shell across every workspace. Workspaces are kept but emptied.",
        kill_all,
    ));

    if is_on(state, ToggleKey::RememberCloseChoice) {
        let kill_on_close = is_on(state, ToggleKey::KillAllOnClose);
        let desc = if kill_on_close {
            "Close currently kills every terminal and quits without asking. Reset to show the confirm prompt again."
        } else {
            "Close currently quits while leaving terminals running on the daemon. Reset to show the confirm prompt again."
        };
        let reset_shared = shared.clone();
        let reset = ElementDef::new(Tag::Button)
            .with_class("btn")
            .with_class("ghost")
            .with_id("settings-close-prompt-reset")
            .on_click(move || {
                mutate_with(&reset_shared, |st| {
                    dispatch(st, "app.close.reset_preference");
                });
            })
            .with_text("reset".to_string());
        section = section.with_child(setting_row("Close behavior", desc, reset));
    }

    section
}

fn build_modal_footer(shared: &SharedState) -> ElementDef {
    let cancel_state = shared.clone();
    let save_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-footer")
        .with_class("set-foot")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("modal-hint")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("kbd")
                        .with_text("esc"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("modal-hint-text")
                        .with_text(" close"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-footer-actions")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("ghost")
                        .with_id("settings-cancel")
                        .with_text("cancel")
                        .on_click(move || {
                            mutate_with(&cancel_state, |st| dispatch(st, "modal.close"));
                        }),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("primary")
                        .with_text("save changes")
                        .on_click(move || {
                            mutate_with(&save_state, |st| dispatch(st, "modal.close"));
                        }),
                ),
        )
}

// -- helpers ----------------------------------------------------------------

fn section_shell(title: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_class("set-card")
        .with_style(StyleDeclaration::Display(Display::Flex))
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_class("set-card-head")
                .with_class("name")
                .with_text(title),
        )
}

fn setting_meta(label: &str, desc: Option<&str>) -> ElementDef {
    setting_meta_impl(label, desc, None)
}

fn setting_meta_with_config_font(
    label: &str,
    desc: Option<&str>,
    config_font_size_pt: u32,
) -> ElementDef {
    setting_meta_impl(
        label,
        desc,
        Some((
            scaled_config_font_px(11.0, config_font_size_pt),
            scaled_config_font_px(10.0, config_font_size_pt),
        )),
    )
}

fn setting_meta_impl(
    label: &str,
    desc: Option<&str>,
    font_sizes: Option<(Option<f32>, Option<f32>)>,
) -> ElementDef {
    let mut label_el = ElementDef::new(Tag::Span)
        .with_class("setting-label")
        .with_class("set-label")
        .with_text(label);
    if let Some((Some(label_px), _)) = font_sizes {
        label_el = label_el.with_style(StyleDeclaration::FontSize(label_px));
    }

    let mut meta = ElementDef::new(Tag::Div)
        .with_class("setting-meta")
        .with_style(StyleDeclaration::Display(Display::Flex))
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_child(label_el);
    if let Some(desc) = desc {
        let mut desc_el = ElementDef::new(Tag::Span)
            .with_class("setting-desc")
            .with_class("set-desc")
            .with_text(desc);
        if let Some((_, Some(desc_px))) = font_sizes {
            desc_el = desc_el.with_style(StyleDeclaration::FontSize(desc_px));
        }
        meta = meta.with_child(desc_el);
    }
    meta
}

fn setting_row(label: &str, desc: &str, control: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_class("set-field")
        .with_child(setting_meta(label, Some(desc)))
        .with_child(control)
}

type StepCallback = Box<dyn Fn() + Send + Sync + 'static>;

struct StepCallbacks {
    on_dec: StepCallback,
    on_inc: StepCallback,
}

fn stepper(value: &str, callbacks: StepCallbacks) -> ElementDef {
    let dec = ElementDef::new(Tag::Button)
        .with_class("stepper-btn")
        .with_text("\u{2212}")
        .on_click(callbacks.on_dec);
    let inc = ElementDef::new(Tag::Button)
        .with_class("stepper-btn")
        .with_text("+")
        .on_click(callbacks.on_inc);
    ElementDef::new(Tag::Div)
        .with_class("stepper")
        .with_child(dec)
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("stepper-val")
                .with_class("tnum")
                .with_text(value),
        )
        .with_child(inc)
}

fn font_stepper(
    value: u32,
    dec_command: &'static str,
    inc_command: &'static str,
    shared: &SharedState,
) -> ElementDef {
    command_stepper(value.to_string(), dec_command, inc_command, shared)
}

fn command_stepper(
    value: String,
    dec_command: &'static str,
    inc_command: &'static str,
    shared: &SharedState,
) -> ElementDef {
    let dec_shared = shared.clone();
    let inc_shared = shared.clone();
    let callbacks = StepCallbacks {
        on_dec: Box::new(move || {
            mutate_with(&dec_shared, |st| dispatch(st, dec_command));
        }),
        on_inc: Box::new(move || {
            mutate_with(&inc_shared, |st| dispatch(st, inc_command));
        }),
    };
    stepper(&value, callbacks)
}

fn density_segmented(active: UiDensity, shared: &SharedState) -> ElementDef {
    let mut segmented = ElementDef::new(Tag::Div).with_class("input-segmented");
    for density in UiDensity::all() {
        let s = shared.clone();
        let command = format!("appearance.density:{}", density.id());
        let mut button = ElementDef::new(Tag::Button)
            .with_class("seg-btn")
            .with_text(density.label())
            .on_click(move || {
                let command = command.clone();
                mutate_with(&s, move |st| dispatch(st, &command));
            });
        if density == active {
            button = button.with_class("active");
        }
        segmented = segmented.with_child(button);
    }
    segmented
}

fn pill(base: &str, modifier: Option<&str>, text: &str) -> ElementDef {
    let mut el = ElementDef::new(Tag::Span).with_class(base).with_text(text);
    if let Some(m) = modifier {
        el = el.with_class(m);
    }
    el
}

fn svg_icon(svg: SvgNode) -> ElementDef {
    ElementDef::new(Tag::Div).with_svg(svg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SettingsSection};
    use std::sync::{Arc, Mutex};
    use unshit::core::element::{ElementContent, ElementTree, LayoutRect};
    use unshit::core::style::types::{
        Background, Color, CssPosition, Dimension, FontWeight, Overflow, TextAlign,
    };
    use unshit_test::TestHarness;

    fn make_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn make_snapshot() -> UiSnapshot {
        seed_state().ui_snapshot()
    }

    fn make_snapshot_section(section: SettingsSection) -> UiSnapshot {
        let mut state = seed_state();
        state.settings_section = section;
        state.ui_snapshot()
    }

    fn text_of(el: &ElementDef) -> Option<&str> {
        match &el.content {
            ElementContent::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    // -- build_settings_modal ---------------------------------------------------

    #[test]
    fn settings_modal_has_modal_class() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_settings_modal(&snap, &shared);
        assert!(el.classes.contains(&"modal".to_string()));
    }

    #[test]
    fn settings_modal_matches_design_system_shell_structure() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_settings_modal(&snap, &shared);
        // header, set-body(nav + content), footer
        assert_eq!(el.children.len(), 3);
        assert!(el.children[1].classes.contains(&"set-body".to_string()));
        assert!(el.children[1].children[0]
            .classes
            .contains(&"set-nav-rail".to_string()));
        assert!(el.children[1].children[1]
            .classes
            .contains(&"set-content".to_string()));
    }

    #[test]
    fn settings_page_matches_design_system_page_structure() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_settings_page(&snap, &shared);

        assert!(el.classes.contains(&"settings-page".to_string()));
        assert_eq!(el.children.len(), 2);
        assert!(el.children[0]
            .classes
            .contains(&"set-page-rail".to_string()));
        assert!(el.children[1]
            .classes
            .contains(&"set-page-content".to_string()));
        assert!(has_class_anywhere(&el, "set-page-search"));
        assert!(has_class_anywhere(&el, "set-page-header"));
        assert!(has_class_anywhere(&el, "set-page-savebar"));
    }

    #[test]
    fn settings_page_appearance_renders_applied_controls_and_preview() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_settings_page(&snap, &shared);

        assert_eq!(count_with_class(&el, "set-card"), 5);
        assert!(has_class_anywhere(&el, "stepper"));
        assert!(has_class_anywhere(&el, "set-inline-control"));
        assert!(has_class_anywhere(&el, "input-num"));
        assert!(has_class_anywhere(&el, "input-segmented"));
        assert!(has_class_anywhere(&el, "preview-tile"));
        assert!(has_class_anywhere(&el, "preview-head"));
        assert!(has_class_anywhere(&el, "preview-body"));
        assert!(has_class_anywhere(&el, "theme-picker"));
        assert!(has_class_anywhere(&el, "agent-tag"));
        assert!(has_class_anywhere(&el, "cur"));
        assert!(!has_class_anywhere(&el, "color-swatches"));
        assert!(!has_class_anywhere(&el, "toggle"));
        let text = collect_text_recursive(&el);
        assert!(text.contains("Theme"));
        assert!(text.contains(
            "Themes, density, and the visual feel of the terminal. Changes apply immediately."
        ));
        assert!(text.contains("ptyd up · session"));
        assert!(text.contains("\u{2318}"));
        assert!(text.contains('F'));
        assert!(has_class_anywhere(&el, "kbd-command"));
        assert!(has_class_anywhere(&el, "kbd-key"));
        assert!(text.contains("Config font size"));
        assert!(text.contains("Density"));
        assert!(text.contains("Vertical padding inside lists and panes"));
        assert!(text.contains("compact"));
        assert!(text.contains("cozy"));
        assert!(text.contains("comfy"));
        assert!(text.contains("Wheel scroll step"));
        assert!(text.contains("Pixels moved per wheel notch"));
        assert!(text.contains("Smooth scroll duration"));
        assert!(text.contains("Animation time after wheel input"));
        assert!(text.contains("Terminal font size"));
        assert!(text.contains("Settings and app chrome text size"));
        assert!(text.contains("Terminal output size"));
        assert!(text.contains("Sidebar width"));
        assert!(text.contains("Width of the workspace sidebar"));
        // Tabs card: sizing mode, fixed width stepper, and row mode.
        assert!(text.contains("Tab sizing"));
        assert!(text.contains("fit content"));
        assert!(text.contains("Tab width"));
        assert!(text.contains("Tab rows"));
        assert!(text.contains("single"));
        assert!(text.contains("double"));
        assert!(text.contains("triple"));
        assert!(text.contains("syntax tints across the whole app"));
        assert!(text.contains("~/code/main/dashboard"));
        assert!(text.contains("patching format.ts"));
        assert!(text.contains("changes apply immediately"));
        for stripped in ["Accent", "Scanline overlay", "Background grain"] {
            assert!(
                !text.contains(stripped),
                "settings page should not render unapplied/fake setting {stripped:?}"
            );
        }
    }

    #[test]
    fn tabs_card_hides_width_stepper_in_fit_content_mode() {
        // In fixed mode the width stepper is offered; in fit-content mode
        // there is no width to tune, so the "Tab width" field is dropped.
        let fixed = crate::state::UiSnapshot {
            tab_width_mode: crate::state::TabWidthMode::Fixed,
            ..make_snapshot_section(SettingsSection::Appearance)
        };
        let shared = make_shared();
        let fixed_card = build_tabs_card(&fixed, &shared);
        assert!(collect_text_recursive(&fixed_card).contains("Tab width"));

        let fit = crate::state::UiSnapshot {
            tab_width_mode: crate::state::TabWidthMode::FitContent,
            ..make_snapshot_section(SettingsSection::Appearance)
        };
        let fit_card = build_tabs_card(&fit, &shared);
        let fit_text = collect_text_recursive(&fit_card);
        assert!(fit_text.contains("Tab sizing"));
        assert!(fit_text.contains("Tab rows"));
        assert!(
            !fit_text.contains("Tab width"),
            "fit-content mode must not render the fixed-width stepper"
        );
    }

    #[test]
    fn settings_page_appearance_has_theme_picker() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_settings_page(&snap, &shared);
        let picker = find_first_with_class(&el, "theme-picker")
            .expect("theme page should include a theme picker");
        assert!(
            !picker.children.is_empty(),
            "theme picker should have at least one chip"
        );
        assert!(count_with_class(picker, "theme-chip") >= 2);
        let theme_row =
            find_first_with_class(&el, "theme-chip").expect("theme picker should render chips");
        assert!(
            theme_row.classes.contains(&"theme-chip".to_string()),
            "theme picker chip should have theme-chip class"
        );
    }

    #[test]
    fn appearance_page_density_control_updates_density_immediately() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_settings_page(&snap, &shared);
        let segmented =
            find_first_with_class(&el, "input-segmented").expect("density segmented control");

        assert_eq!(segmented.children.len(), UiDensity::all().len());
        assert!(segmented.children[1]
            .classes
            .contains(&"active".to_string()));
        (segmented.children[2].on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().ui_density, UiDensity::Comfy);
    }

    #[test]
    fn appearance_page_scroll_controls_update_tuning_immediately() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_settings_page(&snap, &shared);
        let mut steppers = Vec::new();
        collect_with_class(&el, "stepper", &mut steppers);
        let wheel_stepper = steppers
            .iter()
            .copied()
            .find(|stepper| {
                collect_text_recursive(stepper)
                    .contains(&crate::state::DEFAULT_SCROLL_LINE_PX.to_string())
            })
            .expect("wheel scroll stepper");
        let duration_stepper = steppers
            .iter()
            .copied()
            .find(|stepper| {
                collect_text_recursive(stepper).contains(&format!(
                    "{} ms",
                    crate::state::DEFAULT_SMOOTH_SCROLL_DURATION_MS
                ))
            })
            .expect("smooth duration stepper");

        (wheel_stepper.children[2].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().scroll_line_px,
            crate::state::DEFAULT_SCROLL_LINE_PX + 4
        );
        (duration_stepper.children[0].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().smooth_scroll_duration_ms,
            crate::state::DEFAULT_SMOOTH_SCROLL_DURATION_MS - 10
        );
    }

    #[test]
    fn appearance_page_theme_picker_click_updates_theme_immediately() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let first_render = build_settings_page(&snap, &shared);
        let picker = find_first_with_class(&first_render, "theme-picker")
            .expect("theme picker should exist");
        let dracula_chip = picker.children.iter().find(|chip| {
            chip.classes
                .iter()
                .any(|class_name| class_name == "dracula")
        });
        let dracula_chip = dracula_chip.expect("Dracula chip should exist");
        (dracula_chip.on_click.as_ref().unwrap())();
        let active = shared.lock().unwrap().theme.clone();
        assert_eq!(active, "dracula");

        let snap = shared.lock().unwrap().ui_snapshot();
        let second_render = build_settings_page(&snap, &shared);
        let second_picker = find_first_with_class(&second_render, "theme-picker")
            .expect("theme picker should re-render");
        let active = second_picker
            .children
            .iter()
            .find(|chip| chip.classes.iter().any(|c| c == "active"));
        assert!(
            active
                .and_then(|chip| {
                    chip.classes
                        .iter()
                        .find(|class_name| *class_name == "dracula")
                        .map(|_| ())
                })
                .is_some(),
            "active theme chip should update to Dracula after click"
        );
    }

    #[test]
    fn appearance_page_custom_theme_chip_opens_editor_and_updates_color() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let first_render = build_settings_page(&snap, &shared);
        let picker = find_first_with_class(&first_render, "theme-picker")
            .expect("theme picker should exist");
        let custom_chip = picker
            .children
            .iter()
            .find(|chip| chip.classes.iter().any(|class_name| class_name == "custom"))
            .expect("custom chip should exist");
        assert!(has_class_anywhere(custom_chip, "theme-chip-pattern"));

        (custom_chip.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().theme, theme::CUSTOM_THEME_ID);

        let snap = shared.lock().unwrap().ui_snapshot();
        let second_render = build_settings_page(&snap, &shared);
        assert!(has_class_anywhere(&second_render, "custom-editor"));
        assert_eq!(
            count_with_class(&second_render, "color-field"),
            theme::custom_theme_slots().len()
        );

        let color_input =
            find_first_with_class(&second_render, "color-input").expect("color input");
        (color_input.on_change.as_ref().unwrap())("#123456");
        assert_eq!(
            theme::color_to_hex(shared.lock().unwrap().custom_theme.accent),
            "#123456"
        );
    }

    #[test]
    fn settings_page_controls_have_visible_layout_with_stylesheet() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_class("theme-amber")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            1280.0,
            800.0,
        );
        harness.set_scale_factor(1.5);
        harness.step();

        for selector in [
            ".stepper",
            ".stepper-btn",
            ".set-inline-control",
            ".input-text",
            ".input-num",
            ".set-unit",
            ".preview-tile",
            ".theme-picker",
            ".theme-chip-screen",
            ".set-page-savebar",
        ] {
            let snap = harness.query(selector).expect(selector);
            assert!(
                snap.layout_rect.width > 0.0 && snap.layout_rect.height > 0.0,
                "{selector} should have non-zero layout, got {:?}",
                snap.layout_rect
            );
            assert!(
                snap.layout_rect.x >= 0.0 && snap.layout_rect.x + snap.layout_rect.width <= 1280.0,
                "{selector} should be horizontally visible, got {:?}",
                snap.layout_rect
            );
        }

        let stepper_btn = harness.query(".stepper-btn").expect(".stepper-btn");
        assert_eq!(
            stepper_btn.computed_style.text_align,
            TextAlign::Center,
            "button UA defaults should center stepper glyphs unless CSS overrides them"
        );
        let number = harness.query(".input-num").expect(".input-num");
        assert_eq!(
            number.computed_style.text_align,
            TextAlign::Right,
            "settings number readouts rely on source CSS text-align"
        );
    }

    #[test]
    fn settings_page_theme_picker_is_compact_and_scrollable() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_class("theme-amber")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            900.0,
            700.0,
        );
        harness.step();

        let content = harness
            .query(".set-page-content")
            .expect("settings page content should exist");
        assert_eq!(
            content.computed_style.overflow_y,
            Overflow::Scroll,
            "settings page content must remain wheel-scrollable"
        );

        let chips = harness.query_all(".theme-chip");
        assert_eq!(chips.len(), theme::themes().len() + 1);
        assert_eq!(
            chips
                .first()
                .expect("theme picker should render chips")
                .computed_style
                .overflow_x,
            Overflow::Hidden,
            "theme chip should clip pattern and selected pin like the Claude design"
        );

        let bodies = harness.query_all(".theme-chip-screen");
        assert_eq!(bodies.len(), theme::themes().len() + 1);
        let first_body = bodies
            .first()
            .expect("theme chip should render a visible body row");
        assert!(
            first_body.layout_rect.width > 100.0 && first_body.layout_rect.height > 40.0,
            "theme chip body should be visible, got {:?}",
            first_body.layout_rect
        );

        let labels = harness.query_all(".tcs-name");
        assert_eq!(labels.len(), theme::themes().len() + 1);
        let first_label = labels
            .first()
            .expect("theme chip should render a visible label");
        assert!(
            first_label.layout_rect.width > 0.0 && first_label.layout_rect.height > 0.0,
            "theme chip label should have non-zero layout, got {:?}",
            first_label.layout_rect
        );

        let pins = harness.query_all(".theme-chip-pin");
        assert_eq!(pins.len(), theme::themes().len() + 1);
        let active_pin = harness
            .query(".theme-chip.active .theme-chip-pin")
            .expect("active theme chip pin");
        assert_eq!(
            active_pin.computed_style.position,
            CssPosition::Absolute,
            "selected checkmark should be absolutely positioned like the Claude design"
        );
        assert!(
            active_pin.computed_style.border_radius.top_left >= 5.0,
            "selected checkmark should render rounded, got {:?}",
            active_pin.computed_style.border_radius
        );

        let first_y = chips
            .first()
            .expect("theme picker should render chips")
            .layout_rect
            .y;
        let first_row = chips
            .iter()
            .filter(|chip| (chip.layout_rect.y - first_y).abs() < 1.0)
            .count();
        assert!(
            first_row >= 2,
            "theme chips should use multiple columns instead of one clipped vertical list"
        );
    }

    #[test]
    fn theme_picker_layout_does_not_depend_on_set_field_class() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div).with_class("app").with_child(
                    ElementDef::new(Tag::Div).with_class("set-card").with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("setting-row")
                            .with_class("theme-field")
                            .with_child(
                                ElementDef::new(Tag::Div)
                                    .with_class("setting-meta")
                                    .with_child(
                                        ElementDef::new(Tag::Div)
                                            .with_class("set-label")
                                            .with_text("Color theme"),
                                    )
                                    .with_child(
                                        ElementDef::new(Tag::Div)
                                            .with_class("set-desc")
                                            .with_text("Choose an appearance preset."),
                                    ),
                            )
                            .with_child(
                                ElementDef::new(Tag::Div)
                                    .with_class("set-control")
                                    .with_child(build_theme_picker(&tree_snap, &tree_shared)),
                            ),
                    ),
                ),
            },
            1321.0,
            415.0,
        );
        let can_render = harness.try_with_gpu();
        harness.step();

        let picker = harness.query(".theme-picker").expect("theme picker");
        assert!(
            picker.layout_rect.width > 700.0,
            "target-style theme-field markup should give the picker full width, got {:?}",
            picker.layout_rect
        );

        let amber = harness.query(".theme-chip.amber").expect("amber chip");
        let catppuccin = harness
            .query(".theme-chip.catppuccin")
            .expect("catppuccin chip");
        assert!(
            amber.layout_rect.width > 180.0 && catppuccin.layout_rect.x > amber.layout_rect.x,
            "theme chips should render as visible cards, got amber {:?}, catppuccin {:?}",
            amber.layout_rect,
            catppuccin.layout_rect
        );

        let amber_label = harness.query(".theme-chip.amber .tcs-name").expect("label");
        assert!(
            amber_label.layout_rect.width > 0.0 && amber_label.layout_rect.height > 0.0,
            "theme chip label should not collapse, got {:?}",
            amber_label.layout_rect
        );

        if can_render {
            let pixels = harness.render();
            assert!(
                rect_has_near_rgb(
                    &pixels,
                    1321,
                    415,
                    amber_label.layout_rect,
                    Color::rgb(0xeb, 0xdc, 0xb6),
                    48,
                ),
                "theme chip label should paint readable text, got {:?}",
                amber_label.layout_rect
            );
        }
    }

    #[test]
    fn settings_page_target_viewport_has_visible_scrollbar_geometry() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            925.0,
            540.0,
        );
        harness.step();

        let content = harness
            .query(".set-page-content")
            .expect("settings page content");
        let (vertical, _) = unshit::core::scroll::compute_scrollbar_geometry(
            harness.arena(),
            content.node_id,
            content.layout_rect.x,
            content.layout_rect.y,
        );

        let vertical = vertical.unwrap_or_else(|| {
            panic!(
                "target viewport should expose a vertical scrollbar, content rect {:?}",
                content.layout_rect
            )
        });
        assert!(
            (vertical.track_x - 913.0).abs() <= 1.0
                && (vertical.track_y - 52.0).abs() <= 1.0
                && (vertical.track_w - 12.0).abs() <= 0.1
                && vertical.thumb_h >= 120.0,
            "target viewport scrollbar should match the browser-like right edge, got {:?}",
            vertical
        );
    }

    #[test]
    fn settings_page_target_viewport_scrollbar_is_visible_at_rest() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let width = 925u32;
        let height = 540u32;
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            width as f32,
            height as f32,
        );
        if !harness.try_with_gpu() {
            return;
        }
        harness.step();

        let pixels = harness.render();
        let page_sample = pixel_at(&pixels, width, 904, 120);
        let thumb_sample = pixel_at(&pixels, width, 918, 120);
        let page_luma =
            u16::from(page_sample[0]) + u16::from(page_sample[1]) + u16::from(page_sample[2]);
        let thumb_luma =
            u16::from(thumb_sample[0]) + u16::from(thumb_sample[1]) + u16::from(thumb_sample[2]);
        assert!(
            thumb_luma > page_luma + 2 && thumb_luma < 100,
            "idle settings scrollbar should be visible but subdued, page={page_sample:?}, thumb={thumb_sample:?}"
        );
    }

    #[test]
    fn settings_page_target_viewport_matches_claude_body_geometry() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            925.0,
            540.0,
        );
        harness.step();

        let card = harness
            .query(".settings-page .set-card")
            .expect("theme card");
        assert!(
            (card.layout_rect.x - 272.0).abs() <= 1.0
                && (card.layout_rect.width - 607.0).abs() <= 1.0,
            "target viewport theme card should match Claude geometry, got {:?}",
            card.layout_rect
        );

        let amber = harness
            .query(".theme-chip.amber")
            .expect("amber theme chip");
        assert!(
            (amber.layout_rect.x - 290.0).abs() <= 1.0
                && (amber.layout_rect.width - 285.0).abs() <= 1.0,
            "target viewport chip width should match Claude two-column picker, got {:?}",
            amber.layout_rect
        );

        let catppuccin = harness
            .query(".theme-chip.catppuccin")
            .expect("catppuccin theme chip");
        assert!(
            (catppuccin.layout_rect.x - 587.0).abs() <= 1.0
                && (catppuccin.layout_rect.width - 285.0).abs() <= 1.0,
            "second theme chip should align with Claude target, got {:?}",
            catppuccin.layout_rect
        );
    }

    #[test]
    fn settings_page_savebar_is_pinned_in_target_viewport() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            925.0,
            540.0,
        );
        harness.step();

        let content = harness
            .query(".set-page-content")
            .expect("settings page content");
        let savebar = harness
            .query(".set-page-savebar")
            .expect("settings page savebar");
        assert_eq!(savebar.computed_style.position, CssPosition::Absolute);
        let saved_dot = harness
            .query(".set-page-savebar .saved-dot")
            .expect("savebar saved dot");
        assert!(
            saved_dot.computed_style.border_radius.top_left >= 3.0,
            "savebar status dot should render rounded, got {:?}",
            saved_dot.computed_style.border_radius
        );

        let content_bottom = content.layout_rect.y + content.layout_rect.height;
        let savebar_bottom = savebar.layout_rect.y + savebar.layout_rect.height;
        assert!(
            (content_bottom - savebar_bottom).abs() <= 1.0,
            "savebar should pin to content bottom: content {:?}, savebar {:?}",
            content.layout_rect,
            savebar.layout_rect
        );
    }

    fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y as usize) * width as usize + x as usize) * 4;
        [
            pixels[idx],
            pixels[idx + 1],
            pixels[idx + 2],
            pixels[idx + 3],
        ]
    }

    fn near_rgb(actual: [u8; 4], expected: Color, tolerance: u8) -> bool {
        [actual[0], actual[1], actual[2]]
            .into_iter()
            .zip([expected.r, expected.g, expected.b])
            .all(|(a, e)| a.abs_diff(e) <= tolerance)
    }

    fn rect_has_near_rgb(
        pixels: &[u8],
        width: u32,
        height: u32,
        rect: LayoutRect,
        expected: Color,
        tolerance: u8,
    ) -> bool {
        let x0 = rect.x.floor().max(0.0) as u32;
        let y0 = rect.y.floor().max(0.0) as u32;
        let x1 = (rect.x + rect.width).ceil().clamp(0.0, width as f32) as u32;
        let y1 = (rect.y + rect.height).ceil().clamp(0.0, height as f32) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                if near_rgb(pixel_at(pixels, width, x, y), expected, tolerance) {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn settings_page_theme_chip_styles_resolve_to_visible_preview_and_text() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            900.0,
            700.0,
        );
        harness.step();

        let amber_screen = harness
            .query(".theme-chip.amber .theme-chip-screen")
            .expect("amber chip screen");
        assert_eq!(
            amber_screen.computed_style.background,
            Background::Color(Color::rgb(0x1c, 0x18, 0x12)),
            "amber chip preview must use Claude target base surface"
        );

        let catppuccin_screen = harness
            .query(".theme-chip.catppuccin .theme-chip-screen")
            .expect("catppuccin chip screen");
        assert_eq!(
            catppuccin_screen.computed_style.background,
            Background::Color(Color::rgb(0x1e, 0x1e, 0x2e)),
            "theme chip previews should use each target theme base color, not the brighter row color"
        );

        let catppuccin_sub = harness
            .query(".theme-chip.catppuccin .tcs-sub")
            .expect("catppuccin chip subtitle");
        assert_eq!(
            catppuccin_sub.computed_style.color,
            Color::rgb(0x66, 0x6f, 0x86),
            "catppuccin chip subtitle should match the measured Claude preview tone"
        );

        let amber_name = harness
            .query(".theme-chip.amber .tcs-name")
            .expect("amber chip name");
        assert_eq!(
            amber_name.computed_style.color,
            Color::rgb(0xeb, 0xdc, 0xb6),
            "amber chip label must be bright enough to read"
        );
        assert_eq!(
            amber_name.computed_style.font_weight,
            FontWeight::W(700),
            "theme chip label should use the filled Claude target weight"
        );
        let header_blurb = harness
            .query(".set-page-header .blurb")
            .expect("settings header blurb");
        assert_eq!(
            header_blurb.computed_style.color,
            Color::rgb(0xb8, 0xa2, 0x75),
            "settings header blurb should use the brighter secondary text token"
        );
        let theme_label = harness
            .query(".theme-field .set-label")
            .expect("theme field label");
        assert_eq!(
            theme_label.computed_style.font_weight,
            FontWeight::W(700),
            "settings labels should use the filled Claude target weight"
        );
        let active_nav = harness
            .query(".set-page-nav-item.active")
            .expect("active settings nav item");
        assert_eq!(
            active_nav.computed_style.font_weight,
            FontWeight::W(700),
            "settings nav labels should use the heavier Claude target weight"
        );
        assert_eq!(
            active_nav.computed_style.background,
            Background::Color(Color::TRANSPARENT),
            "active nav item uses a shifted background layer so the text and stripe stay aligned"
        );
        let active_nav_bg = harness
            .arena()
            .children(active_nav.node_id)
            .iter()
            .filter_map(|child| harness.arena().get(*child))
            .find(|child| {
                child.synthetic
                    && child.computed_style.position == CssPosition::Absolute
                    && child.computed_style.background
                        == Background::Color(Color::rgb(0x29, 0x23, 0x1a))
            })
            .expect(
                "active nav item should draw its browser-matched background as a shifted layer",
            );
        assert!(
            matches!(active_nav_bg.computed_style.top, Some(Dimension::Px(y)) if (y - 4.0).abs() <= 0.01)
        );
        assert!(
            matches!(active_nav_bg.computed_style.bottom, Some(Dimension::Px(y)) if (y + 3.0).abs() <= 0.01)
        );
        assert_eq!(
            active_nav_bg.computed_style.z_index, -1,
            "active nav background should paint behind icon/text and the accent stripe"
        );
        let custom_screen = harness
            .query(".theme-chip.custom .theme-chip-screen")
            .expect("custom chip screen");
        assert!(
            matches!(
                custom_screen.computed_style.background,
                Background::LinearGradient(_)
            ),
            "custom chip preview should use the accent gradient"
        );
        let custom_name = harness
            .query(".theme-chip.custom .tcs-name")
            .expect("custom chip name");
        assert_eq!(
            custom_name.computed_style.color,
            Color::rgb(0x0e, 0x16, 0x20),
            "custom chip text should match the dark Claude accent-on foreground over the gradient"
        );
        let tokyo_sub = harness
            .query(".theme-chip.tokyo-night .tcs-sub")
            .expect("tokyo night chip subtitle");
        assert_eq!(
            tokyo_sub.computed_style.color,
            Color::rgb(0x48, 0x51, 0x6c),
            "tokyo night subtitle uses the browser-matched contrast compensation"
        );
        assert!(
            harness
                .query(".theme-chip.amber.active .theme-chip-top-hairline")
                .is_none(),
            "active chip should not draw the inactive browser hairline"
        );
        let inactive_hairline = harness
            .query(".theme-chip.catppuccin .theme-chip-top-hairline")
            .expect("inactive theme chip hairline");
        assert_eq!(
            inactive_hairline.computed_style.position,
            CssPosition::Absolute
        );
        assert_eq!(
            inactive_hairline.computed_style.background,
            Background::Color(Color::rgb(0x34, 0x2b, 0x1e)),
            "inactive chip top hairline should match the Claude/browser edge color"
        );
        assert!(
            matches!(inactive_hairline.computed_style.height, Dimension::Px(h) if (h - 1.0).abs() <= 0.01)
        );
        assert!(
            matches!(inactive_hairline.computed_style.left, Some(Dimension::Px(x)) if (x - 4.0).abs() <= 0.01)
        );
        assert!(
            matches!(inactive_hairline.computed_style.right, Some(Dimension::Px(x)) if (x - 1.0).abs() <= 0.01)
        );
        assert!(
            matches!(inactive_hairline.computed_style.top, Some(Dimension::Px(y)) if (y + 1.0).abs() <= 0.01)
        );
        for head in harness.query_all(".settings-page .set-card-head") {
            assert_eq!(
                head.computed_style.border_width.bottom, 0.0,
                "settings card header uses a pseudo separator so content geometry does not shift"
            );
            let separator = harness
                .arena()
                .children(head.node_id)
                .iter()
                .filter_map(|child| harness.arena().get(*child))
                .find(|child| {
                    child.synthetic
                        && child.computed_style.position == CssPosition::Absolute
                        && child.computed_style.background
                            == Background::Color(Color::rgb(0x24, 0x1e, 0x15))
                })
                .expect("settings card header separator pseudo should stay visible");
            assert!(
                matches!(separator.computed_style.height, Dimension::Px(h) if (h - 1.0).abs() <= 0.01)
            );
            assert!(
                matches!(separator.computed_style.bottom, Some(Dimension::Px(y)) if (y + 2.0).abs() <= 0.01)
            );
            assert_eq!(
                separator.layout_rect.height, 1.0,
                "settings card header separator should render as a target hairline"
            );
        }
    }

    #[test]
    fn settings_preview_restyles_when_theme_changes() {
        let mut state = seed_state();
        state.settings_section = SettingsSection::Appearance;
        state.theme = "amber".to_string();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let build_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || {
                let snap = build_shared.lock().unwrap().ui_snapshot();
                ElementTree {
                    root: ElementDef::new(Tag::Div)
                        .with_class("app")
                        .with_class(crate::theme::theme_class_name(&snap.theme))
                        .with_class("settings")
                        .with_child(build_settings_page(&snap, &build_shared)),
                }
            },
            900.0,
            700.0,
        );

        shared.lock().unwrap().theme = "dracula".to_string();
        let rebuild_shared = shared.clone();
        harness.rebuild(move || {
            let snap = rebuild_shared.lock().unwrap().ui_snapshot();
            ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class(crate::theme::theme_class_name(&snap.theme))
                    .with_class("settings")
                    .with_child(build_settings_page(&snap, &rebuild_shared)),
            }
        });

        let preview = harness.query(".preview-tile").expect("preview tile");
        assert_eq!(
            preview.computed_style.background,
            Background::Color(Color::rgb(0x28, 0x2a, 0x36)),
            "theme preview background should follow the selected theme"
        );
        assert_eq!(
            preview.computed_style.color,
            Color::rgb(0xf8, 0xf8, 0xf2),
            "theme preview text should follow the selected theme"
        );

        let preview_head = harness.query(".preview-head").expect("preview head");
        assert_eq!(
            preview_head.computed_style.background,
            Background::Color(Color::rgb(0x21, 0x22, 0x2c)),
            "theme preview chrome should follow the selected theme"
        );

        let prompt = harness.query(".preview-tile .prompt").expect("prompt");
        assert_eq!(
            prompt.computed_style.color,
            Color::rgb(0xbd, 0x93, 0xf9),
            "theme preview prompt should follow the selected theme accent"
        );
    }

    #[test]
    fn settings_page_config_font_size_scales_body_labels_immediately() {
        let mut state = seed_state();
        state.settings_section = SettingsSection::Appearance;
        state.config_font_size_pt = 15;
        let snap = state.ui_snapshot();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            925.0,
            540.0,
        );
        harness.step();

        let body = harness.query(".set-page-body").expect("settings page body");
        let expected_body = 12.0 * 15.0 / DEFAULT_CONFIG_FONT_SIZE_PT as f32;
        assert!(
            (body.computed_style.font_size - expected_body).abs() <= 0.01,
            "config font 15pt should scale page body relative to the app default, got {}",
            body.computed_style.font_size
        );

        let label = harness
            .query(".theme-field .set-label")
            .expect("theme field label");
        let expected_label = 11.0 * 15.0 / DEFAULT_CONFIG_FONT_SIZE_PT as f32;
        assert!(
            (label.computed_style.font_size - expected_label).abs() <= 0.01,
            "config font should scale setting labels immediately, got {}",
            label.computed_style.font_size
        );

        let desc = harness
            .query(".theme-field .set-desc")
            .expect("theme field description");
        let expected_desc = 10.0 * 15.0 / DEFAULT_CONFIG_FONT_SIZE_PT as f32;
        assert!(
            (desc.computed_style.font_size - expected_desc).abs() <= 0.01,
            "config font should scale setting descriptions immediately, got {}",
            desc.computed_style.font_size
        );
    }

    #[test]
    fn settings_page_theme_chip_preview_paints_in_renderer() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let width = 925u32;
        let height = 540u32;
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            width as f32,
            height as f32,
        );
        if !harness.try_with_gpu() {
            return;
        }
        harness.step();

        let screen = harness
            .query(".theme-chip.amber .theme-chip-screen")
            .expect("amber chip screen");
        let x = (screen.layout_rect.x + screen.layout_rect.width * 0.82)
            .round()
            .clamp(0.0, (width - 1) as f32) as u32;
        let y = (screen.layout_rect.y + screen.layout_rect.height - 10.0)
            .round()
            .clamp(0.0, (height - 1) as f32) as u32;
        let pixels = harness.render();
        let sample = pixel_at(&pixels, width, x, y);
        assert!(
            near_rgb(sample, Color::rgb(0x1c, 0x18, 0x12), 8),
            "amber preview should paint at ({x}, {y}), got {sample:?}"
        );

        let active_pin = harness
            .query(".theme-chip.amber.active .theme-chip-pin")
            .expect("active amber chip pin");
        let pin_x = (active_pin.layout_rect.x + active_pin.layout_rect.width * 0.5)
            .round()
            .clamp(0.0, (width - 1) as f32) as u32;
        let pin_y = (active_pin.layout_rect.y + active_pin.layout_rect.height * 0.5)
            .round()
            .clamp(0.0, (height - 1) as f32) as u32;
        let pin_sample = pixel_at(&pixels, width, pin_x, pin_y);
        assert!(
            near_rgb(pin_sample, Color::rgb(0x1c, 0x18, 0x12), 8),
            "browser target keeps the active checkmark behind the preview surface at ({pin_x}, {pin_y}), got {pin_sample:?}"
        );

        let amber_name = harness
            .query(".theme-chip.amber .tcs-name")
            .expect("amber chip label");
        assert!(
            rect_has_near_rgb(
                &pixels,
                width,
                height,
                amber_name.layout_rect,
                Color::rgb(0xeb, 0xdc, 0xb6),
                48,
            ),
            "amber theme label should paint readable text, label rect {:?}",
            amber_name.layout_rect
        );

        let amber_chip = harness.query(".theme-chip.amber").expect("amber chip");
        assert_eq!(
            amber_chip.computed_style.outline_width, 1.0,
            "active theme chip should keep a browser-style outline above its children"
        );
        assert_eq!(
            amber_chip.computed_style.outline_color,
            Color::rgb(0xd4, 0xa3, 0x48),
            "active amber chip outline should use the amber accent"
        );
        let outline_x = (amber_chip.layout_rect.x + 4.0)
            .round()
            .clamp(0.0, (width - 1) as f32) as u32;
        let outline_y = (amber_chip.layout_rect.y - 1.0)
            .round()
            .clamp(0.0, (height - 1) as f32) as u32;
        let outline_sample = pixel_at(&pixels, width, outline_x, outline_y);
        assert!(
            near_rgb(outline_sample, Color::rgb(0xd4, 0xa3, 0x48), 80),
            "active theme chip should paint an amber outline at ({outline_x}, {outline_y}), got {outline_sample:?}"
        );
        let border_x = (amber_chip.layout_rect.x + amber_chip.layout_rect.width - 1.0)
            .round()
            .clamp(0.0, (width - 1) as f32) as u32;
        let border_y = (amber_chip.layout_rect.y + amber_chip.layout_rect.height - 4.0)
            .round()
            .clamp(0.0, (height - 1) as f32) as u32;
        let border = pixel_at(&pixels, width, border_x, border_y);
        assert!(
            near_rgb(border, Color::rgb(0xa8, 0x8b, 0xb8), 12),
            "target footer swatch should remain visible at ({border_x}, {border_y}), got {border:?}"
        );
    }

    #[test]
    fn settings_page_custom_theme_chip_gradient_paints_in_renderer() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let width = 925u32;
        let height = 900u32;
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            width as f32,
            height as f32,
        );
        if !harness.try_with_gpu() {
            return;
        }
        harness.step();

        let screen = harness
            .query(".theme-chip.custom .theme-chip-screen")
            .expect("custom chip screen");
        let x = (screen.layout_rect.x + screen.layout_rect.width * 0.58)
            .round()
            .clamp(0.0, (width - 1) as f32) as u32;
        let y = (screen.layout_rect.y + screen.layout_rect.height * 0.48)
            .round()
            .clamp(0.0, (height - 1) as f32) as u32;
        let pixels = harness.render();
        let sample = pixel_at(&pixels, width, x, y);
        assert!(
            sample[0] > 100 && sample[1] > 180 && sample[2] > 210,
            "custom chip should paint a bright cyan gradient at ({x}, {y}), got {sample:?}"
        );

        let custom_name = harness
            .query(".theme-chip.custom .tcs-name")
            .expect("custom chip label");
        assert!(
            rect_has_near_rgb(
                &pixels,
                width,
                height,
                custom_name.layout_rect,
                Color::rgb(0x0e, 0x16, 0x20),
                48,
            ),
            "custom chip label should paint dark accent-on text over the gradient, label rect {:?}",
            custom_name.layout_rect
        );
    }

    #[test]
    fn settings_page_styles_match_design_system_geometry_and_effects() {
        let css = include_str!("../../assets/styles.css");
        let css_lf = css.replace("\r\n", "\n");
        assert!(
            css_lf.contains("JetBrainsMono-Bold.ttf\") format(\"truetype\");\n  font-weight: 700;")
        );
        assert!(css.contains("grid-template-columns: 240px minmax(0, 1fr);"));
        assert!(css.contains("max-width: 920px;"));
        assert!(css.contains("backdrop-filter: blur(8px);"));
        assert!(css.contains(".app.theme-amber .set-page-savebar { background: #171411;"));
        assert!(css.contains(".set-page-savebar .btn.ghost"));
        assert!(
            css_lf.contains(".set-page-savebar .btn.ghost {\n  position: relative;\n  left: -1px;")
        );
        assert!(css.contains(".set-page-nav-item.active::before"));
        assert!(css.contains(".set-page-nav .group:first-child"));
        assert!(css_lf.contains(
            ".set-page-nav .group {\n  padding: 12px 12px 4px;\n  color: var(--fg-tertiary);\n  font: var(--type-meta);\n  letter-spacing: 1.4px;"
        ));
        assert!(css.contains(".app.theme-amber .set-page-header { background: linear-gradient(180deg, rgba(34, 29, 22, 0.62), rgba(34, 29, 22, 0.165) 72%, rgba(28, 24, 18, 0.08)); border-color: #241e15; }"));
        assert!(
            css_lf.contains(".set-page-rail-head .title {\n  position: relative;\n  top: -1px;")
        );
        assert!(css_lf.contains(".set-page-rail-head .sub {\n  position: relative;\n  top: -1px;"));
        assert!(css_lf.contains(".set-page-foot span {\n  position: relative;\n  top: 1px;"));
        assert!(
            css_lf.contains(".set-page-foot .status-running {\n  box-shadow: 0 0 2px var(--sage);")
        );
        assert!(css_lf.contains(
            ".set-page-savebar .saved {\n  display: flex;\n  position: relative;\n  top: -1px;"
        ));
        assert!(css.contains("color: #8ba05c;"));
        assert!(css.contains(".set-page-nav-item.active svg"));
        assert!(css_lf.contains(
            ".set-page-nav-item.active {\n  background: transparent;\n  color: var(--amber-100);\n  font-weight: 700;\n  z-index: 0;"
        ));
        assert!(css_lf.contains(
            ".app.settings .settings-page .set-page-nav-item.active {\n  background: transparent;"
        ));
        assert!(css_lf.contains(
            ".set-page-nav-item.active::after {\n  content: '';\n  position: absolute;\n  left: 0;\n  right: 0;\n  top: 4px;\n  bottom: -3px;"
        ));
        assert!(css_lf.contains(
            ".set-page-nav-item.active::before {\n  content: '';\n  position: absolute;"
        ));
        assert!(css.contains(".app.theme-amber .settings-page .set-page-nav-item.active::before { background: #d4a348; }"));
        assert!(css.contains(".app.theme-amber .settings-page .set-desc { color: #6f5a33; }"));
        assert!(css.contains(".app.theme-amber .settings-page .set-label { color: #ebdcb6; }"));
        assert!(css.contains(".theme-chip.catppuccin .tcs-sub { color: #666f86; }"));
        assert!(css.contains(".theme-chip.tokyo-night .tcs-sub { color: #48516c; }"));
        assert!(css.contains(".settings-page .set-card-head"));
        assert!(css.contains("padding: 9px 14px 9px 15px;"));
        assert!(css_lf
            .contains(".settings-page .set-card-head {\n  position: relative;\n  display: flex;"));
        assert!(css.contains("border-bottom: none;"));
        assert!(css_lf.contains(
            ".settings-page .set-card-head::after {\n  content: '';\n  position: absolute;"
        ));
        assert!(css.contains("bottom: -2px;"));
        assert!(css.contains("background: var(--border-hair);"));
        assert!(css
            .contains(".app.theme-amber .set-card { background: #221d16; border-color: #241e15;"));
        assert!(css.contains("width: 101%;"));
        assert!(css.contains("gap: 14px 12px;"));
        assert!(css.contains("padding: 7px 0 2px;"));
        assert!(css.contains("padding: 0;"));
        assert!(css.contains("box-shadow: inset 0 0 0 1px var(--border-soft);"));
        assert!(css_lf.contains(
            ".theme-chip-top-hairline {\n  position: absolute;\n  left: 4px;\n  right: 1px;\n  top: -1px;"
        ));
        assert!(css.contains("padding: 5px 16px;"));
        assert!(css.contains("outline-color: var(--theme-chip-accent);"));
        assert!(css.contains("outline-width: 1px;"));
        assert!(css_lf.contains(
            ".theme-chip.amber.active { border-color: #221d16; box-shadow: inset 0 0 0 1px #d4a348; }"
        ));
        assert!(css.contains("gap: 10px;"));
        assert!(css.contains("font: 600 24px/1 var(--font-mono);"));
        assert!(css.contains("font: 700 12px/1.35 var(--font-mono);"));
        assert!(css.contains("font: 600 10px/1.35 var(--font-mono);"));
        assert!(css_lf
            .contains(".theme-chip-foot {\n  position: relative;\n  left: 1px;\n  display: flex;"));
        assert!(css_lf.contains(".tcs-sub {\n  position: relative;\n  top: -2px;"));
        assert!(css_lf.contains(
            ".settings-page .set-card-head .name-meta {\n  position: relative;\n  left: 4px;\n  top: -1px;"
        ));
        assert!(css.contains("margin-top: 0;"));
        assert!(css.contains("letter-spacing: 0.2px;"));
        assert!(css_lf.contains(
            ".set-page-savebar .btn.primary {\n  min-height: 27px;\n  background: #d4a348;\n  border-color: #d4a348;\n  color: #746445;\n  border-radius: 4px;"
        ));
        assert!(css.contains("box-shadow: 0 0 6px rgba(212, 163, 72, 0.2);"));
        assert!(css.contains(".app.settings .settings-titlebar .titlebar-left"));
        assert!(css.contains("top: -1px;"));
        assert!(css.contains(".settings-tb-breadcrumb"));
        assert!(css.contains("padding-left: 11px;"));
        assert!(css_lf.contains(".set-page-nav-item svg {\n  position: relative;\n  top: 3px;"));
        assert!(css_lf.contains(
            ".set-page-nav-item.nav-notifications svg,\n.set-page-nav-item.nav-danger-zone svg {\n  top: -1px;"
        ));
        assert!(css_lf.contains(".set-page-nav-item.nav-keybinds svg {\n  top: 1px;"));
        assert!(css_lf.contains(".set-page-nav-item.nav-shell svg {\n  top: 1px;"));
        assert!(
            css_lf.contains(".set-page-nav-item.nav-sessions svg {\n  left: -1px;\n  top: 2px;")
        );
        assert!(css_lf
            .contains(".set-page-nav-item.nav-shell span {\n  position: relative;\n  top: 2px;"));
        assert!(css_lf.contains(
            ".set-page-nav-item.nav-sessions span {\n  position: relative;\n  top: 1px;"
        ));
        assert!(css_lf.contains(
            ".set-page-nav-item.nav-keybinds span {\n  position: relative;\n  top: 1px;"
        ));
        assert!(css.contains(".brand-term"));
        assert!(css.contains(".app.settings .settings-titlebar .brand-term"));
        assert!(css.contains("left: 6px;"));
        assert!(css.contains("color: #e4d2a6;"));
        assert!(
            css_lf.contains(".app.settings .settings-titlebar .brand-name {\n  display: flex;\n  flex-direction: row;\n  gap: 0;\n  color: #d1bd94;")
        );
        assert!(css_lf.contains(
            ".app.settings .settings-titlebar .brand-name .dot {\n  position: relative;\n  left: 4px;"
        ));
        assert!(css_lf.contains(
            ".app.settings .settings-titlebar .brand-mark {\n  position: relative;\n  top: 3px;"
        ));
        assert!(css.contains("font-size: 14px;"));
        assert!(css.contains("padding-right: 8px;"));
        assert!(
            css_lf.contains(".settings-titlebar-help svg {\n  position: relative;\n  left: 2px;")
        );
        assert!(css.contains("letter-spacing: 1.4px;"));
        assert!(css_lf.contains(
            ".app.settings .statusbar {\n  grid-column: 1;\n  grid-row: 3;\n  padding-left: 8px;"
        ));
        assert!(css.contains(".app.settings .settings-statusbar .sb-cell"));
        assert!(css.contains("height: 14px;"));
        assert!(css.contains(".app.settings .settings-statusbar .sb-cell.sage"));
        assert!(css.contains("padding-right: 16px;"));
        assert!(css.contains(".app.settings .settings-statusbar .statusbar-right .sb-cell.amber"));
        assert!(css.contains("padding-right: 11px;"));
        assert!(css.contains("color: #e0a342;"));
        assert!(css.contains(".settings-page .set-label"));
        assert!(css.contains("left: 2px;"));
        assert!(css_lf.contains(
            ".settings-page .set-label {\n  position: relative;\n  left: 2px;\n  top: 2px;"
        ));
        assert!(css_lf.contains(
            ".settings-page .set-label {\n  position: relative;\n  left: 2px;\n  top: 2px;\n  color: var(--fg-primary);\n  font: var(--type-label);\n  font-weight: 700;\n  letter-spacing: 0.2px;"
        ));
        assert!(css_lf.contains(
            ".set-page-rail-head .title {\n  position: relative;\n  top: -1px;\n  color: var(--amber-50);\n  font: var(--type-h1);\n  font-weight: 700;"
        ));
        assert!(css.contains(".set-page-header .page-title"));
        assert!(css.contains("left: -1px;"));
        assert!(css.contains(".set-page-header .blurb"));
        assert!(css.contains(".set-page-search-input"));
        assert!(css.contains("top: 3px;"));
        assert!(css_lf.contains(
            ".set-page-search .kbd {\n  display: flex;\n  align-items: center;\n  gap: 0;\n  position: relative;\n  left: 3px;\n  top: 1px;"
        ));
        assert!(css.contains("font: var(--type-kbd);"));
        assert!(css_lf.contains(".set-page-search .kbd-command {\n  font-size: 12px;"));
        assert!(css_lf.contains(".set-page-search .kbd-key {\n  font-size: 10px;"));
        assert!(css.contains(".kbd-table"));
        assert!(css.contains(".kbd-binding"));
        assert!(
            css.contains(".preview-tile") && css.contains("border: 1px solid var(--border-hair);")
        );
        assert!(css_lf.contains(".settings-page .set-card::before {\n  content: '';"));
        assert!(css.contains("right: -14px;"));
        assert!(css.contains("width: 15px;"));
    }

    #[test]
    fn settings_page_nav_item_click_changes_section() {
        let shared = make_shared();
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let rail = build_settings_page_rail(&snap, &shared);
        let nav = &rail.children[2];
        let shell = &nav.children[2];
        let keybinds = &nav.children[5];
        let notifications = &nav.children[6];
        let danger = &nav.children[7];

        assert!(shell.classes.contains(&"nav-shell".to_string()));
        assert!(keybinds.classes.contains(&"nav-keybinds".to_string()));
        assert!(notifications
            .classes
            .contains(&"nav-notifications".to_string()));
        assert!(danger.classes.contains(&"nav-danger-zone".to_string()));

        (shell.on_click.as_ref().unwrap())();

        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Shell
        );
    }

    #[test]
    fn settings_page_savebar_matches_immediate_apply_actions() {
        let shared = make_shared();
        let el = build_settings_page_savebar(SettingsSection::Appearance, &shared);
        let text = collect_text_recursive(&el);

        assert!(el.classes.contains(&"set-page-savebar".to_string()));
        assert!(text.contains("changes apply immediately"));
        assert!(text.contains("reset"));
        assert!(text.contains("done"));
        assert!(has_class_anywhere(&el, "saved-dot"));
        assert!(!text.contains("2 unsaved changes"));
        assert!(!text.contains("discard"));
        assert!(!text.contains("save changes"));
    }

    // -- build_modal_header -----------------------------------------------------

    #[test]
    fn modal_header_has_correct_class() {
        let shared = make_shared();
        let el = build_modal_header(&shared);
        assert!(el.classes.contains(&"modal-header".to_string()));
    }

    #[test]
    fn modal_header_contains_title_and_close_button() {
        let shared = make_shared();
        let el = build_modal_header(&shared);
        assert_eq!(el.children.len(), 2);
        let close_btn = &el.children[1];
        assert!(close_btn.on_click.is_some());
        assert_eq!(close_btn.id.as_deref(), Some("settings-close"));
    }

    // -- build_modal_nav --------------------------------------------------------

    #[test]
    fn modal_nav_has_nav_class() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        assert!(el.classes.contains(&"modal-nav".to_string()));
    }

    #[test]
    fn modal_nav_has_six_items() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        assert_eq!(el.children.len(), 6);
    }

    #[test]
    fn modal_nav_marks_appearance_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        assert!(el.children[0].classes.contains(&"active".to_string()));
        for child in &el.children[1..] {
            assert!(!child.classes.contains(&"active".to_string()));
        }
    }

    #[test]
    fn modal_nav_marks_shell_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Shell, &shared);
        assert!(el.children[1].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_keybinds_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Keybinds, &shared);
        assert!(el.children[2].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_sessions_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Sessions, &shared);
        assert!(el.children[3].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_notifications_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Notifications, &shared);
        assert!(el.children[4].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_danger_zone_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::DangerZone, &shared);
        assert!(el.children[5].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_items_have_click_handlers() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        for child in &el.children {
            assert!(child.on_click.is_some());
        }
    }

    // -- build_modal_body -------------------------------------------------------

    #[test]
    fn modal_body_renders_only_active_section() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        assert!(el.classes.contains(&"modal-body".to_string()));
        assert_eq!(el.children.len(), 1);
    }

    #[test]
    fn modal_body_switches_to_appearance() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("appearance"));
    }

    #[test]
    fn modal_body_switches_to_keybinds() {
        let snap = make_snapshot_section(SettingsSection::Keybinds);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        // The keybinds section renders without a set-card shell.
        let section = &el.children[0];
        assert!(section.classes.contains(&"kb-page".to_string()));
        assert!(has_class_anywhere(section, "kb-toolbar"));
    }

    #[test]
    fn modal_body_switches_to_shell() {
        let snap = make_snapshot_section(SettingsSection::Shell);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("shell"));
    }

    #[test]
    fn modal_body_switches_to_notifications() {
        let snap = make_snapshot_section(SettingsSection::Notifications);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("notifications"));
    }

    // -- build_appearance_section -----------------------------------------------

    #[test]
    fn appearance_section_has_title_and_applied_font_row() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        // title + separate config and terminal font rows.
        assert_eq!(el.children.len(), 3);
        assert_eq!(
            text_of(&el.children[1].children[0].children[0]),
            Some("Config font size")
        );
        assert_eq!(
            text_of(&el.children[2].children[0].children[0]),
            Some("Terminal font size")
        );
    }

    #[test]
    fn appearance_section_has_font_stepper() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        for row in [&el.children[1], &el.children[2]] {
            let stepper = &row.children[1];
            assert!(stepper.classes.contains(&"stepper".to_string()));
            assert_eq!(stepper.children.len(), 3);
        }
    }

    // -- build_shell_section ----------------------------------------------------

    fn find_first_with_class<'a>(root: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if root.classes.iter().any(|c| c == class) {
            return Some(root);
        }
        root.children
            .iter()
            .find_map(|c| find_first_with_class(c, class))
    }

    fn collect_with_class<'a>(root: &'a ElementDef, class: &str, out: &mut Vec<&'a ElementDef>) {
        if root.classes.iter().any(|c| c == class) {
            out.push(root);
        }
        for child in &root.children {
            collect_with_class(child, class, out);
        }
    }

    fn count_with_class(root: &ElementDef, class: &str) -> usize {
        let here = if root.classes.iter().any(|c| c == class) {
            1
        } else {
            0
        };
        here + root
            .children
            .iter()
            .map(|c| count_with_class(c, class))
            .sum::<usize>()
    }

    fn collect_text_recursive(root: &ElementDef) -> String {
        let mut acc = String::new();
        if let Some(t) = text_of(root) {
            acc.push_str(t);
            acc.push(' ');
        }
        for child in &root.children {
            acc.push_str(&collect_text_recursive(child));
        }
        acc
    }

    #[test]
    fn shell_section_starts_with_app_default_block() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        // first child after the title must be the app default scope block
        let first = &el.children[1];
        assert!(
            first.classes.contains(&"shell-scope-block".to_string()),
            "first body child must be a shell-scope-block, got classes: {:?}",
            first.classes
        );
    }

    #[test]
    fn shell_section_includes_shell_picker_under_app_default_block() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        assert!(
            find_first_with_class(&el, "shell-picker").is_some(),
            "shell section must include a shell-picker"
        );
    }

    #[test]
    fn shell_picker_marks_active_chip_when_program_matches() {
        // Build a snapshot whose default_shell.program matches a fake
        // discovered shell, then assert at least one chip carries the
        // "active" class. We feed the picker directly so the test does
        // not depend on what's installed on the host.
        let installed = vec![std::path::PathBuf::from("/bin/bash")];
        let current = crate::shell::ShellSpec {
            program: "/bin/bash".into(),
            args: vec![],
        };
        let shared = make_shared();
        let picker = shell_picker(ShellScope::AppDefault, &current, &installed, &shared);
        assert!(
            count_with_class(&picker, "active") >= 1,
            "matching program must mark a chip active"
        );
    }

    #[test]
    fn shell_picker_for_workspace_includes_use_default_chip() {
        let installed: Vec<std::path::PathBuf> = vec![];
        let current = crate::shell::ShellSpec::default();
        let shared = make_shared();
        let picker = shell_picker(ShellScope::Workspace(0), &current, &installed, &shared);
        assert!(
            collect_text_recursive(&picker).contains("use default"),
            "workspace picker must include a use default chip"
        );
    }

    #[test]
    fn shell_picker_for_app_default_omits_use_default_chip() {
        let installed: Vec<std::path::PathBuf> = vec![];
        let current = crate::shell::ShellSpec::default();
        let shared = make_shared();
        let picker = shell_picker(ShellScope::AppDefault, &current, &installed, &shared);
        assert!(
            !collect_text_recursive(&picker).contains("use default"),
            "app default picker must NOT have a use default chip"
        );
    }

    #[test]
    fn shell_section_has_one_workspace_override_block_per_workspace() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        let overrides = find_first_with_class(&el, "workspace-overrides")
            .expect("workspace-overrides subsection must be present");
        let blocks = count_with_class(overrides, "shell-scope-block");
        assert_eq!(
            blocks,
            snap.workspaces.len(),
            "workspace overrides must have one block per workspace"
        );
    }

    // -- build_keybinds_section -------------------------------------------------

    /// Find the kb-row whose name label equals `name`.
    fn find_kb_row<'a>(el: &'a ElementDef, name: &str) -> Option<&'a ElementDef> {
        if el.classes.contains(&"kb-row".to_string()) {
            let named = el
                .children
                .iter()
                .find(|c| c.classes.contains(&"kb-row-meta".to_string()))
                .and_then(|meta| meta.children.first())
                .and_then(|n| text_of(n));
            if named == Some(name) {
                return Some(el);
            }
        }
        el.children.iter().find_map(|c| find_kb_row(c, name))
    }

    #[test]
    fn keybinds_section_has_toolbar_groups_rows_and_footer() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        // children: [error banner, toolbar, group x4] — no card shell, no
        // restart banner, per the design mockup.
        assert_eq!(el.children.len(), 6);
        assert!(el.classes.contains(&"kb-page".to_string()));
        assert!(el.children[1].classes.contains(&"kb-toolbar".to_string()));
        assert_eq!(count_with_class(&el, "kb-group"), 4);
        assert_eq!(count_with_class(&el, "kb-row"), KeybindAction::ALL.len());
        // Every row carries an icon, a name, and a description.
        assert_eq!(
            count_with_class(&el, "kb-row-icon"),
            KeybindAction::ALL.len()
        );
        assert_eq!(
            count_with_class(&el, "kb-row-desc"),
            KeybindAction::ALL.len()
        );
    }

    #[test]
    fn keybinds_toolbar_shows_filter_and_count() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        let toolbar = &el.children[1];
        let filter = &toolbar.children[0];
        assert!(filter.classes.contains(&"kb-filter".to_string()));
        assert!(find_first_with_class(filter, "kb-filter-input").is_some());
        // "/" focus hint inside the filter box.
        assert!(filter
            .children
            .iter()
            .any(|c| c.classes.contains(&"kbd".to_string())));
        let count = &toolbar.children[1];
        assert!(count.classes.contains(&"kb-count".to_string()));
        assert_eq!(
            text_of(count),
            Some(format!("{} commands", KeybindAction::ALL.len()).as_str())
        );
    }

    #[test]
    fn keybinds_filter_narrows_rows_and_count() {
        let mut state = seed_state();
        state.keybinds.filter = "palette".to_string();
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        assert_eq!(count_with_class(&el, "kb-row"), 1);
        assert!(find_kb_row(&el, "Command palette").is_some());
        let count = find_first_with_class(&el, "kb-count").unwrap();
        assert_eq!(
            text_of(count),
            Some(format!("1 of {}", KeybindAction::ALL.len()).as_str())
        );
    }

    #[test]
    fn keybinds_filter_with_no_matches_shows_empty_state() {
        let mut state = seed_state();
        state.keybinds.filter = "zzz-no-such-command".to_string();
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        assert_eq!(count_with_class(&el, "kb-row"), 0);
        assert!(find_first_with_class(&el, "kb-empty").is_some());
    }

    #[test]
    fn keybinds_row_shows_effective_combo_parts() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        let row = find_kb_row(&el, "New terminal").expect("New terminal row");
        // row: [icon, meta, binding]
        assert_eq!(row.children.len(), 3);
        assert!(row.children[0].classes.contains(&"kb-row-icon".to_string()));
        let binding = &row.children[2];
        assert!(binding.classes.contains(&"kb-binding".to_string()));
        // binding: [keys, edit-pencil]
        let keys = &binding.children[0];
        assert!(keys.classes.contains(&"keys".to_string()));
        // Default NewTerminal is Ctrl+T: two keycaps joined by a "+".
        assert_eq!(keys.children.len(), 3);
        assert!(keys.children[0].classes.contains(&"keycap".to_string()));
        assert_eq!(text_of(&keys.children[0]), Some("Ctrl"));
        assert!(keys.children[1].classes.contains(&"plus".to_string()));
        assert_eq!(text_of(&keys.children[1]), Some("+"));
        assert!(keys.children[2].classes.contains(&"keycap".to_string()));
        assert_eq!(text_of(&keys.children[2]), Some("T"));
        assert!(binding.children[1]
            .classes
            .contains(&"edit-pencil".to_string()));
    }

    #[test]
    fn keybind_plus_separator_is_a_real_element_not_a_pseudo() {
        let css = include_str!("../../assets/styles.css");
        // Pins the chosen structure: the "+" separators are real spans
        // emitted by keybind_binding (styled via .plus), not pseudo
        // content. (The engine measures text+pseudo hosts correctly via
        // anonymous text boxes; this keeps the combo structure explicit.)
        assert!(!css.contains(".keycap:not(:first-child)::before"));
        assert!(css.contains(".plus {"));
        assert!(css.contains(".keycap {"));
    }

    #[test]
    fn keybind_pills_grow_to_fit_their_text_with_stylesheet() {
        let snap = make_snapshot_section(SettingsSection::Keybinds);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_class("theme-amber")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            1280.0,
            800.0,
        );
        harness.step();

        let pills = harness.query_all(".keycap");
        assert!(!pills.is_empty(), "keybinds page should render keycaps");

        for pill in &pills {
            // Pills are plain childless text leaves by design (separators
            // are sibling spans, not pseudo children).
            assert!(
                harness.arena().children(pill.node_id).is_empty(),
                "pill {:?} must not have children",
                pill.content
            );

            let ElementContent::Text(ref text) = pill.content else {
                panic!("pill should hold text content, got {:?}", pill.content);
            };
            let style = pill.computed_style.clone();
            let (text_w, _) = unshit::core::layout::measure_text_with_style_cached(
                text,
                &style.font_family,
                style.font_weight,
                style.font_style,
                style.font_size,
                style.line_height,
                style.letter_spacing,
                None,
                harness.font_system_mut(),
                None,
            );
            let content_w = pill.layout_rect.width - style.padding.left - style.padding.right;
            assert!(
                content_w + 0.5 >= text_w,
                "\"{text}\" pill content box ({content_w}px) must fit its label ({text_w}px), rect {:?}",
                pill.layout_rect
            );
        }

        // The "+" separators between pills must lay out as visible elements.
        let plusses = harness.query_all(".plus");
        assert!(
            !plusses.is_empty(),
            "multi-key combos should render + separators"
        );
        for plus in &plusses {
            assert!(
                plus.layout_rect.width > 0.0 && plus.layout_rect.height > 0.0,
                "+ separator should have non-zero layout, got {:?}",
                plus.layout_rect
            );
        }
    }

    #[test]
    fn keybinds_savebar_offers_restore_defaults() {
        let snap = make_snapshot_section(SettingsSection::Keybinds);
        let shared = make_shared();
        let page = build_settings_page(&snap, &shared);
        let savebar = find_first_with_class(&page, "set-page-savebar").expect("savebar");
        let labels: Vec<&str> = savebar
            .children
            .iter()
            .filter_map(|c| find_first_with_class(c, "btn-label").or(Some(c)))
            .filter_map(text_of)
            .collect();
        assert!(
            labels.contains(&"restore defaults"),
            "keybinds savebar must offer restore defaults, got {labels:?}"
        );
    }

    #[test]
    fn keybinds_row_with_override_includes_reset_button() {
        let mut state = seed_state();
        state
            .keybinds
            .set(
                crate::keybinds::KeybindAction::NewTerminal,
                unshit::core::shortcut::KeyCombo::parse("Alt+N").unwrap(),
            )
            .unwrap();
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        let row = find_kb_row(&el, "New terminal").expect("New terminal row");
        // With override: [icon, meta, binding, reset_btn] -> 4 children.
        assert_eq!(row.children.len(), 4);
        let reset = &row.children[3];
        assert!(reset.classes.contains(&"keybind-reset".to_string()));
    }

    #[test]
    fn keybinds_row_in_recording_state_shows_placeholder() {
        let mut state = seed_state();
        state
            .keybinds
            .start_recording(crate::keybinds::KeybindAction::NewTerminal);
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        let row = find_kb_row(&el, "New terminal").expect("New terminal row");
        assert!(row.classes.contains(&"recording".to_string()));
        let binding = &row.children[2];
        assert!(binding.classes.contains(&"recording".to_string()));
        // binding: [rec-label [rec-dot, text]]
        assert_eq!(binding.children.len(), 1);
        let rec = &binding.children[0];
        assert!(rec.classes.contains(&"rec-label".to_string()));
        assert!(rec.children[0].classes.contains(&"rec-dot".to_string()));
        assert_eq!(
            text_of(&rec.children[1]),
            Some("press keys... (esc to cancel)")
        );
    }

    #[test]
    fn keybinds_error_banner_visible_on_conflict() {
        let mut state = seed_state();
        // Provoke a conflict: set NewTerminal to Unsplit's default.
        let _ = state.keybinds.set(
            crate::keybinds::KeybindAction::NewTerminal,
            unshit::core::shortcut::KeyCombo::parse("Ctrl+W").unwrap(),
        );
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        let error_banner = &el.children[0];
        assert!(error_banner
            .classes
            .contains(&"keybind-banner-error".to_string()));
        assert!(!error_banner.classes.contains(&"hidden".to_string()));
    }

    // -- build_notifications_section ------------------------------------------

    #[test]
    fn notifications_section_test_button_pushes_notification() {
        let shared = make_shared();
        let el = build_notifications_section(&shared);
        assert_eq!(text_of(&el.children[0]), Some("notifications"));
        let test_btn =
            find_by_id(&el, "settings-test-notification").expect("test notification button");

        (test_btn.on_click.as_ref().unwrap())();

        let state = shared.lock().unwrap();
        let snap = state.ui_snapshot();
        let toast = snap.toasts.first().expect("test notification toast");
        assert_eq!(toast.title.as_deref(), Some("test notification"));
        assert_eq!(
            toast.target,
            Some(crate::state::ToastTarget {
                workspace_id: crate::state::active_workspace_num(&state),
                pane_id: state.active_pane.0,
            })
        );
    }

    // -- build_modal_footer -----------------------------------------------------

    #[test]
    fn modal_footer_has_correct_class() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        assert!(el.classes.contains(&"modal-footer".to_string()));
    }

    #[test]
    fn modal_footer_has_hint_and_actions() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        assert_eq!(el.children.len(), 2);
        assert!(el.children[0].classes.contains(&"modal-hint".to_string()));
        let actions = &el.children[1];
        assert!(actions
            .classes
            .contains(&"modal-footer-actions".to_string()));
        assert_eq!(actions.children.len(), 2);
    }

    #[test]
    fn modal_footer_cancel_button_has_id() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let cancel = &actions.children[0];
        assert_eq!(cancel.id.as_deref(), Some("settings-cancel"));
        assert!(cancel.on_click.is_some());
    }

    #[test]
    fn modal_footer_save_button_has_click_handler() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let save = &actions.children[1];
        assert!(save.classes.contains(&"primary".to_string()));
        assert!(save.on_click.is_some());
    }

    // -- setting_row ------------------------------------------------------------

    #[test]
    fn setting_row_has_meta_and_control() {
        let control = ElementDef::new(Tag::Input).with_class("input");
        let el = setting_row("Label", "Description", control);
        assert!(el.classes.contains(&"setting-row".to_string()));
        assert_eq!(el.children.len(), 2);
        let meta = &el.children[0];
        assert!(meta.classes.contains(&"setting-meta".to_string()));
        assert_eq!(meta.children.len(), 2);
    }

    // -- closure invocation tests ----------------------------------------------

    #[test]
    fn close_button_click_closes_modal() {
        let shared = make_shared();
        shared.lock().unwrap().settings_open = true;
        let el = build_modal_header(&shared);
        let close_btn = &el.children[1];
        (close_btn.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().settings_open);
    }

    #[test]
    fn nav_item_click_changes_to_shell() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[1].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Shell
        );
    }

    #[test]
    fn nav_item_click_changes_to_keybinds() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[2].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Keybinds
        );
    }

    #[test]
    fn nav_item_click_changes_to_sessions() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[3].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Sessions
        );
    }

    #[test]
    fn nav_item_click_changes_to_danger_zone() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[5].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::DangerZone
        );
    }

    #[test]
    fn nav_item_click_changes_to_notifications() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[4].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Notifications
        );
    }

    #[test]
    fn font_dec_button_decreases_font_size() {
        let shared = make_shared();
        let initial = shared.lock().unwrap().terminal_font_size_pt;
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let stepper = &el.children[2].children[1];
        let dec_btn = &stepper.children[0];
        (dec_btn.on_click.as_ref().unwrap())();
        let after = shared.lock().unwrap().terminal_font_size_pt;
        assert!(after <= initial);
    }

    #[test]
    fn font_inc_button_increases_font_size() {
        let shared = make_shared();
        let initial = shared.lock().unwrap().terminal_font_size_pt;
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let stepper = &el.children[2].children[1];
        let inc_btn = &stepper.children[2];
        (inc_btn.on_click.as_ref().unwrap())();
        let after = shared.lock().unwrap().terminal_font_size_pt;
        assert!(after >= initial);
    }

    #[test]
    fn config_font_buttons_change_config_font_size_only() {
        let shared = make_shared();
        let terminal_initial = shared.lock().unwrap().terminal_font_size_pt;
        let config_initial = shared.lock().unwrap().config_font_size_pt;
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let stepper = &el.children[1].children[1];
        let inc_btn = &stepper.children[2];
        (inc_btn.on_click.as_ref().unwrap())();
        let guard = shared.lock().unwrap();
        assert!(guard.config_font_size_pt >= config_initial);
        assert_eq!(guard.terminal_font_size_pt, terminal_initial);
    }

    #[test]
    fn cancel_button_click_closes_modal() {
        let shared = make_shared();
        shared.lock().unwrap().settings_open = true;
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let cancel = &actions.children[0];
        (cancel.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().settings_open);
    }

    #[test]
    fn save_button_click_closes_modal() {
        let shared = make_shared();
        shared.lock().unwrap().settings_open = true;
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let save = &actions.children[1];
        (save.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().settings_open);
    }

    // -- helper widget tests ----------------------------------------------------

    #[test]
    fn stepper_wires_callbacks_to_buttons() {
        let callbacks = StepCallbacks {
            on_dec: Box::new(|| {}),
            on_inc: Box::new(|| {}),
        };
        let el = stepper("7", callbacks);
        assert!(el.classes.contains(&"stepper".to_string()));
        assert_eq!(el.children.len(), 3);
        let dec = &el.children[0];
        let inc = &el.children[2];
        assert_eq!(text_of(&el.children[1]), Some("7"));
        assert!(dec.on_click.is_some());
        assert!(inc.on_click.is_some());
    }

    // -- build_sessions_section -------------------------------------------------

    #[test]
    fn sessions_section_empty_state_shows_placeholder() {
        let snap = make_snapshot_section(SettingsSection::Sessions);
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        assert!(el
            .children
            .iter()
            .any(|c| c.classes.contains(&"sessions-empty".to_string())));
    }

    #[test]
    fn sessions_section_renders_row_per_session() {
        let mut state = seed_state();
        state.settings_section = SettingsSection::Sessions;
        state.sessions = vec![
            crate::state::SessionSnapshot {
                session_id: 1,
                pane_id: 1,
                workspace_id: 1,
                name: Some("build".into()),
                pid: Some(1234),
                alive: true,
            },
            crate::state::SessionSnapshot {
                session_id: 2,
                pane_id: 2,
                workspace_id: 1,
                name: None,
                pid: Some(5678),
                alive: false,
            },
        ];
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let rows: Vec<_> = el
            .children
            .iter()
            .filter(|c| c.classes.contains(&"setting-row".to_string()))
            .collect();
        // First row is the "daemon sessions / refresh" header row, then
        // one row per session.
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn sessions_section_named_session_shows_custom_label() {
        let snap = crate::state::UiSnapshot {
            sessions: vec![crate::state::SessionSnapshot {
                session_id: 1,
                pane_id: 1,
                workspace_id: 42,
                name: Some("api-server".into()),
                pid: Some(1234),
                alive: true,
            }],
            ..seed_state().ui_snapshot()
        };
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let labels: Vec<&str> = el
            .children
            .iter()
            .filter_map(|c| {
                c.children
                    .iter()
                    .find(|m| m.classes.contains(&"setting-meta".to_string()))
                    .and_then(|m| m.children.first())
                    .and_then(text_of)
            })
            .collect();
        assert!(labels.contains(&"api-server"));
    }

    #[test]
    fn sessions_section_unnamed_session_shows_pid_fallback() {
        let snap = crate::state::UiSnapshot {
            sessions: vec![crate::state::SessionSnapshot {
                session_id: 1,
                pane_id: 1,
                workspace_id: 1,
                name: None,
                pid: Some(9999),
                alive: true,
            }],
            ..seed_state().ui_snapshot()
        };
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let labels: Vec<String> = el
            .children
            .iter()
            .filter_map(|c| {
                c.children
                    .iter()
                    .find(|m| m.classes.contains(&"setting-meta".to_string()))
                    .and_then(|m| m.children.first())
                    .and_then(text_of)
                    .map(|s| s.to_string())
            })
            .collect();
        assert!(labels.iter().any(|l| l == "shell (9999)"));
    }

    #[test]
    fn sessions_section_alive_session_has_alive_status_class() {
        let snap = crate::state::UiSnapshot {
            sessions: vec![
                crate::state::SessionSnapshot {
                    session_id: 1,
                    pane_id: 1,
                    workspace_id: 1,
                    name: Some("a".into()),
                    pid: None,
                    alive: true,
                },
                crate::state::SessionSnapshot {
                    session_id: 2,
                    pane_id: 2,
                    workspace_id: 1,
                    name: Some("b".into()),
                    pid: None,
                    alive: false,
                },
            ],
            ..seed_state().ui_snapshot()
        };
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let rows: Vec<_> = el
            .children
            .iter()
            .filter(|c| c.classes.contains(&"setting-row".to_string()))
            .collect();
        // rows[0] is header; rows[1] alive, rows[2] dead.
        let alive_meta = &rows[1].children[0];
        let dead_meta = &rows[2].children[0];
        let has_status_class = |meta: &ElementDef, cls: &str| {
            meta.children.iter().any(|c| {
                c.children
                    .iter()
                    .any(|span| span.classes.iter().any(|k| k == cls))
            })
        };
        assert!(has_status_class(alive_meta, "session-status-alive"));
        assert!(has_status_class(dead_meta, "session-status-dead"));
    }

    #[test]
    fn sessions_section_refresh_button_click_dispatches_refresh() {
        let snap = make_snapshot_section(SettingsSection::Sessions);
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let refresh_btn = find_by_id(&el, "settings-sessions-refresh").expect("refresh button");
        // Invoking succeeds without panic; actual daemon call is a no-op
        // because no daemon is connected in unit tests.
        (refresh_btn.on_click.as_ref().unwrap())();
    }

    fn find_by_id<'a>(el: &'a ElementDef, target: &str) -> Option<&'a ElementDef> {
        if el.id.as_deref() == Some(target) {
            return Some(el);
        }
        el.children.iter().find_map(|c| find_by_id(c, target))
    }

    // refs #130: stale chip surfaces failed refreshes next to the button.
    #[test]
    fn sessions_section_renders_stale_chip_when_flag_set() {
        let mut snap = make_snapshot_section(SettingsSection::Sessions);
        assert!(!snap.sessions_stale);
        let shared = make_shared();
        let clean = build_sessions_section(&snap, &shared);
        assert!(!has_class_anywhere(&clean, "sessions-refresh-stale"));

        snap.sessions_stale = true;
        let stale = build_sessions_section(&snap, &shared);
        assert!(has_class_anywhere(&stale, "sessions-refresh-stale"));
    }

    fn has_class_anywhere(el: &ElementDef, class: &str) -> bool {
        if el.classes.iter().any(|c| c == class) {
            return true;
        }
        el.children.iter().any(|c| has_class_anywhere(c, class))
    }
}
