//! Name, version and ident validators shared by the compiler and the registry
//! (plan-126-C, moved from `repository/src/validation.rs` and
//! `src/manifest/package.rs`).
//!
//! # Two package-name policies, and why there are still two
//!
//! The tree has two package-name validators. Read side by side they are closer
//! than the existence of two functions suggests: **their charsets are
//! identical** — first character `[A-Za-z0-9_]`, the rest
//! `[A-Za-z0-9_.-]` — and the only substantive difference is that the registry's
//! adds a [`PACKAGE_LIMIT`] byte cap.
//!
//! They are named here for *what each one guards*, because that is the only
//! thing that makes keeping two defensible:
//!
//! * [`validate_path_component_name`] guards a **filesystem path**. A `.mfp`
//!   header name and an `mfb.lock` name are both turned into
//!   `packages/<name>.mfp`, so a name of `../../x` escapes the project and a
//!   name beginning with `.` hides the file.
//! * [`validate_registry_package_name`] guards a **log payload and a URL
//!   route** — `{"ident":…}` and `/index/<ident>` (REPO-17). It keeps control
//!   characters, quotes, `#`, `/`, whitespace and the SQL `LIKE` wildcard `%`
//!   out of the transparency log entirely.
//!
//! Unifying them is a *behavior change*, not a code move: the merged function
//! would have to pick one length rule, and `validate_path_component_name` would
//! start rejecting names over 128 bytes. plan-126-C measured whether that
//! matters — the longest header name across all 160 committed `.mfp` fixtures is
//! **34 bytes**, and none exceeds 128 — so the change is de-risked, but it is
//! deliberately left to a separately-tested follow-up rather than smuggled in
//! behind a refactor. `the_two_package_name_policies_differ_only_by_a_length_cap`
//! below is the executable statement of exactly what still differs.

/// Maximum owner-name length in bytes (REPO-17).
pub const OWNER_LIMIT: usize = 255;
/// Maximum registry package-name length in bytes (REPO-17).
///
/// This cap is the **only** substantive difference between the two
/// package-name policies; see the module doc.
pub const PACKAGE_LIMIT: usize = 128;
/// Maximum version-string length in bytes (REPO-17).
pub const VERSION_LIMIT: usize = 64;

/// Case-fold an owner name for uniqueness comparison.
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

/// Reject a package name that cannot be used as a single path component.
///
/// A `.mfp` header name and an `mfb.lock` name are untrusted: both are turned
/// into `packages/<name>.mfp`. Without this guard a name of `../../x` escapes
/// the project, and a name beginning with `.` hides the file. Legitimate names
/// are identifier-like, so the charset is deliberately narrow.
///
/// **No length cap**, unlike [`validate_registry_package_name`] — that is the
/// one substantive difference between the two. An empty name fails here because
/// `chars.next()` is `None`, without a distinct message.
pub fn validate_path_component_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let leading_ok = chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    let rest_ok = chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if !leading_ok || !rest_ok {
        return Err(format!(
            "package name `{name}` is not a valid path component (expected [A-Za-z0-9_][A-Za-z0-9_.-]*)"
        ));
    }
    Ok(())
}

/// Validate the `package` component of an `owner#package` ident (REPO-17). The
/// package part reaches the log payload (`{"ident":...}`), the `/index/<ident>`
/// route, and the REPO-14 log-lookup pattern, so it is restricted to an explicit
/// safe charset: ASCII letters, digits, `_`, `-`, and `.`. This keeps control
/// characters, quotes, `#`, `/`, whitespace, and the SQL `LIKE` wildcard `%` out
/// of the payload entirely (`_` is a common package char and is safe now that the
/// log lookup escapes `LIKE` metacharacters).
///
/// Differs from [`validate_path_component_name`] only by the [`PACKAGE_LIMIT`]
/// cap and by having a distinct empty-input message.
pub fn validate_registry_package_name(package: &str) -> Result<(), String> {
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
/// `.`, `-`, `+`, `_`) with a [`VERSION_LIMIT`] cap, rejecting control characters,
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
///
/// The package half uses [`validate_registry_package_name`] — an ident is a
/// registry concept and reaches the log and the route, not the filesystem.
pub fn validate_ident(ident: &str) -> Result<(), String> {
    let Some((owner, package)) = ident.split_once('#') else {
        return Err("invalid ident: expected 'owner#package'".to_string());
    };
    validate_owner_name(owner)?;
    validate_registry_package_name(package)?;
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
            validate_registry_package_name(package).expect(package);
        }
    }

    #[test]
    fn package_validation_rejects_unsafe_names() {
        for package in [
            "", "pk g", "pk\"g", "pk%g", "pk/g", "pk#g", "pk\ng", "pk\0g", "éclair", "-lead",
        ] {
            assert!(
                validate_registry_package_name(package).is_err(),
                "{package:?}"
            );
        }
        assert!(validate_registry_package_name(&"p".repeat(PACKAGE_LIMIT + 1)).is_err());
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

    #[test]
    fn ident_validation_requires_both_parts() {
        validate_ident("alice#pkg").unwrap();
        assert!(validate_ident("no-hash").is_err());
        assert!(validate_ident("alice#").is_err());
        assert!(validate_ident("#pkg").is_err());
        assert!(validate_ident("bad owner#pkg").is_err());
    }

    /// **plan-126-C: the executable statement of what still differs between the
    /// two package-name policies.**
    ///
    /// This exists so the next reader does not have to diff two functions to
    /// find out. The answer is: one length cap, and one error message. Their
    /// charsets are identical, which the loop below asserts across the
    /// interesting cases rather than leaving to inspection.
    ///
    /// If the two are ever unified, this test is the one that has to change,
    /// and changing it is the deliberate act that a silent merge would skip.
    #[test]
    fn the_two_package_name_policies_differ_only_by_a_length_cap() {
        // A 200-character all-legal-charset name: accepted as a path component,
        // rejected by the registry's `PACKAGE_LIMIT`.
        let long = "a".repeat(200);
        validate_path_component_name(&long)
            .expect("no length cap guards a filesystem path component");
        let err = validate_registry_package_name(&long).unwrap_err();
        assert_eq!(err, "invalid package name: package name is too long");

        // Exactly at the cap: both accept.
        let at_cap = "a".repeat(PACKAGE_LIMIT);
        validate_path_component_name(&at_cap).unwrap();
        validate_registry_package_name(&at_cap).unwrap();

        // The charsets agree — every one of these is accepted or rejected by
        // BOTH, so the cap really is the only substantive divergence.
        for name in [
            "pkg", "tool-box", "a.b.c", "pkg_1", "1package", "_hidden", &at_cap,
        ] {
            assert_eq!(
                validate_path_component_name(name).is_ok(),
                validate_registry_package_name(name).is_ok(),
                "charsets must agree on {name:?}",
            );
        }
        for name in [
            "pk g", "pk\"g", "pk%g", "pk/g", "pk#g", "pk\ng", "pk\0g", "éclair", "-lead",
            "../../x", ".hidden",
        ] {
            assert!(validate_path_component_name(name).is_err(), "{name:?}");
            assert!(validate_registry_package_name(name).is_err(), "{name:?}");
        }

        // The one other difference is cosmetic: an empty name gets a distinct
        // message from the registry validator and the generic one from the path
        // validator (where it falls out of `chars.next()` being `None`).
        assert_eq!(
            validate_registry_package_name("").unwrap_err(),
            "invalid package name: missing package name",
        );
        assert!(validate_path_component_name("")
            .unwrap_err()
            .contains("is not a valid path component"));
    }
}
