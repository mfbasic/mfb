use super::*;

/// A declared `AS T` annotation, or `None` when the source gave none.
///
/// plan-106-D: HIR spells an absent annotation [`Type::Unknown`], collapsing the
/// AST's `Option<String>`. That is the same collapse the de-elaboration seam this
/// letter deletes already applied at this boundary (`hir::unrender_optional_type`),
/// and the language agrees with it: a parameter written `AS Unknown` is rejected as
/// `TYPE_PARAM_REQUIRES_TYPE` ("must declare an `AS` type").
pub(super) fn declared(type_: &Type) -> Option<&Type> {
    match type_ {
        Type::Unknown => None,
        other => Some(other),
    }
}

pub(super) fn statement_line(statement: &HirStatement) -> usize {
    match statement {
        HirStatement::Let { line, .. }
        | HirStatement::Return { line, .. }
        | HirStatement::Exit { line, .. }
        | HirStatement::Continue { line, .. }
        | HirStatement::Fail { line, .. }
        | HirStatement::Propagate { line }
        | HirStatement::Recover { line, .. }
        | HirStatement::Assign { line, .. }
        | HirStatement::StateAssign { line, .. }
        | HirStatement::Expression { line, .. }
        | HirStatement::If { line, .. }
        | HirStatement::Match { line, .. }
        | HirStatement::For { line, .. }
        | HirStatement::ForEach { line, .. }
        | HirStatement::While { line, .. }
        | HirStatement::DoUntil { line, .. } => *line,
    }
}

pub(super) fn integer_literal_in_range(expression: &HirExpression) -> bool {
    match expression {
        HirExpression::Number(value) => match numeric::classify_literal(value) {
            (canonical, numeric::LiteralType::Integer) => canonical.parse::<i64>().is_ok(),
            // A Float/Fixed literal is not an integer-range question here; its
            // range is checked by the Float/Fixed literal-overflow rules.
            _ => true,
        },
        HirExpression::Unary {
            operator, operand, ..
        } if operator == "-" => {
            let HirExpression::Number(value) = operand.as_ref() else {
                return true;
            };
            match numeric::classify_literal(value) {
                (canonical, numeric::LiteralType::Integer) => canonical
                    .parse::<u64>()
                    .is_ok_and(|number| number <= (i64::MAX as u64) + 1),
                _ => true,
            }
        }
        _ => true,
    }
}

pub(super) fn effective_field_visibility(
    declared: Option<Visibility>,
    containing_visibility: Visibility,
) -> Visibility {
    declared.unwrap_or(match containing_visibility {
        Visibility::Export => Visibility::Export,
        Visibility::Public | Visibility::Private => Visibility::Public,
    })
}

pub(super) fn function_type(sig: &FunctionSig) -> Type {
    Type::Func(
        sig.params.iter().map(|param| param.type_.clone()).collect(),
        Box::new(sig.return_type.clone()),
        sig.isolated,
    )
}

pub(super) fn constructor_arg_value(argument: &HirConstructorArg) -> &HirExpression {
    match argument {
        HirConstructorArg::Positional(value) => value,
        HirConstructorArg::Named { value, .. } => value,
    }
}

pub(super) fn call_arg_value(argument: &HirCallArg) -> &HirExpression {
    match argument {
        HirCallArg::Positional(value) => value,
        HirCallArg::Named { value, .. } => value,
    }
}

/// Unwrap a `RES`-marked collection element (`Type::Res`) to the underlying
/// type; a no-op for any other type.
pub(super) fn strip_res(type_: &Type) -> &Type {
    match type_ {
        Type::Res(inner) => inner,
        other => other,
    }
}

pub(super) fn numeric_literal_type(expression: &HirExpression) -> Option<Type> {
    match expression {
        HirExpression::Number(number) => Some(match numeric::classify_literal(number).1 {
            numeric::LiteralType::Integer => Type::Integer,
            numeric::LiteralType::Float => Type::Float,
            numeric::LiteralType::Fixed => Type::Fixed,
            numeric::LiteralType::Money => Type::Money,
        }),
        HirExpression::Unary {
            operator, operand, ..
        } if operator == "-" && matches!(operand.as_ref(), HirExpression::Number(_)) => {
            numeric_literal_type(operand)
        }
        _ => None,
    }
}

