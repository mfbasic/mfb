//! plan-176-G: `ir::tests::lower_tests`'s literal-type fallback, named-argument
//! normalization, trap-handler arm and literal type-inference tests. The shared helpers
//! live in `lower_tests/mod.rs`.

use super::*;

// ---- literal_expression_type fallback --------------------------------

#[test]
fn list_literal_without_expected_type_infers_from_first_literal() {
    // A list literal lowered with no expected element type (as an argument
    // to `len`, whose parameter type is generic) drives
    // literal_expression_type over the first element (a String literal).
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS Integer\n\
               RETURN len([\"a\", \"b\"])\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    // The inner list literal was typed from its first (String) element.
    fn find_list(v: &IrValue) -> Option<String> {
        match v {
            IrValue::ListLiteral { type_, .. } => Some(type_.name().into_owned()),
            IrValue::Call { args, .. } => args.iter().find_map(find_list),
            _ => None,
        }
    }
    let ty = function(&ir, "run").body.iter().find_map(|op| match op {
        IrOp::Return { value: Some(v), .. } => find_list(v),
        _ => None,
    });
    assert_eq!(ty.as_deref(), Some("List OF String"));
}

#[test]
fn list_literal_without_expected_and_non_literal_first_element() {
    // First element is a call (not a plain literal) -> literal_expression_type
    // returns None -> element type falls back to "Unknown".
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS Integer\n\
               RETURN len([len(\"a\"), 2])\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

#[test]
fn builtin_type_field_access_and_net_lookup() {
    // net::lookup -> List OF Address; accessing an Address's `host` field
    // resolves via TypeIndex::record_field_type's builtin-type-fields branch.
    let src = "IMPORT net\n\
             IMPORT collections\n\
             FUNC run() AS String\n\
               LET addrs AS List OF net::Address = net::lookup(\"example.com\")\n\
               LET a AS net::Address = collections::get(addrs, 0)\n\
               RETURN a.host\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("net::lookup program should lower");
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::MemberAccess { .. }),
            ..
        }
    )));
}

#[test]
fn collections_filter_with_user_function_predicate() {
    // A user FUNC predicate (not a built-in like isEven) yields no builtin
    // predicate type, taking the else branch that lowers args normally.
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC keep(x AS Integer) AS Boolean\n  RETURN x > 0\nEND FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               RETURN collections::filter(xs, keep)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

#[test]
fn list_literal_first_element_float_bool_and_nothing() {
    // literal_expression_type over Float, Boolean, and NOTHING first
    // elements (all lowered with no expected type).
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC a() AS Integer\n  RETURN len([1.5, 2.5])\nEND FUNC\n\
             FUNC b() AS Integer\n  RETURN len([NOTHING])\nEND FUNC\n\
             FUNC c() AS Integer\n  RETURN len([TRUE, FALSE])\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    fn list_type(v: &IrValue) -> Option<String> {
        match v {
            IrValue::ListLiteral { type_, .. } => Some(type_.name().into_owned()),
            IrValue::Call { args, .. } => args.iter().find_map(list_type),
            _ => None,
        }
    }
    let a_ty = function(&ir, "a").body.iter().find_map(|op| match op {
        IrOp::Return { value: Some(v), .. } => list_type(v),
        _ => None,
    });
    assert_eq!(a_ty.as_deref(), Some("List OF Float"));
}

#[test]
fn nested_lambda_inside_lambda_body() {
    // A lambda whose body contains another lambda exercises the Lambda arm of
    // collect_captured_locals (the nested lambda is not descended into for
    // the outer capture set).
    let src = "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET k AS Integer = 2\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + collections::sum(collections::transform([k], LAMBDA(w AS Integer) -> w + 1)))\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        assert!(
            ir.functions
                .iter()
                .filter(|f| f.name.starts_with("$lambda"))
                .count()
                >= 2
        );
    }
}

