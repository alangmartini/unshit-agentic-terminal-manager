//! Version parsing and ordering for release tags.
//!
//! Release tags are `vX.Y.Z` (the `v` is optional) and follow semver, so the
//! `semver` crate does the ordering, including pre-release precedence
//! (`0.5.0-rc.1 < 0.5.0`). The running version is `CARGO_PKG_VERSION`.

use semver::Version;

/// The version this binary was built as.
pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is valid semver")
}

/// Parse a release tag such as `v0.4.0`, `0.4.0` or `V0.5.0-rc.1`.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let trimmed = tag.trim();
    let bare = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    Version::parse(bare).ok()
}

/// `true` when `candidate` should be offered over `current`.
pub fn is_newer(candidate: &Version, current: &Version) -> bool {
    candidate > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_with_and_without_prefix() {
        assert_eq!(parse_tag("v0.4.0"), Some(Version::new(0, 4, 0)));
        assert_eq!(parse_tag("0.4.0"), Some(Version::new(0, 4, 0)));
        assert_eq!(parse_tag("  V1.2.3 "), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_tag("release-1"), None);
        assert_eq!(parse_tag(""), None);
    }

    #[test]
    fn ordering_follows_semver_including_prereleases() {
        let current = Version::new(0, 4, 0);
        assert!(is_newer(&parse_tag("v0.4.1").unwrap(), &current));
        assert!(is_newer(&parse_tag("v1.0.0").unwrap(), &current));
        assert!(!is_newer(&parse_tag("v0.4.0").unwrap(), &current));
        assert!(!is_newer(&parse_tag("v0.3.9").unwrap(), &current));
        // A pre-release of the *current* version is older than the release.
        assert!(!is_newer(&parse_tag("v0.4.0-rc.1").unwrap(), &current));
        // A pre-release of the next version is newer.
        assert!(is_newer(&parse_tag("v0.5.0-rc.1").unwrap(), &current));
    }

    #[test]
    fn current_version_matches_cargo() {
        assert_eq!(current().to_string(), env!("CARGO_PKG_VERSION"));
    }
}
