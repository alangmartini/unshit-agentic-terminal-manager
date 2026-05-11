#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteMetadata {
    pub id: &'static str,
    pub title: &'static str,
    pub tags: &'static [&'static str],
    pub coverage: &'static str,
    pub observability_needs: &'static [&'static str],
    pub supported_platforms: &'static [&'static str],
}

const SUITES: &[SuiteMetadata] = &[
    SuiteMetadata {
        id: "edge-resize-stability",
        title: "Edge resize stability",
        tags: &["windows", "resize", "layout", "black-box"],
        coverage: "Left-edge window resize stability, geometry changes, and visual continuity.",
        observability_needs: &["win32-window-bounds", "screenshots", "runner-events"],
        supported_platforms: &["windows"],
    },
    SuiteMetadata {
        id: "post-resize-glitches",
        title: "Post-resize visual glitch detection",
        tags: &["windows", "snap", "resize", "visual-regression"],
        coverage: "Windows snap/resize growth artifacts and stale renderer output after resize.",
        observability_needs: &[
            "win32-window-bounds",
            "screenshots",
            "renderer-state",
            "layout-state",
        ],
        supported_platforms: &["windows"],
    },
];

pub fn all_suites() -> &'static [SuiteMetadata] {
    SUITES
}

pub fn resolve_suites(ids: &[String]) -> Result<Vec<&'static SuiteMetadata>, String> {
    if ids.is_empty() {
        return Ok(SUITES.iter().collect());
    }

    ids.iter()
        .map(|id| {
            SUITES.iter().find(|suite| suite.id == id).ok_or_else(|| {
                format!(
                    "unknown desktop-regression suite '{id}' (known: {})",
                    SUITES
                        .iter()
                        .map(|suite| suite.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_current_windows_suites() {
        let ids: Vec<_> = all_suites().iter().map(|suite| suite.id).collect();
        assert_eq!(ids, vec!["edge-resize-stability", "post-resize-glitches"]);
    }

    #[test]
    fn resolves_selected_suite() {
        let selected = resolve_suites(&["post-resize-glitches".to_owned()]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].title, "Post-resize visual glitch detection");
        assert!(selected[0].supported_platforms.contains(&"windows"));
    }

    #[test]
    fn rejects_unknown_suite() {
        let err = resolve_suites(&["missing".to_owned()]).unwrap_err();
        assert!(err.contains("missing"));
    }
}
