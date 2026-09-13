//! Re-export of the shared name/version/ident validators, plus the registry's
//! own release-state predicate.
//!
//! The validators live in `mfb_wire::validation` (plan-126-C): they are needed
//! by both this crate and the compiler, and `mfb_wire` is the one crate both may
//! depend on. Keeping them there rather than a copy here is what stops the two
//! drifting — a charset that differs across the crate boundary is worse than
//! either rule alone.
//!
//! `validate_package_name` is re-exported under its original name and bound to
//! the **registry** policy (`validate_registry_package_name`), which is the one
//! this crate has always applied: it carries the `PACKAGE_LIMIT` cap and guards
//! the log payload and the `/index/<ident>` route. The compiler's sibling policy
//! guards a filesystem path component instead. See that module's doc for why
//! there are still two.

pub use mfb_wire::validation::{
    fold_owner, validate_ident, validate_owner_name,
    validate_registry_package_name as validate_package_name, validate_version, OWNER_LIMIT,
    PACKAGE_LIMIT, VERSION_LIMIT,
};

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
///
/// This one stays in the registry crate rather than moving to `mfb_wire` with
/// the validators above: it is not a wire format, and unifying it with the
/// client's copy would mean editing `src/cli/pkg.rs`, which plan-126-A's
/// Non-goals forbid. The duplication is recorded in plan-126-C's Corrections as
/// a candidate for a later, deliberate pass.
pub fn state_is_active(state: &str) -> bool {
    matches!(state, "available" | "deprecated")
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The shim really does bind `validate_package_name` to the registry policy,
    /// not to the compiler's path-component one. The two differ only by the
    /// `PACKAGE_LIMIT` cap, so an over-cap name is the single input that tells
    /// them apart — and this crate must keep rejecting it.
    #[test]
    fn the_reexported_package_name_validator_is_the_registry_policy() {
        validate_package_name("pkg").unwrap();
        let err = validate_package_name(&"a".repeat(PACKAGE_LIMIT + 1)).unwrap_err();
        assert_eq!(err, "invalid package name: package name is too long");
    }
}