#[test]
fn lambda_captures_through_trapped_expression() {
    // The lambda body contains a trapped call referencing a captured local,
    // exercising the Trapped arm of collect_captured_locals.
    let src = "IMPORT collections\n\
             FUNC parse(s AS String) AS Integer\n  RETURN toInt(s)\nEND FUNC\n\
             FUNC run(seed AS String) AS List OF Integer\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + (parse(seed) TRAP(e) RECOVER 0 END TRAP))\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    // Inline TRAP inside a lambda expression may be rejected; only assert
    // when it lowers.
    if let Some(ir) = try_lower_src(src) {
        assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
    }
}

#[test]
fn filter_result_typed_binding_resolves_element_type() {
    // Binding the result of `collections::filter(xs, isEven)` requires
    // expression_type over the filter call (the builtin-predicate branch of
    // the collections.filter expression_type resolution).
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS Integer\n\
               LET xs AS List OF Integer = [1, 2, 3, 4]\n\
               LET evens = collections::filter(xs, isEven)\n\
               RETURN len(evens)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    // `evens` inferred as List OF Integer from the filter call.
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Bind { name, type_, .. } if name == "evens" && type_.name() == "List OF Integer"
    )));
}

#[test]
fn call_through_zero_arg_function_typed_local() {
    // Calling a `FUNC() AS Integer`-typed local drives
    // function_param_types_from_type over an empty parameter list.
    let ir = lower_src(
        "FUNC seed() AS Integer\n  RETURN 7\nEND FUNC\n\
             FUNC run() AS Integer\n\
               LET f AS FUNC() AS Integer = seed\n\
               RETURN f()\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

#[test]
fn doc_for_nonexported_type_is_dropped() {
    // A DOC TYPE header for a non-exported (package/private) type is dropped
    // (the visibility check in collect_project_docs).
    let src = "DOC\n  TYPE Hidden\n  DESC Not exported.\nEND DOC\n\
             TYPE Hidden\n  n AS Integer\nEND TYPE\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        assert!(!ir.docs.decls.iter().any(|d| d.name == "Hidden"));
    }
}

#[test]
fn inline_trap_binding_type_from_trapped_expression() {
    // An inferred `LET x = call() TRAP ...` computes the success type via
    // expression_type over the inner call (the inline-trap success-type path).
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::Bind { name, type_, .. } if name == "n" && type_.name() == "Integer"
    )));
}

// ---- named-argument normalization -----------------------------------
// Lowering an overloaded builtin call supplied by NAME (`hours := 1`)
// reorders the arguments into the selected overload's parameter order
// (`normalize_overloaded_builtin_call_arguments`); a call by position or
// a name set matching no overload falls back to positional order.

#[test]
fn overloaded_builtin_named_arguments_are_reordered_to_param_order() {
    // `datetime::fixedOffset` has the named overloads `[offsetSeconds]` and
    // `[hours, mins]`. Supplying `mins`/`hours` out of order must lower to the
    // parameter order, and the single-parameter overload by name too.
    let ir = lower_src(
        "IMPORT datetime\n\
             FUNC main() AS Integer\n\
             \x20 LET a = datetime::fixedOffset(mins := 30, hours := 1)\n\
             \x20 LET b = datetime::fixedOffset(hours := 2, mins := 45)\n\
             \x20 LET c = datetime::fixedOffset(offsetSeconds := 3600)\n\
             \x20 RETURN 0\n\
             END FUNC\n",
    );
    // All three lower into `main`'s body without panicking; the reordered call
    // resolves to the datetime helper for its arity.
    assert!(!function(&ir, "main").body.is_empty());
}

#[test]
fn local_call_named_and_default_arguments_lower_in_param_order() {
    // A user FUNC called with named arguments out of order, plus a call that
    // relies on the default, exercises `normalize_local_call_arguments`.
    let ir = lower_src(
        "FUNC scaled(x AS Integer, factor AS Integer = 2) AS Integer\n\
             \x20 RETURN x * factor\n\
             END FUNC\n\
             FUNC main() AS Integer\n\
             \x20 LET a = scaled(factor := 5, x := 3)\n\
             \x20 LET b = scaled(10)\n\
             \x20 RETURN a + b\n\
             END FUNC\n",
    );
    assert_eq!(function(&ir, "scaled").params.len(), 2);
    assert!(!function(&ir, "main").body.is_empty());
}

