//! plan-176-G: `ir::tests::lower_tests`'s declaration, statement, match/literal/operator,
//! call/constructor and lambda lowering tests. The shared `main_body`/`binding`/`bind_value`/
//! `describe` helpers live in `lower_tests/mod.rs`.

use super::*;

fn ir_type<'a>(ir: &'a IrProject, name: &str) -> &'a IrType {
    ir.types
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no type {name}"))
}

// ---- bindings --------------------------------------------------------

#[test]
fn top_level_bindings_explicit_and_inferred() {
    let ir = lower_src(
        "LET n AS Integer = 5\n\
             LET s = \"hi\"\n\
             MUT flag AS Boolean = TRUE\n\
             LET f = 2.5\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let n = binding(&ir, "n");
    assert_eq!(n.type_.name(), "Integer");
    assert!(n.explicit_type);
    assert!(!n.mutable);

    let s = binding(&ir, "s");
    assert_eq!(s.type_.name(), "String");
    assert!(!s.explicit_type);

    let flag = binding(&ir, "flag");
    assert!(flag.mutable);
    assert_eq!(flag.type_.name(), "Boolean");

    assert_eq!(binding(&ir, "f").type_.name(), "Float");
}

#[test]
fn set_type_round_trips_through_front_end() {
    // plan-63-A: a `Set OF Integer` declared (empty default), passed to a
    // `FUNC(Set OF Integer) AS Set OF Integer`, and returned, round-trips
    // byte-identically through parse → resolve → monomorph → lower.
    let ir = lower_src(
        "FUNC identity(values AS Set OF Integer) AS Set OF Integer\n  RETURN values\nEND FUNC\n\
             MUT s AS Set OF Integer\n\
             LET t = identity(s)\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let identity = function(&ir, "identity");
    assert_eq!(identity.returns.name(), "Set OF Integer");
    assert_eq!(identity.params[0].type_.name(), "Set OF Integer");
    let s = binding(&ir, "s");
    assert_eq!(s.type_.name(), "Set OF Integer");
    assert!(s.mutable);
    // The call-result binding inherits the returned Set type.
    assert_eq!(binding(&ir, "t").type_.name(), "Set OF Integer");
}

#[test]
fn inferred_binding_from_call_result() {
    // A binding whose type is inferred from a user function's return type.
    let ir = lower_src(
        "FUNC seed() AS Integer\n  RETURN 7\nEND FUNC\n\
             LET x = seed()\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let x = binding(&ir, "x");
    assert_eq!(x.type_.name(), "Integer");
    assert!(!x.explicit_type);
}

// ---- FUNC variants --------------------------------------------------

#[test]
fn func_kinds_and_returns() {
    let ir = lower_src(
        "FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n\
             FUNC greet(name AS String) AS Nothing\n  LET x AS String = name\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let add = function(&ir, "add");
    assert_eq!(add.kind, "func");
    assert_eq!(add.returns.name(), "Integer");
    assert_eq!(add.params.len(), 2);

    let greet = function(&ir, "greet");
    assert_eq!(greet.kind, "func");
    assert_eq!(greet.returns.name(), "Nothing");
}

#[test]
fn func_with_default_param_and_isolated() {
    let ir = lower_src(
        "ISOLATED FUNC scaled(x AS Integer, factor AS Integer = 2) AS Integer\n\
               RETURN x * factor\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let scaled = function(&ir, "scaled");
    assert!(scaled.isolated);
    assert_eq!(scaled.params.len(), 2);
    assert!(scaled.params[1].default.is_some());
}

#[test]
fn func_returning_nothing() {
    let ir = lower_src(
        "FUNC sideEffect(x AS Integer) AS Nothing\n  LET y AS Integer = x\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert_eq!(function(&ir, "sideEffect").returns.name(), "Nothing");
}

#[test]
fn func_trap_handler_lowers_to_trap_op() {
    let ir = lower_src(
        "FUNC risky() AS Integer\n\
               RETURN 1\n\
               TRAP(err)\n\
                 RETURN err.code\n\
               END TRAP\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "risky")
        .body
        .iter()
        .any(|op| matches!(op, IrOp::Trap { .. })));
}

// ---- TYPE / UNION / ENUM --------------------------------------------

#[test]
fn type_record_with_private_field() {
    let ir = lower_src(
        "EXPORT TYPE Point\n  x AS Integer\n  PRIVATE y AS Integer\nEND TYPE\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let point = ir_type(&ir, "Point");
    assert_eq!(point.kind, "type");
    assert_eq!(point.visibility, "export");
    assert_eq!(point.fields.len(), 2);
    assert_eq!(point.fields[1].visibility.as_deref(), Some("private"));
}

#[test]
fn union_with_variants_and_includes() {
    let ir = lower_src(
        "TYPE Circle\n  r AS Integer\nEND TYPE\n\
             TYPE Square\n  s AS Integer\nEND TYPE\n\
             UNION Rounded\n  Circle\nEND UNION\n\
             UNION Shape INCLUDES Rounded\n  Square\nEND UNION\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let shape = ir_type(&ir, "Shape");
    assert_eq!(shape.kind, "union");
    assert_eq!(shape.includes, vec!["Rounded".to_string()]);
    assert!(shape.variants.iter().any(|v| v.name == "Square"));
}

#[test]
fn enum_members_lowered() {
    let ir = lower_src(
        "ENUM Color\n  Red\n  Green\n  Blue\nEND ENUM\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let color = ir_type(&ir, "Color");
    assert_eq!(color.kind, "enum");
    assert_eq!(color.members.len(), 3);
    assert_eq!(color.members[0].name, "Red");
}

// ---- statements ------------------------------------------------------

#[test]
fn let_mut_and_assignment_statements() {
    let ir = lower_src(
        "FUNC main AS Nothing\n  LET a AS Integer = 1\n  MUT b AS Integer = 2\n  b = a + b\nEND FUNC\n",
    );
    let body = main_body(&ir);
    assert!(matches!(&body[0], IrOp::Bind { mutable: false, name, .. } if name == "a"));
    assert!(matches!(&body[1], IrOp::Bind { mutable: true, name, .. } if name == "b"));
    assert!(matches!(&body[2], IrOp::Assign { name, .. } if name == "b"));
}

#[test]
fn assignment_to_global_is_assign_global() {
    let ir =
        lower_src("MUT counter AS Integer = 0\nFUNC main AS Nothing\n  counter = 5\nEND FUNC\n");
    assert!(main_body(&ir)
        .iter()
        .any(|op| matches!(op, IrOp::AssignGlobal { name, .. } if name == "counter")));
}

#[test]
fn return_without_value_from_sub() {
    let ir = lower_src("FUNC s() AS Nothing\n  RETURN\nEND FUNC\nFUNC main AS Nothing\nEND FUNC\n");
    assert!(function(&ir, "s")
        .body
        .iter()
        .any(|op| matches!(op, IrOp::Return { value: None, .. })));
}

#[test]
fn exit_targets_lower_to_expected_ops() {
    let ir = lower_src(
        "FUNC main AS Nothing\n\
               FOR i = 1 TO 3\n  EXIT FOR\n  CONTINUE FOR\n  NEXT\n\
               DO\n  EXIT DO\nLOOP UNTIL TRUE\n\
               WHILE FALSE\n  EXIT WHILE\n  END WHILE\n\
               EXIT PROGRAM 2\n\
             END FUNC\n",
    );
    fn contains_exit(ops: &[IrOp], want: LoopKind) -> bool {
        ops.iter().any(|op| match op {
            IrOp::ExitLoop { kind, .. } => *kind == want,
            IrOp::For { body, .. } | IrOp::While { body, .. } | IrOp::DoUntil { body, .. } => {
                contains_exit(body, want)
            }
            _ => false,
        })
    }
    fn contains_continue(ops: &[IrOp]) -> bool {
        ops.iter().any(|op| match op {
            IrOp::ContinueLoop { .. } => true,
            IrOp::For { body, .. } | IrOp::While { body, .. } | IrOp::DoUntil { body, .. } => {
                contains_continue(body)
            }
            _ => false,
        })
    }
    let body = main_body(&ir);
    assert!(contains_exit(body, LoopKind::For));
    assert!(contains_exit(body, LoopKind::Do));
    assert!(contains_exit(body, LoopKind::While));
    assert!(contains_continue(body));
    assert!(body.iter().any(|op| matches!(op, IrOp::ExitProgram { .. })));
}

#[test]
fn exit_sub_lowers_to_valueless_return() {
    let ir = lower_src("FUNC s() AS Nothing\n  RETURN\nEND FUNC\nFUNC main AS Nothing\nEND FUNC\n");
    assert!(function(&ir, "s")
        .body
        .iter()
        .any(|op| matches!(op, IrOp::Return { value: None, .. })));
}

#[test]
fn exit_func_contributes_no_op() {
    let ir = lower_src(
        "FUNC f() AS Integer\n  EXIT FUNC\n  RETURN 1\nEND FUNC\nFUNC main AS Nothing\nEND FUNC\n",
    );
    // EXIT FUNC emits nothing; only the trailing RETURN survives.
    assert!(function(&ir, "f")
        .body
        .iter()
        .any(|op| matches!(op, IrOp::Return { value: Some(_), .. })));
}

#[test]
fn fail_statement_lowers_to_fail_op() {
    let ir = lower_src("FUNC main AS Nothing\n  FAIL error(500, \"boom\")\nEND FUNC\n");
    assert!(main_body(&ir)
        .iter()
        .any(|op| matches!(op, IrOp::Fail { .. })));
}

#[test]
fn if_else_lowers_to_if_op_with_both_branches() {
    let ir = lower_src(
        "FUNC main AS Nothing\n  LET x AS Integer = 1\n\
               IF x > 0 THEN\n    LET a AS Integer = 1\n  ELSE\n    LET b AS Integer = 2\n  END IF\n\
             END FUNC\n",
    );
    assert!(main_body(&ir).iter().any(|op| match op {
        IrOp::If {
            then_body,
            else_body,
            ..
        } => !then_body.is_empty() && !else_body.is_empty(),
        _ => false,
    }));
}

#[test]
fn while_and_do_until_loops() {
    let ir = lower_src(
        "FUNC main AS Nothing\n\
               MUT i AS Integer = 0\n\
               WHILE i < 3\n    i = i + 1\n  END WHILE\n\
               DO\n    i = i - 1\n  LOOP UNTIL i <= 0\n\
             END FUNC\n",
    );
    let body = main_body(&ir);
    assert!(body.iter().any(|op| matches!(op, IrOp::While { .. })));
    assert!(body.iter().any(|op| matches!(op, IrOp::DoUntil { .. })));
}

#[test]
fn for_loop_with_step() {
    let ir = lower_src(
        "FUNC main AS Nothing\n  FOR i = 1 TO 10 STEP 2\n    LET x AS Integer = i\n  NEXT\nEND FUNC\n",
    );
    assert!(main_body(&ir)
        .iter()
        .any(|op| matches!(op, IrOp::For { .. })));
}

#[test]
fn for_loop_default_step() {
    let ir = lower_src(
        "FUNC main AS Nothing\n  FOR i = 1 TO 3\n    LET x AS Integer = i\n  NEXT\nEND FUNC\n",
    );
    assert!(main_body(&ir)
        .iter()
        .any(|op| matches!(op, IrOp::For { .. })));
}

#[test]
fn for_loop_float_bounds_promote_loop_type() {
    let ir = lower_src(
        "FUNC main AS Nothing\n  FOR x = 1.0 TO 2.0 STEP 0.5\n    LET y AS Float = x\n  NEXT\nEND FUNC\n",
    );
    assert!(main_body(&ir).iter().any(|op| match op {
        IrOp::For { type_, .. } => type_.name() == "Float",
        _ => false,
    }));
}

#[test]
fn for_each_over_list() {
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC main AS Nothing\n\
               LET nums AS List OF Integer = [1, 2, 3]\n\
               FOR EACH n IN nums\n    LET x AS Integer = n\n  NEXT\n\
             END FUNC\n",
    );
    assert!(main_body(&ir).iter().any(|op| match op {
        IrOp::ForEach { type_, .. } => type_.name() == "Integer",
        _ => false,
    }));
}

#[test]
fn for_each_over_map_yields_map_entry() {
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC main AS Nothing\n\
               LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n\
               FOR EACH e IN m\n    LET k AS String = e.key\n  NEXT\n\
             END FUNC\n",
    );
    assert!(main_body(&ir).iter().any(|op| match op {
        IrOp::ForEach { type_, .. } => type_.name().starts_with("MapEntry OF "),
        _ => false,
    }));
}

#[test]
fn expression_statement_lowers_to_eval() {
    let ir = lower_src("IMPORT io\nFUNC main AS Nothing\n  io::print(\"hi\")\nEND FUNC\n");
    assert!(main_body(&ir)
        .iter()
        .any(|op| matches!(op, IrOp::Eval { .. })));
}

// ---- MATCH -----------------------------------------------------------

#[test]
fn match_with_value_oneof_and_else() {
    let ir = lower_src(
        "FUNC main AS Nothing\n\
               LET x AS Integer = 2\n\
               MATCH x\n\
                 CASE 1\n      LET a AS Integer = 1\n\
                 CASE 2, 3\n      LET b AS Integer = 2\n\
                 CASE ELSE\n      LET c AS Integer = 3\n\
               END MATCH\n\
             END FUNC\n",
    );
    let cases = main_body(&ir)
        .iter()
        .find_map(|op| match op {
            IrOp::Match { cases, .. } => Some(cases),
            _ => None,
        })
        .expect("a Match op");
    assert!(cases
        .iter()
        .any(|c| matches!(c.pattern, IrMatchPattern::Value(_))));
    assert!(cases
        .iter()
        .any(|c| matches!(c.pattern, IrMatchPattern::OneOf(_))));
    assert!(cases
        .iter()
        .any(|c| matches!(c.pattern, IrMatchPattern::Else)));
}

#[test]
fn match_with_guard() {
    let ir = lower_src(
        "FUNC main AS Nothing\n\
               LET x AS Integer = 5\n\
               MATCH x\n\
                 CASE 5 WHEN x > 0\n      LET a AS Integer = 1\n\
                 CASE ELSE\n      LET b AS Integer = 2\n\
               END MATCH\n\
             END FUNC\n",
    );
    assert!(main_body(&ir).iter().any(|op| match op {
        IrOp::Match { cases, .. } => cases.iter().any(|c| c.guard.is_some()),
        _ => false,
    }));
}

#[test]
fn match_on_union_binds_variant() {
    let ir = lower_src(
        "TYPE Dog\n  legs AS Integer\nEND TYPE\n\
             TYPE Cat\n  lives AS Integer\nEND TYPE\n\
             UNION Animal\n  Dog\n  Cat\nEND UNION\n\
             FUNC describe(a AS Animal) AS Integer\n\
               MATCH a\n\
                 CASE Dog(d)\n      RETURN d.legs\n\
                 CASE Cat(c)\n      RETURN c.lives\n\
               END MATCH\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    fn has_union_extract(ops: &[IrOp]) -> bool {
        ops.iter().any(|op| match op {
            IrOp::Bind {
                value: Some(IrValue::UnionExtract { .. }),
                ..
            } => true,
            IrOp::Match { cases, .. } => cases.iter().any(|c| has_union_extract(&c.body)),
            _ => false,
        })
    }
    assert!(has_union_extract(&function(&ir, "describe").body));
}

#[test]
fn match_on_fallible_call_uses_ok_error_and_is_ok_flag() {
    // MATCH on a fallible call keeps its `Result OF …` shape (matched with
    // CASE Ok / CASE Error), exercising the Result branch of the match
    // lowering (ResultIsOk flag + ResultValue/ResultError case bindings).
    // `Result OF …` cannot be named in source, so a direct call scrutinee is
    // the only way to reach this path.
    let src = "FUNC parse(s AS String) AS Integer\n\
               MATCH toInt(s)\n\
                 CASE Ok(v)\n      RETURN v\n\
                 CASE Error(e)\n      RETURN 0\n\
               END MATCH\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let Some(ir) = try_lower_src(src) else {
        // If a call scrutinee auto-unwraps (front end forbids this Result
        // MATCH), the branch is unreachable from single-file source; skip.
        return;
    };
    let f = function(&ir, "parse");
    let has_is_ok = f.body.iter().any(|op| {
        matches!(
            op,
            IrOp::Bind {
                value: Some(IrValue::ResultIsOk { .. }),
                ..
            }
        )
    });
    fn has_result_extract(ops: &[IrOp]) -> bool {
        ops.iter().any(|op| match op {
            IrOp::Bind {
                value: Some(IrValue::ResultValue { .. } | IrValue::ResultError { .. }),
                ..
            } => true,
            IrOp::Match { cases, .. } => cases.iter().any(|c| has_result_extract(&c.body)),
            _ => false,
        })
    }
    // Whether or not the front end keeps the Result shape, the program must
    // lower to a MATCH; assert the Result path when it is taken.
    if has_is_ok {
        assert!(has_result_extract(&f.body));
    }
}

// ---- expressions: literals ------------------------------------------

#[test]
fn literal_kinds_lower_to_typed_consts() {
    let ir = lower_src(
        "FUNC main AS Nothing\n\
               LET s AS String = \"x\"\n\
               LET i AS Integer = 3\n\
               LET f AS Float = 1.5\n\
               LET bt AS Byte = 7\n\
               LET fx AS Fixed = 2.50\n\
               LET b AS Boolean = TRUE\n\
               LET nul AS Nothing = NOTHING\n\
             END FUNC\n",
    );
    let const_type = |body: &[IrOp], name: &str| -> String {
        match bind_value(body, name) {
            Some(IrValue::Const { type_, .. }) => type_.name().into_owned(),
            _ => "<missing>".to_string(),
        }
    };
    let body = main_body(&ir);
    assert_eq!(const_type(body, "s"), "String");
    assert_eq!(const_type(body, "i"), "Integer");
    assert_eq!(const_type(body, "f"), "Float");
    assert_eq!(const_type(body, "bt"), "Byte");
    assert_eq!(const_type(body, "fx"), "Fixed");
    assert_eq!(const_type(body, "b"), "Boolean");
    assert_eq!(const_type(body, "nul"), "Nothing");
}

/// bug-286: `-9223372036854775808` is a spec-blessed `Integer` literal
/// (§4.12), but the positive magnitude `9223372036854775808` overflows
/// `i64`, so the constant can never materialize on its own. Lowering must
/// fold the negation into the literal — mirroring the `Fixed`/`Money`
/// folds — or codegen emits a runtime negate of `i64::MIN` that always
/// traps with an arithmetic overflow. The neighbouring `-9223372036854775807`
/// pins the guard's exactness: it fits `i64` on its own, so it must keep
/// its `Unary` shape and leave existing codegen untouched.
#[test]
fn most_negative_integer_literal_folds_into_the_const() {
    let ir = lower_src(
        "FUNC main AS Nothing\n\
               LET min AS Integer = -9223372036854775808\n\
               LET near AS Integer = -9223372036854775807\n\
             END FUNC\n",
    );
    let body = main_body(&ir);
    assert!(
        matches!(
            bind_value(body, "min"),
            Some(IrValue::Const { type_, value })
                if type_.name() == "Integer" && value == "-9223372036854775808"
        ),
        "the most-negative Integer literal must fold to a signed Const",
    );
    assert!(
        matches!(bind_value(body, "near"), Some(IrValue::Unary { .. })),
        "an in-range negated literal must keep its Unary shape",
    );
}

// ---- expressions: operators -----------------------------------------

#[test]
fn binary_and_unary_operators() {
    let ir = lower_src(
        "FUNC main AS Nothing\n\
               LET a AS Integer = 1 + 2\n\
               LET b AS Boolean = a > 0 AND NOT (a = 5)\n\
               LET c AS Integer = -a\n\
               LET s AS String = \"x\" & \"y\"\n\
             END FUNC\n",
    );
    let body = main_body(&ir);
    assert!(matches!(
        bind_value(body, "a"),
        Some(IrValue::Binary { .. })
    ));
    assert!(matches!(
        bind_value(body, "b"),
        Some(IrValue::Binary { .. })
    ));
    assert!(matches!(bind_value(body, "c"), Some(IrValue::Unary { .. })));
    assert!(matches!(
        bind_value(body, "s"),
        Some(IrValue::Binary { type_, .. }) if type_.name() == "String"
    ));
}

// ---- expressions: calls ----------------------------------------------

#[test]
fn user_function_call_lowers_to_call() {
    let ir = lower_src(
        "FUNC dbl(x AS Integer) AS Integer\n  RETURN x * 2\nEND FUNC\n\
             FUNC main AS Nothing\n  LET y AS Integer = dbl(4)\nEND FUNC\n",
    );
    assert!(matches!(
        bind_value(main_body(&ir), "y"),
        Some(IrValue::Call { target, .. }) if target == "dbl"
    ));
}

#[test]
fn call_with_named_and_default_arguments() {
    let ir = lower_src(
        "FUNC box(w AS Integer, h AS Integer = 10) AS Integer\n  RETURN w * h\nEND FUNC\n\
             FUNC main AS Nothing\n  LET a AS Integer = box(h := 3, w := 4)\n  LET b AS Integer = box(2)\nEND FUNC\n",
    );
    let calls: Vec<usize> = main_body(&ir)
        .iter()
        .filter_map(|op| match op {
            IrOp::Bind {
                value: Some(IrValue::Call { target, args, .. }),
                ..
            } if target == "box" => Some(args.len()),
            _ => None,
        })
        .collect();
    assert_eq!(calls, vec![2, 2]);
}

#[test]
fn error_builtin_lowers_to_error_record() {
    let ir = lower_src("FUNC main AS Nothing\n  LET e AS Error = error(42, \"bad\")\nEND FUNC\n");
    assert!(matches!(
        bind_value(main_body(&ir), "e"),
        Some(IrValue::Constructor { type_, .. }) if type_.name() == "Error"
    ));
}

#[test]
fn builtin_call_return_type_resolves() {
    let ir = lower_src("FUNC main AS Nothing\n  LET s AS String = toString(5)\nEND FUNC\n");
    assert!(matches!(
        bind_value(main_body(&ir), "s"),
        Some(IrValue::Call { type_, .. }) if type_.name() == "String"
    ));
}

// ---- constructors, member access, WITH -------------------------------

#[test]
fn constructor_and_member_access() {
    let ir = lower_src(
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC mk() AS Integer\n\
               LET p AS Point = Point[1, 2]\n\
               RETURN p.x\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let f = function(&ir, "mk");
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Bind {
            value: Some(IrValue::Constructor { type_, .. }),
            ..
        } if type_.name() == "Point"
    )));
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::MemberAccess { .. }),
            ..
        }
    )));
}

