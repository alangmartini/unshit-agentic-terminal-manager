use crate::style::types::Color;

/// Semantic color roles for a design theme.
#[derive(Clone, Debug)]
pub struct ThemeColors {
    pub background: Color,
    pub surface: Color,
    pub primary: Color,
    pub secondary: Color,
    pub text: Color,
    pub text_muted: Color,
    pub border: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
}

/// Motion/animation presets.
#[derive(Clone, Debug)]
pub struct ThemeMotion {
    /// Duration in ms for subtle transitions (hover, focus).
    pub duration_fast: u32,
    /// Duration in ms for standard transitions.
    pub duration_normal: u32,
    /// Duration in ms for elaborate animations.
    pub duration_slow: u32,
}

/// Framework-level design tokens.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub colors: ThemeColors,
    /// Spacing scale in px: [xs, sm, md, lg, xl].
    pub spacing: [f32; 5],
    /// Type scale in px: [caption, body, subheading, heading, display].
    pub type_scale: [f32; 5],
    /// Border radii in px: [sm, md, lg, full].
    pub radii: [f32; 4],
    pub motion: ThemeMotion,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            name: "dark".into(),
            colors: ThemeColors {
                background: Color { r: 24, g: 24, b: 27, a: 255 },
                surface: Color { r: 39, g: 39, b: 42, a: 255 },
                primary: Color { r: 99, g: 102, b: 241, a: 255 },
                secondary: Color { r: 139, g: 92, b: 246, a: 255 },
                text: Color { r: 250, g: 250, b: 250, a: 255 },
                text_muted: Color { r: 161, g: 161, b: 170, a: 255 },
                border: Color { r: 63, g: 63, b: 70, a: 255 },
                error: Color { r: 239, g: 68, b: 68, a: 255 },
                warning: Color { r: 245, g: 158, b: 11, a: 255 },
                success: Color { r: 34, g: 197, b: 94, a: 255 },
            },
            spacing: [4.0, 8.0, 16.0, 24.0, 48.0],
            type_scale: [12.0, 14.0, 18.0, 24.0, 36.0],
            radii: [4.0, 8.0, 12.0, 9999.0],
            motion: ThemeMotion { duration_fast: 100, duration_normal: 200, duration_slow: 400 },
        }
    }

    pub fn light() -> Self {
        Self {
            name: "light".into(),
            colors: ThemeColors {
                background: Color { r: 255, g: 255, b: 255, a: 255 },
                surface: Color { r: 244, g: 244, b: 245, a: 255 },
                primary: Color { r: 79, g: 70, b: 229, a: 255 },
                secondary: Color { r: 124, g: 58, b: 237, a: 255 },
                text: Color { r: 9, g: 9, b: 11, a: 255 },
                text_muted: Color { r: 113, g: 113, b: 122, a: 255 },
                border: Color { r: 212, g: 212, b: 216, a: 255 },
                error: Color { r: 220, g: 38, b: 38, a: 255 },
                warning: Color { r: 217, g: 119, b: 6, a: 255 },
                success: Color { r: 22, g: 163, b: 74, a: 255 },
            },
            spacing: [4.0, 8.0, 16.0, 24.0, 48.0],
            type_scale: [12.0, 14.0, 18.0, 24.0, 36.0],
            radii: [4.0, 8.0, 12.0, 9999.0],
            motion: ThemeMotion { duration_fast: 100, duration_normal: 200, duration_slow: 400 },
        }
    }

    pub fn high_contrast() -> Self {
        Self {
            name: "high-contrast".into(),
            colors: ThemeColors {
                background: Color { r: 0, g: 0, b: 0, a: 255 },
                surface: Color { r: 30, g: 30, b: 30, a: 255 },
                primary: Color { r: 255, g: 255, b: 0, a: 255 },
                secondary: Color { r: 0, g: 255, b: 255, a: 255 },
                text: Color { r: 255, g: 255, b: 255, a: 255 },
                text_muted: Color { r: 200, g: 200, b: 200, a: 255 },
                border: Color { r: 255, g: 255, b: 255, a: 255 },
                error: Color { r: 255, g: 80, b: 80, a: 255 },
                warning: Color { r: 255, g: 200, b: 0, a: 255 },
                success: Color { r: 0, g: 255, b: 128, a: 255 },
            },
            spacing: [4.0, 8.0, 16.0, 24.0, 48.0],
            type_scale: [12.0, 14.0, 18.0, 24.0, 36.0],
            radii: [4.0, 8.0, 12.0, 9999.0],
            motion: ThemeMotion { duration_fast: 100, duration_normal: 200, duration_slow: 400 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_has_distinct_non_zero_colors() {
        let theme = Theme::dark();
        let c = &theme.colors;
        // Background should be dark (low value)
        assert!(c.background.r < 50);
        // Primary should be non-zero
        assert!(c.primary.r > 0 || c.primary.g > 0 || c.primary.b > 0);
        // Text should be bright
        assert!(c.text.r > 200);
        // All colors have full alpha
        assert_eq!(c.background.a, 255);
        assert_eq!(c.text.a, 255);
        // Background and surface are distinct
        assert_ne!(c.background.r, c.surface.r);
        // Primary and secondary are distinct
        assert_ne!(c.primary.r, c.secondary.r);
    }

    #[test]
    fn light_theme_has_light_background() {
        let theme = Theme::light();
        let c = &theme.colors;
        // Background should be very light (r > 200)
        assert!(c.background.r > 200, "light background r={}", c.background.r);
        assert!(c.background.g > 200);
        assert!(c.background.b > 200);
        // Text should be dark
        assert!(c.text.r < 50);
    }

    #[test]
    fn high_contrast_theme_has_maximum_contrast() {
        let theme = Theme::high_contrast();
        let c = &theme.colors;
        // Background is black
        assert_eq!(c.background.r, 0);
        assert_eq!(c.background.g, 0);
        assert_eq!(c.background.b, 0);
        // Text is white
        assert_eq!(c.text.r, 255);
        assert_eq!(c.text.g, 255);
        assert_eq!(c.text.b, 255);
    }

    #[test]
    fn all_themes_share_spacing_and_type_scale_structure() {
        let dark = Theme::dark();
        let light = Theme::light();
        let hc = Theme::high_contrast();

        assert_eq!(dark.spacing, light.spacing);
        assert_eq!(dark.spacing, hc.spacing);
        assert_eq!(dark.type_scale, light.type_scale);
        assert_eq!(dark.type_scale, hc.type_scale);
        assert_eq!(dark.radii, light.radii);
        assert_eq!(dark.radii, hc.radii);

        // Verify the scale has 5 entries
        assert_eq!(dark.spacing.len(), 5);
        assert_eq!(dark.type_scale.len(), 5);
        assert_eq!(dark.radii.len(), 4);
    }
}
