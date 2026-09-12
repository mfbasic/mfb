pub const OWNER_LIMIT: usize = 255;
pub const PACKAGE_LIMIT: usize = 128;
pub const VERSION_LIMIT: usize = 64;

pub fn fold_owner(owner: &str) -> String {
    owner.to_ascii_lowercase()
}

pub fn validate_owner_name(owner: &str) -> Result<(), String> {
    if owner.is_empty() {
        return Err("missing owner name".to_string());
    }
    if owner.len() > OWNER_LIMIT {
        return Err("invalid owner name: owner name is too long".to_string());
    }
    if !owner.is_ascii() {
        return Err("invalid owner name: owner name must be ASCII".to_string());
    }
    if owner.eq_ignore_ascii_case("std") {
        return Err("reserved owner name: std".to_string());
    }

    let mut chars = owner.chars();
    let Some(first) = chars.next() else {
        return Err("missing owner name".to_string());
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err("invalid owner name: must start with a letter or underscore".to_string());
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(
            "invalid owner name: only ASCII letters, digits, and underscores are allowed"
                .to_string(),
        );
    }

    Ok(())
}

/// Whether a release state counts as **active** — eligible to be advertised as
/// a package's headline "latest" version.
///
/// Mirrors the install client's `state_is_floating_eligible`
/// (`grep -n "fn state_is_floating_eligible" src/cli/pkg.rs`): the registry must
/// not name a version the installer would refuse on a floating add.
///
/// The release-state vocabulary has **five** members, and this is deliberately
/// an **allowlist** of two rather than a denylist of `yanked`:
///
/// * `available` — active.
/// * `deprecated` — active. Still installed on a floating add, so a package
///   whose only maintained line is deprecated must not read as having no
///   release at all.
/// * `yanked` — not active; installable only by exact pin.
/// * `blocked` — not active; operator-set, never installed.
/// * `legal-tombstoned` — not active; operator-set, never installed.
///
/// A `state != "yanked"` filter would silently admit the last two, which is the
/// same class of bug this predicate exists to prevent.
pub fn state_is_active(state: &str) -> bool {
    matches!(state, "available" | "deprecated")
}

/// Validate the `package` component of an `owner#package` ident (REPO-17). The
/// package part reaches the log payload (`{"ident":...}`), the `/index/<ident>`
/// route, and the REPO-14 log-lookup pattern, so it is restricted to an explicit
/// safe charset: ASCII letters, digits, `_`, `-`, and `.`. This keeps control
/// characters, quotes, `#`, `/`, whitespace, and the SQL `LIKE` wildcard `%` out
/// of the payload entirely (`_` is a common package char and is safe now that the
/// log lookup escapes `LIKE` metacharacters).
pub fn validate_package_name(package: &str) -> Result<(), String> {
    if package.is_empty() {
        return Err("invalid package name: missing package name".to_string());
    }
    if package.len() > PACKAGE_LIMIT {
        return Err("invalid package name: package name is too long".to_string());
    }
    let mut chars = package.chars();
    let first = chars.next().expect("non-empty checked above");
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return Err(
            "invalid package name: must start with an ASCII letter, digit, or underscore"
                .to_string(),
        );
    }
    if !package
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        return Err(
            "invalid package name: only ASCII letters, digits, '_', '-', and '.' are allowed"
                .to_string(),
        );
    }
    Ok(())
}

/// Validate a package version string (REPO-17). Same reachability as the package
/// name; restricted to a semver-friendly safe charset (ASCII letters, digits, and
/// `.`, `-`, `+`, `_`) with a `VERSION_LIMIT` cap, rejecting control characters,
/// quotes, `#`, `/`, whitespace, and `%`.
pub fn validate_version(version: &str) -> Result<(), String> {
    if version.is_empty() {
        return Err("invalid version: missing version".to_string());
    }
    if version.len() > VERSION_LIMIT {
        return Err("invalid version: version is too long".to_string());
    }
    if !version
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '+' || ch == '_')
    {
        return Err(
            "invalid version: only ASCII letters, digits, '.', '-', '+', and '_' are allowed"
                .to_string(),
        );
    }
    Ok(())
}