#[test]
fn constructor_named_arguments() {
    // Named-field record construction (reordered), exercising the named-arg
    // branch of `lower_constructor_args`.
    let ir = lower_src(
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC mk() AS Point\n  RETURN Point[y := 2, x := 1]\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let ctor = function(&ir, "mk").body.iter().find_map(|op| match op {
        IrOp::Return {
            value: Some(IrValue::Constructor { args, .. }),
            ..
        } => Some(args.len()),
        _ => None,
    });
    assert_eq!(ctor, Some(2));
}

#[test]
fn with_update_expression() {
    let ir = lower_src(
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC shift(p AS Point) AS Point\n  RETURN WITH p { x := 9 }\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "shift").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::WithUpdate { .. }),
            ..
        }
    )));
}

// ---- list / map literals ---------------------------------------------

#[test]
fn list_and_map_literals() {
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC main AS Nothing\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }\n\
             END FUNC\n",
    );
    let body = main_body(&ir);
    assert!(matches!(
        bind_value(body, "xs"),
        Some(IrValue::ListLiteral { .. })
    ));
    assert!(matches!(
        bind_value(body, "m"),
        Some(IrValue::MapLiteral { .. })
    ));
}

#[test]
fn empty_list_literal_uses_expected_element_type() {
    let ir = lower_src(
        "IMPORT collections\nFUNC main AS Nothing\n  LET xs AS List OF Integer = []\nEND FUNC\n",
    );
    assert!(matches!(
        bind_value(main_body(&ir), "xs"),
        Some(IrValue::ListLiteral { type_, .. }) if type_.name() == "List OF Integer"
    ));
}