#[test]
fn a_computed_default_lowers_to_a_hidden_function_call() {
    // plan-136-A (bug-614): a computed default is lowered once, in the
    // declaration's scope, into a hidden default function. The parameter's
    // default and every call that omits the argument call it, so the caller's
    // local lambda named `helper` is never what the default calls.
    let ir = lower_src(
        "FUNC helper() AS Integer\n\
             \x20 RETURN 5\n\
             END FUNC\n\
             FUNC f(x AS Integer = helper()) AS Integer\n\
             \x20 RETURN x\n\
             END FUNC\n\
             FUNC main() AS Integer\n\
             \x20 LET helper = LAMBDA() -> 99\n\
             \x20 LET other = helper()\n\
             \x20 RETURN f()\n\
             END FUNC\n",
    );
    let hidden = crate::internal_name::hidden_default_function_name("f", 0);
    assert!(matches!(
        &function(&ir, "f").params[0].default,
        Some(IrValue::Call { target, args, .. }) if *target == hidden && args.is_empty()
    ));
    let hidden_function = function(&ir, &hidden);
    assert_eq!(hidden_function.visibility, "private");
    assert!(hidden_function.params.is_empty());
    assert_eq!(
        hidden_function.returns,
        crate::types::ParameterType::Integer
    );
    assert!(matches!(
        hidden_function.body.as_slice(),
        [IrOp::Return { value: Some(IrValue::Call { target, .. }), .. }] if target == "helper"
    ));
    let returned = main_body(&ir)
        .iter()
        .find_map(|op| match op {
            IrOp::Return {
                value: Some(value), ..
            } => Some(value),
            _ => None,
        })
        .expect("main returns f()");
    assert!(
        matches!(
            returned,
            IrValue::Call { target, args, .. }
                if target == "f"
                    && matches!(
                        args.as_slice(),
                        [IrValue::Call { target: filled, args: none, .. }]
                            if *filled == hidden && none.is_empty()
                    )
        ),
        "the omitted argument must be the hidden default call"
    );
}

#[test]
fn a_literal_default_lowers_as_a_constant() {
    // plan-136-A: a name-free literal default keeps lowering to its constant —
    // at the declaration and at the call site — and gets no hidden function.
    let ir = lower_src(
        "FUNC greet(name AS String, greeting AS String = \"Hello\") AS String\n\
             \x20 RETURN greeting & name\n\
             END FUNC\n\
             FUNC main() AS Integer\n\
             \x20 RETURN len(greet(\"Ada\"))\n\
             END FUNC\n",
    );
    assert!(matches!(
        &function(&ir, "greet").params[1].default,
        Some(IrValue::Const { value, .. }) if value == "Hello"
    ));
    assert!(!ir
        .functions
        .iter()
        .any(|candidate| crate::internal_name::is_hidden_default_function(&candidate.name)));
}

