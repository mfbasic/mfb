//! Overload selection that `monomorph::lower` owns and the build reports from
//! here, while the overload set is still visible: a VALUE reference to an
//! overloaded function (bug-822), a template name declared more than once
//! (bug-913), a file-PRIVATE name standing for its file's PUBLIC sibling
//! (bug-912), and the instantiation caps (bug-399).
//!
//! The diagnostics are collected (`rules::collecting_diagnostics`) and asserted
//! by rule AND detail: these are the messages whose job is to name the overload
//! set and the callable type the author wrote, so a code alone would still pass
//! if the message named the wrong thing.

use super::*;
use crate::rules::collecting_diagnostics;

type Diagnostics = Vec<(String, String)>;

/// Monomorphize `files` with every diagnostic collected, on a thread with the
/// stack an instantiation chain to the depth cap needs (libtest's 2 MiB is not
/// it in a debug build; a release build's main thread is).
fn diagnose_files(files: &[(&str, &str)]) -> (Result<crate::hir::HirProject, ()>, Diagnostics) {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(path, src)| (path.to_string(), src.to_string()))
        .collect();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let borrowed: Vec<(&str, &str)> = owned
                .iter()
                .map(|(path, src)| (path.as_str(), src.as_str()))
                .collect();
            collecting_diagnostics(|| monomorphize_files(&borrowed))
        })
        .expect("spawn")
        .join()
        .expect("monomorphization must not panic")
}

fn diagnose(src: &str) -> (Result<crate::hir::HirProject, ()>, Diagnostics) {
    diagnose_files(&[("src/main.mfb", src)])
}

fn rules(diagnostics: &Diagnostics) -> Vec<&str> {
    diagnostics.iter().map(|(rule, _)| rule.as_str()).collect()
}

/// The rendered body of the function named `name`.
fn body_of(project: &crate::hir::HirProject, name: &str) -> String {
    let function = functions(project)
        .into_iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function `{name}` in {:?}", function_names(project)));
    format!("{:?}", function.body)
}

const PICK: &str = "\
FUNC pick(x AS Integer) AS Integer
  RETURN x + 1
END FUNC
FUNC pick(x AS String) AS Integer
  RETURN len(x)
END FUNC
";

#[test]
fn an_overloaded_value_reference_with_no_expected_type_is_ambiguous() {
    let src = format!("{PICK}FUNC main() AS Integer\n  LET f = pick\n  RETURN 0\nEND FUNC\n");
    let (result, diagnostics) = diagnose(&src);
    assert!(result.is_err());
    assert_eq!(rules(&diagnostics), ["TYPE_OVERLOAD_AMBIGUOUS"]);
    let detail = &diagnostics[0].1;
    assert!(
        detail.contains("`pick` names 2 overloaded functions")
            && detail.contains("`FUNC(Integer) AS Integer`, `FUNC(String) AS Integer`")
            && detail.contains("LET f AS FUNC(…) AS … = pick"),
        "the message must name the set, both signatures and the fix: {detail}"
    );
}

#[test]
fn an_overloaded_value_reference_matching_no_member_names_the_expected_type() {
    let src = format!(
        "{PICK}FUNC main() AS Integer
  LET a AS FUNC(Boolean) AS Integer = pick
  LET e AS Integer = pick
  RETURN 0
END FUNC
"
    );
    let (result, diagnostics) = diagnose(&src);
    assert!(result.is_err());
    assert_eq!(
        rules(&diagnostics),
        ["TYPE_OVERLOAD_NO_MATCH", "TYPE_OVERLOAD_NO_MATCH"]
    );
    assert!(
        diagnostics[0]
            .1
            .contains("No overload of `pick` has the callable type `FUNC(Boolean) AS Integer`"),
        "{}",
        diagnostics[0].1
    );
    assert!(
        diagnostics[1]
            .1
            .contains("which cannot be used where `Integer` is expected"),
        "{}",
        diagnostics[1].1
    );
}

