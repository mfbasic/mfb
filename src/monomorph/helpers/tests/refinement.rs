//! `refines` over every shape it walks, the concrete-nominal leaf in
//! `unify_type`, and an unreadable package in `collect_imported_overloads`.
//!
//! `refines(general, specific)` is what lets a provisional template binding
//! taken from an untyped `[]` be replaced by a later concrete argument
//! (bug-442): `specific` must be `general` with `Unknown` leaves filled in, at
//! any depth, in any container — and nothing else.

use super::*;

fn t(spelling: &str) -> ParameterType {
    ParameterType::parse(spelling)
}

#[test]
fn refines_fills_unknown_leaves_in_every_container_and_nothing_else() {
    let thread = |worker: bool, msg: ParameterType, res: ParameterType, out: ParameterType| {
        ParameterType::thread_handle(worker, msg, res, out)
    };
    let cases: Vec<(ParameterType, ParameterType, bool)> = vec![
        (t("Set OF Unknown"), t("Set OF Integer"), true),
        (t("Set OF Integer"), t("Set OF String"), false),
        (t("#Result OF Unknown"), t("#Result OF Integer"), true),
        (
            t("MapEntry OF Unknown TO Integer"),
            t("MapEntry OF String TO Integer"),
            true,
        ),
        (
            t("MapEntry OF String TO Integer"),
            t("MapEntry OF String TO String"),
            false,
        ),
        (
            t("Pair OF Unknown, String"),
            t("Pair OF Integer, String"),
            true,
        ),
        (
            t("Pair OF Unknown, String"),
            t("Box OF Integer, String"),
            false,
        ),
        (t("Pair OF Unknown, String"), t("Pair OF Integer"), false),
        (
            t("FUNC(Unknown) AS String"),
            t("FUNC(Integer) AS String"),
            true,
        ),
        (
            t("FUNC(Integer) AS Unknown"),
            t("FUNC(Integer) AS Boolean"),
            true,
        ),
        (
            t("FUNC(Unknown) AS String"),
            t("ISOLATED FUNC(Integer) AS String"),
            false,
        ),
        (
            t("FUNC(Unknown) AS String"),
            t("FUNC(Integer, Integer) AS String"),
            false,
        ),
        (
            thread(
                false,
                ParameterType::Unknown,
                ParameterType::Nothing,
                ParameterType::String,
            ),
            thread(
                false,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::String,
            ),
            true,
        ),
        (
            thread(
                false,
                ParameterType::Unknown,
                ParameterType::Nothing,
                ParameterType::String,
            ),
            thread(
                true,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::String,
            ),
            false,
        ),
        (
            thread(
                true,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::Unknown,
            ),
            thread(
                true,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
            true,
        ),
        // Two different shapes, or two different leaves: only equality refines.
        (t("List OF Unknown"), t("Set OF Integer"), false),
        (t("Integer"), t("Integer"), true),
        (t("Integer"), t("String"), false),
    ];
    for (general, specific, want) in cases {
        assert_eq!(
            refines(&general, &specific),
            want,
            "refines({}, {})",
            general.name(),
            specific.name()
        );
    }
}

/// A concrete nominal in a template's parameter list (`p AS Point`) is not a
/// template parameter: it unifies by equality and binds nothing.
#[test]
fn a_concrete_nominal_parameter_unifies_by_equality() {
    let params: HashSet<crate::intern::Symbol> =
        HashSet::from([crate::intern::Symbol::intern("T")]);
    let mut substitutions = HashMap::new();
    assert!(unify_type(
        &t("Point"),
        &t("Point"),
        &params,
        &mut substitutions
    ));
    assert!(substitutions.is_empty());
    assert!(!unify_type(
        &t("Point"),
        &t("Line"),
        &params,
        &mut substitutions
    ));
    // A container pattern refuses a different container outright.
    assert!(!unify_type(
        &t("Set OF T"),
        &t("List OF Integer"),
        &params,
        &mut substitutions
    ));
    assert!(substitutions.is_empty());
}

/// An import whose `.mfp` cannot be read contributes no overloads; it is not a
/// panic, and the other imports' sets are still collected.
#[test]
fn an_unreadable_package_contributes_no_overloads() {
    let fixture = crate::testutil::fixture_dir("package-simple")
        .join("golden")
        .join("package_simple.mfp");
    let dir = tempfile::tempdir().expect("tempdir");
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();
    std::fs::write(packages.join("broken.mfp"), b"not a package").unwrap();
    let src =
        "IMPORT broken\nIMPORT package_simple\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";
    let file = crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
        .expect("parse");
    let project = crate::ast::AstProject {
        name: "app".to_string(),
        files: vec![file],
    };
    let (overloads, _) = collect_imported_overloads(dir.path(), &crate::hir::elaborate(&project));
    assert!(
        overloads.keys().all(|key| !key.starts_with("broken.")),
        "{:?}",
        overloads.keys().collect::<Vec<_>>()
    );
    assert!(overloads.contains_key("package_simple.score"));
}