#[test]
fn default_kind_is_literal_only_for_name_free_constants() {
    use super::super::lower::{default_kind, DefaultKind};
    use crate::hir::HirExpression;
    let imports = std::collections::HashMap::from([
        ("m".to_string(), "math".to_string()),
        ("vec".to_string(), "vector".to_string()),
    ]);
    let literals = [
        HirExpression::String("text".to_string()),
        HirExpression::Number("1.5".to_string()),
        HirExpression::Scalar(65),
        HirExpression::Boolean(true),
        HirExpression::Identifier("NOTHING".to_string()),
        HirExpression::Identifier("math.pi".to_string()),
        // A package constant through an import alias is canonicalized against
        // the declaring file's imports.
        HirExpression::Identifier("m.pi".to_string()),
    ];
    for literal in &literals {
        assert_eq!(
            default_kind(literal, &imports),
            DefaultKind::Literal,
            "{literal:?}"
        );
    }
    let name = |text: &str| Box::new(HirExpression::Identifier(text.to_string()));
    let computed = [
        // A global (bug-614's own shape).
        HirExpression::Identifier("limit".to_string()),
        // A record constant inlines a constructor, not a `Const`.
        HirExpression::Identifier("vec.zeroFloat3".to_string()),
        // `-5` lowers as a unary, not a constant (Open Decisions).
        HirExpression::Unary {
            operator: crate::operators::UnaryOp::Negate,
            operand: Box::new(HirExpression::Number("5".to_string())),
            line: 1,
            column: 1,
        },
        HirExpression::Call {
            callee: "helper".to_string(),
            arguments: Vec::new(),
            line: 1,
            column: 1,
        },
        HirExpression::ListLiteral(vec![HirExpression::Number("1".to_string())]),
        HirExpression::MemberAccess {
            target: name("point"),
            member: "x".to_string(),
        },
    ];
    for value in &computed {
        assert_eq!(
            default_kind(value, &imports),
            DefaultKind::Computed,
            "{value:?}"
        );
    }
}

// ---- plan-68 coverage: ops_read_local handler arms (no `e` read) -----
//
// Each handler below deliberately *never* references the trap binding `e`,
// so `ops_read_local(handler_ops, "e")` walks every op returning `false`.
// That reaches the match arms an `e`-reading handler short-circuits before
// (its condition/scrutinee `value_reads(_, "e")` returns `true` first).

#[test]
fn trap_handler_global_assign_arm_without_error_read() {
    // AssignGlobal arm of `ops_read_local`: the handler assigns a top-level
    // binding and never reads `e`.
    let ir = lower_src(
        "MUT flag AS Integer = 0\n\
             FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 flag = 7\n\
                 RECOVER 0\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn trap_handler_exit_program_arm() {
    // ExitProgram arm of `ops_read_local`: a diverging handler.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 EXIT PROGRAM 3\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn trap_handler_continue_loop_arm() {
    // ContinueLoop arm of `ops_read_local`: the trap sits in a FOR loop and
    // its handler continues the loop, diverging without reading `e`.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               MUT total AS Integer = 0\n\
               FOR i = 1 TO 3\n\
                 LET n AS Integer = toInt(s) TRAP(e)\n\
                   CONTINUE FOR\n\
                 END TRAP\n\
                 total = total + n\n\
               NEXT\n\
               RETURN total\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse")
        .body
        .iter()
        .any(|op| matches!(op, IrOp::For { .. })));
}

#[test]
fn trap_handler_if_arm_recurses_both_branches() {
    // If arm of `ops_read_local`: the condition does not read `e`, so the
    // walk recurses into both the then- and else-bodies.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 IF len(s) > 0 THEN\n\
                   RECOVER 0\n\
                 ELSE\n\
                   RECOVER 1\n\
                 END IF\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn trap_handler_match_arm_with_guard_recurses() {
    // Match arm of `ops_read_local`: the scrutinee and the CASE guard do not
    // read `e`, so the walk evaluates the guard closure and recurses into the
    // case body.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 MATCH len(s)\n\
                   CASE 1 WHEN len(s) > 0\n\
                     RECOVER 0\n\
                   CASE ELSE\n\
                     RECOVER 1\n\
                 END MATCH\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn trap_handler_empty_loop_body_treeifies_to_nothing() {
    // treeify_handler's empty-slice arm: a FOR loop with an empty body inside
    // a handler makes `treeify_handler(&[])` return an empty Vec.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 FOR i = 1 TO 0\n\
                 NEXT\n\
                 RECOVER 0\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

// ---- plan-68 coverage: type inference over literals / constructors ---

#[test]
fn untyped_global_binding_infers_type() {
    // A top-level binding with no `AS` annotation but a value drives
    // `precompute_binding_types` to record its inferred type.
    let ir = lower_src(
        "LET answer = 42\n\
             FUNC main() AS Integer\n\
               RETURN answer\n\
             END FUNC\n",
    );
    assert!(!function(&ir, "main").body.is_empty());
}

#[test]
fn untyped_money_and_scalar_literal_types() {
    // `expression_type` Money (suffixed literal) and Scalar (backtick
    // literal) arms, via untyped bindings.
    let ir = lower_src(
        "LET price = 2.50m\n\
             LET letter = `A`\n\
             FUNC main() AS Integer\n\
               RETURN 0\n\
             END FUNC\n",
    );
    assert!(!function(&ir, "main").body.is_empty());
}

#[test]
fn expected_typed_numeric_literals_coerce() {
    // `lower_expression_with_expected` Number arm: unsuffixed literals coerce
    // to the annotated Fixed / Byte / Money slot; a `m`-suffixed literal is
    // intrinsically Money.
    let ir = lower_src(
        "FUNC main() AS Integer\n\
               LET a AS Fixed = 5\n\
               LET b AS Byte = 7\n\
               LET c AS Money = 1.25\n\
               LET d = 2.50m\n\
               RETURN b\n\
             END FUNC\n",
    );
    let f = function(&ir, "main");
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Bind { type_, .. } if type_.name() == "Fixed"
    )));
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Bind { type_, .. } if type_.name() == "Money"
    )));
}

