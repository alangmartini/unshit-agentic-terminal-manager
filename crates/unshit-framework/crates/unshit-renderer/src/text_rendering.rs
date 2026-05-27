use std::ffi::OsString;
use std::sync::OnceLock;

fn env_bool(value: Option<OsString>) -> Option<bool> {
    let value = value?;
    let normalized = value.to_string_lossy().trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => Some(true),
    }
}

pub(crate) fn subpixel_text_enabled_from_env(
    force: Option<OsString>,
    disable: Option<OsString>,
    platform_default: bool,
) -> bool {
    if matches!(env_bool(disable), Some(true)) {
        return false;
    }
    env_bool(force).unwrap_or(platform_default)
}

pub(crate) fn use_subpixel_text_shader() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        subpixel_text_enabled_from_env(
            std::env::var_os("TM_FORCE_SUBPIXEL_TEXT"),
            std::env::var_os("TM_DISABLE_SUBPIXEL_TEXT"),
            default_subpixel_text_enabled(),
        )
    })
}

fn default_subpixel_text_enabled() -> bool {
    cfg!(target_os = "windows")
}

#[cfg(test)]
mod tests {
    use super::{default_subpixel_text_enabled, subpixel_text_enabled_from_env};
    use std::ffi::OsString;

    #[test]
    fn subpixel_text_defaults_to_platform_policy() {
        assert!(subpixel_text_enabled_from_env(None, None, true));
        assert!(!subpixel_text_enabled_from_env(None, None, false));
    }

    #[test]
    fn text_rendering_default_matches_platform_policy() {
        assert_eq!(
            subpixel_text_enabled_from_env(None, None, default_subpixel_text_enabled()),
            cfg!(target_os = "windows")
        );
    }

    #[test]
    fn subpixel_shader_keeps_browser_parity_tuning() {
        let shader = include_str!("shaders/text_subpixel.wgsl");
        assert!(
            shader.contains("let chroma = 0.8077;"),
            "settings/browser parity depends on the tuned BGR subpixel chroma"
        );
        assert!(
            shader.contains("let gamma = 0.8330;"),
            "settings/browser parity depends on the tuned UI text coverage gamma"
        );
        assert!(
            shader.contains("let cr = pow(mix(gray, tex.r, chroma), gamma) * 0.9997;"),
            "settings/browser parity depends on the tuned red subpixel coverage"
        );
        assert!(
            shader.contains("let cg = min(pow(mix(gray, tex.g, chroma), gamma) * 1.0032, 1.0);"),
            "settings/browser parity depends on the tuned green subpixel coverage"
        );
    }

    #[test]
    fn subpixel_text_force_env_overrides_default() {
        assert!(subpixel_text_enabled_from_env(Some(OsString::from("1")), None, false,));
        assert!(!subpixel_text_enabled_from_env(Some(OsString::from("0")), None, true,));
    }

    #[test]
    fn subpixel_text_disable_env_wins() {
        assert!(!subpixel_text_enabled_from_env(
            Some(OsString::from("1")),
            Some(OsString::from("true")),
            true,
        ));
        assert!(subpixel_text_enabled_from_env(None, Some(OsString::from("0")), true,));
    }
}
