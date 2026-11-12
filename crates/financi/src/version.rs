//! The app version, baked in at build time by `build.rs` (3.39, plan.md →
//! *Show the version subtly in the UI*). M5's export manifest reuses
//! [`VERSION`].

/// The version string: the output of `git describe --tags --always --dirty`
/// (e.g. `v0.17.0` or `v0.17.0-3-gabc1234-dirty`), or this crate's version
/// from `Cargo.toml` when git was unavailable at build time.
pub const VERSION: &str = version_from_git_describe(env!("FINANCI_VERSION"));

/// The choice `build.rs` makes, factored out so it is unit-tested: use the
/// `git describe` output verbatim when git produced one, and fall back to
/// the package version when it didn't — git missing, not a repository, or
/// empty output (`build.rs` leaves `FINANCI_VERSION` empty in those cases).
pub const fn version_from_git_describe(git: &str) -> &str {
    if git.is_empty() {
        env!("CARGO_PKG_VERSION")
    } else {
        git
    }
}

/// The text the UI shows: [`VERSION`] with a leading `v`. Git tags
/// (`v0.17.0`) already carry one, the `--always` bare hash and the
/// `CARGO_PKG_VERSION` fallback don't.
pub fn version_label() -> String {
    version_label_from(VERSION)
}

/// [`version_label`] for any version string (pure, for tests).
pub fn version_label_from(version: &str) -> String {
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_describe_output_is_used_verbatim() {
        assert_eq!(version_from_git_describe("v0.17.0"), "v0.17.0");
        assert_eq!(
            version_from_git_describe("v0.17.0-3-gabc1234-dirty"),
            "v0.17.0-3-gabc1234-dirty"
        );
        // `--always` with no tags: the bare hash stands as the version.
        assert_eq!(version_from_git_describe("abc1234"), "abc1234");
    }

    #[test]
    fn missing_git_output_falls_back_to_the_package_version() {
        assert_eq!(version_from_git_describe(""), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_label_carries_exactly_one_v_prefix() {
        assert_eq!(
            version_label_from("v0.17.0-3-gabc1234-dirty"),
            "v0.17.0-3-gabc1234-dirty"
        );
        assert_eq!(version_label_from("0.1.0"), "v0.1.0");
    }

    #[test]
    fn the_footer_label_equals_the_const_with_one_v() {
        // Whatever build.rs produced, the shown text is `VERSION` with at
        // most one "v" added — never a "vv" and never a dropped one.
        assert_eq!(version_label(), version_label_from(VERSION));
        assert!(version_label().starts_with('v'));
        assert_eq!(
            version_label().trim_start_matches('v'),
            VERSION.trim_start_matches('v')
        );
    }
}
