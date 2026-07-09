use super::*;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn glob_patterns_match_nested_and_root_files() {
    assert!(glob_matches("**/*.mfb", "main.mfb"));
    assert!(glob_matches("**/*.mfb", "pkg/main.mfb"));
    assert!(glob_matches("pkg/*.mfb", "pkg/main.mfb"));
    assert!(!glob_matches("pkg/*.mfb", "pkg/nested/main.mfb"));
    assert!(glob_matches("**/*_test.mfb", "pkg/math_test.mfb"));
    assert!(!glob_matches("**/*_test.mfb", "pkg/math.mfb"));
}

#[test]
fn parse_import_aliases() {
    let file = parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "IMPORT io AS term\nIMPORT math\n",
    )
    .expect("parse source");

    assert_eq!(file.imports.len(), 2);
    assert_eq!(file.imports[0].module, "io");
    assert_eq!(file.imports[0].alias.as_deref(), Some("term"));
    assert_eq!(file.imports[0].binding_name(), "term");
    assert_eq!(file.imports[0].package_name(), "io");
    assert_eq!(file.imports[1].module, "math");
    assert_eq!(file.imports[1].alias, None);
    assert_eq!(file.imports[1].binding_name(), "math");
    assert_eq!(file.imports[1].package_name(), "math");
}

#[test]
fn string_concat_has_lower_precedence_than_addition() {
    let file = parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "FUNC main AS String\n  RETURN a & b + c\nEND FUNC\n",
    )
    .expect("parse source");

    let Item::Function(function) = &file.items[0] else {
        panic!("expected function item");
    };
    let Statement::Return {
        value: Some(expression),
        ..
    } = &function.body[0]
    else {
        panic!("expected return expression");
    };

    let Expression::Binary {
        left,
        operator,
        right,
        ..
    } = expression
    else {
        panic!("expected binary expression");
    };
    assert_eq!(operator, "&");
    assert!(matches!(&**left, Expression::Identifier(name) if name == "a"));

    let Expression::Binary {
        left: add_left,
        operator: add_operator,
        right: add_right,
        ..
    } = &**right
    else {
        panic!("expected addition on concat right side");
    };
    assert_eq!(add_operator, "+");
    assert!(matches!(&**add_left, Expression::Identifier(name) if name == "b"));
    assert!(matches!(&**add_right, Expression::Identifier(name) if name == "c"));
}

