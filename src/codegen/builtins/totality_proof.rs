//! plan-165-B: which built-ins a raise-checking guard proves total.
//!
//! A built-in call is treated as unable to fail only with positive proof (owner
//! decision 2026-10-04: missing or unknown information counts as "can fail").
//! A member's empty registry `errors` list is a claim; it becomes a proof only
//! when a test lowers the member and checks that its code raises nothing it does
//! not declare. This module is the list of members such a guard covers, shared
//! by the fallibility census ([`super::inline_builtin_is_infallible`]) and the
//! tests that enforce it (`tests/infallible_proof_coverage.rs`), so a new grant
//! without a guard fails a test before it can delete a live handler.
//!
//! Two guards count:
//!
//! - a per-package declared-errors sweep (`tests/declared_errors_all.rs`,
//!   `tests/fs_declared_errors.rs`), which lowers every native member of the
//!   package on all five backends and compares the error codes the emitted code
//!   loads with the declaration — except the members the sweep skips
//!   ([`RAISE_CHECK_SKIPPED_MEMBERS`]), which therefore have no proof;
//! - the per-overload rows of `tests/general_declared_errors.rs`, which compile
//!   a real call per overload of a `general` intrinsic.
//!
//! User functions are proven by the fallibility fixpoint (`ir::fallible`) and
//! imported package functions are never proven (no `.mfp` fallibility metadata),
//! so neither appears here.

use crate::codegen::registry::{registry, Body, RegistryFunction};

/// Packages whose native (`AbiFunction`/`AbiInline`) members are all lowered by a
/// declared-errors sweep, apart from [`RAISE_CHECK_SKIPPED_MEMBERS`]. Each entry
/// needs an `every_<package>_member_declares_exactly_the_errors_its_lowering_raises`
/// test; `infallible_proof_coverage` checks that it exists.
pub(crate) const RAISE_CHECKED_PACKAGES: &[&str] = &[
    "app",
    "astrings",
    "audio",
    "big",
    "bits",
    "canvas",
    "collections",
    "crypto",
    "datetime",
    "fs",
    "io",
    "json",
    "math",
    "money",
    "net",
    "os",
    "process",
    "regex",
    "strings",
    "tcp",
    "term",
    "thread",
    "tls",
    "udp",
];

/// Members of a [`RAISE_CHECKED_PACKAGES`] package that its sweep does not lower
/// (a harness fixture gap, documented at the sweep), so their declarations are
/// unproved: `(package, member)` pairs. The sweeps read their skip lists from here
/// (`tests/declared_errors_all.rs`, `raise_check_skipped`).
pub(crate) const RAISE_CHECK_SKIPPED_MEMBERS: &[(&str, &str)] = &[];

/// `general` intrinsics whose every overload has a `general_declared_errors`
/// row raising nothing. Other `general` members with an empty `errors` list
/// (`isEmpty`, `isZero`, …) are not granted: granting them is a precision change
/// outside plan-165-B. `toInt` is total only over a single enum argument, which
/// the census grants per overload (bug-679), not by name.
pub(crate) const RAISE_CHECKED_GENERAL_TOTAL: &[&str] = &["len", "typeName"];

/// Whether `function` (a member of `package`) is proven unable to fail: it has an
/// inline call-site lowering (`Body::AbiInline` — the only members the census
/// grants), **every** implementation declares no error (not just the inline
/// ones: an inline overload that declares nothing beside a non-inline one that
/// does is not total), and a raise-checking guard covers it.
pub(crate) fn member_is_proven_total(package: &str, function: &RegistryFunction) -> bool {
    let implementations = function.implementations();
    implementations
        .iter()
        .any(|implementation| matches!(implementation.body, Body::AbiInline(_)))
        && implementations
            .iter()
            .all(|implementation| implementation.errors.is_empty())
        && builtin_is_raise_checked(&format!("{package}.{}", function.name))
}

/// For a qualified `target` naming a registry member with an inline call-site
/// lowering: `Some(true)` when it is proven total ([`member_is_proven_total`]),
/// `Some(false)` when it is not. `None` for anything else — a bare `general`
/// name, a non-inline member, an unknown name.
pub(crate) fn inline_member_is_proven_total(target: &str) -> Option<bool> {
    let resolved = registry().resolve_func(target)?;
    resolved
        .function
        .implementations()
        .iter()
        .any(|implementation| matches!(implementation.body, Body::AbiInline(_)))
        .then(|| member_is_proven_total(resolved.package.import_name(), resolved.function))
}

/// Whether a raise-checking guard covers the built-in `target` — a qualified
/// `package.member` or a bare `general` name.
pub(crate) fn builtin_is_raise_checked(target: &str) -> bool {
    match target.split_once('.') {
        Some((package, member)) => {
            RAISE_CHECKED_PACKAGES.contains(&package)
                && !RAISE_CHECK_SKIPPED_MEMBERS.contains(&(package, member))
        }
        None => RAISE_CHECKED_GENERAL_TOTAL.contains(&target),
    }
}
