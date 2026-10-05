//! plan-165-B: a built-in call is unable to fail only with positive proof.
//!
//! The census [`inline_builtin_is_infallible`] decides whether an inline `TRAP`
//! handler on a built-in is dead, whether a function is total, and whether a
//! call is a failure edge into a function handler (bug-797). A wrong `true`
//! deletes a live handler, so every grant must be backed by a raise-checking
//! guard: a declared-errors sweep that lowers the member, or a
//! `general_declared_errors` row per overload. `totality_proof` is the shared
//! list; these tests hold the census to it and the list to the guards.

use crate::codegen::builtins::totality_proof::{
    builtin_is_raise_checked, raise_check_skipped_members, RAISE_CHECKED_GENERAL_TOTAL,
    RAISE_CHECKED_PACKAGES, RAISE_CHECK_SKIPPED_MEMBERS,
};
use crate::codegen::builtins::{inline_builtin_is_infallible, NoTypeKinds, TypeKinds};
use crate::codegen::registry::registry;
use crate::types::ParameterType;

use super::general_declared_errors::OVERLOADS;

/// The census target for a registry member: `general` members are called by
/// their bare name, every other package's by `package.member`.
fn census_target(package: &str, member: &str) -> String {
    if package == "general" {
        member.to_string()
    } else {
        format!("{package}.{member}")
    }
}

/// An oracle for which every declared type is an enum: the widest grant the
/// census's one argument-dependent rule (`toInt` over an enum, bug-679) can
/// make, so the enumeration below sees it.
struct EveryNamedIsEnum;

impl TypeKinds for EveryNamedIsEnum {
    fn is_enum(&self, t: &ParameterType) -> bool {
        matches!(t, ParameterType::Named(_))
    }
}

/// Every registry member the census grants with no argument types, plus the
/// argument-dependent grants under an all-enum oracle, as `(target, overload)`.
fn census_grants() -> Vec<String> {
    let mut grants = Vec::new();
    for package in registry().packages() {
        for function in package.functions() {
            let target = census_target(package.import_name(), function.name);
            if inline_builtin_is_infallible(&target, &[], &NoTypeKinds) {
                grants.push(target.clone());
            }
            let color = [ParameterType::named("Color")];
            if !inline_builtin_is_infallible(&target, &[], &NoTypeKinds)
                && inline_builtin_is_infallible(&target, &color, &EveryNamedIsEnum)
            {
                grants.push(format!("{target}(<enum>)"));
            }
        }
    }
    grants
}

/// Whether a raise-checking guard covers one census grant.
fn grant_is_covered(grant: &str) -> bool {
    match grant.strip_suffix("(<enum>)") {
        // bug-679's `toInt(<enum>)`: `general_declared_errors` has the `Color`
        // row, and it raises nothing.
        Some(member) => OVERLOADS
            .iter()
            .any(|(name, args, _, codes)| *name == member && *args == ["Color"] && codes.is_empty()),
        None => builtin_is_raise_checked(grant),
    }
}