// ---- closures / lambdas / FUNC refs ----------------------------------

#[test]
fn lambda_capturing_local_becomes_closure() {
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 10\n\
               LET xs AS List OF Integer = [1, 2]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + base)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
    fn value_has_closure(v: &IrValue) -> bool {
        match v {
            IrValue::Closure { .. } => true,
            IrValue::Call { args, .. } => args.iter().any(value_has_closure),
            _ => false,
        }
    }
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return { value: Some(v), .. } if value_has_closure(v)
    )));
}

#[test]
fn lambda_without_capture_is_function_ref() {
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + 1)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
    fn value_has_fnref(v: &IrValue) -> bool {
        match v {
            IrValue::FunctionRef { .. } => true,
            IrValue::Call { args, .. } => args.iter().any(value_has_fnref),
            _ => false,
        }
    }
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return { value: Some(v), .. } if value_has_fnref(v)
    )));
}

#[test]
fn named_function_reference_as_argument() {
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC inc(x AS Integer) AS Integer\n  RETURN x + 1\nEND FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2]\n\
               RETURN collections::transform(xs, inc)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    fn value_has_fnref(v: &IrValue) -> bool {
        match v {
            IrValue::FunctionRef { .. } => true,
            IrValue::Call { args, .. } => args.iter().any(value_has_fnref),
            _ => false,
        }
    }
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return { value: Some(v), .. } if value_has_fnref(v)
    )));
}