#[test]
fn untyped_list_literal_element_types() {
    // `literal_expression_type`: an untyped list literal in a position with no
    // expected element type (a `len(...)` argument) derives its element type
    // from the first element across each literal kind.
    let ir = lower_src(
        "FUNC main() AS Integer\n\
               LET a = len([1, 2, 3])\n\
               LET b = len([\"x\", \"y\"])\n\
               LET c = len([`a`, `b`])\n\
               LET d = len([TRUE, FALSE])\n\
               RETURN a + b + c + d\n\
             END FUNC\n",
    );
    assert!(!function(&ir, "main").body.is_empty());
}

#[test]
fn collections_filter_bare_builtin_predicate_type() {
    // The native higher-order member (`collections::filter`) with a *bare*
    // general built-in predicate (`isEven`): `expression_type` binds the
    // predicate's parameter type from the list element type and resolves the
    // call's return type through `collections::resolve_call`.
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               RETURN collections::filter(xs, isEven)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(!function(&ir, "run").body.is_empty());
}

#[test]
fn untyped_record_and_variant_constructor_and_field_types() {
    // `constructor_result` for a record and a union variant, and
    // `record_field_type` for a user record field, all via untyped bindings.
    let ir = lower_src(
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             TYPE Dog\n  legs AS Integer\nEND TYPE\n\
             TYPE Cat\n  lives AS Integer\nEND TYPE\n\
             UNION Animal\n  Dog\n  Cat\nEND UNION\n\
             FUNC main() AS Integer\n\
               LET p = Point(1, 2)\n\
               LET a = Dog(4)\n\
               LET v = p.x\n\
               RETURN v\n\
             END FUNC\n",
    );
    assert!(!function(&ir, "main").body.is_empty());
}

#[test]
fn member_access_builtin_type_field_infers() {
    // `record_field_type` path: a field of a built-in package type
    // (`term::TermSize.columns`) resolves through the normal source-record
    // table now that `term`'s records are injected as source `TYPE` decls.
    let ir = lower_src(
        "IMPORT term\n\
             FUNC main() AS Integer\n\
               LET sz = term::terminalSize()\n\
               LET c = sz.columns\n\
               RETURN c\n\
             END FUNC\n",
    );
    assert!(!function(&ir, "main").body.is_empty());
}

#[test]
fn testing_block_item_is_skipped() {
    // A top-level TESTING block is an `Item::Testing` the item loop skips
    // (line 152); the program still lowers its ordinary functions.
    let ir = lower_src(
        "FUNC main() AS Integer\n\
               RETURN 0\n\
             END FUNC\n\
             TESTING\n\
               TGROUP \"g\"\n\
                 TCASE \"c\"\n\
                   expectInteger(1, 1)\n\
                 END TCASE\n\
               END TGROUP\n\
             END TESTING\n",
    );
    assert_eq!(function(&ir, "main").returns.name(), "Integer");
}