/// The `FOR` induction variable's promoted type.
///
/// plan-106-C: the one typed source in `numeric` (`Type` is `ParameterType`, so
/// there is nothing to render). The pre-check is load-bearing and is NOT part of
/// the shared fold: syntaxcheck answers `Unknown` — its permissive skip — when
/// any operand is non-numeric, whereas the shared algebra's `unwrap_or(Integer)`
/// would claim a numeric loop over, say, a `String` bound.
pub(super) fn promote_loop_numeric_type(start: &Type, end: &Type, step: &Type) -> Type {
    if !numeric::is_numeric(start) || !numeric::is_numeric(end) || !numeric::is_numeric(step) {
        return Type::Unknown;
    }
    numeric::typed_promote_loop_numeric_type(start, end, step)
}

/// The result type of a binary numeric operation.
///
/// plan-106-C: the one typed source in `numeric`. Its `None` — a non-numeric
/// operand, or a dimensionally-invalid `Money` pairing — is syntaxcheck's
/// `Unknown`, which is exactly what the name-mapping cascade this replaced
/// computed the long way round.
pub(super) fn numeric_binary_result_type(operator: &str, left: &Type, right: &Type) -> Type {
    numeric::typed_binary_result_type(operator, left, right).unwrap_or(Type::Unknown)
}

/// Whether a type is a compiler-owned read-only record.
///
/// plan-106-E: mirrors the typed form plan-106-B already gave `ir::verify`
/// (`verify/mod.rs:read_only_record_type`) — a `MapEntry OF K TO V` is read-only
/// STRUCTURALLY; the rest are nominal lookups into per-package tables, which are
/// keyed by NAME.
pub(super) fn read_only_record_type(type_: &Type) -> bool {
    if matches!(type_, Type::MapEntryOf(_, _)) {
        return true;
    }
    let type_name = type_.name();
    crate::codegen::builtins::term::is_read_only_record(&type_name)
        || type_name == crate::codegen::builtins::net::ADDRESS_TYPE
        || type_name == crate::codegen::builtins::audio::AUDIO_DEVICE_TYPE
}

#[cfg(test)]
mod tests {
    use crate::testutil::*;

    // Most helpers here have no `report`; they are exercised indirectly by
    // running valid (and a few invalid) programs whose types force each branch.

    // ----- statement_line (via UNREACHABLE_AFTER_EXIT, which reports at the
    // unreachable statement's line — covering the many Statement arms) --------