/// Validate a full `owner#package` ident: both components with their respective
/// charset rules (REPO-17).
pub fn validate_ident(ident: &str) -> Result<(), String> {
    let Some((owner, package)) = ident.split_once('#') else {
        return Err("invalid ident: expected 'owner#package'".to_string());
    };
    validate_owner_name(owner)?;
    validate_package_name(package)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_validation_accepts_valid_names() {
        for owner in ["alice", "Alice", "_owner", "owner_1", "A123"] {
            validate_owner_name(owner).expect(owner);
        }
    }

    #[test]
    fn owner_validation_rejects_invalid_names() {
        for owner in [
            "",
            "std",
            "STD",
            "1alice",
            "alice-bob",
            "alice/bob",
            "alice.bob",
            "éclair",
        ] {
            assert!(validate_owner_name(owner).is_err(), "{owner}");
        }
        assert!(validate_owner_name(&"a".repeat(OWNER_LIMIT + 1)).is_err());
    }

    #[test]
    fn package_validation_accepts_valid_names() {
        for package in ["pkg", "tool-box", "a.b.c", "pkg_1", "1package", "_hidden"] {
            validate_package_name(package).expect(package);
        }
    }

    #[test]
    fn package_validation_rejects_unsafe_names() {
        for package in [
            "", "pk g", "pk\"g", "pk%g", "pk/g", "pk#g", "pk\ng", "pk\0g", "éclair", "-lead",
        ] {
            assert!(validate_package_name(package).is_err(), "{package:?}");
        }
        assert!(validate_package_name(&"p".repeat(PACKAGE_LIMIT + 1)).is_err());
    }

    #[test]
    fn version_validation_accepts_and_rejects() {
        for version in ["1", "1.0.0", "1.2.3-rc.1", "2.0.0+build_7"] {
            validate_version(version).expect(version);
        }
        for version in ["", "1.0 0", "1.0\"0", "1.0%0", "1/0", "1.0\n0", "vé"] {
            assert!(validate_version(version).is_err(), "{version:?}");
        }
        assert!(validate_version(&"1".repeat(VERSION_LIMIT + 1)).is_err());
    }

    /// plan-126-A: `state_is_active` must agree with the install client's
    /// `state_is_floating_eligible` across the **whole** five-member release
    /// vocabulary, not just the three the maintainer route accepts.
    ///
    /// The sibling table lives in the compiler crate — see the states enumerated
    /// by `grep -n "state_is_floating_eligible" -A 6 src/cli/pkg.rs` and its test
    /// (`grep -n "legal-tombstoned" src/cli/pkg.rs`). This crate cannot import
    /// it, so the vocabulary is restated here and the *agreement* is the
    /// assertion: a denylist implementation (`state != "yanked"`) fails on the
    /// `blocked` and `legal-tombstoned` rows.
    #[test]
    fn active_states_are_an_allowlist_matching_the_install_client() {
        for (state, expected) in [
            ("available", true),
            ("deprecated", true),
            ("yanked", false),
            ("blocked", false),
            ("legal-tombstoned", false),
        ] {
            assert_eq!(state_is_active(state), expected, "{state}");
        }
        // An unknown state is not active: a vocabulary this predicate has never
        // heard of must not be advertised as a package's headline release.
        assert!(!state_is_active(""));
        assert!(!state_is_active("Available"));
        assert!(!state_is_active("withdrawn"));
    }

    #[test]
    fn ident_validation_requires_both_parts() {
        validate_ident("alice#pkg").unwrap();
        assert!(validate_ident("no-hash").is_err());
        assert!(validate_ident("alice#").is_err());
        assert!(validate_ident("#pkg").is_err());
        assert!(validate_ident("bad owner#pkg").is_err());
    }
}
