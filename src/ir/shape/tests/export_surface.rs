//! bug-624 B: a package's EXPORT surface may not name a type it does not
//! export (`export_names_non_exported_type_diagnostics`). The `.mfp` type
//! table would not carry the hidden type, so every importer would reject the
//! package. Each position the writer serializes is checked — an exported
//! record's field, an exported union's variant field (its `INCLUDES` expanded
//! first), an exported function's parameter and return, an exported
//! binding's type — and each hidden type is named once per position.
//!
//! Plus `check_project`, the render-and-report form `mfb audit` calls.

use super::*;

const PACKAGE: &str = "\
TYPE Hidden
  n AS Integer
END TYPE

TYPE Inner
  h AS Hidden
END TYPE

EXPORT TYPE Shown
  h AS Hidden
  pair AS List OF Hidden
END TYPE

EXPORT TYPE Leaf
  h AS Hidden
END TYPE

EXPORT TYPE Other
  n AS Integer
END TYPE

UNION Base
  Leaf
END UNION

EXPORT UNION Wide INCLUDES Base
  Other
END UNION

EXPORT FUNC make(seed AS Hidden) AS Hidden
  RETURN seed
END FUNC

EXPORT LET fallback AS Hidden = Hidden[1]
";

fn surface(is_package: bool) -> Vec<(String, String)> {
    let ir = crate::testutil::lower_src(PACKAGE);
    export_names_non_exported_type_diagnostics(is_package, &ir, Path::new("/proj"))
        .into_iter()
        .map(|d| (d.rule, d.detail))
        .collect()
}

#[test]
fn every_exported_position_naming_a_hidden_type_is_reported_once() {
    let found = surface(true);
    assert!(
        found
            .iter()
            .all(|(rule, _)| rule == "EXPORT_NAMES_NON_EXPORTED_TYPE"),
        "{found:?}"
    );
    let details: Vec<&str> = found.iter().map(|(_, detail)| detail.as_str()).collect();
    for want in [
        "EXPORT TYPE `Shown` names `Hidden` in its field `h`",
        "EXPORT TYPE `Shown` names `Hidden` in its field `pair`",
        "EXPORT UNION `Wide` names `Hidden` in its variant `Leaf` field `h`",
        "EXPORT FUNC `make` names `Hidden` in its parameter `seed`",
        "EXPORT FUNC `make` names `Hidden` in its return type",
        "EXPORT LET `fallback` names `Hidden` in its type",
    ] {
        assert!(
            details.iter().any(|detail| detail.starts_with(want)),
            "missing `{want}` in {details:#?}"
        );
    }
    // `Leaf` is exported, so its own field is reported once, under `Leaf`.
    assert_eq!(
        details
            .iter()
            .filter(|detail| detail.starts_with("EXPORT TYPE `Leaf`"))
            .count(),
        1
    );
    // `Inner` is not exported: nothing it names is on the surface.
    assert!(!details.iter().any(|detail| detail.contains("`Inner`")));
}

#[test]
fn an_executable_has_no_export_surface_to_check() {
    assert!(surface(false).is_empty());
}

#[test]
fn check_project_reports_whether_any_diagnostic_was_an_error() {
    let clean = hir_from("FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n");
    assert!(check_project(Path::new("/proj"), &clean, &HashMap::new()).is_ok());
    let wrong = hir_from(
        "FUNC f(x AS Integer) AS Integer\n  RETURN x\nEND FUNC\n\
         FUNC main() AS Integer\n  RETURN f(1, 2)\nEND FUNC\n",
    );
    let (result, collected) = crate::rules::collecting_diagnostics(|| {
        check_project(Path::new("/proj"), &wrong, &HashMap::new())
    });
    assert!(result.is_err());
    assert!(
        collected
            .iter()
            .any(|(rule, _)| rule.starts_with("TYPE_CALL")),
        "{collected:?}"
    );
}

/// One position naming a hidden type twice (`Map OF Hidden TO Hidden`) reports
/// it once; a declaration with no source file (a generated one) reports
/// against `<generated>`; a union included along two paths is expanded once.
#[test]
fn a_hidden_type_is_reported_once_per_position_and_located() {
    let src = "\
TYPE Hidden
  n AS Integer
END TYPE

EXPORT TYPE Twice
  m AS Map OF Hidden TO Hidden
END TYPE

EXPORT TYPE Leaf
  h AS Hidden
END TYPE

UNION Base
  Leaf
END UNION

UNION Left INCLUDES Base
END UNION

UNION Right INCLUDES Base
END UNION

EXPORT UNION Both INCLUDES Left, Right
END UNION
";
    let mut ir = crate::testutil::lower_src(src);
    for ty in &mut ir.types {
        if ty.name == "Twice" {
            ty.file = String::new();
        }
    }
    let found = export_names_non_exported_type_diagnostics(true, &ir, Path::new("/proj"));
    let twice: Vec<_> = found
        .iter()
        .filter(|d| d.detail.starts_with("EXPORT TYPE `Twice`"))
        .collect();
    assert_eq!(twice.len(), 1, "named twice in one field, reported once");
    assert_eq!(twice[0].path, Path::new("/proj").join("<generated>"));
    let both = found
        .iter()
        .filter(|d| d.detail.starts_with("EXPORT UNION `Both`"))
        .count();
    assert_eq!(
        both, 1,
        "`Base` reached through `Left` and `Right` is walked once"
    );
}
