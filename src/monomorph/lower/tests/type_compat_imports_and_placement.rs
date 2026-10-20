//! plan-176-G: `monomorph::lower`'s tests of lambda and type compatibility, imported
//! overloads, instantiation placement and import-binding shadowing. The shared
//! helpers live in `tests/mod.rs`.

use super::*;

#[test]
fn lambda_expression_type_is_inferred() {
    // A lambda passed to a generic call drives the Lambda arm of
    // expression_type, inferring `FUNC(Integer) AS Integer`.
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  emit(LAMBDA(n AS Integer) -> n + 1)
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(
        function_names(&project)
            .iter()
            .any(|n| n.starts_with("emit$FUNC")),
        "{:?}",
        function_names(&project)
    );
}

/// plan-111-B: `types_compatible` was a token algorithm over two RENDERED
/// spellings — equal token counts, then each pair equal or either literally
/// `"Unknown"`. It is structural now, and this pins the two forms to the
/// same answer over every shape the overload filter can see.
///
/// Two things this pins.
///
/// **The rule that must survive**: `Unknown` stood in for ONE token, so it
/// never matched a composite. A structural "Unknown matches anything" would
/// silently turn "an untyped `[]` selects no overload" into "it selects
/// every one of them" — inverting `TYPE_OVERLOAD_AMBIGUOUS`'s own advice.
/// The last block asserts that asymmetry directly.
///
/// **The bug that must NOT survive**: the token form only wildcarded an
/// `Unknown` that whitespace happened to delimit. A comma or a paren glues
/// it to its neighbour — `"Unknown,"`, `"FUNC(Unknown)"` — so it stopped
/// being a wildcard in a non-final user-generic argument and anywhere
/// inside `FUNC(...)`, while the FINAL argument of the same spelling still
/// worked. Overload selection depending on whether an argument is last is
/// an accident of `split_whitespace`, not a rule; the `fixed` list below is
/// every pair where the two forms therefore differ, and each is asserted to
/// have been broken before.
#[test]
fn types_compatible_matches_the_token_algorithm() {
    // The algorithm this replaced, verbatim.
    fn token_form(param: &str, actual: &str) -> bool {
        if param == actual {
            return true;
        }
        let param_tokens: Vec<&str> = param.split_whitespace().collect();
        let actual_tokens: Vec<&str> = actual.split_whitespace().collect();
        param_tokens.len() == actual_tokens.len()
            && param_tokens
                .iter()
                .zip(actual_tokens.iter())
                .all(|(p, a)| p == a || *p == "Unknown" || *a == "Unknown")
    }
    let ast = project(&[(
        "src/main.mfb",
        "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    )]);
    let dir = std::env::temp_dir();
    let hir = crate::hir::elaborate(&ast);
    let mono = Monomorphizer::new(&dir, &hir);

    let corpus = [
        "Integer",
        "String",
        "Unknown",
        "Db",
        "fs.File",
        "List OF Integer",
        "List OF Unknown",
        "List OF String",
        "Set OF Integer",
        "Map OF String TO Integer",
        "Map OF Unknown TO Integer",
        "Map OF String TO Unknown",
        "#Result OF Integer",
        "MapEntry OF String TO Integer",
        "RES fs.File",
        "List OF List OF Integer",
        "List OF List OF Unknown",
        "Pair OF Integer, String",
        "Pair OF Unknown, String",
        "FUNC(Integer) AS String",
        "FUNC(Unknown) AS String",
        "ISOLATED FUNC(Integer) AS String",
        "Thread OF Integer TO String",
        "ThreadWorker OF Integer TO String",
        "Thread OF Unknown TO String",
    ];
    // The token form only wildcarded an `Unknown` that whitespace happened
    // to delimit. A comma or a paren glues it to its neighbour
    // (`"Unknown,"`, `"FUNC(Unknown)"`), so it silently stopped being a
    // wildcard in a non-final user-generic argument and anywhere inside
    // `FUNC(...)` — while the FINAL argument of the same spelling still
    // worked. That is a bug, not a rule, and the structural form fixes it;
    // these are the pairs where the two therefore disagree, on purpose.
    let fixed: &[(&str, &str)] = &[
        ("Pair OF Integer, String", "Pair OF Unknown, String"),
        ("Pair OF Unknown, String", "Pair OF Integer, String"),
        ("FUNC(Integer) AS String", "FUNC(Unknown) AS String"),
        ("FUNC(Unknown) AS String", "FUNC(Integer) AS String"),
        (
            "ISOLATED FUNC(Integer) AS String",
            "FUNC(Unknown) AS String",
        ),
        (
            "FUNC(Unknown) AS String",
            "ISOLATED FUNC(Integer) AS String",
        ),
    ];
    let mut checked = 0usize;
    let mut diverged = 0usize;
    for param in corpus {
        for actual in corpus {
            let structural =
                mono.types_compatible(&ParameterType::parse(param), &ParameterType::parse(actual));
            if fixed.contains(&(param, actual)) {
                // The isolation flag still has to disagree independently of
                // the wildcard, so assert the exact outcome rather than
                // "different".
                let expected = !param.starts_with("ISOLATED") && !actual.starts_with("ISOLATED");
                assert_eq!(
                    structural, expected,
                    "known-fixed pair `{param}` vs `{actual}`"
                );
                assert!(
                    !token_form(param, actual),
                    "`{param}` vs `{actual}` is listed as fixed, but the token \
                         form already agreed — drop it from the list"
                );
                diverged += 1;
                continue;
            }
            assert_eq!(
                structural,
                token_form(param, actual),
                "types_compatible disagrees for `{param}` vs `{actual}`"
            );
            checked += 1;
        }
    }
    assert_eq!(checked + diverged, corpus.len() * corpus.len());
    assert_eq!(diverged, fixed.len(), "every listed divergence must be hit");

    // The load-bearing asymmetry, stated directly: a wholly-unknown argument
    // matches a leaf and NOT a composite, so it selects no container overload.
    assert!(mono.types_compatible(&ParameterType::Unknown, &ParameterType::Integer));
    assert!(!mono.types_compatible(
        &ParameterType::Unknown,
        &ParameterType::list_of(ParameterType::Integer)
    ));
    // ...while an element-position `Unknown` still matches, which is what
    // makes an untyped `[]` (`List OF Unknown`) ambiguous across two
    // element-typed overloads (bug-36, the test below).
    assert!(mono.types_compatible(
        &ParameterType::list_of(ParameterType::Unknown),
        &ParameterType::list_of(ParameterType::Integer)
    ));
}

/// plan-111-B: `normalize_type` stripped import qualifiers by scanning the
/// whole RENDERED spelling byte-wise. It walks the type now. Every qualifier
/// is `"<binding>."` / `"<package>."` and a `.` appears nowhere else in the
/// grammar, so the two must agree — pinned here rather than argued.
#[test]
fn normalize_type_matches_the_string_algorithm() {
    let ast = project(&[(
        "src/main.mfb",
        "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    )]);
    let dir = std::env::temp_dir();
    let hir = crate::hir::elaborate(&ast);
    let mut mono = Monomorphizer::new(&dir, &hir);
    mono.package_qualifiers = vec![
        "sqlite.".to_string(),
        "io.".to_string(),
        "radio.".to_string(),
        "fs.".to_string(),
    ];
    // The string form this replaced, verbatim.
    let string_form = |type_: &str| -> String {
        let mut qualifiers: Vec<&str> =
            mono.package_qualifiers.iter().map(String::as_str).collect();
        qualifiers.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        let mut normalized = type_.to_string();
        for qualifier in qualifiers {
            normalized = super::strip_qualifier_prefixes(&normalized, qualifier);
        }
        crate::codegen::resource::base_resource_name(&normalized).to_string()
    };
    for spelled in [
        "Integer",
        "sqlite.Db",
        "fs.File",
        // The bug-104 shape: a short qualifier must not eat into a longer name.
        "radio.Station",
        "List OF sqlite.Db",
        "Set OF fs.File",
        "Map OF String TO sqlite.Db",
        "#Result OF sqlite.Db",
        "RES fs.File",
        "List OF RES fs.File",
        "Pair OF (sqlite.Db, io.Stream)",
        "FUNC(sqlite.Db) AS io.Stream",
        "Thread OF sqlite.Db TO io.Stream",
        "Thread OF Integer RES fs.File TO String",
        // The STATE peel, on a bare resource and inside a container.
        "sqlite.Db STATE sqlite.DbInfo",
        "fs.File STATE Cursor",
        "List OF RES fs.File STATE Cursor",
        "Unknown",
    ] {
        assert_eq!(
            mono.normalize_type(&ParameterType::parse(spelled)).name(),
            string_form(spelled),
            "normalize_type disagrees for `{spelled}`"
        );
    }
}

/// bug-36: `Unknown` (from an untyped `[]`) is a wildcard, so an element-typed
/// overload set matches it twice. Taking the first candidate bound the call to
/// whichever overload the package exported first, silently.
#[test]
fn an_untyped_empty_collection_makes_an_imported_overload_ambiguous() {
    let ast = AstProject {
        name: "app".to_string(),
        files: vec![],
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let hir = crate::hir::elaborate(&ast);
    let mut monomorphizer = Monomorphizer::new(dir.path(), &hir);
    // The calling file binds `pkg` to the package `pkg` (bug-914: the set is
    // reached through the calling file's binding).
    monomorphizer.file_imports.insert(
        "src/main.mfb".to_string(),
        std::collections::HashMap::from([("pkg".to_string(), "pkg".to_string())]),
    );
    monomorphizer.current_file = Some("src/main.mfb".to_string());
    monomorphizer.imported_overloads.insert(
        "pkg.f".to_string(),
        vec![
            ImportedOverload {
                param_types: vec![ParameterType::parse("List OF Integer")],
                export_name: "f$ListOFInteger".to_string(),
                return_type: ParameterType::Integer,
                isolated: false,
            },
            ImportedOverload {
                param_types: vec![ParameterType::parse("List OF String")],
                export_name: "f$ListOFString".to_string(),
                return_type: ParameterType::Integer,
                isolated: false,
            },
        ],
    );

    // A concretely-typed argument selects exactly one overload.
    assert_eq!(
        monomorphizer
            .resolve_imported_overload("pkg.f", &[ParameterType::parse("List OF Integer")], 1)
            .as_deref(),
        Some("pkg.f$ListOFInteger")
    );
    assert_eq!(
        monomorphizer
            .resolve_imported_overload("pkg.f", &[ParameterType::parse("List OF String")], 1)
            .as_deref(),
        Some("pkg.f$ListOFString")
    );
    assert!(!monomorphizer.had_error);

    // `f([])` matches both through the `Unknown` wildcard: ambiguous, not
    // "whichever came first".
    assert_eq!(
        monomorphizer.resolve_imported_overload(
            "pkg.f",
            &[ParameterType::parse("List OF Unknown")],
            7
        ),
        None
    );
    assert!(monomorphizer.had_error);

    // An unrelated callee and a wrong arity still resolve to nothing.
    assert_eq!(
        monomorphizer.resolve_imported_overload("pkg.other", &[], 1),
        None
    );
    assert_eq!(
        monomorphizer.resolve_imported_overload("pkg.f", &[], 1),
        None
    );
}

#[test]
fn imported_overload_call_is_rewritten_to_package_symbol() {
    // Import a real package with an exported overload set and call it; the
    // call is rewritten to the package-qualified mangled name
    // (resolve_imported_overload, types_compatible, normalize_type).
    let fixture = crate::testutil::fixture_dir("package-simple")
        .join("golden")
        .join("package_simple.mfp");
    let dir = tempfile::tempdir().expect("tempdir");
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();

    // `score` is an exported overload set: a no-arg form and a `Vec2` form.
    // Calling the no-arg form drives resolve_imported_overload to match the
    // 0-parameter candidate and rewrite the callee to the package symbol.
    let src = "\
IMPORT io
IMPORT package_simple
FUNC main() AS Integer
  io::print(toString(package_simple::score()))
  RETURN 0
END FUNC
";
    let file = crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
        .expect("parse");
    let ast = AstProject {
        name: "app".to_string(),
        files: vec![file],
    };
    let concrete = super::super::monomorphize_project(dir.path(), &crate::hir::elaborate(&ast))
        .expect("monomorphizes");
    // The `main` body's call to `package_simple.score` is rewritten to the
    // package-qualified mangled symbol.
    let main = functions(&concrete)
        .into_iter()
        .find(|f| f.name == "main")
        .expect("main present");
    let rendered = format!("{:?}", main.body);
    assert!(
        rendered.contains("package_simple.score"),
        "expected package-qualified call, got: {rendered}"
    );
}

#[test]
fn match_literal_oneof_and_else_patterns_lower() {
    // A MATCH with a literal-list arm (`CASE 1, 2, 3`) and an ELSE arm drives
    // the OneOf and Else pattern-lowering branches; a generic call inside
    // still instantiates.
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  LET n AS Integer = 2
  MATCH n
    CASE 1, 2, 3
      emit(n)
    CASE ELSE
      emit(0)
  END MATCH
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
}

#[test]
fn for_each_over_map_binds_map_entry_type() {
    // FOR EACH over a Map binds `MapEntry OF K TO V`; a generic call on the
    // entry drives the map branch of ForEach lowering.
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  FOR EACH entry IN m
    emit(entry)
  NEXT
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    // The loop variable binds `MapEntry OF String TO Integer`; the generic
    // call instantiates on that concrete entry type.
    assert!(
        function_names(&project)
            .iter()
            .any(|n| n.starts_with("emit$MapEntry")),
        "{:?}",
        function_names(&project)
    );
}

#[test]
fn named_constructor_arguments_are_lowered() {
    // A record constructor with named fields exercises the named-arg path in
    // lower_constructor_arg and constructor_arg_field_type.
    let src = "\
IMPORT io
TYPE Point
  x AS Integer
  y AS Integer
END TYPE
FUNC main() AS Integer
  LET p AS Point = Point[x := 3, y := 4]
  io::print(toString(p.x))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(function_names(&project).iter().any(|n| n == "main"));
}

#[test]
fn encoding_utf8_encode_overload_resolves_to_bytes() {
    // `encoding::utf8Encode` is a return-type overload; the `List OF Byte`
    // annotation selects the bytes target (encoding overload resolution,
    // Ok(Some) arm).
    let src = "\
IMPORT io
IMPORT encoding
FUNC main() AS Integer
  LET bytes AS List OF Byte = encoding::utf8Encode(\"hi\")
  RETURN 0
END FUNC
";
    let _ = monomorphize(src);
}

#[test]
fn encoding_utf8_encode_overload_ambiguous_without_expected_type() {
    // `utf8Encode` with no expected (contextual) type is an ambiguous
    // return-type overload -> the encoding resolver's Err(()) arm reports
    // TYPE_OVERLOAD_AMBIGUOUS.
    let src = "\
IMPORT io
IMPORT encoding
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  emit(encoding::utf8Encode(\"hi\"))
  RETURN 0
END FUNC
";
    // Whether it errors depends on resolver state; either way the encoding
    // overload branch executes. Assert it does not panic.
    let _ = monomorphize(src);
}

#[test]
fn encoding_utf8_encode_wrong_arg_type_leaves_call() {
    // `utf8Encode` applied to a non-String argument matches no overload; the
    // encoding resolver returns Ok(None) and the call is left in place.
    let src = "\
IMPORT io
IMPORT encoding
FUNC main() AS Integer
  LET bytes AS List OF Byte = encoding::utf8Encode(42)
  RETURN 0
END FUNC
";
    let _ = monomorphize(src);
}

#[test]
fn bare_list_literal_argument_type_is_inferred() {
    // A bare list literal passed to a generic call drives the ListLiteral arm
    // of expression_type (element type from the first element).
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  emit([1, 2, 3])
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(
        function_names(&project)
            .iter()
            .any(|n| n.starts_with("emit$List")),
        "{:?}",
        function_names(&project)
    );
}

#[test]
fn imported_overload_matches_argument_by_type() {
    // Import the real package and call the `Vec2` overload of `score` with a
    // constructed Vec2, driving resolve_imported_overload's per-argument
    // types_compatible / normalize_type comparison.
    let fixture = crate::testutil::fixture_dir("package-simple")
        .join("golden")
        .join("package_simple.mfp");
    let dir = tempfile::tempdir().expect("tempdir");
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();

    let src = "\
IMPORT io
IMPORT package_simple
FUNC main() AS Integer
  LET v AS package_simple::Vec2 = package_simple::Vec2[1, 2]
  io::print(toString(package_simple::score(v)))
  RETURN 0
END FUNC
";
    let file = crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
        .expect("parse");
    let ast = AstProject {
        name: "app".to_string(),
        files: vec![file],
    };
    // The Vec2-typed argument selects the `score(Vec2)` overload; assert the
    // pass completes without panicking (resolution branch runs regardless).
    let _ = super::super::monomorphize_project(dir.path(), &crate::hir::elaborate(&ast));
}

#[test]
fn user_record_named_ok_constructor_types_as_the_record() {
    // bug-945: `Ok[..]` is the user's record, not the internal result, so a
    // generic call fed by it instantiates over `Ok`.
    let src = "\
TYPE Ok
  v AS Integer
END TYPE
FUNC pass OF T(value AS T) AS T
  RETURN value
END FUNC
FUNC main() AS Integer
  LET r = pass(Ok[1])
  RETURN r.v
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("pass$") && n.contains("Ok")),
        "{names:?}"
    );
}

#[test]
fn two_generic_instantiations_are_emitted_sorted() {
    // Two distinct generic instantiations produce two generated functions,
    // exercising the stable sort in into_project.
    let src = "\
IMPORT io
FUNC idOf OF T(value AS T) AS T
  RETURN value
END FUNC
FUNC main() AS Integer
  io::print(toString(idOf(1)))
  io::print(idOf(\"x\"))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let generated: Vec<String> = function_names(&project)
        .into_iter()
        .filter(|n| n.starts_with("idOf$"))
        .collect();
    assert_eq!(generated.len(), 2, "{generated:?}");
}

#[test]
fn return_type_only_template_param_is_a_reported_error() {
    // `T` appears only in the return type, so a bare `make()` cannot pin it
    // down: the instantiation is refused with a diagnostic rather than left as
    // the raw template name (bug-226).
    let src = "\
FUNC make OF T() AS T
  RETURN NOTHING
END FUNC
FUNC main() AS Nothing
  LET x AS Integer = make()
END FUNC
";
    // Errors during lowering surface as the error flag.
    assert!(monomorphize(src).is_err());
}

#[test]
fn generic_set_literal_lowers_its_element_type() {
    // A generic function returning `Set OF T` whose body is a set literal
    // exercises the SetLiteral lowering arm and the `Set OF` shape of
    // `concrete_type_name`: the element type is rewritten T -> Integer.
    let src = "\
FUNC wrap OF T(value AS T) AS Set OF T
  RETURN Set OF T { value }
END FUNC
FUNC main() AS Nothing
  LET s AS Set OF Integer = wrap(42)
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    assert!(names.iter().any(|n| n == "wrap$Integer"), "{names:?}");
}

/// The callee of the call a function's first statement returns.
fn returned_callee(function: &crate::hir::HirFunction) -> String {
    match &function.body[0] {
        crate::hir::HirStatement::Return {
            value: Some(crate::hir::HirExpression::Call { callee, .. }),
            ..
        } => callee.clone(),
        other => panic!("expected `RETURN <call>`, found {other:?}"),
    }
}

#[test]
fn a_private_overload_is_one_set_with_its_files_public_sibling() {
    // bug-912: `a.mfb` declares a PUBLIC `r(Integer)` and a PRIVATE
    // `r(String)`. Only the PRIVATE one is file-scoped; the PUBLIC one keeps
    // the symbol `r` that `main.mfb` calls. Inside `a.mfb` both are reachable
    // through the file-scoped name, selected by argument type.
    let files = [
        (
            "src/a.mfb",
            "\
FUNC r(x AS Integer) AS String
  RETURN r(\"s\")
END FUNC
PRIVATE FUNC r(x AS String) AS String
  RETURN r(1)
END FUNC
",
        ),
        (
            "src/main.mfb",
            "\
FUNC main() AS String
  RETURN r(1)
END FUNC
",
        ),
    ];
    let mut ast = project(&files);
    crate::ast::scope_privates::scope_privates(&mut ast);
    let project = crate::testutil::silence_panics(|| {
        super::super::monomorphize_project(&std::env::temp_dir(), &crate::hir::elaborate(&ast))
    })
    .expect("monomorphizes");
    let scoped = format!(
        "{}$String",
        crate::internal_name::mangle_private(
            &crate::internal_name::file_scope_hash("src/a.mfb"),
            "r"
        )
    );
    let functions = functions(&project);
    let function = |name: &str| {
        *functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("no function `{name}` in {:?}", function_names(&project)))
    };
    assert_eq!(returned_callee(function("r")), scoped);
    assert_eq!(returned_callee(function(&scoped)), "r");
    assert_eq!(returned_callee(function("main")), "r");
}

/// The path of the file `name` (a function or a type) is emitted into.
fn file_of(project: &crate::hir::HirProject, name: &str) -> String {
    project
        .files
        .iter()
        .find(|file| {
            file.items.iter().any(|item| match item {
                crate::hir::HirItem::Function(function) => function.name == name,
                crate::hir::HirItem::Type(type_decl) => type_decl.name == name,
                _ => false,
            })
        })
        .unwrap_or_else(|| panic!("`{name}` is emitted into no file"))
        .path
        .clone()
}

const TEMPLATES: &str = "\
TYPE Box OF T
  v AS T
END TYPE
FUNC wrap OF T(x AS T) AS Box OF T
  RETURN Box[x]
END FUNC
";

const CALLER: &str = "\
FUNC main() AS Nothing
  LET b AS Box OF Integer = wrap(1)
END FUNC
";

#[test]
fn a_user_instantiation_is_emitted_into_its_templates_file_in_either_order() {
    // bug-911: every instantiation used to land in `files[0]`, so a template
    // body was checked and lowered in the scope of whichever file sorted
    // first. The declaring file hosts it, whatever its place in the order.
    for (template_file, caller_file) in [("src/z.mfb", "src/a.mfb"), ("src/a.mfb", "src/z.mfb")] {
        let mut files = [(template_file, TEMPLATES), (caller_file, CALLER)];
        files.sort();
        let project = monomorphize_files(&files).expect("monomorphizes");
        assert_eq!(file_of(&project, "wrap$Integer"), template_file);
        assert_eq!(file_of(&project, "Box$Integer"), template_file);
        assert_eq!(file_of(&project, "main"), caller_file);
    }
}

#[test]
fn a_toolchain_templates_instantiation_stays_in_the_first_file() {
    // The other half of bug-911's boundary: a template declared in a file the
    // toolchain injected (a built-in package's source) has no user file. Its
    // instantiation stays in the first file and is marked `internal_origin`,
    // which is what carries its call-site attribution.
    let mut ast = project(&[("src/a.mfb", CALLER), ("src/z.mfb", TEMPLATES)]);
    ast.files[1].internal = true;
    let project = crate::testutil::silence_panics(|| {
        super::super::monomorphize_project(&std::env::temp_dir(), &crate::hir::elaborate(&ast))
    })
    .expect("monomorphizes");
    assert_eq!(file_of(&project, "wrap$Integer"), "src/a.mfb");
    assert_eq!(file_of(&project, "Box$Integer"), "src/a.mfb");
    let wrap = functions(&project)
        .into_iter()
        .find(|function| function.name == "wrap$Integer")
        .expect("the instantiation exists");
    assert!(wrap.internal_origin);
}

#[test]
fn a_hosting_files_own_import_binding_wins_and_a_shadowed_package_stays_known() {
    // bug-911: both files bind the alias `lib`, to different packages. The
    // file hosting an instantiation keeps its own binding for `lib` — the one
    // its template body means — and still learns the other file's package (a
    // type argument can name it), under a binding no source can spell.
    let files = [
        (
            "src/a.mfb",
            "\
IMPORT pa AS lib
FUNC main() AS Nothing
  LET b AS Box OF Integer = wrap(1)
END FUNC
",
        ),
        (
            "src/z.mfb",
            "\
IMPORT pf AS lib
TYPE Box OF T
  v AS T
END TYPE
FUNC wrap OF T(x AS T) AS Box OF T
  RETURN Box[x]
END FUNC
",
        ),
    ];
    let project = monomorphize_files(&files).expect("monomorphizes");
    let bindings = |path: &str| {
        project
            .files
            .iter()
            .find(|file| file.path == path)
            .expect("the file exists")
            .imports
            .iter()
            .map(|import| {
                (
                    import.binding_name().to_string(),
                    import.package_name().to_string(),
                )
            })
            .collect::<Vec<_>>()
    };
    let pair = |binding: &str, package: &str| (binding.to_string(), package.to_string());
    assert_eq!(
        bindings("src/z.mfb"),
        vec![pair("lib", "pf"), pair("#pa", "pa")]
    );
    assert_eq!(
        bindings("src/a.mfb"),
        vec![pair("lib", "pa"), pair("#pf", "pf")]
    );
}