/// Every member the census proves total is covered by a raise-checking guard.
/// A grant with no guard rests on a hand-written `errors: vec![]` alone, and a
/// member whose lowering raises an undeclared error would silently lose its
/// handler.
#[test]
fn every_inline_builtin_census_grant_is_raise_checked() {
    let grants = census_grants();
    assert!(
        grants.iter().any(|grant| grant == "len"),
        "the enumeration must see the census's grants at all: {grants:?}"
    );
    let uncovered: Vec<&String> = grants
        .iter()
        .filter(|grant| !grant_is_covered(grant))
        .collect();
    assert!(
        uncovered.is_empty(),
        "{} census grant(s) have no raise-checking guard (add the member's package \
         sweep, or let the member stay fallible):\n  {}",
        uncovered.len(),
        uncovered
            .iter()
            .map(|grant| grant.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// Each package `totality_proof` names as swept has its sweep test, and every
/// sweep reads its skip list from `totality_proof` rather than its own literal,
/// so a member the sweep skips can never be counted as proven.
#[test]
fn every_inline_builtin_raise_checked_package_has_its_sweep() {
    let sweeps = [
        include_str!("declared_errors_all.rs"),
        include_str!("fs_declared_errors.rs"),
    ];
    for package in RAISE_CHECKED_PACKAGES {
        let test = format!("fn every_{package}_member_declares_exactly_the_errors_its_lowering_raises");
        assert!(
            sweeps.iter().any(|source| source.contains(&test)),
            "totality_proof lists `{package}` as swept, but `{test}` does not exist"
        );
        assert!(
            registry().resolve_package(package).is_some(),
            "totality_proof lists `{package}`, which is not a registry package"
        );
    }
    for source in sweeps {
        for line in source.lines() {
            assert!(
                !(line.contains("SKIP_MEMBERS: &[&str] = &[")
                    || line.contains("SKIP_MEMBERS: &[&str] = &[\"")),
                "a sweep declares its own skip list; read it from \
                 totality_proof::RAISE_CHECK_SKIPPED_MEMBERS instead: `{line}`"
            );
        }
    }
    for (package, members) in RAISE_CHECK_SKIPPED_MEMBERS {
        assert!(
            RAISE_CHECKED_PACKAGES.contains(package),
            "`{package}` has skipped members but is not a swept package"
        );
        let registered = registry()
            .resolve_package(package)
            .unwrap_or_else(|| panic!("{package}: not a registry package"));
        for member in *members {
            assert!(
                registered.function(member).is_some(),
                "{package}::{member} is skipped but is not a member"
            );
        }
    }
    assert_eq!(raise_check_skipped_members("strings"), &[] as &[&str]);
}

/// A fixture member named `name` with the given implementations, built from
/// real registry bodies so only the declarations differ.
fn fixture(
    name: &'static str,
    implementations: Vec<crate::codegen::registry::Implementation>,
) -> crate::codegen::registry::RegistryFunction {
    crate::codegen::registry::RegistryFunction {
        name,
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations,
    }
}

/// The first implementation of `package.member`.
fn implementation_of(package: &str, member: &str) -> crate::codegen::registry::Implementation {
    registry()
        .resolve_func(&format!("{package}.{member}"))
        .unwrap_or_else(|| panic!("{package}.{member}"))
        .function
        .implementations()[0]
        .clone()
}

/// The registry rule reads EVERY implementation: an inline overload that
/// declares nothing beside a non-inline overload that declares an error is not
/// proven total. (`native_member_declares_error`, the census's old input, read
/// only the `AbiInline` ones and would have granted this member.)
#[test]
fn an_inline_builtin_with_a_declaring_non_inline_overload_is_not_granted() {
    use crate::codegen::builtins::totality_proof::member_is_proven_total;
    use crate::codegen::registry::Body;
    let inline = implementation_of("bits", "band");
    assert!(matches!(inline.body, Body::AbiInline(_)) && inline.errors.is_empty());
    let declaring = registry()
        .packages()
        .iter()
        .flat_map(|package| package.functions())
        .flat_map(|function| function.implementations())
        .find(|implementation| {
            matches!(implementation.body, Body::AbiFunction { .. })
                && !implementation.errors.is_empty()
        })
        .expect("a declaring AbiFunction implementation")
        .clone();
    assert!(member_is_proven_total(
        "bits",
        &fixture("band", vec![inline.clone()])
    ));
    assert!(!member_is_proven_total(
        "bits",
        &fixture("band", vec![inline, declaring])
    ));
}

/// The bare-name route that granted any package's inline `contains`/`hasKey`/
/// `getOr` is gone: a `contains` that declares an error is not proven, and a
/// `contains` in a package no guard covers is not proven either.
#[test]
fn an_inline_builtin_named_like_a_pure_query_is_not_granted_by_name() {
    use crate::codegen::builtins::totality_proof::member_is_proven_total;
    let mut declaring = implementation_of("collections", "contains");
    declaring.errors = vec!["ErrOutOfMemory"];
    assert!(!member_is_proven_total(
        "collections",
        &fixture("contains", vec![declaring])
    ));
    let quiet = implementation_of("collections", "contains");
    assert!(!member_is_proven_total(
        "unguarded",
        &fixture("contains", vec![quiet.clone()])
    ));
    assert!(member_is_proven_total(
        "collections",
        &fixture("contains", vec![quiet])
    ));
    assert!(!inline_builtin_is_infallible("contains", &[], &NoTypeKinds));
}

/// Fail-closed: an argument of unknown type never earns a grant. `toInt` is
/// total only over a proven enum, and `replace` only as the proven `List`
/// overload; `Unknown` proves neither, whatever the oracle says about names.
#[test]
fn an_inline_builtin_with_unknown_argument_types_is_not_granted() {
    for kinds in [&NoTypeKinds as &dyn TypeKinds, &EveryNamedIsEnum] {
        assert!(!inline_builtin_is_infallible(
            "toInt",
            &[ParameterType::Unknown],
            kinds
        ));
        for replace in ["strings.replace", "collections.replace"] {
            let unknown = [
                ParameterType::Unknown,
                ParameterType::Unknown,
                ParameterType::Unknown,
            ];
            assert!(!inline_builtin_is_infallible(replace, &unknown, kinds));
            assert!(!inline_builtin_is_infallible(replace, &[], kinds));
        }
    }
}

/// Fail-closed: a name the registry does not know — an imported package
/// function, a typo — is never a proven-total built-in.
#[test]
fn an_inline_builtin_census_never_grants_an_unknown_name() {
    for target in ["mylib.helper", "helper", "bits.nope", "collections.nope"] {
        assert!(
            !inline_builtin_is_infallible(target, &[], &NoTypeKinds),
            "{target}"
        );
    }
}

/// The `general` intrinsics granted by name have a `general_declared_errors` row
/// for every overload their descriptor declares, each raising nothing, and the
/// descriptor itself declares nothing.
#[test]
fn every_inline_builtin_raise_checked_general_member_has_empty_rows() {
    let general = registry()
        .resolve_package("general")
        .expect("general package");
    for member in RAISE_CHECKED_GENERAL_TOTAL {
        let function = general
            .function(member)
            .unwrap_or_else(|| panic!("general::{member}: not a member"));
        assert!(
            function
                .implementations()
                .iter()
                .all(|implementation| implementation.errors.is_empty()),
            "general::{member} declares an error but is listed as proven total"
        );
        let rows: Vec<_> = OVERLOADS
            .iter()
            .filter(|(name, ..)| name == member)
            .collect();
        assert!(!rows.is_empty(), "general::{member} has no overload rows");
        for (_, args, _, codes) in rows {
            assert!(
                codes.is_empty(),
                "general::{member}({args:?}) raises {codes:?} but is listed as total"
            );
        }
    }
}