#[test]
fn file_root_ignores_include_patterns() {
    let root = test_temp_dir("file_root_ignores_include_patterns");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("project src");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    fs::write(project_dir.join("src/other.mfb"), "SUB other\nEND SUB\n").expect("write other");

    let manifest = manifest_with_sources(vec![source_entry(
        "src/main.mfb",
        Some(vec!["missing/**/*.mfb"]),
        None,
    )]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");
    let files = collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest)
        .expect("files");

    assert_eq!(
        files,
        vec![SelectedSource {
            actual_path: canonical_project_dir.join("src/main.mfb"),
            display_path: project_dir.join("src/main.mfb"),
        }]
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn directory_root_applies_include_and_exclude_patterns() {
    let root = test_temp_dir("directory_root_applies_include_and_exclude_patterns");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src/pkg")).expect("project pkg");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    fs::write(project_dir.join("src/pkg/keep.mfb"), "SUB keep\nEND SUB\n").expect("write keep");
    fs::write(
        project_dir.join("src/pkg/skip_test.mfb"),
        "SUB skip_test\nEND SUB\n",
    )
    .expect("write skip");

    let manifest = manifest_with_sources(vec![source_entry(
        "src",
        Some(vec!["pkg/**/*.mfb"]),
        Some(vec!["**/*_test.mfb"]),
    )]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");
    let files = collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest)
        .expect("files");

    assert_eq!(
        files,
        vec![SelectedSource {
            actual_path: canonical_project_dir.join("src/pkg/keep.mfb"),
            display_path: project_dir.join("src/pkg/keep.mfb"),
        }]
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn overlapping_source_entries_are_rejected() {
    let root = test_temp_dir("overlapping_source_entries_are_rejected");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("project src");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");

    let manifest = manifest_with_sources(vec![
        source_entry("src", Some(vec!["**/*.mfb"]), None),
        source_entry("src/main.mfb", None, None),
    ]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");

    assert!(
        collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest).is_err()
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn symlinked_source_paths_must_stay_inside_project() {
    let root = test_temp_dir("symlinked_source_paths_must_stay_inside_project");
    let project_dir = root.join("project");
    let outside_dir = root.join("outside");
    fs::create_dir_all(&project_dir).expect("project dir");
    fs::create_dir_all(&outside_dir).expect("outside dir");
    fs::write(outside_dir.join("escape.mfb"), "SUB escape\nEND SUB\n").expect("write escape");
    symlink(&outside_dir, project_dir.join("src")).expect("symlink src");

    let manifest = manifest_with_sources(vec![source_entry("src", Some(vec!["**/*.mfb"]), None)]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");

    assert!(
        collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest).is_err()
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

// ---------------------------------------------------------------------------
// Expression-parser coverage (src/ast/expr.rs)
// ---------------------------------------------------------------------------

/// Parse a whole program, expecting success.
fn parse_ok(src: &str) -> AstFile {
    parse_source(Path::new("main.mfb"), "main.mfb", src).expect("expected parse to succeed")
}

/// Parse a whole program, expecting a parse error.
fn parse_err(src: &str) {
    assert!(
        parse_source(Path::new("main.mfb"), "main.mfb", src).is_err(),
        "expected parse error for: {src:?}"
    );
}

/// Wrap an expression in a `FUNC main` whose body returns it.
fn ret(expr: &str) -> String {
    format!("FUNC main AS Integer\n  RETURN {expr}\nEND FUNC\n")
}

/// Extract the first return-expression of `main`.
fn first_return_expr(file: &AstFile) -> &Expression {
    let Item::Function(function) = &file.items[0] else {
        panic!("expected a function item");
    };
    let Statement::Return {
        value: Some(expr), ..
    } = &function.body[0]
    else {
        panic!("expected a return with a value");
    };
    expr
}

/// Parse `RETURN <expr>` and return the resulting expression.
fn expr_of(expr: &str) -> Expression {
    let file = parse_ok(&ret(expr));
    first_return_expr(&file).clone()
}

#[test]
fn parses_primary_literal_forms() {
    assert!(matches!(expr_of("\"hi\""), Expression::String(s) if s == "hi"));
    assert!(matches!(expr_of("42"), Expression::Number(n) if n == "42"));
    assert!(matches!(expr_of("TRUE"), Expression::Boolean(true)));
    assert!(matches!(expr_of("FALSE"), Expression::Boolean(false)));
    assert!(matches!(expr_of("NOTHING"), Expression::Identifier(n) if n == "NOTHING"));
    assert!(matches!(expr_of("(1)"), Expression::Number(n) if n == "1"));
    assert!(matches!(expr_of("x"), Expression::Identifier(n) if n == "x"));
}

#[test]
fn parses_list_literal_including_empty() {
    assert!(matches!(expr_of("[]"), Expression::ListLiteral(v) if v.is_empty()));
    let Expression::ListLiteral(values) = expr_of("[1, 2, 3]") else {
        panic!("expected list literal");
    };
    assert_eq!(values.len(), 3);
}

#[test]
fn parses_map_literal() {
    let Expression::MapLiteral {
        key_type,
        value_type,
        entries,
    } = expr_of("Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }")
    else {
        panic!("expected map literal");
    };
    assert_eq!(key_type, "String");
    assert_eq!(value_type, "Integer");
    assert_eq!(entries.len(), 2);
}

#[test]
fn parses_empty_map_literal() {
    let Expression::MapLiteral { entries, .. } = expr_of("Map OF String TO Integer {}") else {
        panic!("expected map literal");
    };
    assert!(entries.is_empty());
}

#[test]
fn parses_map_literal_with_res_value() {
    let Expression::MapLiteral { value_type, .. } =
        expr_of("Map OF String TO RES File { \"a\" := f }")
    else {
        panic!("expected map literal");
    };
    assert_eq!(value_type, "RES File");
}

#[test]
fn map_literal_requires_to_keyword() {
    parse_err(&ret("Map OF String FROM Integer { }"));
}

#[test]
fn map_literal_requires_colon_equal_between_key_and_value() {
    parse_err(&ret("Map OF String TO Integer { \"a\" 1 }"));
}

#[test]
fn parses_operator_precedence_chain() {
    // OR / XOR
    let Expression::Binary { operator, .. } = expr_of("a OR b") else {
        panic!("or");
    };
    assert_eq!(operator, "OR");
    let Expression::Binary { operator, .. } = expr_of("a XOR b") else {
        panic!("xor");
    };
    assert_eq!(operator, "XOR");
    // AND
    let Expression::Binary { operator, .. } = expr_of("a AND b") else {
        panic!("and");
    };
    assert_eq!(operator, "AND");
    // NOT (unary, right-recursive)
    let Expression::Unary { operator, .. } = expr_of("NOT NOT a") else {
        panic!("not");
    };
    assert_eq!(operator, "NOT");
}

#[test]
fn parses_every_comparison_operator() {
    for (src, op) in [
        ("a = b", "="),
        ("a <> b", "<>"),
        ("a < b", "<"),
        ("a <= b", "<="),
        ("a > b", ">"),
        ("a >= b", ">="),
    ] {
        let Expression::Binary { operator, .. } = expr_of(src) else {
            panic!("comparison {src}");
        };
        assert_eq!(operator, op);
    }
}

#[test]
fn parses_arithmetic_and_multiplicative_operators() {
    for (src, op) in [
        ("a + b", "+"),
        ("a - b", "-"),
        ("a * b", "*"),
        ("a / b", "/"),
        ("a MOD b", "MOD"),
        ("a DIV b", "DIV"),
        ("a & b", "&"),
    ] {
        let Expression::Binary { operator, .. } = expr_of(src) else {
            panic!("arith {src}");
        };
        assert_eq!(operator, op);
    }
}

#[test]
fn parses_power_right_associative_and_unary_minus() {
    let Expression::Binary {
        operator, right, ..
    } = expr_of("a ^ b ^ c")
    else {
        panic!("power");
    };
    assert_eq!(operator, "^");
    // Right-associative: right side is itself a power binary.
    assert!(matches!(&*right, Expression::Binary { operator, .. } if operator == "^"));
    let Expression::Unary { operator, .. } = expr_of("-a") else {
        panic!("unary minus");
    };
    assert_eq!(operator, "-");
}

#[test]
fn parses_pipeline_with_placeholder() {
    // `a |> f(_)` substitutes `a` for the placeholder in `f(_)`.
    let expr = expr_of("a |> f(_)");
    let Expression::Call {
        callee, arguments, ..
    } = expr
    else {
        panic!("expected call after pipeline substitution");
    };
    assert_eq!(callee, "f");
    assert_eq!(arguments.len(), 1);
}

#[test]
fn pipeline_without_placeholder_is_rejected() {
    parse_err(&ret("a |> f(b)"));
}

#[test]
fn parses_call_with_positional_and_named_args() {
    let Expression::Call {
        callee, arguments, ..
    } = expr_of("f(1, name := 2)")
    else {
        panic!("expected call");
    };
    assert_eq!(callee, "f");
    assert_eq!(arguments.len(), 2);
    assert!(matches!(arguments[0], CallArg::Positional(_)));
    assert!(matches!(&arguments[1], CallArg::Named { name, .. } if name == "name"));
}

#[test]
fn parses_empty_call() {
    let Expression::Call { arguments, .. } = expr_of("f()") else {
        panic!("expected call");
    };
    assert!(arguments.is_empty());
}

#[test]
fn call_on_non_identifier_is_rejected() {
    // A literal followed by `(` is not a callable identifier.
    parse_err(&ret("\"s\"(1)"));
}

#[test]
fn call_missing_closing_paren_is_rejected() {
    parse_err(&ret("f(1"));
}

#[test]
fn parses_constructor_positional_and_named() {
    let Expression::Constructor {
        type_name,
        arguments,
    } = expr_of("Point[1, y := 2]")
    else {
        panic!("expected constructor");
    };
    assert_eq!(type_name, "Point");
    assert_eq!(arguments.len(), 2);
    assert!(matches!(arguments[0], ConstructorArg::Positional(_)));
    assert!(matches!(&arguments[1], ConstructorArg::Named { name, .. } if name == "y"));
}

#[test]
fn parses_empty_constructor() {
    let Expression::Constructor { arguments, .. } = expr_of("Point[]") else {
        panic!("expected constructor");
    };
    assert!(arguments.is_empty());
}

#[test]
fn constructor_on_non_identifier_is_rejected() {
    parse_err(&ret("\"s\"[1]"));
}

#[test]
fn constructor_missing_closing_bracket_is_rejected() {
    parse_err(&ret("Point[1"));
}

#[test]
fn parses_member_access_chain() {
    let Expression::MemberAccess { target, member } = expr_of("a.b.c") else {
        panic!("expected member access");
    };
    assert_eq!(member, "c");
    assert!(matches!(&*target, Expression::MemberAccess { member, .. } if member == "b"));
}

#[test]
fn member_access_requires_identifier() {
    parse_err(&ret("a.1"));
}

#[test]
fn parses_with_update() {
    let Expression::WithUpdate { target, updates } = expr_of("WITH p { x := 1, y := 2 }") else {
        panic!("expected WITH update");
    };
    assert!(matches!(&*target, Expression::Identifier(n) if n == "p"));
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].field, "x");
}

#[test]
fn parses_with_update_empty() {
    let Expression::WithUpdate { updates, .. } = expr_of("WITH p { }") else {
        panic!("expected WITH update");
    };
    assert!(updates.is_empty());
}

#[test]
fn with_update_requires_open_brace() {
    parse_err(&ret("WITH p x := 1 }"));
}

#[test]
fn with_update_field_must_be_identifier() {
    parse_err(&ret("WITH p { 1 := 2 }"));
}

#[test]
fn with_update_requires_colon_equal() {
    parse_err(&ret("WITH p { x = 1 }"));
}

#[test]
fn with_update_requires_close_brace() {
    parse_err(&ret("WITH p { x := 1 "));
}

#[test]
fn parses_lambda_expression() {
    let Expression::Lambda {
        params,
        body,
        assign_target,
    } = expr_of("LAMBDA(x AS Integer) -> x + 1")
    else {
        panic!("expected lambda");
    };
    assert_eq!(params.len(), 1);
    assert!(assign_target.is_none());
    assert!(matches!(&*body, Expression::Binary { operator, .. } if operator == "+"));
}

#[test]
fn parses_lambda_with_assignment_body() {
    let Expression::Lambda { assign_target, .. } = expr_of("LAMBDA(x AS Integer) -> total = x")
    else {
        panic!("expected lambda");
    };
    assert_eq!(assign_target.as_deref(), Some("total"));
}

#[test]
fn parses_lambda_with_no_params() {
    let Expression::Lambda { params, .. } = expr_of("LAMBDA() -> 1") else {
        panic!("expected lambda");
    };
    assert!(params.is_empty());
}

#[test]
fn lambda_requires_open_paren() {
    parse_err(&ret("LAMBDA x -> 1"));
}

#[test]
fn lambda_requires_close_paren() {
    parse_err(&ret("LAMBDA(x AS Integer -> 1"));
}

#[test]
fn lambda_requires_arrow() {
    parse_err(&ret("LAMBDA(x AS Integer) 1"));
}

#[test]
fn bare_expression_error_is_rejected() {
    // A statement starting with a stray operator produces an "Expected an
    // expression" error.
    parse_err("FUNC main AS Integer\n  RETURN *\nEND FUNC\n");
}

// --- Type-name parsing (reached via `LET x AS <type>`) ---

/// Parse `LET v AS <type> = NOTHING` inside `main` and return the parsed type.
fn type_of(type_name: &str) -> String {
    let src = format!("SUB main\n  LET v AS {type_name} = NOTHING\nEND SUB\n");
    let file = parse_ok(&src);
    let Item::Function(function) = &file.items[0] else {
        panic!("expected function");
    };
    let Statement::Let { type_name, .. } = &function.body[0] else {
        panic!("expected LET");
    };
    type_name.clone().expect("type name present")
}

fn type_err(type_name: &str) {
    let src = format!("SUB main\n  LET v AS {type_name} = NOTHING\nEND SUB\n");
    parse_err(&src);
}

#[test]
fn parses_simple_and_generic_types() {
    assert_eq!(type_of("Integer"), "Integer");
    assert_eq!(type_of("List OF Integer"), "List OF Integer");
    assert_eq!(type_of("List OF RES File"), "List OF RES File");
    assert_eq!(type_of("Result OF Integer"), "Result OF Integer");
    assert_eq!(type_of("(Integer)"), "(Integer)");
    assert_eq!(type_of("Nothing"), "Nothing");
}

#[test]
fn parses_map_type_variants() {
    assert_eq!(
        type_of("Map OF String TO Integer"),
        "Map OF String TO Integer"
    );
    assert_eq!(
        type_of("Map OF String TO RES File"),
        "Map OF String TO RES File"
    );
    assert_eq!(
        type_of("MapEntry OF String TO Integer"),
        "MapEntry OF String TO Integer"
    );
}

#[test]
fn map_type_requires_to_keyword() {
    type_err("Map OF String Integer");
}

#[test]
fn parses_template_type_with_multiple_args() {
    assert_eq!(
        type_of("Pair OF Integer, String"),
        "Pair OF Integer, String"
    );
}

#[test]
fn parses_thread_type_variants() {
    assert_eq!(type_of("Thread OF Msg TO Out"), "Thread OF Msg TO Out");
    assert_eq!(
        type_of("Thread OF Msg RES Handle TO Out"),
        "Thread OF Msg RES Handle TO Out"
    );
    assert_eq!(
        type_of("Thread OF RES Handle TO Out"),
        "Thread OF RES Handle TO Out"
    );
    assert_eq!(
        type_of("ThreadWorker OF Msg TO Out"),
        "ThreadWorker OF Msg TO Out"
    );
}

#[test]
fn thread_type_requires_to_keyword() {
    type_err("Thread OF Msg Out");
}

#[test]
fn parses_function_type_names() {
    assert_eq!(
        type_of("FUNC(Integer, String) AS Boolean"),
        "FUNC(Integer, String) AS Boolean"
    );
    assert_eq!(type_of("FUNC() AS Integer"), "FUNC() AS Integer");
    assert_eq!(
        type_of("ISOLATED FUNC(Integer) AS Integer"),
        "ISOLATED FUNC(Integer) AS Integer"
    );
}

#[test]
fn function_type_requires_open_paren() {
    type_err("FUNC Integer AS Boolean");
}

#[test]
fn function_type_requires_close_paren() {
    type_err("FUNC(Integer AS Boolean");
}

#[test]
fn function_type_requires_as() {
    type_err("FUNC(Integer) Boolean");
}

#[test]
fn isolated_type_requires_func() {
    type_err("ISOLATED Integer");
}

#[test]
fn grouped_type_requires_close_paren() {
    type_err("(Integer");
}

#[test]
fn type_base_name_rejects_non_identifier() {
    type_err("123");
}

#[test]
fn qualified_names_must_have_two_parts() {
    // Three-part qualified name is rejected.
    parse_err(&ret("a::b::c"));
}

#[test]
fn parses_qualified_identifier() {
    // A two-part qualified name normalizes to a dotted identifier.
    let Expression::Identifier(name) = expr_of("math::pi") else {
        panic!("expected identifier");
    };
    assert_eq!(name, "math.pi");
}

// ---------------------------------------------------------------------------
// Project / manifest assembly coverage (src/ast/manifest.rs)
// ---------------------------------------------------------------------------

#[test]
fn parse_project_reads_files_and_appends_prelude() {
    let root = test_temp_dir("parse_project_reads_files_and_appends_prelude");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let project = parse_project("demo", &project_dir, &manifest).expect("parse project");

    assert_eq!(project.name, "demo");
    // The user's source is files[0]; the compiler-owned prelude is appended.
    assert_eq!(project.files[0].path, "src/main.mfb");
    assert!(
        project
            .files
            .iter()
            .any(|file| file.path == BUILTIN_PRELUDE_PATH),
        "prelude file must be appended"
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn parse_project_rejects_missing_project_dir() {
    let missing = std::env::temp_dir().join("mfb_ast_parse_project_missing_dir_zzz_nope");
    let _ = fs::remove_dir_all(&missing);
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(parse_project("demo", &missing, &manifest).is_err());
}

#[test]
fn selected_source_paths_are_sorted_actual_paths() {
    let root = test_temp_dir("selected_source_paths_are_sorted_actual_paths");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(project_dir.join("src/a.mfb"), "SUB a\nEND SUB\n").expect("write a");
    fs::write(project_dir.join("src/b.mfb"), "SUB b\nEND SUB\n").expect("write b");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let paths = selected_source_paths(&project_dir, &manifest).expect("paths");
    assert_eq!(paths.len(), 2);
    assert!(paths[0].ends_with("a.mfb"));
    assert!(paths[1].ends_with("b.mfb"));

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn selected_source_paths_rejects_missing_project_dir() {
    let missing = std::env::temp_dir().join("mfb_ast_selected_paths_missing_zzz_nope");
    let _ = fs::remove_dir_all(&missing);
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(selected_source_paths(&missing, &manifest).is_err());
}

#[test]
fn write_ast_writes_ast_json_file() {
    let root = test_temp_dir("write_ast_writes_ast_json_file");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let project = parse_project("demo", &project_dir, &manifest).expect("parse project");
    let ast_path = write_ast(&project_dir, &project).expect("write ast");
    assert_eq!(ast_path, project_dir.join("demo.ast"));
    let contents = fs::read_to_string(&ast_path).expect("read ast");
    assert!(contents.contains("\"files\""), "{contents}");

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn write_ast_reports_write_failure() {
    let root = test_temp_dir("write_ast_reports_write_failure");
    // Point at a non-existent directory so the write fails.
    let bogus_dir = root.join("does_not_exist");
    let project = AstProject {
        name: "demo".to_string(),
        files: Vec::new(),
    };
    assert!(write_ast(&bogus_dir, &project).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn parse_source_internal_marks_file_internal() {
    let file = parse_source_internal(Path::new("pkg.mfb"), "pkg.mfb", "SUB main\nEND SUB\n")
        .expect("parse internal");
    assert!(file.internal);
    assert_eq!(file.path, "pkg.mfb");
}

#[test]
fn source_root_missing_is_rejected() {
    let root = test_temp_dir("source_root_missing_is_rejected");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project dir");

    let manifest = manifest_with_sources(vec![source_entry("missing_root", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn empty_source_root_is_rejected() {
    let root = test_temp_dir("empty_source_root_is_rejected");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    // No .mfb files under src.
    fs::write(project_dir.join("src/readme.txt"), "not source\n").expect("write txt");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn source_entry_without_root_is_ignored() {
    let root = test_temp_dir("source_entry_without_root_is_ignored");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");

    // One entry has no `root` key (filtered out by source_entries), the other is valid.
    let bad = JsonValue::Object(HashMap::from([(
        "include".to_string(),
        JsonValue::Array(vec![JsonValue::String("**/*.mfb".to_string())]),
    )]));
    let good = source_entry("src", None, None);
    let manifest = manifest_with_sources(vec![bad, good]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 1);

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn file_root_ignores_non_mfb_extension() {
    let root = test_temp_dir("file_root_ignores_non_mfb_extension");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    fs::write(project_dir.join("src/notes.txt"), "notes\n").expect("write notes");

    // A file root pointing at a non-.mfb file selects nothing, so the entry is
    // empty and rejected.
    let manifest = manifest_with_sources(vec![source_entry("src/notes.txt", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn directory_cycle_via_symlink_is_tolerated() {
    let root = test_temp_dir("directory_cycle_via_symlink_is_tolerated");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src/sub")).expect("src/sub");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    // A symlink pointing back to src forms a directory cycle; visited-dir
    // tracking must break it and still return the real file.
    symlink(project_dir.join("src"), project_dir.join("src/sub/loop")).expect("symlink loop");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 1);
    assert!(files[0].actual_path.ends_with("main.mfb"));

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn nested_symlink_escape_is_rejected() {
    let root = test_temp_dir("nested_symlink_escape_is_rejected");
    let project_dir = root.join("project");
    let outside_dir = root.join("outside");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::create_dir_all(&outside_dir).expect("outside dir");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    fs::write(outside_dir.join("escape.mfb"), "SUB escape\nEND SUB\n").expect("write escape");
    // A file symlink inside a scanned directory pointing outside the project.
    symlink(
        outside_dir.join("escape.mfb"),
        project_dir.join("src/escape.mfb"),
    )
    .expect("symlink escape");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn glob_star_and_question_components_match() {
    // Exercise `*` (with backtracking) and `?` single-char wildcards.
    assert!(glob_matches("f*o", "foo"));
    assert!(glob_matches("f*o", "fabco"));
    assert!(glob_matches("a?c", "abc"));
    assert!(!glob_matches("a?c", "ac"));
    assert!(!glob_matches("abc", "abd"));
    // Trailing star matches the remaining suffix (including empty).
    assert!(glob_matches("ab*", "ab"));
    assert!(glob_matches("ab*", "abcdef"));
    // `**` matches across an empty path tail.
    assert!(glob_matches("src/**", "src"));
    // A literal segment that does not match fails immediately.
    assert!(!glob_matches("src/*.mfb", "lib/main.mfb"));
}

#[test]
fn parse_project_propagates_collection_failure() {
    // A missing source root makes collection fail; the error propagates out of
    // `parse_project` (the `?` on `collect_selected_source_files`).
    let root = test_temp_dir("parse_project_propagates_collection_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let manifest = manifest_with_sources(vec![source_entry("missing_root", None, None)]);
    assert!(parse_project("demo", &project_dir, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn selected_source_paths_propagates_collection_failure() {
    let root = test_temp_dir("selected_source_paths_propagates_collection_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let manifest = manifest_with_sources(vec![source_entry("missing_root", None, None)]);
    assert!(selected_source_paths(&project_dir, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn broken_symlink_entry_reports_read_failure() {
    // A dangling symlink with an `.mfb` name exists() as a directory entry but
    // fails to canonicalize, exercising the per-entry canonicalize `?`.
    let root = test_temp_dir("broken_symlink_entry_reports_read_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    symlink(
        project_dir.join("src/nonexistent_target.mfb"),
        project_dir.join("src/dangling.mfb"),
    )
    .expect("symlink dangling");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn parse_source_reports_lex_failure() {
    // A stray control/invalid byte makes the lexer fail before parsing.
    assert!(parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "SUB main\n  LET x = `\nEND SUB\n"
    )
    .is_err());
    assert!(parse_source_internal(
        Path::new("pkg.mfb"),
        "pkg.mfb",
        "SUB main\n  LET x = `\nEND SUB\n"
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn parse_project_reports_unreadable_source_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_temp_dir("parse_project_reports_unreadable_source_file");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    let file = project_dir.join("src/main.mfb");
    fs::write(&file, "SUB main\nEND SUB\n").expect("write main");
    // Strip all read permission so parse_file's read_to_string fails.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("chmod");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let result = parse_project("demo", &project_dir, &manifest);

    // Restore permission before assertions so cleanup can run.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("restore chmod");
    assert!(result.is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn unreadable_directory_reports_read_failure() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_temp_dir("unreadable_directory_reports_read_failure");
    let project_dir = root.join("project");
    let locked = project_dir.join("src/locked");
    fs::create_dir_all(&locked).expect("locked dir");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    fs::write(locked.join("inner.mfb"), "SUB inner\nEND SUB\n").expect("write inner");
    // Remove read/exec permission so read_dir on the nested directory fails
    // with a non-PermissionDenied-mapped error path.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let result = collect_selected_source_files(&project_dir, &canonical, &manifest);

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore chmod");
    assert!(result.is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

fn manifest_with_sources(sources: Vec<JsonValue>) -> HashMap<String, JsonValue> {
    HashMap::from([("sources".to_string(), JsonValue::Array(sources))])
}

fn source_entry(root: &str, include: Option<Vec<&str>>, exclude: Option<Vec<&str>>) -> JsonValue {
    let mut source = HashMap::from([("root".to_string(), JsonValue::String(root.to_string()))]);
    if let Some(include) = include {
        source.insert(
            "include".to_string(),
            JsonValue::Array(
                include
                    .into_iter()
                    .map(|pattern| JsonValue::String(pattern.to_string()))
                    .collect(),
            ),
        );
    }
    if let Some(exclude) = exclude {
        source.insert(
            "exclude".to_string(),
            JsonValue::Array(
                exclude
                    .into_iter()
                    .map(|pattern| JsonValue::String(pattern.to_string()))
                    .collect(),
            ),
        );
    }
    JsonValue::Object(source)
}

fn test_temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("timestamp")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_ast_{name}_{stamp}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp dir");
    root
}

// ---------------------------------------------------------------------------
// plan-12 coverage: source-driven parse + serialize tests.
// ---------------------------------------------------------------------------

/// Parse a single source string into an `AstFile`, panicking on parse error.
fn parse_file(src: &str) -> AstFile {
    parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse source")
}

/// Attempt to parse a source string; `Ok(file)` on success, `Err(())` on a
/// reported diagnostic.
fn try_parse(src: &str) -> Result<AstFile, ()> {
    parse_source(Path::new("main.mfb"), "main.mfb", src)
}

/// Wrap a parsed file in a project and render its `.ast` JSON dump.
fn project_json(src: &str) -> String {
    let file = parse_file(src);
    let project = AstProject {
        name: "demo".to_string(),
        files: vec![file],
    };
    project.to_json()
}

fn function<'a>(file: &'a AstFile, name: &str) -> &'a Function {
    file.items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == name => Some(function),
            _ => None,
        })
        .expect("function item")
}

// ---------------------------------------------------------------------------
// serialize.rs — one rich fixture exercising every AST node kind.
// ---------------------------------------------------------------------------

#[test]
fn serialize_covers_all_item_kinds() {
    let src = r#"
IMPORT io AS term
IMPORT math

EXPORT LET greeting AS String = "hi"
MUT counter AS Integer = 0
RES handle AS File STATE Integer

EXPORT TYPE Point OF T
  PUBLIC x AS T
  y AS Integer
END TYPE

EXPORT UNION Shape INCLUDES Other
  Point
  Circle
END UNION

ENUM Color
  Red, Green, Blue
END ENUM

EXPORT RESOURCE Db CLOSE BY sqlite::close THREAD_SENDABLE

EXPORT FUNC dbClose AS Db::teardown

LINK "sqlite3" AS sqlite
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, db OUT CPtr) AS ret CInt32
    CONST flags = 1
    SUCCESS_ON ret = 0
    RESULT db
    FREE db
      SYMBOL "sqlite3_free"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK

DOC INTERNAL
FUNC greet(String)
DESC Greets a person.
WARN Be nice.
DEPRECATED use hello
GROUP greetings
ARG name the name
RET the greeting
ERROR MFB_BAD bad thing
EXAMPLE
  greet("x")
END EXAMPLE
END DOC

DOC
TYPE Point
PROP x the x coord
END DOC
"#;
    let json = project_json(src);
    // Item kinds.
    assert!(json.contains("\"project\": \"demo\""));
    assert!(json.contains("\"module\": \"io\", \"alias\": \"term\""));
    assert!(json.contains("\"module\": \"math\""));
    assert!(json.contains("\"kind\": \"binding\""));
    assert!(json.contains("\"resource\": true, \"state\": \"Integer\""));
    assert!(json.contains("\"kind\": \"type\""));
    assert!(json.contains("\"templateParams\": [\"T\"]"));
    assert!(json.contains("\"kind\": \"union\""));
    assert!(json.contains("\"includes\": [\"Other\"]"));
    assert!(json.contains("\"kind\": \"enum\""));
    assert!(json.contains("\"kind\": \"resource\""));
    assert!(json.contains("\"threadSendable\": true"));
    assert!(json.contains("\"kind\": \"funcAlias\""));
    assert!(json.contains("\"target\": \"Db.teardown\""));
    assert!(json.contains("\"kind\": \"link\""));
    assert!(json.contains("\"kind\": \"linkFunc\""));
    assert!(json.contains("\"returnResource\": true"));
    assert!(json.contains("\"kind\": \"doc\""));
    // Doc details.
    assert!(json.contains("\"header\": \"FUNC\""));
    assert!(json.contains("\"signature\": [\"String\"]"));
    assert!(json.contains("\"attrs\": [\"INTERNAL\"]"));
    assert!(json.contains("\"kind\": \"warn\""));
    assert!(json.contains("\"deprecated\": [\"use hello\"]"));
    assert!(json.contains("\"group\": [\"greetings\"]"));
    assert!(json.contains("\"code\": \"MFB_BAD\""));
    assert!(json.contains("\"example\": [\"greet(\\\"x\\\")\"]"));
    // The PACKAGE-visibility field prefix and OUT ABI slot.
    assert!(json.contains("\"visibility\": \"public\""));
    assert!(json.contains("\"out\": true"));
}

#[test]
fn serialize_covers_all_statement_and_expression_kinds() {
    let src = r#"
ISOLATED FUNC compute(a AS Integer, b AS Integer = 2) AS RES Db STATE Integer
  LET total AS Integer = a + b
  notify()
  MUT items AS List OF Integer = [1, 2, 3]
  RES conn AS Db STATE Integer = open()
  total = total - 1
  conn.state = 5
  conn.state.field = 9
  IF total > 0 THEN
    total = total + 1
  ELSEIF total < 0 THEN
    total = 0
  ELSE
    total = 2
  END IF
  IF a = 1 THEN RETURN a ELSE RETURN b
  MATCH total
    CASE 0
      total = 1
    CASE 1, 2 WHEN a > 0
      total = 2
    CASE Shape(s)
      total = 3
    CASE ELSE
      total = 4
  END MATCH
  FOR i = 1 TO 10 STEP 2
    total = total + i
  NEXT
  FOR EACH item IN items
    total = total + item
  NEXT
  WHILE total < 100
    total = total + 1
  WEND
  DO
    total = total + 1
  LOOP UNTIL total > 200
  DO WHILE total < 300
    total = total + 1
  LOOP
  LET pt AS Point = Point[x := 1, 2]
  LET updated AS Point = WITH pt { x := 9 }
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  LET member AS Integer = pt.x
  LET neg AS Integer = -total
  LET flag AS Boolean = TRUE
  LET flag2 AS Boolean = FALSE
  LET truth AS Boolean = NOT (a = b)
  LET both AS Boolean = a > 0 AND b > 0 OR a < 0
  LET cat AS String = "a" & "b"
  LET pw AS Integer = a ^ b
  LET md AS Integer = a MOD b
  LET cb AS Integer = greet(name := "x", 5)
  LET fn2 AS Integer = piped |> add(_, 1)
  LET lam AS Integer = LAMBDA(x AS Integer) -> x + 1
  LET lam2 AS Integer = LAMBDA(x AS Integer) -> total = total + x
  LET trapped AS Integer = risky() TRAP(err)
    total = 0
  END TRAP
  EXIT FOR
  CONTINUE FOR
  FAIL "boom"
  RECOVER 0
  PROPAGATE
  EXIT PROGRAM 1
  RETURN conn
  TRAP(err)
    RETURN open()
  END TRAP
END FUNC
"#;
    let json = project_json(src);
    // Statement kinds.
    for kind in [
        "\"kind\": \"binding\"",
        "\"kind\": \"assignment\"",
        "\"kind\": \"stateAssignment\"",
        "\"kind\": \"if\"",
        "\"kind\": \"match\"",
        "\"kind\": \"for\"",
        "\"kind\": \"forEach\"",
        "\"kind\": \"while\"",
        "\"kind\": \"doUntil\"",
        "\"kind\": \"exit\"",
        "\"kind\": \"continue\"",
        "\"kind\": \"fail\"",
        "\"kind\": \"recover\"",
        "\"kind\": \"propagate\"",
        "\"kind\": \"return\"",
        "\"kind\": \"expression\"",
    ] {
        assert!(json.contains(kind), "missing statement kind {kind}");
    }
    // Match patterns + guard.
    assert!(json.contains("\"kind\": \"else\""));
    assert!(json.contains("\"kind\": \"literal\""));
    assert!(json.contains("\"kind\": \"oneOf\""));
    assert!(json.contains("\"kind\": \"union\", \"type\": \"Shape\", \"binding\": \"s\""));
    assert!(json.contains("\"guard\":"));
    // Expression kinds.
    for kind in [
        "\"kind\": \"binary\"",
        "\"kind\": \"unary\"",
        "\"kind\": \"call\"",
        "\"kind\": \"lambda\"",
        "\"kind\": \"constructor\"",
        "\"kind\": \"with\"",
        "\"kind\": \"list\"",
        "\"kind\": \"map\"",
        "\"kind\": \"memberAccess\"",
        "\"kind\": \"trapped\"",
        "\"kind\": \"identifier\"",
        "\"kind\": \"string\"",
        "\"kind\": \"number\"",
        "\"kind\": \"boolean\"",
        "\"kind\": \"named\"",
    ] {
        assert!(json.contains(kind), "missing expression kind {kind}");
    }
    // Function metadata: isolated + return resource/state + trap + default param.
    assert!(json.contains("\"kind\": \"func\""));
    assert!(json.contains("\"returnResource\": true, \"returnState\": \"Integer\""));
    assert!(json.contains("\"trap\":"));
    assert!(json.contains("\"assignTarget\": \"total\""));
    assert!(json.contains("\"loop\": \"for\""));
    assert!(json.contains("\"target\": \"program\""));
}

// ---------------------------------------------------------------------------
// serialize.rs — signature_line + placeholder helpers.
// ---------------------------------------------------------------------------

#[test]
fn function_signature_line_renders_visibility_isolated_and_params() {
    let file = parse_file(
        "EXPORT ISOLATED FUNC f(x AS Integer, RES h AS File) AS String\n  RETURN \"\"\nEND FUNC\n",
    );
    let f = function(&file, "f");
    assert_eq!(
        f.signature_line(),
        "EXPORT ISOLATED FUNC f(x AS Integer, RES h AS File) AS String"
    );

    // A SUB has no return clause; a PACKAGE bare param has no `AS`.
    let file = parse_file("PRIVATE SUB g(a)\nEND SUB\n");
    let g = function(&file, "g");
    assert_eq!(g.signature_line(), "PRIVATE SUB g(a)");

    // A private FUNC with no explicit return type defaults to Nothing.
    let file = parse_file("FUNC h()\nEND FUNC\n");
    let h = function(&file, "h");
    assert_eq!(h.signature_line(), "FUNC h() AS Nothing");
}

#[test]
fn type_decl_signature_line_covers_all_kinds_and_visibility() {
    let file = parse_file("EXPORT TYPE T\n  x AS Integer\nEND TYPE\n");
    let Item::Type(decl) = &file.items[0] else {
        panic!("type");
    };
    assert_eq!(decl.signature_line(), "EXPORT TYPE T");

    let file = parse_file("PRIVATE UNION U\n  T\nEND UNION\n");
    let Item::Type(decl) = &file.items[0] else {
        panic!("union");
    };
    assert_eq!(decl.signature_line(), "PRIVATE UNION U");

    let file = parse_file("ENUM E\n  A, B\nEND ENUM\n");
    let Item::Type(decl) = &file.items[0] else {
        panic!("enum");
    };
    assert_eq!(decl.signature_line(), "ENUM E");
}

#[test]
fn pipeline_placeholder_substitutes_across_all_expression_forms() {
    // A pipeline `x |> rhs` requires `_` somewhere in rhs; the parser rewrites it
    // via substitute_placeholder, walking every expression form.
    let file = parse_file(
        "FUNC main AS Integer\n  RETURN base |> wrap(-_, [_, 1], Point[_], WITH _ { f := _ }, m.field, _ + 1)\nEND FUNC\n",
    );
    let f = function(&file, "main");
    let Statement::Return {
        value: Some(expr), ..
    } = &f.body[0]
    else {
        panic!("return");
    };
    // The rewritten call has `base` substituted in for each `_`.
    assert!(matches!(expr, Expression::Call { .. }));
    let json = project_json(
        "FUNC main AS Integer\n  RETURN base |> wrap(-_, [_, 1], Point[_], WITH _ { f := _ }, m.field, _ + 1)\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"call\""));
    assert!(json.contains("\"value\": \"base\""));
    // No literal placeholder identifier survives.
    assert!(!json.contains("\"value\": \"_\""));
}

#[test]
fn pipeline_placeholder_missing_is_rejected() {
    assert!(try_parse("FUNC main AS Integer\n  RETURN x |> add(1)\nEND FUNC\n").is_err());
}

#[test]
fn pipeline_placeholder_in_lambda_and_map_and_boolean() {
    // Exercise the Lambda/MapLiteral/Boolean/String/Number arms of the
    // placeholder walk (both contains_placeholder and substitute_placeholder).
    let file = parse_file(
        "FUNC main AS Integer\n  RETURN seed |> pick(LAMBDA(x AS Integer) -> _, Map OF String TO Integer { \"k\" := _ }, TRUE, \"s\", 3)\nEND FUNC\n",
    );
    let f = function(&file, "main");
    assert!(matches!(&f.body[0], Statement::Return { .. }));
}

// ---------------------------------------------------------------------------
// Parse error paths (items / stmt / expr / lexical). Valid-program e2e tests
// never exercise these, so they concentrate the uncovered lines.
// ---------------------------------------------------------------------------

#[test]
fn top_level_errors_are_reported() {
    // Unexpected top-level token.
    assert!(try_parse("garbage\n").is_err());
    // IMPORT with no name.
    assert!(try_parse("IMPORT\n").is_err());
    // Binding with no name.
    assert!(try_parse("LET\n").is_err());
    // Function with no name.
    assert!(try_parse("FUNC\n").is_err());
    // Unterminated function.
    assert!(try_parse("FUNC main AS Integer\n  RETURN 0\n").is_err());
    // Type with no name.
    assert!(try_parse("TYPE\n").is_err());
    // Unterminated type.
    assert!(try_parse("TYPE T\n  x AS Integer\n").is_err());
    // END without matching kind.
    assert!(try_parse("FUNC main\nEND SUB\n").is_err());
}

#[test]
fn function_header_and_body_errors() {
    // Missing close paren on params.
    assert!(try_parse("FUNC f(a AS Integer\nEND FUNC\n").is_err());
    // ISOLATED on a SUB is rejected.
    assert!(try_parse("ISOLATED SUB s\nEND SUB\n").is_err());
    // Two TRAPs.
    assert!(try_parse(
        "FUNC f AS Integer\n  RETURN 0\n  TRAP(a)\n    RETURN 1\n  END TRAP\n  TRAP(b)\n    RETURN 2\n  END TRAP\nEND FUNC\n"
    )
    .is_err());
    // Statement after TRAP.
    assert!(try_parse(
        "FUNC f AS Integer\n  TRAP(a)\n    RETURN 1\n  END TRAP\n  RETURN 0\nEND FUNC\n"
    )
    .is_err());
    // Bad parameter name.
    assert!(try_parse("FUNC f(123)\nEND FUNC\n").is_err());
}

#[test]
fn trap_header_errors() {
    // TRAP without paren.
    assert!(try_parse("FUNC f AS Integer\n  RETURN 0\n  TRAP\n  END TRAP\nEND FUNC\n").is_err());
    // TRAP without binding.
    assert!(try_parse("FUNC f AS Integer\n  RETURN 0\n  TRAP()\n  END TRAP\nEND FUNC\n").is_err());
    // TRAP unterminated.
    assert!(
        try_parse("FUNC f AS Integer\n  RETURN 0\n  TRAP(e)\n    RETURN 1\nEND FUNC\n").is_err()
    );
}

#[test]
fn type_decl_errors() {
    // Field without AS.
    assert!(try_parse("TYPE T\n  x\nEND TYPE\n").is_err());
    // Union member that is not a name.
    assert!(try_parse("UNION U\n  123\nEND UNION\n").is_err());
    // Enum member that is not a name.
    assert!(try_parse("ENUM E\n  123\nEND ENUM\n").is_err());
    // Template param that is not a name.
    assert!(try_parse("TYPE T OF 123\n  x AS Integer\nEND TYPE\n").is_err());
    // END names wrong kind.
    assert!(try_parse("TYPE T\n  x AS Integer\nEND UNION\n").is_err());
}

#[test]
fn statement_errors() {
    // Inline IF branch must be simple (not a block statement).
    assert!(
        try_parse("FUNC f AS Integer\n  IF TRUE THEN FOR i = 1 TO 2\n  NEXT\nEND FUNC\n").is_err()
    );
    // EXIT with bad target.
    assert!(try_parse("FUNC f\n  EXIT NOPE\nEND FUNC\n").is_err());
    // CONTINUE with bad target.
    assert!(try_parse("FUNC f\n  CONTINUE NOPE\nEND FUNC\n").is_err());
    // IF without THEN.
    assert!(try_parse("FUNC f\n  IF TRUE\n  END IF\nEND FUNC\n").is_err());
    // MATCH without CASE.
    assert!(try_parse("FUNC f\n  MATCH x\n    total = 1\n  END MATCH\nEND FUNC\n").is_err());
    // FOR without `=`.
    assert!(try_parse("FUNC f\n  FOR i 1 TO 2\n  NEXT\nEND FUNC\n").is_err());
    // FOR without TO.
    assert!(try_parse("FUNC f\n  FOR i = 1 2\n  NEXT\nEND FUNC\n").is_err());
    // FOR EACH without IN.
    assert!(try_parse("FUNC f\n  FOR EACH i xs\n  NEXT\nEND FUNC\n").is_err());
    // WHILE without WEND.
    assert!(try_parse("FUNC f\n  WHILE TRUE\n    x = 1\nEND FUNC\n").is_err());
    // DO without LOOP.
    assert!(try_parse("FUNC f\n  DO\n    x = 1\nEND FUNC\n").is_err());
}

#[test]
fn expression_errors() {
    // Only identifiers can be called.
    assert!(try_parse("FUNC f AS Integer\n  RETURN (1)(2)\nEND FUNC\n").is_err());
    // Only identifiers can be constructors.
    assert!(try_parse("FUNC f AS Integer\n  RETURN (1)[2]\nEND FUNC\n").is_err());
    // WITH without brace.
    assert!(try_parse("FUNC f AS Integer\n  RETURN WITH x y\nEND FUNC\n").is_err());
    // Member access without identifier.
    assert!(try_parse("FUNC f AS Integer\n  RETURN x.\nEND FUNC\n").is_err());
    // Unterminated grouping.
    assert!(try_parse("FUNC f AS Integer\n  RETURN (1\nEND FUNC\n").is_err());
    // Map literal missing TO.
    assert!(
        try_parse("FUNC f AS Integer\n  RETURN Map OF String Integer { }\nEND FUNC\n").is_err()
    );
    // Bare expression that is not recognized.
    assert!(try_parse("FUNC f\n  *\nEND FUNC\n").is_err());
    // Three-part qualified name.
    assert!(try_parse("FUNC f AS Integer\n  RETURN a::b::c\nEND FUNC\n").is_err());
}

// ---------------------------------------------------------------------------
// types.rs — DocHeaderKind / DocProseKind helper methods.
// ---------------------------------------------------------------------------

#[test]
fn doc_header_kind_keywords() {
    assert_eq!(DocHeaderKind::Func.keyword(), "FUNC");
    assert_eq!(DocHeaderKind::Sub.keyword(), "SUB");
    assert_eq!(DocHeaderKind::Type.keyword(), "TYPE");
    assert_eq!(DocHeaderKind::Union.keyword(), "UNION");
    assert_eq!(DocHeaderKind::Enum.keyword(), "ENUM");
    assert_eq!(DocHeaderKind::Package.keyword(), "PACKAGE");
}

#[test]
fn doc_prose_kind_roundtrips_all_variants() {
    for (kw, kind, code, label) in [
        ("DESC", DocProseKind::Desc, 0u8, "desc"),
        ("warn", DocProseKind::Warn, 1, "warn"),
        ("Info", DocProseKind::Info, 2, "info"),
        ("SEC", DocProseKind::Sec, 3, "sec"),
    ] {
        assert_eq!(DocProseKind::from_keyword(kw), Some(kind));
        assert_eq!(kind.code(), code);
        assert_eq!(DocProseKind::from_code(code), kind);
        assert_eq!(kind.label(), label);
    }
    assert_eq!(DocProseKind::from_keyword("nope"), None);
    // Unknown code falls back to Desc.
    assert_eq!(DocProseKind::from_code(99), DocProseKind::Desc);
}

// ---------------------------------------------------------------------------
// serialize.rs — the null/None branches (defaults) of each renderer.
// ---------------------------------------------------------------------------

#[test]
fn serialize_covers_none_and_default_branches() {
    let src = r#"
LET plain = 1
RES bare AS File
SUB doer(a, RES h AS File)
  LET x = 1
  MUT y
  EXIT SUB
  RETURN
  RECOVER
  FOR i = 1 TO 3
    y = i
  NEXT
  MATCH y
    CASE ELSE
      y = 0
  END MATCH
END SUB

LINK "lib" AS l
  FUNC noRet(a AS Integer)
    SYMBOL "sym"
    ABI (a CInt32) AS r CInt32
  END FUNC
END LINK
"#;
    let json = project_json(src);
    // A binding/param/return with no type serializes `null`.
    assert!(json.contains("\"type\": null"));
    // A RES declaration with no STATE serializes state null.
    assert!(json.contains("\"resource\": true, \"state\": null"));
    // A SUB with no return type.
    assert!(json.contains("\"kind\": \"sub\""));
    assert!(json.contains("\"returnType\": null"));
    // RETURN / RECOVER with no value.
    assert!(json.contains("\"kind\": \"return\", \"value\": null"));
    assert!(json.contains("\"kind\": \"recover\", \"value\": null"));
    // EXIT SUB (no code).
    assert!(json.contains("\"kind\": \"exit\", \"target\": \"sub\", \"code\": null"));
    // FOR without STEP.
    assert!(json.contains("\"step\": null"));
    // MATCH else.
    assert!(json.contains("\"kind\": \"else\""));
    // A LINK func with no successOn / result / free.
    assert!(json.contains("\"successOn\": null"));
    assert!(json.contains("\"result\": null"));
    assert!(json.contains("\"free\": null"));
    // A LINK func whose MFBASIC return is absent renders returnType null.
    assert!(json.contains("\"returnType\": null"));
}

#[test]
fn serialize_exit_targets_func_and_do_and_while() {
    let src = r#"
FUNC f AS Integer
  DO
    EXIT DO
    EXIT WHILE
    EXIT FUNC
    CONTINUE DO
    CONTINUE WHILE
  LOOP UNTIL TRUE
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    assert!(json.contains("\"target\": \"do\""));
    assert!(json.contains("\"target\": \"while\""));
    assert!(json.contains("\"target\": \"func\""));
    assert!(json.contains("\"loop\": \"do\""));
    assert!(json.contains("\"loop\": \"while\""));
}

#[test]
fn serialize_lambda_without_assign_target() {
    let json = project_json(
        "FUNC f AS Integer\n  LET g AS Integer = LAMBDA(x AS Integer) -> x + 1\n  RETURN 0\nEND FUNC\n",
    );
    // The plain-body lambda arm (no assignTarget key).
    assert!(json.contains("\"kind\": \"lambda\", \"params\":"));
    assert!(!json.contains("\"assignTarget\""));
}

#[test]
fn pipeline_placeholder_walks_named_args_and_trapped_and_with() {
    // Named call/constructor args + a WITH update + member access all recurse
    // through both contains_placeholder and substitute_placeholder.
    let json = project_json(
        "FUNC main AS Integer\n  RETURN seed |> build(k := _, Point[y := _], WITH base { z := _ })\nEND FUNC\n",
    );
    assert!(json.contains("\"value\": \"seed\""));
    assert!(!json.contains("\"value\": \"_\""));

    // A named argument that does NOT hold the placeholder still parses (the
    // false branch of the named-arg walk), with `_` supplied elsewhere.
    let json =
        project_json("FUNC main AS Integer\n  RETURN seed |> build(k := 1, v := _)\nEND FUNC\n");
    assert!(json.contains("\"value\": \"seed\""));
}

// ---------------------------------------------------------------------------
// lexical.rs — qualified member after `::` may be a keyword or a
// number-adjacent identifier (e.g. `blake::2b`).
// ---------------------------------------------------------------------------

#[test]
fn qualified_member_may_be_keyword_or_numeric() {
    // A keyword in the member position (`STEP` reserved word) via
    // consume_name_or_keyword.
    let file = parse_file("FUNC main AS Integer\n  RETURN hash::step\nEND FUNC\n");
    let f = function(&file, "main");
    let Statement::Return {
        value: Some(Expression::Identifier(name)),
        ..
    } = &f.body[0]
    else {
        panic!("identifier");
    };
    assert_eq!(name, "hash.step");

    // A number-adjacent identifier member (`2b`) via consume_numeric_identifier_part.
    let file = parse_file("FUNC main AS Integer\n  RETURN blake::2b\nEND FUNC\n");
    let f = function(&file, "main");
    let Statement::Return {
        value: Some(Expression::Identifier(name)),
        ..
    } = &f.body[0]
    else {
        panic!("identifier");
    };
    assert_eq!(name, "blake.2b");
}

#[test]
fn qualified_member_missing_after_colon_is_error() {
    // `::` at end of file: consume_qualified_identifier_part fails.
    assert!(try_parse("FUNC main AS Integer\n  RETURN pkg::\nEND FUNC\n").is_err());
}

// ---------------------------------------------------------------------------
// items.rs — DOC block structural parsing arms and errors.
// ---------------------------------------------------------------------------

#[test]
fn doc_block_prose_kinds_and_continuation() {
    // INFO/SEC prose, multi-line continuation, blank-line flush, a kind switch.
    let src = r#"
DOC
FUNC f
DESC First line
DESC continues here
INFO an info note
SEC a security note
WARN warning

DESC after a blank
END DOC
FUNC f AS Integer
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    assert!(json.contains("\"kind\": \"info\""));
    assert!(json.contains("\"kind\": \"sec\""));
    assert!(json.contains("\"kind\": \"warn\""));
    // Continued DESC lines joined with a space.
    assert!(json.contains("First line continues here"));
    assert!(json.contains("after a blank"));
}

#[test]
fn doc_block_header_variants_and_errors() {
    // PACKAGE header takes no name.
    assert!(try_parse("DOC\nPACKAGE\nDESC ok\nEND DOC\nSUB main\nEND SUB\n").is_ok());
    // PACKAGE with a name is rejected.
    assert!(try_parse("DOC\nPACKAGE thing\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // A FUNC header with no name is rejected.
    assert!(try_parse("DOC\nFUNC\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // Unknown header keyword.
    assert!(try_parse("DOC\nWIDGET x\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // Empty DOC (no header line) is rejected.
    assert!(try_parse("DOC\n\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // A TYPE header with a signature disambiguator is fine (non-callable path).
    assert!(try_parse("DOC\nTYPE Widget\nDESC ok\nEND DOC\nSUB main\nEND SUB\n").is_ok());
}

#[test]
fn doc_block_named_line_errors_and_unknown() {
    // ARG with no name.
    assert!(try_parse("DOC\nFUNC f\nARG\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // PROP with no name.
    assert!(try_parse("DOC\nTYPE T\nPROP\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // ERROR with no code.
    assert!(try_parse("DOC\nFUNC f\nERROR\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // Unknown line keyword.
    assert!(try_parse("DOC\nFUNC f\nFLARB nope\nEND DOC\nSUB main\nEND SUB\n").is_err());
    // Unterminated EXAMPLE (no END EXAMPLE before END DOC).
    assert!(try_parse("DOC\nFUNC f\nEXAMPLE\n  code here\nEND DOC\nSUB main\nEND SUB\n").is_err());
}

#[test]
fn doc_func_header_signature_forms() {
    // A header with a function-type param whose commas are inside parens must not
    // be split (exercises parse_header_signature nesting).
    let src = r#"
DOC
FUNC apply(FUNC(Integer, Integer) AS Integer, List OF Integer)
DESC ok
END DOC
FUNC apply AS Integer
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    assert!(json.contains("FUNC(Integer, Integer) AS Integer"));
    // An empty parameter list `f()` yields an empty signature array.
    let json = project_json("DOC\nFUNC g()\nDESC ok\nEND DOC\nSUB main\nEND SUB\n");
    assert!(json.contains("\"signature\": []"));
}

// ---------------------------------------------------------------------------
// items.rs — LINK / native FUNC / ABI / FREE / CONST error and edge paths.
// ---------------------------------------------------------------------------

#[test]
fn link_block_errors() {
    // LINK without a library string.
    assert!(try_parse("LINK foo AS l\nEND LINK\n").is_err());
    // LINK without AS.
    assert!(try_parse("LINK \"x\" l\nEND LINK\n").is_err());
    // LINK without alias.
    assert!(try_parse("LINK \"x\" AS\nEND LINK\n").is_err());
    // A non-FUNC statement inside LINK.
    assert!(try_parse("LINK \"x\" AS l\n  LET y = 1\nEND LINK\n").is_err());
    // Unterminated LINK.
    assert!(try_parse("LINK \"x\" AS l\n").is_err());
}

#[test]
fn native_func_missing_symbol_or_abi() {
    // Missing SYMBOL.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f(a AS Integer) AS Integer\n    ABI (a CInt32) AS r CInt32\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // Missing ABI.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f(a AS Integer) AS Integer\n    SYMBOL \"s\"\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // Unknown clause in native FUNC body.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f AS Integer\n    SYMBOL \"s\"\n    ABI () AS r CInt32\n    BOGUS 1\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // END must name FUNC.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f AS Integer\n    SYMBOL \"s\"\n    ABI () AS r CInt32\n  END SUB\nEND LINK\n"
    )
    .is_err());
}

#[test]
fn native_func_error_on_and_return_and_const() {
    // ERROR_ON is stored as a NOT of SUCCESS_ON; CONST pin; `return` OUT slot.
    let src = r#"
LINK "x" AS l
  FUNC f(a AS Integer) AS Integer
    SYMBOL "s"
    ABI (a CInt32, return OUT CInt32) AS r CInt32
    CONST a = 7
    ERROR_ON r <> 0
    RESULT r
  END FUNC
END LINK
"#;
    let json = project_json(src);
    // ERROR_ON becomes a NOT unary in successOn.
    assert!(json.contains("\"kind\": \"unary\", \"operator\": \"NOT\""));
    assert!(json.contains("\"slot\": \"a\""));
    // The `return` OUT slot serializes with its literal name.
    assert!(json.contains("\"name\": \"return\""));
}

#[test]
fn native_func_free_block_and_errors() {
    // A well-formed FREE block.
    let src = r#"
LINK "x" AS l
  FUNC f() AS RES Db
    SYMBOL "s"
    ABI (return OUT CPtr) AS r CInt32
    FREE return
      SYMBOL "free_it"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK
"#;
    assert!(try_parse(src).is_ok());
    // FREE with an unknown clause.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f() AS RES Db\n    SYMBOL \"s\"\n    ABI (return OUT CPtr) AS r CInt32\n    FREE return\n      NONSENSE 1\n    END FREE\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // FREE END must name FREE.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f() AS RES Db\n    SYMBOL \"s\"\n    ABI (return OUT CPtr) AS r CInt32\n    FREE return\n      SYMBOL \"g\"\n      ABI (ptr CPtr) AS CVoid\n    END FUNC\n  END FUNC\nEND LINK\n"
    )
    .is_err());
}

#[test]
fn resource_declaration_errors() {
    // Missing CLOSE.
    assert!(try_parse("RESOURCE Db foo\n").is_err());
    // CLOSE not followed by BY.
    assert!(try_parse("RESOURCE Db CLOSE foo\n").is_err());
    // Missing close op after BY.
    assert!(try_parse("RESOURCE Db CLOSE BY\n").is_err());
    // Missing name.
    assert!(try_parse("RESOURCE CLOSE BY x::y\n").is_err());
}

#[test]
fn func_alias_errors() {
    // Missing AS after the alias name.
    assert!(try_parse("FUNC alias foo::bar\n").is_err());
}

// ---------------------------------------------------------------------------
// expr.rs — parse_type_name variants (thread/function/map/list/RES/grouped).
// ---------------------------------------------------------------------------

fn param_type(src: &str) -> String {
    let file = parse_file(src);
    let f = &file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(f) => Some(f),
            _ => None,
        })
        .expect("function");
    f.params[0].type_name.clone().expect("param type")
}

#[test]
fn type_names_cover_thread_map_list_and_function_forms() {
    // Thread with message + resource + output.
    assert_eq!(
        param_type("SUB s(t AS Thread OF Msg RES Handle TO Out)\nEND SUB\n"),
        "Thread OF Msg RES Handle TO Out"
    );
    // Thread resource-only (message defaults to Nothing).
    assert_eq!(
        param_type("SUB s(t AS Thread OF RES Handle TO Out)\nEND SUB\n"),
        "Thread OF RES Handle TO Out"
    );
    // Thread with just message + output.
    assert_eq!(
        param_type("SUB s(t AS Thread OF Msg TO Out)\nEND SUB\n"),
        "Thread OF Msg TO Out"
    );
    // ThreadWorker canonicalizes.
    assert_eq!(
        param_type("SUB s(t AS ThreadWorker OF Msg TO Out)\nEND SUB\n"),
        "ThreadWorker OF Msg TO Out"
    );
    // Map type with RES value.
    assert_eq!(
        param_type("SUB s(m AS Map OF String TO RES File)\nEND SUB\n"),
        "Map OF String TO RES File"
    );
    // List of RES.
    assert_eq!(
        param_type("SUB s(xs AS List OF RES File)\nEND SUB\n"),
        "List OF RES File"
    );
    // Multi-arg template type.
    assert_eq!(
        param_type("SUB s(p AS Pair OF Integer, String)\nEND SUB\n"),
        "Pair OF Integer, String"
    );
    // Function type.
    assert_eq!(
        param_type("SUB s(f AS FUNC(Integer, String) AS Boolean)\nEND SUB\n"),
        "FUNC(Integer, String) AS Boolean"
    );
    // ISOLATED function type.
    assert_eq!(
        param_type("SUB s(f AS ISOLATED FUNC() AS Integer)\nEND SUB\n"),
        "ISOLATED FUNC() AS Integer"
    );
    // Grouped type.
    assert_eq!(param_type("SUB s(f AS (Integer))\nEND SUB\n"), "(Integer)");
    // Nothing base type.
    assert_eq!(
        param_type("SUB s(f AS FUNC() AS Nothing)\nEND SUB\n"),
        "FUNC() AS Nothing"
    );
}

#[test]
fn type_name_errors() {
    // Map type missing TO.
    assert!(try_parse("SUB s(m AS Map OF String Integer)\nEND SUB\n").is_err());
    // Thread type missing TO.
    assert!(try_parse("SUB s(t AS Thread OF Msg Out)\nEND SUB\n").is_err());
    // ISOLATED not followed by FUNC.
    assert!(try_parse("SUB s(f AS ISOLATED Integer)\nEND SUB\n").is_err());
    // Function type missing (.
    assert!(try_parse("SUB s(f AS FUNC Integer)\nEND SUB\n").is_err());
    // Function type missing AS.
    assert!(try_parse("SUB s(f AS FUNC(Integer))\nEND SUB\n").is_err());
    // Grouped type missing ).
    assert!(try_parse("SUB s(f AS (Integer)\nEND SUB\n").is_err());
    // A bare type where a type name is required but token is bad.
    assert!(try_parse("SUB s(f AS 123)\nEND SUB\n").is_err());
}

#[test]
fn map_literal_with_res_value_type() {
    // A `Map OF K TO RES File { }` literal carries the RES marker on its value.
    let json = project_json(
        "FUNC main AS Integer\n  LET m = Map OF Integer TO RES File { }\n  RETURN 0\nEND FUNC\n",
    );
    assert!(json.contains("\"valueType\": \"RES File\""));
}

#[test]
fn map_literal_missing_to_is_error() {
    assert!(try_parse(
        "FUNC main AS Integer\n  LET m = Map OF Integer File { }\n  RETURN 0\nEND FUNC\n"
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// manifest.rs — parse_project / write_ast / selected_source_paths and the
// collect_selected_source_files error paths.
// ---------------------------------------------------------------------------

#[test]
fn parse_project_propagates_parse_errors() {
    let root = test_temp_dir("parse_project_propagates_parse_errors");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    fs::write(project_dir.join("src/main.mfb"), "garbage nonsense\n").expect("main");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(parse_project("demo", &project_dir, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn parse_project_missing_directory_is_error() {
    let root = test_temp_dir("parse_project_missing_directory_is_error");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    // The project directory itself does not exist -> canonicalize fails.
    assert!(parse_project("demo", &root.join("nope"), &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn collect_reports_missing_and_empty_roots() {
    let root = test_temp_dir("collect_reports_missing_and_empty_roots");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");

    // A source root that does not exist.
    let missing = manifest_with_sources(vec![source_entry("nope", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &missing).is_err());

    // A source root that has no matching .mfb files.
    let empty = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &empty).is_err());

    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn collect_handles_single_file_and_non_mfb_root() {
    let root = test_temp_dir("collect_handles_single_file_and_non_mfb_root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("dir");
    fs::write(project_dir.join("main.mfb"), "SUB main\nEND SUB\n").expect("main");
    fs::write(project_dir.join("notes.txt"), "hi\n").expect("notes");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");

    // A direct file root that is an .mfb file is selected.
    let file_manifest = manifest_with_sources(vec![source_entry("main.mfb", None, None)]);
    let files =
        collect_selected_source_files(&project_dir, &canonical, &file_manifest).expect("files");
    assert_eq!(files.len(), 1);

    // A direct file root that is not .mfb yields an empty selection -> error.
    let txt_manifest = manifest_with_sources(vec![source_entry("notes.txt", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &txt_manifest).is_err());

    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn source_entries_default_include_when_absent() {
    // A manifest with no `sources` key yields no entries -> empty selection error.
    let root = test_temp_dir("source_entries_default_include_when_absent");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("dir");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest: HashMap<String, JsonValue> = HashMap::new();
    let files =
        collect_selected_source_files(&project_dir, &canonical, &manifest).expect("empty ok");
    assert!(files.is_empty());
    fs::remove_dir_all(root).expect("remove");
}

// ---------------------------------------------------------------------------
// manifest.rs — nested directory walk, escape checks, and glob edge cases.
// ---------------------------------------------------------------------------

#[test]
fn collect_walks_nested_directories() {
    let root = test_temp_dir("collect_walks_nested_directories");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src/a/b")).expect("dirs");
    fs::write(project_dir.join("src/top.mfb"), "SUB top\nEND SUB\n").expect("top");
    fs::write(project_dir.join("src/a/mid.mfb"), "SUB mid\nEND SUB\n").expect("mid");
    fs::write(project_dir.join("src/a/b/deep.mfb"), "SUB deep\nEND SUB\n").expect("deep");
    fs::write(project_dir.join("src/a/skip.txt"), "x\n").expect("txt");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 3);
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn selected_source_paths_missing_dir_is_error() {
    let root = test_temp_dir("selected_source_paths_missing_dir_is_error");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(selected_source_paths(&root.join("nope"), &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[test]
fn nested_symlink_escaping_project_is_rejected() {
    let root = test_temp_dir("nested_symlink_escaping_project_is_rejected");
    let project_dir = root.join("project");
    let outside = root.join("outside");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("escape.mfb"), "SUB escape\nEND SUB\n").expect("escape");
    // A symlink *inside* a walked subdirectory pointing outside the project.
    symlink(&outside, project_dir.join("src/link")).expect("symlink");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn glob_component_edge_cases() {
    // Trailing `*` after the value is consumed.
    assert!(glob_matches("a*", "a"));
    // `?` matches exactly one character.
    assert!(glob_matches("a?c", "abc"));
    assert!(!glob_matches("a?c", "ac"));
    // A `*` in the middle backtracks.
    assert!(glob_matches("a*c", "abbbc"));
    assert!(!glob_matches("a*c", "abbb"));
    // `**` matches zero segments.
    assert!(glob_matches("**/x", "x"));
    // A literal mismatch fails fast.
    assert!(!glob_matches("abc", "xbc"));
}

// ---------------------------------------------------------------------------
// expr.rs — remaining operator/argument/lambda/with edge and error paths.
// ---------------------------------------------------------------------------

#[test]
fn div_operator_and_lambda_bare_body() {
    let json = project_json("FUNC f AS Integer\n  RETURN a DIV b\nEND FUNC\n");
    assert!(json.contains("\"operator\": \"DIV\""));

    // A lambda whose body is not an assignment (bare identifier).
    let json = project_json(
        "FUNC f AS Integer\n  LET g AS Integer = LAMBDA() -> 7\n  RETURN 0\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"lambda\""));
}

#[test]
fn with_update_error_paths() {
    // WITH update field is not an identifier.
    assert!(try_parse("FUNC f AS Integer\n  RETURN WITH x { 1 := 2 }\nEND FUNC\n").is_err());
    // Missing := between field and value.
    assert!(try_parse("FUNC f AS Integer\n  RETURN WITH x { a b }\nEND FUNC\n").is_err());
    // Missing closing }.
    assert!(try_parse("FUNC f AS Integer\n  RETURN WITH x { a := 1\nEND FUNC\n").is_err());
    // A multi-update WITH (comma loop) parses.
    let json = project_json(
        "FUNC f AS Integer\n  LET r = WITH x { a := 1, b := 2 }\n  RETURN 0\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"with\""));
}

#[test]
fn call_and_constructor_argument_errors() {
    // Constructor with a named field missing := ... actually missing close bracket.
    assert!(try_parse("FUNC f AS Integer\n  RETURN Point[a := 1\nEND FUNC\n").is_err());
    // Call missing closing paren.
    assert!(try_parse("FUNC f AS Integer\n  RETURN g(1, 2\nEND FUNC\n").is_err());
    // Constructor with positional + named args parses.
    let json =
        project_json("FUNC f AS Integer\n  LET p = Point[1, y := 2]\n  RETURN 0\nEND FUNC\n");
    assert!(json.contains("\"kind\": \"constructor\""));
    assert!(json.contains("\"kind\": \"named\""));
}

#[test]
fn list_and_map_literal_forms() {
    // Empty list literal.
    let json = project_json("FUNC f AS Integer\n  LET xs = []\n  RETURN 0\nEND FUNC\n");
    assert!(json.contains("\"kind\": \"list\", \"values\": []"));
    // Unterminated list.
    assert!(try_parse("FUNC f AS Integer\n  RETURN [1, 2\nEND FUNC\n").is_err());
    // Map literal with a missing := between key and value.
    assert!(try_parse(
        "FUNC f AS Integer\n  RETURN Map OF String TO Integer { \"a\" 1 }\nEND FUNC\n"
    )
    .is_err());
    // Multi-entry map literal (comma loop).
    let json = project_json(
        "FUNC f AS Integer\n  LET m = Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }\n  RETURN 0\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"map\""));
}

// ---------------------------------------------------------------------------
// items.rs — malformed ABI / FREE-ABI / native param error paths.
// ---------------------------------------------------------------------------

fn link_fn(body: &str) -> String {
    format!("LINK \"x\" AS l\n  FUNC f() AS Integer\n{body}  END FUNC\nEND LINK\n")
}

#[test]
fn abi_spec_malformed_paths() {
    // ABI missing opening `(`.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI a CInt32\n")).is_err());
    // ABI missing closing `)`.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI (a CInt32\n")).is_err());
    // ABI missing AS after slot list.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI (a CInt32) r CInt32\n")).is_err());
    // ABI slot missing its C type.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI (a)\n")).is_err());
    // A native FUNC param list left unclosed.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f(a AS Integer\n    SYMBOL \"s\"\n  END FUNC\nEND LINK\n"
    )
    .is_err());
}

#[test]
fn free_block_malformed_abi_paths() {
    let head = "LINK \"x\" AS l\n  FUNC f() AS RES Db\n    SYMBOL \"s\"\n    ABI (return OUT CPtr) AS r CInt32\n";
    let tail = "  END FUNC\nEND LINK\n";
    // FREE ABI missing `(`.
    assert!(try_parse(&format!(
        "{head}    FREE return\n      SYMBOL \"g\"\n      ABI ptr CPtr\n    END FREE\n{tail}"
    ))
    .is_err());
    // FREE ABI missing `)`.
    assert!(try_parse(&format!(
        "{head}    FREE return\n      SYMBOL \"g\"\n      ABI (ptr CPtr\n    END FREE\n{tail}"
    ))
    .is_err());
    // FREE ABI missing AS.
    assert!(try_parse(&format!(
        "{head}    FREE return\n      SYMBOL \"g\"\n      ABI (ptr CPtr) CVoid\n    END FREE\n{tail}"
    ))
    .is_err());
    // FREE missing SYMBOL yields `free = None` silently (symbol? short-circuit),
    // so the enclosing native FUNC still parses.
    assert!(try_parse(&format!(
        "{head}    FREE return\n      ABI (ptr CPtr) AS CVoid\n    END FREE\n{tail}"
    ))
    .is_ok());
    // FREE missing ABI likewise yields `free = None` (param? short-circuit).
    assert!(try_parse(&format!(
        "{head}    FREE return\n      SYMBOL \"g\"\n    END FREE\n{tail}"
    ))
    .is_ok());
}

#[test]
fn const_pin_errors() {
    // CONST with no slot name.
    assert!(try_parse(&link_fn(
        "    SYMBOL \"s\"\n    ABI () AS r CInt32\n    CONST = 1\n"
    ))
    .is_err());
    // CONST with no `=`.
    assert!(try_parse(&link_fn(
        "    SYMBOL \"s\"\n    ABI () AS r CInt32\n    CONST a 1\n"
    ))
    .is_err());
}

#[test]
fn doc_block_bare_prose_keyword_flushes() {
    // A bare `DESC` line (no text) flushes the current prose block; the following
    // `DESC text` starts a fresh one.
    let src = r#"
DOC
FUNC f
DESC one
DESC
DESC two
END DOC
FUNC f AS Integer
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    // Two separate desc entries (the bare DESC split them).
    assert!(json.contains("\"text\": \"one\""));
    assert!(json.contains("\"text\": \"two\""));
}

// ---------------------------------------------------------------------------
// stmt.rs — inline-trap propagation, match/for/do edge and error paths.
// ---------------------------------------------------------------------------

#[test]
fn inline_trap_error_paths() {
    // A bare-expression statement whose inline TRAP has no binding.
    assert!(try_parse("FUNC f\n  notify() TRAP()\n  END TRAP\nEND FUNC\n").is_err());
    // An inline TRAP missing its closing `)`.
    assert!(try_parse("FUNC f\n  LET x = risky() TRAP(e\n  END TRAP\nEND FUNC\n").is_err());
    // An inline TRAP that is well-formed on a bare expression statement.
    let json = project_json("FUNC f\n  risky() TRAP(e)\n    notify()\n  END TRAP\nEND FUNC\n");
    assert!(json.contains("\"kind\": \"trapped\""));
    // A state assignment with an inline trap.
    let json = project_json(
        "FUNC f(RES h AS File STATE Integer)\n  h.state = risky() TRAP(e)\n    notify()\n  END TRAP\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"stateAssignment\""));
}

#[test]
fn match_union_pattern_errors() {
    // Union CASE pattern missing `(`.
    assert!(
        try_parse("FUNC f\n  MATCH x\n    CASE Shape y\n      z = 1\n  END MATCH\nEND FUNC\n")
            .is_err()
    );
    // Union CASE pattern missing closing `)`.
    assert!(
        try_parse("FUNC f\n  MATCH x\n    CASE Shape(y\n      z = 1\n  END MATCH\nEND FUNC\n")
            .is_err()
    );
    // MATCH not closed with END MATCH.
    assert!(try_parse("FUNC f\n  MATCH x\n    CASE 1\n      z = 1\nEND FUNC\n").is_err());
}

#[test]
fn if_elseif_error_paths() {
    // ELSEIF with a bad condition.
    assert!(try_parse(
        "FUNC f\n  IF a THEN\n    x = 1\n  ELSEIF *\n    x = 2\n  END IF\nEND FUNC\n"
    )
    .is_err());
    // ELSEIF missing THEN.
    assert!(try_parse(
        "FUNC f\n  IF a THEN\n    x = 1\n  ELSEIF b\n    x = 2\n  END IF\nEND FUNC\n"
    )
    .is_err());
    // A full IF / ELSEIF / ELSE chain parses.
    let json = project_json(
        "FUNC f\n  IF a THEN\n    x = 1\n  ELSEIF b THEN\n    x = 2\n  ELSE\n    x = 3\n  END IF\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"if\""));
}

#[test]
fn loop_close_errors() {
    // FOR block not closed with NEXT.
    assert!(try_parse("FUNC f\n  FOR i = 1 TO 2\n    x = 1\nEND FUNC\n").is_err());
    // FOR EACH block not closed with NEXT.
    assert!(try_parse("FUNC f\n  FOR EACH i IN xs\n    x = 1\nEND FUNC\n").is_err());
    // DO WHILE block not closed with LOOP.
    assert!(try_parse("FUNC f\n  DO WHILE a\n    x = 1\nEND FUNC\n").is_err());
    // DO block closed with LOOP but missing UNTIL.
    assert!(try_parse("FUNC f\n  DO\n    x = 1\n  LOOP\nEND FUNC\n").is_err());
}

#[test]
fn enum_multiple_member_lines() {
    // Two separate member lines exercise the enum-member loop across statements.
    let json = project_json("ENUM E\n  A, B\n  C\nEND ENUM\n");
    assert!(json.contains("\"name\": \"A\""));
    assert!(json.contains("\"name\": \"C\""));
}

#[test]
fn pipeline_placeholder_each_expression_kind_as_rhs() {
    // Each pipeline RHS is a *single* expression whose sole placeholder forces
    // contains_placeholder to visit that specific match arm (they short-circuit,
    // so one placeholder per RHS is required to reach every arm).
    let cases = [
        "x |> -_",                                   // Unary
        "x |> _ + 1",                                // Binary
        "x |> [_]",                                  // ListLiteral
        "x |> Point[_]",                             // Constructor
        "x |> LAMBDA() -> _",                        // Lambda
        "x |> Map OF Integer TO Integer { 1 := _ }", // MapLiteral (value)
        "x |> Map OF Integer TO Integer { _ := 1 }", // MapLiteral (key)
        "x |> _.field",                              // MemberAccess
        "x |> WITH base { z := _ }",                 // WithUpdate value
        "x |> g(_)",                                 // Call positional
        "x |> g(k := _)",                            // Call named
        "x |> Point[k := _]",                        // Constructor named
    ];
    for rhs in cases {
        // Wrap in parentheses so a placeholder that would otherwise be the final
        // token on the line is not consumed as a `_` line-continuation marker.
        let src = format!("FUNC f AS Integer\n  RETURN ({rhs})\nEND FUNC\n");
        assert!(try_parse(&src).is_ok(), "failed to parse: {rhs}");
    }
}

#[test]
fn lambda_and_function_type_close_errors() {
    // Lambda missing `(`.
    assert!(try_parse("FUNC f AS Integer\n  LET g = LAMBDA -> 1\n  RETURN 0\nEND FUNC\n").is_err());
    // Lambda missing `)`.
    assert!(
        try_parse("FUNC f AS Integer\n  LET g = LAMBDA(x -> 1\n  RETURN 0\nEND FUNC\n").is_err()
    );
    // Lambda missing `->`.
    assert!(try_parse("FUNC f AS Integer\n  LET g = LAMBDA(x) 1\n  RETURN 0\nEND FUNC\n").is_err());
    // Function type missing the closing `)` of its parameter list.
    assert!(try_parse("SUB s(f AS FUNC(Integer AS Boolean)\nEND SUB\n").is_err());
}

#[test]
fn native_symbol_must_be_string_and_name_required() {
    // SYMBOL with a non-string operand.
    assert!(try_parse(&link_fn("    SYMBOL 123\n    ABI () AS r CInt32\n")).is_err());
    // A native FUNC with no name (FUNC immediately followed by `(`).
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC (a AS Integer)\n    SYMBOL \"s\"\n    ABI (a CInt32) AS r CInt32\n  END FUNC\nEND LINK\n"
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// manifest.rs — canonicalize/read failure diagnostic paths.
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn broken_symlink_root_reports_canonicalize_failure() {
    let root = test_temp_dir("broken_symlink_root_reports_canonicalize_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("dir");
    // A source root that is a dangling symlink: exists() is true (the link),
    // but canonicalize() fails on the missing target.
    symlink(project_dir.join("missing_target"), project_dir.join("src")).expect("symlink");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[test]
fn unreadable_source_file_reports_read_failure() {
    use std::os::unix::fs::PermissionsExt;
    let root = test_temp_dir("unreadable_source_file_reports_read_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    let file = project_dir.join("src/main.mfb");
    fs::write(&file, "SUB main\nEND SUB\n").expect("write");
    // Remove all read permission so parse_file's read_to_string fails.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("chmod");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let result = parse_project("demo", &project_dir, &manifest);
    // Restore permissions so cleanup can proceed regardless of the outcome.
    let _ = fs::set_permissions(&file, fs::Permissions::from_mode(0o644));
    assert!(result.is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[test]
fn symlink_cycle_within_project_is_skipped() {
    let root = test_temp_dir("symlink_cycle_within_project_is_skipped");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("main");
    // A symlink pointing back at its own parent creates a cycle; the visited-dirs
    // guard must break it rather than recursing forever.
    symlink(project_dir.join("src"), project_dir.join("src/loop")).expect("symlink");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 1);
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn qualified_numeric_member_edge_cases() {
    // A `::` member that is a bare number not followed by an adjacent identifier
    // falls back to consume_name_or_keyword, which rejects the number.
    assert!(try_parse("FUNC f AS Integer\n  RETURN pkg::2 + 1\nEND FUNC\n").is_err());
    // A number and identifier separated by whitespace are not fused (not adjacent).
    assert!(try_parse("FUNC f AS Integer\n  RETURN pkg::2 b\nEND FUNC\n").is_err());
}

#[test]
fn all_comparison_and_arithmetic_operators_serialize() {
    let src = r#"
FUNC f AS Boolean
  LET a = x <= y
  LET b = x >= y
  LET c = x <> y
  LET d = x < y
  LET e = x > y
  LET g = x = y
  LET h = x * y / z
  LET i = -x
  RETURN a
END FUNC
"#;
    let json = project_json(src);
    for op in ["<=", ">=", "<>", "<", ">", "=", "*", "/"] {
        assert!(
            json.contains(&format!("\"operator\": \"{op}\"")),
            "missing operator {op}"
        );
    }
}

#[test]
fn doc_example_dedent_handles_multibyte_whitespace() {
    // bug-19: `dedent` measured indentation in BYTES but sliced every line at the
    // byte minimum. `trim_start` is Unicode-whitespace-aware, so an EXAMPLE mixing
    // a one-byte space with a two-byte NBSP (U+00A0) put the minimum (1) inside
    // the NBSP line's first char and panicked "byte index 1 is not a char
    // boundary", aborting the whole compile. Indentation is now a CHAR prefix.
    let src = "DOC\nFUNC foo()\nEXAMPLE\n a\n\u{a0}\u{a0}b\nEND EXAMPLE\nEND DOC\n\nSUB foo()\nEND SUB\n";
    let json = project_json(src);
    // One char stripped from each line: " a" -> "a", "\u{a0}\u{a0}b" -> "\u{a0}b".
    assert!(json.contains("\"example\""), "doc block parsed: {json}");
    assert!(json.contains("a\\n\u{a0}b"), "dedent by one char: {json}");

    // All-ASCII indentation is unchanged (the overwhelmingly common case).
    let ascii = project_json("DOC\nFUNC foo()\nEXAMPLE\n    a\n      b\nEND EXAMPLE\nEND DOC\n\nSUB foo()\nEND SUB\n");
    assert!(ascii.contains("a\\n  b"), "ascii dedent: {ascii}");
}
