//! plan-176-K: `ast::tests` — the expressions_and_types tests. The shared `parse_ok`/`parse_err` helpers\n//! and the project/source-root tests live in `ast/tests/mod.rs`.

use super::*;

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
    assert_eq!(*operator, BinaryOp::Concat);
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
    assert_eq!(*add_operator, BinaryOp::Add);
    assert!(matches!(&**add_left, Expression::Identifier(name) if name == "b"));
    assert!(matches!(&**add_right, Expression::Identifier(name) if name == "c"));
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
    assert_eq!(operator, BinaryOp::Or);
    let Expression::Binary { operator, .. } = expr_of("a XOR b") else {
        panic!("xor");
    };
    assert_eq!(operator, BinaryOp::Xor);
    // AND
    let Expression::Binary { operator, .. } = expr_of("a AND b") else {
        panic!("and");
    };
    assert_eq!(operator, BinaryOp::And);
    // NOT (unary, right-recursive)
    let Expression::Unary { operator, .. } = expr_of("NOT NOT a") else {
        panic!("not");
    };
    assert_eq!(operator, UnaryOp::Not);
}

#[test]
fn parses_every_comparison_operator() {
    for (src, op) in [
        ("a = b", BinaryOp::Equal),
        ("a <> b", BinaryOp::NotEqual),
        ("a < b", BinaryOp::Less),
        ("a <= b", BinaryOp::LessEqual),
        ("a > b", BinaryOp::Greater),
        ("a >= b", BinaryOp::GreaterEqual),
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
        ("a + b", BinaryOp::Add),
        ("a - b", BinaryOp::Subtract),
        ("a * b", BinaryOp::Multiply),
        ("a / b", BinaryOp::Divide),
        ("a MOD b", BinaryOp::Mod),
        ("a DIV b", BinaryOp::IntDiv),
        ("a & b", BinaryOp::Concat),
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
    assert_eq!(operator, BinaryOp::Power);
    // Right-associative: right side is itself a power binary.
    assert!(matches!(&*right, Expression::Binary { operator, .. } if *operator == BinaryOp::Power));
    let Expression::Unary { operator, .. } = expr_of("-a") else {
        panic!("unary minus");
    };
    assert_eq!(operator, UnaryOp::Negate);
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
    assert!(matches!(&*body, Expression::Binary { operator, .. } if *operator == BinaryOp::Add));
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
    let src = format!("FUNC main AS Nothing\n  LET v AS {type_name} = NOTHING\nEND FUNC\n");
    let file = parse_ok(&src);
    let Item::Function(function) = &file.items[0] else {
        panic!("expected function");
    };
    let Statement::Let { type_name, .. } = &function.body[0] else {
        panic!("expected LET");
    };
    type_name.clone().expect("type name present")
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
fn parses_res_collection_element_state_clause() {
    // bug-427: a `RES` collection element may carry a uniform `STATE T` clause
    // (a stateful resource union), folded into the element type string exactly
    // as the thread resource plane does — the STATE rides the element, not the
    // binding, so an extracted element can read `.state`.
    assert_eq!(
        type_of("List OF RES File STATE Cursor"),
        "List OF RES File STATE Cursor"
    );
    assert_eq!(
        type_of("Map OF String TO RES File STATE Cursor"),
        "Map OF String TO RES File STATE Cursor"
    );
}

#[test]
fn state_clause_requires_res_collection_element() {
    // A `STATE` clause is only meaningful on a `RES` element; after a bare
    // (no-`RES`) element it is a parse error, not a dangling token (bug-427).
    type_err("List OF File STATE Cursor");
    type_err("Map OF String TO File STATE Cursor");
}

#[test]
fn map_type_requires_to_keyword() {
    type_err("Map OF String Integer");
}

#[test]
fn parses_set_type_variants() {
    // `Set OF T` is single-element (like `List`), round-tripping byte-identically.
    assert_eq!(type_of("Set OF Integer"), "Set OF Integer");
    assert_eq!(type_of("Set OF String"), "Set OF String");
    // A nested collection element parses structurally here (the comparability
    // rejection is ir::verify's job, not the parser's).
    assert_eq!(type_of("Set OF List OF Integer"), "Set OF List OF Integer");
}

#[test]
fn set_type_rejects_res_element() {
    // A Set element can never be a resource (`RES` is a parse error after `Set OF`).
    type_err("Set OF RES File");
}

#[test]
fn parses_template_type_with_multiple_args() {
    // bug-833: a multi-argument list is parenthesized, and the AST keeps it.
    assert_eq!(
        type_of("Pair OF (Integer, String)"),
        "Pair OF (Integer, String)"
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
        param_type("FUNC s(t AS Thread OF Msg RES Handle TO Out) AS Nothing\nEND FUNC\n"),
        "Thread OF Msg RES Handle TO Out"
    );
    // Thread resource-only (message defaults to Nothing).
    assert_eq!(
        param_type("FUNC s(t AS Thread OF RES Handle TO Out) AS Nothing\nEND FUNC\n"),
        "Thread OF RES Handle TO Out"
    );
    // Thread with just message + output.
    assert_eq!(
        param_type("FUNC s(t AS Thread OF Msg TO Out) AS Nothing\nEND FUNC\n"),
        "Thread OF Msg TO Out"
    );
    // ThreadWorker canonicalizes.
    assert_eq!(
        param_type("FUNC s(t AS ThreadWorker OF Msg TO Out) AS Nothing\nEND FUNC\n"),
        "ThreadWorker OF Msg TO Out"
    );
    // Map type with RES value.
    assert_eq!(
        param_type("FUNC s(m AS Map OF String TO RES File) AS Nothing\nEND FUNC\n"),
        "Map OF String TO RES File"
    );
    // List of RES.
    assert_eq!(
        param_type("FUNC s(xs AS List OF RES File) AS Nothing\nEND FUNC\n"),
        "List OF RES File"
    );
    // Multi-arg template type.
    assert_eq!(
        param_type("FUNC s(p AS Pair OF (Integer, String)) AS Nothing\nEND FUNC\n"),
        "Pair OF (Integer, String)"
    );
    // Function type.
    assert_eq!(
        param_type("FUNC s(f AS FUNC(Integer, String) AS Boolean) AS Nothing\nEND FUNC\n"),
        "FUNC(Integer, String) AS Boolean"
    );
    // ISOLATED function type.
    assert_eq!(
        param_type("FUNC s(f AS ISOLATED FUNC() AS Integer) AS Nothing\nEND FUNC\n"),
        "ISOLATED FUNC() AS Integer"
    );
    // Grouped type.
    assert_eq!(
        param_type("FUNC s(f AS (Integer)) AS Nothing\nEND FUNC\n"),
        "(Integer)"
    );
    // Nothing base type.
    assert_eq!(
        param_type("FUNC s(f AS FUNC() AS Nothing) AS Nothing\nEND FUNC\n"),
        "FUNC() AS Nothing"
    );
}

#[test]
fn type_name_errors() {
    // Map type missing TO.
    assert!(try_parse("FUNC s(m AS Map OF String Integer) AS Nothing\nEND FUNC\n").is_err());
    // Thread type missing TO.
    assert!(try_parse("FUNC s(t AS Thread OF Msg Out) AS Nothing\nEND FUNC\n").is_err());
    // ISOLATED not followed by FUNC.
    assert!(try_parse("FUNC s(f AS ISOLATED Integer) AS Nothing\nEND FUNC\n").is_err());
    // Function type missing (.
    assert!(try_parse("FUNC s(f AS FUNC Integer) AS Nothing\nEND FUNC\n").is_err());
    // Function type missing AS.
    assert!(try_parse("FUNC s(f AS FUNC(Integer)) AS Nothing\nEND FUNC\n").is_err());
    // Grouped type missing ).
    assert!(try_parse("FUNC s(f AS (Integer)\nEND FUNC\n").is_err());
    // A bare type where a type name is required but token is bad.
    assert!(try_parse("FUNC s(f AS 123) AS Nothing\nEND FUNC\n").is_err());
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