#[test]
fn an_annotated_value_reference_selects_the_one_member_of_that_type() {
    let src = format!(
        "{PICK}FUNC main() AS Integer
  LET f AS FUNC(String) AS Integer = pick
  LET g AS FUNC(Integer) AS Integer = pick
  RETURN f(\"ab\") + g(1)
END FUNC
"
    );
    let (result, diagnostics) = diagnose(&src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let project = result.expect("monomorphizes");
    let body = body_of(&project, "main");
    let names = function_names(&project);
    let string_member = names
        .iter()
        .find(|n| n.starts_with("pick") && n.contains("String"))
        .expect("the String member keeps a mangled symbol");
    let integer_member = names
        .iter()
        .find(|n| n.starts_with("pick") && n.contains("Integer") && !n.contains("String"))
        .expect("the Integer member keeps a mangled symbol");
    assert!(
        body.contains(&format!("{string_member:?}"))
            && body.contains(&format!("{integer_member:?}")),
        "each reference must be rewritten to its member's symbol: {body}"
    );
}

/// A user override of an overridable general built-in is selected by a value
/// reference only where the built-in itself rejects the expected parameter
/// types (the call rule's gap-fill), and only by its complete callable type.
#[test]
fn a_value_reference_to_an_overridden_builtin_selects_the_override_only_in_the_gap() {
    let src = "\
TYPE Grid
  cells AS List OF Integer
END TYPE
TYPE Ring
  items AS List OF Integer
END TYPE
FUNC len(g AS Grid) AS Integer
  RETURN len(g.cells)
END FUNC
FUNC len(r AS Ring) AS Integer
  RETURN len(r.items) * 2
END FUNC
FUNC main() AS Integer
  LET byGrid AS FUNC(Grid) AS Integer = len
  LET byText AS FUNC(String) AS Integer = len
  LET wrongReturn AS FUNC(Ring) AS String = len
  LET notCallable AS Integer = len
  RETURN 0
END FUNC
";
    let (result, diagnostics) = diagnose(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let project = result.expect("monomorphizes");
    let names = function_names(&project);
    let grid = names
        .iter()
        .find(|n| n.contains("len") && n.contains("Grid"))
        .expect("the Grid override keeps a mangled symbol");
    let body = body_of(&project, "main");
    assert!(
        body.contains(&format!("{grid:?}")),
        "`FUNC(Grid) AS Integer` must select the Grid override ({grid}): {body}"
    );
    // The built-in accepts `String`, so that reference keeps the bare built-in
    // name; `FUNC(Ring) AS String` matches no override and `Integer` is not
    // callable, so those two stay bare as well, left to IR lowering's own
    // built-in reference handling.
    let bare = format!("{:?}", "len");
    assert_eq!(
        body.matches(&bare).count(),
        3,
        "the built-in reference, the unmatched one and the non-callable one stay bare: {body}"
    );
}

#[test]
fn a_value_reference_to_an_imported_overload_set_is_selected_by_its_callable_type() {
    let fixture = crate::testutil::fixture_dir("package-simple")
        .join("golden")
        .join("package_simple.mfp");
    let dir = tempfile::tempdir().expect("tempdir");
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();
    let src = "\
IMPORT package_simple
FUNC main() AS Integer
  LET f AS FUNC() AS Integer = package_simple::score
  LET g AS FUNC(String) AS Integer = package_simple::score
  LET h = package_simple::score
  RETURN f()
END FUNC
";
    let file = crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
        .expect("parse");
    let ast = AstProject {
        name: "app".to_string(),
        files: vec![file],
    };
    let (result, diagnostics) = collecting_diagnostics(|| {
        super::super::monomorphize_project(dir.path(), &crate::hir::elaborate(&ast))
    });
    assert!(result.is_err());
    assert_eq!(
        rules(&diagnostics),
        ["TYPE_OVERLOAD_NO_MATCH", "TYPE_OVERLOAD_AMBIGUOUS"],
        "{diagnostics:?}"
    );
    // The set is named as the author spelled it, `package::member`.
    assert!(
        diagnostics[0]
            .1
            .contains("No overload of `package_simple::score` has the callable type `FUNC(String) AS Integer`"),
        "{}",
        diagnostics[0].1
    );
    assert!(
        diagnostics[1]
            .1
            .contains("`package_simple::score` names 2 overloaded functions"),
        "{}",
        diagnostics[1].1
    );
}

/// bug-912: inside a file that declares a PRIVATE template and a PUBLIC plain
/// function of one name, a value reference to the name is the PUBLIC function.
#[test]
fn a_file_private_name_references_its_public_sibling() {
    let files = [
        (
            "src/a.mfb",
            "\
FUNC r(x AS Integer) AS String
  RETURN \"i\"
END FUNC
PRIVATE FUNC r OF T(x AS List OF T) AS String
  RETURN \"l\"
END FUNC
FUNC other() AS Integer
  RETURN 1
END FUNC
FUNC user() AS String
  LET f AS FUNC(Integer) AS String = r
  RETURN f(1) & r([1])
END FUNC
",
        ),
        (
            "src/main.mfb",
            "FUNC main() AS Integer\n  RETURN len(user())\nEND FUNC\n",
        ),
    ];
    let mut ast = project(&files);
    crate::ast::scope_privates::scope_privates(&mut ast);
    let (result, diagnostics) = collecting_diagnostics(|| {
        super::super::monomorphize_project(&std::env::temp_dir(), &crate::hir::elaborate(&ast))
    });
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let project = result.expect("monomorphizes");
    let body = body_of(&project, "user");
    assert!(
        body.contains(&format!("{:?}", "r")),
        "the value reference must name the PUBLIC `r`: {body}"
    );
}

/// bug-912 + bug-822: inside the file, the PRIVATE overload and its PUBLIC
/// sibling are one set, and a value reference selects between them by type.
#[test]
fn a_value_reference_selects_between_a_private_overload_and_its_public_sibling() {
    let files = [
        (
            "src/a.mfb",
            "\
FUNC r(x AS Integer) AS String
  RETURN \"i\"
END FUNC
PRIVATE FUNC r(x AS String) AS String
  RETURN x
END FUNC
FUNC user() AS String
  LET byInt AS FUNC(Integer) AS String = r
  LET byText AS FUNC(String) AS String = r
  RETURN byInt(1) & byText(\"s\")
END FUNC
",
        ),
        (
            "src/main.mfb",
            "FUNC main() AS Integer\n  RETURN len(user())\nEND FUNC\n",
        ),
    ];
    let mut ast = project(&files);
    crate::ast::scope_privates::scope_privates(&mut ast);
    let (result, diagnostics) = collecting_diagnostics(|| {
        super::super::monomorphize_project(&std::env::temp_dir(), &crate::hir::elaborate(&ast))
    });
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let project = result.expect("monomorphizes");
    let body = body_of(&project, "user");
    let private = format!(
        "{}$String",
        crate::internal_name::mangle_private(
            &crate::internal_name::file_scope_hash("src/a.mfb"),
            "r"
        )
    );
    assert!(
        body.contains(&format!("{:?}", "r")) && body.contains(&format!("{private:?}")),
        "`FUNC(Integer)` must name the PUBLIC `r` and `FUNC(String)` the PRIVATE \
         member `{private}`: {body}"
    );
}

/// An imported set compares through the `Unknown` wildcard (the call rule's
/// `types_compatible`), so an expected callable type carrying an `Unknown`
/// leaf — a generic callback parameter bound from an untyped `[]` — can match
/// two members. That is ambiguous, never "whichever the package exported
/// first" (the reference twin of bug-36).
#[test]
fn an_imported_value_reference_matching_two_members_is_ambiguous() {
    let ast = AstProject {
        name: "app".to_string(),
        files: vec![],
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let hir = crate::hir::elaborate(&ast);
    let mut mono = Monomorphizer::new(dir.path(), &hir);
    mono.file_imports.insert(
        "src/main.mfb".to_string(),
        std::collections::HashMap::from([("pkg".to_string(), "pkg".to_string())]),
    );
    mono.current_file = Some("src/main.mfb".to_string());
    let member = |param: &str, export_name: &str| ImportedOverload {
        param_types: vec![ParameterType::parse(param)],
        export_name: export_name.to_string(),
        return_type: ParameterType::Integer,
        isolated: false,
    };
    mono.imported_overloads.insert(
        "pkg.f".to_string(),
        vec![
            member("List OF Integer", "f$ListOFInteger"),
            member("List OF String", "f$ListOFString"),
        ],
    );
    let context = crate::monomorph::FunctionContext {
        locals: std::collections::HashMap::new(),
        enclosing_return: None,
        bound: std::collections::HashSet::new(),
    };
    let no_substitutions = std::collections::HashMap::new();
    let concrete = ParameterType::parse("FUNC(List OF Integer) AS Integer");
    let (selected, diagnostics) = collecting_diagnostics(|| {
        mono.resolve_function_reference("pkg.f", Some(&concrete), &no_substitutions, &context, 3)
    });
    assert_eq!(selected.as_deref(), Some("pkg.f$ListOFInteger"));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let wildcard = ParameterType::parse("FUNC(List OF Unknown) AS Integer");
    let (selected, diagnostics) = collecting_diagnostics(|| {
        mono.resolve_function_reference("pkg.f", Some(&wildcard), &no_substitutions, &context, 3)
    });
    assert_eq!(selected, None);
    assert_eq!(rules(&diagnostics), ["TYPE_OVERLOAD_AMBIGUOUS"]);
    assert!(
        diagnostics[0].1.contains(
            "2 overloads of `pkg::f` have the callable type `FUNC(List OF Unknown) AS Integer`"
        ),
        "{}",
        diagnostics[0].1
    );
}

/// bug-912: a PRIVATE plain function beside its file's PUBLIC template: a call
/// that only the template binds instantiates the PUBLIC template.
#[test]
fn a_file_private_name_instantiates_its_public_template_sibling() {
    let files = [
        (
            "src/a.mfb",
            "\
FUNC w OF T(x AS List OF T) AS Integer
  RETURN len(x)
END FUNC
PRIVATE FUNC w(x AS String) AS Integer
  RETURN w([1, 2, 3])
END FUNC
FUNC user() AS Integer
  RETURN w(\"s\")
END FUNC
",
        ),
        (
            "src/main.mfb",
            "FUNC main() AS Integer\n  RETURN user()\nEND FUNC\n",
        ),
    ];
    let mut ast = project(&files);
    crate::ast::scope_privates::scope_privates(&mut ast);
    let (result, diagnostics) = collecting_diagnostics(|| {
        super::super::monomorphize_project(&std::env::temp_dir(), &crate::hir::elaborate(&ast))
    });
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let names = function_names(&result.expect("monomorphizes"));
    assert!(
        names.iter().any(|n| n == "w$Integer"),
        "the PUBLIC template is instantiated under its own name: {names:?}"
    );
}

const WRAP: &str = "\
FUNC wrap OF T(x AS T) AS Integer
  RETURN 1
END FUNC
FUNC wrap OF T(x AS List OF T) AS Integer
  RETURN 2
END FUNC
";

/// bug-913: two templates of one name; the call that both bind takes the more
/// specific one, and each instantiation keeps its own symbol.
#[test]
fn the_most_specific_of_two_binding_templates_is_instantiated() {
    let src = format!("{WRAP}FUNC main() AS Integer\n  RETURN wrap(5) + wrap([1, 2])\nEND FUNC\n");
    let (result, diagnostics) = diagnose(&src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let project = result.expect("monomorphizes");
    let wraps: Vec<&crate::hir::HirFunction> = functions(&project)
        .into_iter()
        .filter(|f| f.name.starts_with("wrap$"))
        .collect();
    let params: Vec<String> = wraps
        .iter()
        .map(|f| f.params[0].type_.name().into_owned())
        .collect();
    assert_eq!(wraps.len(), 2, "{params:?}");
    // `wrap([1, 2])` binds `T := List OF Integer` in the first template and
    // `T := Integer` in the second; the second is more specific.
    assert!(params.contains(&"Integer".to_string()), "{params:?}");
    assert!(
        params.contains(&"List OF Integer".to_string()),
        "{params:?}"
    );
    let returns: Vec<String> = wraps.iter().map(|f| format!("{:?}", f.body)).collect();
    assert!(
        returns.iter().any(|b| b.contains("\"2\"")),
        "the List template's body must be instantiated: {returns:?}"
    );
}

#[test]
fn templates_that_bind_a_call_equally_are_ambiguous() {
    let src = "\
FUNC two OF (A, B)(x AS A, y AS List OF B) AS Integer
  RETURN 1
END FUNC
FUNC two OF (A, B)(x AS List OF A, y AS B) AS Integer
  RETURN 2
END FUNC
FUNC main() AS Integer
  RETURN two([1], [2])
END FUNC
";
    let (result, diagnostics) = diagnose(src);
    assert!(result.is_err());
    assert_eq!(rules(&diagnostics), ["TYPE_OVERLOAD_AMBIGUOUS"]);
    assert!(
        diagnostics[0]
            .1
            .contains("Call to `two` matches 2 template overloads equally"),
        "{}",
        diagnostics[0].1
    );
}

#[test]
fn a_call_no_template_binds_names_the_template_count() {
    let src = "\
FUNC pair OF T(x AS T, y AS T) AS Integer
  RETURN 1
END FUNC
FUNC pair OF T(x AS List OF T, y AS T) AS Integer
  RETURN 2
END FUNC
FUNC main() AS Integer
  RETURN pair(1, 2, 3) + pair(1)
END FUNC
";
    let (result, diagnostics) = diagnose(src);
    assert!(result.is_err());
    // Too many arguments for both, then too few (a missing parameter without
    // a default disqualifies a template of an overloaded name).
    assert_eq!(
        rules(&diagnostics),
        ["TYPE_CALL_ARGUMENT_MISMATCH", "TYPE_CALL_ARGUMENT_MISMATCH"]
    );
    for (_, detail) in &diagnostics {
        assert!(
            detail.contains("Call to `pair` matches none of its 2 template overloads"),
            "{detail}"
        );
    }
}

/// bug-196 with a positional argument ahead of named ones: the positional fills
/// the first free slot, the named ones their own, and the type parameters bind
/// in declared order.
#[test]
fn a_mixed_positional_and_named_call_binds_in_declared_order() {
    let src = "\
FUNC f OF (A, B, C)(x AS A, y AS B, z AS C) AS A
  RETURN x
END FUNC
FUNC main() AS Integer
  LET r = f(1, z := \"s\", y := TRUE)
  RETURN r
END FUNC
";
    let (result, diagnostics) = diagnose(src);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let names = function_names(&result.expect("monomorphizes"));
    assert!(
        names.iter().any(|n| n == "f$Integer$Boolean$String"),
        "A, B, C bind Integer, Boolean, String: {names:?}"
    );
}

/// bug-399: a generic whose every self-call widens its argument descends
/// without bound; the depth cap reports once and halts the enumeration.
#[test]
fn a_self_widening_generic_stops_at_the_depth_cap() {
    let src = "\
FUNC deep OF T(x AS T, n AS Integer) AS Integer
  IF n = 0 THEN
    RETURN 0
  END IF
  RETURN deep([x], n - 1)
END FUNC
FUNC main() AS Integer
  RETURN deep(1, 3)
END FUNC
";
    let (result, diagnostics) = diagnose(src);
    assert!(result.is_err());
    assert_eq!(rules(&diagnostics), ["TYPE_INSTANTIATION_TOO_DEEP"]);
    assert!(
        diagnostics[0]
            .1
            .contains("Template instantiation of `deep` exceeds the 256 level limit"),
        "{}",
        diagnostics[0].1
    );
}

/// The type-template twin of the depth cap: a generic record whose field
/// names the same template at a wider argument.
#[test]
fn a_self_widening_generic_type_stops_at_the_depth_cap() {
    let src = "\
TYPE Tower OF T
  here AS T
  above AS List OF Tower OF List OF T
END TYPE
FUNC main() AS Integer
  MUT t AS Tower OF Integer
  RETURN 0
END FUNC
";
    let (result, diagnostics) = diagnose(src);
    assert!(result.is_err());
    assert_eq!(rules(&diagnostics), ["TYPE_INSTANTIATION_TOO_DEEP"]);
    assert!(
        diagnostics[0]
            .1
            .contains("Template instantiation of `Tower` exceeds the 256 level limit"),
        "{}",
        diagnostics[0].1
    );
}

/// Past the instantiation limit, and for a name with no type template, a type
/// instantiation answers the mangled nominal and records no declaration.
#[test]
fn a_type_instantiation_past_the_limit_or_without_a_template_records_nothing() {
    let ast = project(&[(
        "src/main.mfb",
        "TYPE Box OF T\n  value AS T\nEND TYPE\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    )]);
    let dir = std::env::temp_dir();
    let hir = crate::hir::elaborate(&ast);
    let mut mono = Monomorphizer::new(&dir, &hir);

    let none = mono.instantiate_type("NoSuchTemplate", &[ParameterType::Integer]);
    assert!(!mono.concrete_types.contains_key(&none));

    mono.instantiation_limit_reached = true;
    let boxed = mono.instantiate_type("Box", &[ParameterType::String]);
    assert_eq!(boxed.name(), "Box$String");
    assert!(
        !mono.concrete_types.contains_key(&boxed),
        "no instantiation is admitted once the limit has latched"
    );
}

/// bug-226: three distinct argument tuples that mangle alike get three
/// symbols, and re-instantiating the third returns the symbol it already has.
#[test]
fn a_third_mangle_collision_takes_the_next_free_suffix_and_keeps_it() {
    let ast = project(&[(
        "src/main.mfb",
        "TYPE Box OF T\n  value AS T\nEND TYPE\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    )]);
    let dir = std::env::temp_dir();
    let hir = crate::hir::elaborate(&ast);
    let mut mono = Monomorphizer::new(&dir, &hir);
    let spellings = [
        "FUNC(Integer) AS Nothing",
        "FUNC{Integer} AS Nothing",
        "FUNC[Integer] AS Nothing",
    ];
    let mangled: Vec<String> = spellings
        .iter()
        .map(|s| mangle_name("Box", &[s.to_string()]))
        .collect();
    assert!(
        mangled.iter().all(|m| *m == mangled[0]),
        "the three spellings must mangle-collide for this test to mean anything: {mangled:?}"
    );
    let symbols: Vec<String> = spellings
        .iter()
        .map(|s| {
            mono.instantiate_type("Box", &[ParameterType::parse(s)])
                .name()
                .into_owned()
        })
        .collect();
    assert_eq!(
        symbols,
        [
            mangled[0].clone(),
            format!("{}$2", mangled[0]),
            format!("{}$3", mangled[0])
        ]
    );
    let again = mono
        .instantiate_type("Box", &[ParameterType::parse(spellings[2])])
        .name()
        .into_owned();
    assert_eq!(again, symbols[2]);
}