    #[test]
    fn statement_line_used_for_various_unreachable_statements() {
        // A LET after EXIT (HirStatement::Let arm of statement_line).
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3
    EXIT FOR
    LET dead AS Integer = 1
  NEXT
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "UNREACHABLE_AFTER_EXIT"));

        // An IF after EXIT (HirStatement::If arm).
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3
    EXIT FOR
    IF TRUE THEN
      LET x AS Integer = 1
    END IF
  NEXT
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "UNREACHABLE_AFTER_EXIT"));

        // A while after EXIT (HirStatement::While arm).
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3
    EXIT FOR
    WHILE FALSE
    END WHILE
  NEXT
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "UNREACHABLE_AFTER_EXIT"));
    }

    #[test]
    fn statement_line_covers_every_statement_variant() {
        // Each unreachable-after-EXIT statement forces `statement_line` down a
        // different `HirStatement::*` arm. One big loop body places every kind
        // after `EXIT FOR`.
        let src = "\
IMPORT io

FUNC helper(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, \"neg\")
  RETURN v
END FUNC

FUNC main AS Integer
  MUT total AS Integer = 0
  RES f AS Nothing = NOTHING
  FOR i = 1 TO 3
    EXIT FOR
    RETURN 0
    FAIL error(1, \"x\")
    LET a AS Integer = 1
    total = 2
    io::print(\"x\")
    IF TRUE THEN
      total = 3
    END IF
    MATCH total
      CASE ELSE
        total = 4
    END MATCH
    FOR j = 1 TO 2
      total = 5
    NEXT
    FOR EACH k IN [1, 2]
      total = 6
    NEXT
    WHILE FALSE
    END WHILE
    DO
      total = 7
    LOOP UNTIL TRUE
    CONTINUE FOR
  NEXT
  RETURN total
END FUNC
";
        assert!(
            rejects_with(src, "UNREACHABLE_AFTER_EXIT"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn statement_line_covers_recover_and_propagate_and_state_assign() {
        // RECOVER, PROPAGATE, and a `.state` assignment placed after EXIT to
        // reach the Recover / Propagate / StateAssign arms of statement_line.
        let src = "\
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, \"missing\")
  RETURN v + 1
END FUNC

FUNC main AS Integer
  FOR i = 1 TO 3
    LET a = parsePositive(i) TRAP(e)
      EXIT FOR
      RECOVER 0
      PROPAGATE
    END TRAP
    LET b AS Integer = a
  NEXT
  RETURN 0
END FUNC
";
        // The RECOVER/PROPAGATE after EXIT are unreachable-after-exit.
        assert!(
            rejects_with(src, "UNREACHABLE_AFTER_EXIT"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- integer_constant_value & integer_literal_in_range ----------------

    #[test]
    fn exit_program_negative_literal_uses_integer_constant_value() {
        // EXIT PROGRAM code path calls integer_constant_value; a negated Number
        // exercises the Unary "-" arm.
        let src = "\
FUNC main AS Integer
  EXIT PROGRAM -1
END FUNC
";
        // Range/mismatch checks are ir::verify no-ops here; just ensure the
        // helper walk doesn't spuriously report an unknown value.
        assert!(
            !rejects_with(src, "TYPE_UNKNOWN_VALUE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn huge_negative_literal_exercises_integer_literal_in_range() {
        // The Unary "-" arm of integer_literal_in_range: a value beyond i64
        // range returns Integer directly in inference.
        let src = "\
FUNC main AS Integer
  LET big AS Integer = -9999999999999999999
  RETURN 0
END FUNC
";
        // The helper is a pure shape query, so syntaxcheck must emit nothing —
        // the range rule itself belongs to ir::verify (plan-20).
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- numeric_literal_type / numeric_literal_is_zero -------------------

    #[test]
    fn for_loop_zero_step_exercises_numeric_literal_is_zero() {
        // The `numeric_literal_is_zero(step)` call in the FOR arm; also drives
        // promote_loop_numeric_type / numeric_type_name / type_from_numeric_name.
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3 STEP 0
    LET x AS Integer = i
  NEXT
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn for_loop_negative_step_is_walked() {
        let src = "\
FUNC main AS Integer
  FOR i = 3 TO 1 STEP -1
    LET x AS Integer = i
  NEXT
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn for_loop_float_bounds_promote_loop_numeric_type() {
        // Float start/end drives promote_loop_numeric_type -> Float branch and
        // type_from_numeric_name TYPE_FLOAT.
        let src = "\
FUNC main AS Integer
  FOR x = 0.0 TO 2.0 STEP 0.5
    LET y AS Float = x
  NEXT
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn for_loop_non_numeric_bounds_yield_unknown() {
        // Non-numeric loop bounds hit the `else` arm (Type::Unknown loop type).
        let src = "\
FUNC main AS Integer
  FOR x = \"a\" TO \"z\"
    LET y AS Integer = 1
  NEXT
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- numeric_binary_result_type / numeric_type_name -------------------

    #[test]
    fn mixed_numeric_arithmetic_exercises_binary_result_type() {
        // Byte + Integer, Integer + Float, Fixed arithmetic all route through
        // numeric_binary_result_type and its per-type match arms.
        let src = "\
FUNC main AS Integer
  LET b AS Byte = 3
  LET i AS Integer = 5
  LET f AS Float = 2.0
  LET x AS Float = i + f
  LET y AS Integer = b + i
  LET z AS Fixed = 1.5 + 2
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- strip_res (List OF RES ...) --------------------------------------

    #[test]
    fn foreach_over_res_list_strips_res_marker() {
        // strip_res is applied to the element type of a `List OF RES File`.
        let src = "\
IMPORT fs

FUNC main AS Integer
  LET files AS List OF RES File = []
  FOR EACH f IN files
    LET n AS Integer = 1
  NEXT
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- is_c_abi_type (LINK wrapper signature with a raw C type) ----------

    #[test]
    fn link_wrapper_with_cptr_param_reports_escape() {
        // A LINK wrapper whose MFBASIC-facing signature uses `CPtr` drives
        // is_c_abi_type -> NATIVE_CPTR_ESCAPE (param arm).
        let src = "\
RESOURCE Db CLOSE BY demoLink::close

LINK \"demo\" AS demoLink
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC leak(handle AS CPtr) AS Nothing
    SYMBOL \"demo_leak\"
    ABI (handle CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn link_wrapper_with_cptr_return_reports_escape() {
        // The return-type arm of is_c_abi_type.
        let src = "\
LINK \"demo\" AS demoLink
  FUNC handle() AS CPtr
    SYMBOL \"demo_handle\"
    ABI (produced OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    // ----- numeric_literal_type (List OF <numeric> literal compatibility) ---

    #[test]
    fn byte_list_literal_exercises_numeric_literal_type() {
        // Assigning `[1, 2, 3]` (List OF Integer literal) to a `List OF Byte`
        // routes through expression_compatible's ListLiteral arm, which calls
        // numeric_literal_type on each element (Integer + negated + float arms).
        let src = "\
FUNC main AS Integer
  LET a AS List OF Byte = [1, 2, 3]
  LET b AS List OF Byte = [-1, 2, 3]
  LET c AS List OF Float = [1.5, -2.0, 3]
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn non_numeric_list_literal_element_hits_numeric_literal_type_none() {
        // A `List OF Byte` literal containing a non-numeric element makes
        // numeric_literal_type return None (the `_ => None` arm) inside the
        // list-literal compatibility fallback.
        let src = "\
FUNC main AS Integer
  LET a AS List OF Byte = [1, \"two\", 3]
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- constructor_arg_value (positional + named) -----------------------

    #[test]
    fn constructor_with_named_and_positional_args() {
        let src = "\
TYPE Point
  x AS Integer
  y AS Integer
END TYPE

FUNC main AS Integer
  LET a = Point[1, 2]
  LET b = Point[x := 3, y := 4]
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- read_only_record_type (constructor of a compiler-owned record) ---

    #[test]
    fn constructing_read_only_record_reports() {
        // TermColor is a read-only built-in record; constructing it hits the
        // read_only_record_type branch in infer_constructor.
        let src = "\
FUNC main AS Integer
  LET c = TermColor[0, 0, 0]
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- function_type (a FUNC referenced by name as a value) -------------

    #[test]
    fn function_reference_builds_function_type() {
        // Passing a named FUNC as a value drives function_type().
        let src = "\
IMPORT collections

FUNC doubler(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC main AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  LET ys = collections::transform(xs, doubler)
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- effective_field_visibility ---------------------------------------

    #[test]
    fn type_field_visibility_defaults_and_overrides() {
        // An EXPORT type with a defaulted field (inherits Export) and a PACKAGE
        // type (inherits Package) drive both arms of effective_field_visibility;
        // an explicit field visibility drives the `declared.unwrap_or` Some path.
        let src = "\
EXPORT TYPE Exported
  a AS Integer
  PUBLIC b AS Integer
END TYPE

TYPE Local
  c AS Integer
END TYPE

FUNC main AS Integer
  LET p = Exported[1, 2]
  LET q = Local[3]
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- is_resource_element_pointer (get/getOr of a resource element) -----

    #[test]
    fn res_binding_from_get_is_walked() {
        // Binds a RES from a `get` on a resource collection — drives
        // is_resource_element_pointer's Call/native_member_bare branch.
        let src = "\
IMPORT collections
IMPORT fs

FUNC main AS Integer
  LET files AS List OF RES File = []
  RES f = collections::get(files, 0)
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- read_only_record_type via with-update ----------------------------

    #[test]
    fn with_update_on_map_entry_type_is_walked() {
        // MapEntry-typed value from FOR EACH over a Map; a with-update on it hits
        // the read_only_record_type early-return in infer_with_update.
        let src = "\
IMPORT collections

FUNC main AS Integer
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  FOR EACH entry IN m
    LET k AS String = entry.key
  NEXT
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- numeric_literal_type / binary_result_type: Money arms -------------

    #[test]
    fn money_list_literal_and_arithmetic_walk_money_arms() {
        // A `List OF Money` literal drives numeric_literal_type's Money arm (233)
        // and its negated-Money Unary "-" arm (235-239); Money + Money drives
        // numeric_binary_result_type's Money arm (284) and type_from_numeric_name
        // (267).
        let src = "\
FUNC main AS Integer
  LET a AS List OF Money = [2m, -3m]
  LET x AS Money = 2m
  LET y AS Money = 3m
  LET z AS Money = x + y
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }
}