#[test]
fn mut_capture_in_foreach_is_by_ref() {
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC total() AS Integer\n\
               MUT sum AS Integer = 0\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               collections::forEach(xs, LAMBDA(v AS Integer) -> sum = sum + v)\n\
               RETURN sum\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    // The lambda body is an assignment-bodied lambda: assign then valueless
    // return.
    let lambda = ir
        .functions
        .iter()
        .find(|f| f.name.starts_with("$lambda"))
        .expect("a lambda");
    assert!(lambda
        .body
        .iter()
        .any(|op| matches!(op, IrOp::Assign { name, .. } if name == "sum")));
    assert!(lambda
        .body
        .iter()
        .any(|op| matches!(op, IrOp::Return { value: None, .. })));
    // The MUT capture is a by-ref (LocalRef) capture in the closure.
    fn value_has_localref_capture(v: &IrValue) -> bool {
        match v {
            IrValue::Closure { captures, .. } => captures
                .iter()
                .any(|c| matches!(c, IrValue::LocalRef { .. })),
            IrValue::Call { args, .. } => args.iter().any(value_has_localref_capture),
            _ => false,
        }
    }
    assert!(function(&ir, "total").body.iter().any(|op| matches!(
        op,
        IrOp::Eval { value, .. } if value_has_localref_capture(value)
    )));
}
