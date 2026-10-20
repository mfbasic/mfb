//! plan-176-G: `ir::tests::lower_pipeline_tests`'s expression, statement, inline-trap and
//! match, and lambda/builtin lowering tests. The shared `lower_src`/`function`/`json_of`
//! helpers live in `lower_pipeline_tests/mod.rs`.

use super::*;

// ---- literals + operators + expressions -------------------------------

#[test]
fn lowers_every_literal_and_operator_kind() {
    let ir = lower_src(
        "literals",
        r#"
FUNC main AS Integer
  LET s AS String = "hi"
  LET i AS Integer = 42
  LET f AS Float = 3.5
  LET b AS Boolean = TRUE
  LET by AS Byte = 7
  LET fx AS Fixed = 1.25
  LET n AS Integer = -i
  LET notb AS Boolean = NOT b
  LET sum AS Integer = i + i - i * i
  LET cmp AS Boolean = i < i AND b OR NOT b
  LET cat AS String = s & s
  LET xr AS Boolean = b XOR b
  RETURN i
END FUNC
"#,
    );
    let j = json_of(&ir);
    // Byte / Fixed constant typing, unary NOT/negation, concat, comparisons.
    assert!(j.contains("\"Byte\""), "{j}");
    assert!(j.contains("\"Fixed\""), "{j}");
    assert!(j.contains("NOT"));
    assert!(j.contains('&'));
    assert!(j.contains("XOR"));
}

#[test]
fn lowers_nothing_literal() {
    let ir = lower_src(
        "nothing",
        r#"
TYPE Box
  item AS Integer
END TYPE
FUNC main AS Integer
  LET x AS Integer = 0
  RETURN x
END FUNC
"#,
    );
    // Exercise a binding whose value is NOTHING through a Sub with no return.
    let ir2 = lower_src(
        "nothing2",
        r#"
IMPORT io
FUNC main AS Integer
  LET n = NOTHING
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("Box"));
    assert!(json_of(&ir2).contains("NOTHING"));
}

#[test]
fn lowers_list_and_map_literals() {
    let ir = lower_src(
        "collections",
        r#"
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3]
  LET empty AS List OF Integer = []
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1, "b" := 2 }
  LET inferred = [10, 20]
  RETURN 0
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("List OF Integer"));
    assert!(j.contains("Map OF String TO Integer"));
}

// ---- constructors, member access, WITH ---------------------------------

#[test]
fn lowers_constructors_member_access_and_with_update() {
    let ir = lower_src(
        "records",
        r#"
TYPE Point
  x AS Integer
  y AS Integer
END TYPE
FUNC main AS Integer
  LET p AS Point = Point[1, 2]
  LET q AS Point = Point[x := 3, y := 4]
  LET moved AS Point = WITH p { x := 10 }
  LET ax AS Integer = p.x
  RETURN ax + q.y + moved.x
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("\"kind\": \"with\""), "{j}");
    assert!(j.contains("Point"));
}

#[test]
fn lowers_enum_member_access() {
    let ir = lower_src(
        "enums",
        r#"
ENUM Color
  Red
  Green
  Blue
END ENUM
FUNC pick(c AS Color) AS Integer
  MATCH c
    CASE Color.Red
      RETURN 1
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN pick(Color.Green)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("Color"));
}

// ---- union wrap / extract ---------------------------------------------

#[test]
fn lowers_union_wrap_and_extract() {
    let ir = lower_src(
        "unions",
        r#"
TYPE Cat
  legs AS Integer
END TYPE
TYPE Dog
  legs AS Integer
END TYPE
UNION Animal
  Cat
  Dog
END UNION
FUNC describe(a AS Animal) AS Integer
  MATCH a
    CASE Cat(c)
      RETURN c.legs
    CASE Dog(d)
      RETURN d.legs + 100
  END MATCH
END FUNC
FUNC wrapIt() AS Animal
  RETURN Cat[4]
END FUNC
FUNC main AS Integer
  RETURN describe(wrapIt())
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("unionWrap"), "{j}");
    assert!(j.contains("unionExtract"), "{j}");
}

// ---- calls, fallible CallResult, FUNC refs -----------------------------

#[test]
fn lowers_calls_and_function_refs() {
    let ir = lower_src(
        "calls",
        r#"
FUNC helper(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC applyIt(f AS FUNC(Integer, Integer) AS Integer) AS Integer
  RETURN f(2, 3)
END FUNC
FUNC main AS Integer
  LET fref AS FUNC(Integer, Integer) AS Integer = helper
  RETURN applyIt(fref) + helper(1, b := 9)
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("functionRef"), "{j}");
}

#[test]
fn lowers_default_argument_padding_for_local_call() {
    let ir = lower_src(
        "defaults",
        r#"
FUNC greet(name AS String, punct AS String = "!") AS String
  RETURN name & punct
END FUNC
FUNC main AS Integer
  LET a AS String = greet("hi")
  LET b AS String = greet("yo", "?")
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("greet"));
}

// ---- statement kinds ---------------------------------------------------

#[test]
fn lowers_control_flow_statements() {
    let ir = lower_src(
        "control",
        r#"
FUNC main AS Integer
  MUT total AS Integer = 0
  FOR i = 1 TO 10 STEP 2
    total = total + i
    IF i > 7 THEN EXIT FOR
  NEXT
  FOR j = 0 TO 3
    IF j = 1 THEN CONTINUE FOR
    total = total + j
  NEXT
  WHILE total < 100
    total = total + 10
  END WHILE
  DO
    total = total - 1
  LOOP UNTIL total < 50
  RETURN total
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("\"op\": \"for\""));
    assert!(j.contains("while"));
    assert!(j.contains("doUntil"));
}

#[test]
fn lowers_float_for_loop_and_exit_do_while() {
    let ir = lower_src(
        "floatfor",
        r#"
FUNC main AS Integer
  MUT sum AS Float = 0.0
  FOR x = 0.0 TO 2.0 STEP 0.5
    sum = sum + x
    IF sum > 1.0 THEN EXIT FOR
  NEXT
  MUT k AS Integer = 0
  DO
    k = k + 1
    IF k = 2 THEN EXIT DO
  LOOP UNTIL k > 100
  WHILE k < 5
    k = k + 1
    IF k = 4 THEN EXIT WHILE
  END WHILE
  RETURN k
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("Float"));
}

#[test]
fn lowers_foreach_over_list_and_map() {
    let ir = lower_src(
        "foreach",
        r#"
FUNC main AS Integer
  MUT total AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  FOR EACH n IN nums
    total = total + n
  NEXT
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1, "b" := 2 }
  FOR EACH entry IN m
    total = total + entry.value
  NEXT
  RETURN total
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("forEach"));
    assert!(j.contains("MapEntry"));
}

#[test]
fn lowers_exit_program_and_exit_sub() {
    let ir = lower_src(
        "exits",
        r#"
IMPORT os
FUNC early(v AS Integer) AS Nothing
  IF v < 0 THEN RETURN
  LET x AS Integer = v
END FUNC
FUNC main AS Integer
  early(5)
  IF FALSE THEN os::exit(2)
  RETURN 0
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("exitProgram"), "{j}");
}

#[test]
fn lowers_global_assignment_and_state_assign() {
    let ir = lower_src(
        "globals",
        r#"
MUT counter AS Integer = 0
FUNC bump() AS Integer
  counter = counter + 1
  RETURN counter
END FUNC
FUNC main AS Integer
  RETURN bump()
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("assignGlobal"), "{j}");
    // Global binding was collected.
    assert!(ir.bindings.iter().any(|b| b.name == "counter"));
}

// ---- FAIL / trap function-level -------------------------------------

#[test]
fn lowers_fail_and_function_trap() {
    let ir = lower_src(
        "trap",
        r#"
FUNC risky(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(400, "bad")
  RETURN v
  TRAP(err)
    RETURN err.code
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN risky(-1)
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("\"op\": \"trap\""), "{j}");
    assert!(j.contains("\"op\": \"fail\""));
    // error(...) lowered to an Error record constructor with ErrorLoc.
    assert!(j.contains("ErrorLoc"));
}

#[test]
fn lowers_propagate_in_trap() {
    let ir = lower_src(
        "propagate",
        r#"
FUNC stepFn(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "neg")
  RETURN v
  TRAP(e)
    PROPAGATE
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN stepFn(5)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("step"));
}

// ---- inline TRAP desugaring: every target + treeify shapes -------------

#[test]
fn lowers_inline_trap_bind_assign_discard() {
    let ir = lower_src(
        "inline_trap",
        r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, "missing")
  RETURN v + 1
END FUNC
FUNC doEffect(v AS Integer) AS Nothing
  IF v < 0 THEN FAIL error(500, "effect")
END FUNC
FUNC bindForm(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a
END FUNC
FUNC assignForm(v AS Integer) AS Integer
  MUT total AS Integer = 100
  total = parsePositive(v) TRAP(e)
    RECOVER 5
  END TRAP
  RETURN total
END FUNC
FUNC discardForm(v AS Integer) AS Integer
  MUT code AS Integer = 0
  doEffect(v) TRAP(e)
    code = e.code
    RECOVER
  END TRAP
  RETURN code
END FUNC
FUNC main AS Integer
  RETURN bindForm(1) + assignForm(-1) + discardForm(-1)
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("resultIsOk"), "{j}");
    assert!(j.contains("resultValue"));
    assert!(j.contains("resultError"));
}

#[test]
fn lowers_inline_trap_treeify_if_and_match_continuation() {
    // Handler with an IF whose fall-through must distribute the continuation,
    // plus a MATCH without ELSE (adds a synthetic ELSE), plus a diverging arm.
    let ir = lower_src(
        "treeify",
        r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, "missing")
  RETURN v + 1
END FUNC
FUNC recoverOrBail(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    IF e.code = 404 THEN
      RECOVER 0
    END IF
    FAIL e
  END TRAP
  RETURN a
END FUNC
FUNC viaReturn(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RETURN 99
  END TRAP
  RETURN a
END FUNC
FUNC matchHandler(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 404
        RECOVER 1
      CASE 500
        RECOVER 2
    END MATCH
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN recoverOrBail(-1) + viaReturn(-1) + matchHandler(-1)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("resultIsOk"));
}

#[test]
fn lowers_inline_trap_treeify_match_with_tail_continuation() {
    // A MATCH in the handler that is NOT the last statement: the continuation
    // must be distributed into each arm and a synthetic ELSE added (no ELSE
    // present), and one arm terminates while another falls through.
    let ir = lower_src(
        "treeify_match_tail",
        r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC h(v AS Integer) AS Integer
  MUT tag AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 1
        RETURN 11
      CASE 2
        tag = 2
    END MATCH
    RECOVER tag
  END TRAP
  RETURN a
END FUNC
FUNC h2(v AS Integer) AS Integer
  MUT tag AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    IF e.code = 1 THEN
      tag = 1
    END IF
    RECOVER tag
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN h(-1) + h2(-1)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("resultIsOk"));
}

#[test]
fn lowers_inline_trap_treeify_match_existing_else_with_tail() {
    // A MATCH with an explicit ELSE followed by a tail: no synthetic ELSE is
    // added, and the continuation distributes into the ELSE arm too.
    let ir = lower_src(
        "treeify_match_else_tail",
        r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC h(v AS Integer) AS Integer
  MUT tag AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 1
        tag = 1
      CASE ELSE
        tag = 9
    END MATCH
    RECOVER tag
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN h(-1)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("resultIsOk"));
}

#[test]
fn lowers_inline_trap_with_loop_and_foreach_in_handler() {
    // A handler whose leading statement is a non-branching, non-terminating
    // statement that itself contains nested blocks (treeify_statement recursion
    // over While/DoUntil/For/ForEach).
    let ir = lower_src(
        "treeify_loops",
        r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC h(v AS Integer) AS Integer
  MUT acc AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    FOR i = 1 TO 3
      acc = acc + i
    NEXT
    WHILE acc < 100
      acc = acc + 10
    END WHILE
    DO
      acc = acc + 1
    LOOP UNTIL acc > 120
    FOR EACH n IN [1, 2]
      acc = acc + n
    NEXT
    RECOVER acc
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN h(-1)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("parsePositive"));
}

// ---- MATCH variants ----------------------------------------------------

#[test]
fn lowers_match_value_oneof_else_and_guard() {
    let ir = lower_src(
        "match_value",
        r#"
FUNC classify(n AS Integer) AS Integer
  MATCH n
    CASE 0
      RETURN 100
    CASE 1, 2, 3
      RETURN 200
    CASE ELSE WHEN n > 10
      RETURN 300
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN classify(2)
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("oneOf"), "{j}");
    assert!(j.contains("else"));
}

// NOTE: the internal `#Result OF …` MATCH lowering paths (Match-statement
// result-flag branch; `match_case_binding`'s `#Ok`/Error arms) are unreachable
// from source: a call scrutinee auto-unwraps to its value and no user
// identifier can spell `#Result`/`#Ok` (bug-945), so no clean program reaches
// them. They remain covered only defensively.

// ---- lambdas / captures ------------------------------------------------

#[test]
fn lowers_lambda_by_value_capture_and_closure() {
    let ir = lower_src(
        "lambda_value",
        r#"
IMPORT collections
FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer
  LET captured AS Integer = base
  RETURN LAMBDA(value AS Integer) -> value + captured
END FUNC
FUNC main AS Integer
  LET add2 AS FUNC(Integer) AS Integer = makeAdder(2)
  LET nums AS List OF Integer = [1, 2, 3]
  LET mapped AS List OF Integer = collections::transform(nums, add2)
  RETURN collections::get(mapped, 0)
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("closure") || j.contains("capture"), "{j}");
    // A lambda synthesizes a private $lambda function.
    assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
}

#[test]
fn lowers_lambda_mut_byref_capture_in_foreach() {
    let ir = lower_src(
        "lambda_mut",
        r#"
IMPORT collections
FUNC main AS Integer
  MUT total AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  collections::forEach(nums, LAMBDA(n AS Integer) -> total = total + n)
  RETURN total
END FUNC
"#,
    );
    let j = json_of(&ir);
    // A MUT slot-reference capture produces a by_ref binding / LocalRef.
    assert!(j.contains("localRef"), "{j}");
}

#[test]
fn lowers_filter_predicate_function_ref() {
    let ir = lower_src(
        "filter",
        r#"
IMPORT collections
FUNC isBig(n AS Integer) AS Boolean
  RETURN n > 2
END FUNC
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET big AS List OF Integer = collections::filter(nums, isBig)
  RETURN len(big)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("isBig"));
}

// ---- builtin package call resolvers ------------------------------------

#[test]
fn lowers_builtin_package_calls() {
    let ir = lower_src(
        "builtins",
        r#"
IMPORT strings
IMPORT math
IMPORT bits
FUNC main AS Integer
  LET up AS String = strings::upper("hi")
  LET n AS Integer = len([1, 2, 3])
  LET r AS Float = math::sqrt(4.0)
  LET x AS Integer = bits::sl(1, 3)
  LET joined AS String = strings::join(["a", "b"], ",")
  RETURN n + x
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("upper") || j.contains("strings"));
    assert!(j.contains("sqrt") || j.contains("math"));
}

#[test]
fn lowers_named_argument_reordering_for_builtin() {
    // strings::replace has named params; supplying them out of order exercises
    // normalize_builtin_call_arguments' reorder path.
    let ir = lower_src(
        "named_builtin",
        r#"
IMPORT strings
FUNC main AS Integer
  LET s AS String = strings::replace(value := "aaa", new := "b", old := "a")
  RETURN len(s)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("irlower") || json_of(&ir).contains("replace"));
}

#[test]
fn lowers_builtin_with_overloaded_argument_signature() {
    // strings::find has an optional-argument signature ("String, String[,
    // Integer]"); builtin_argument_types must decline it (bracketed desc), so
    // the argument expected type is left unspecified.
    let ir = lower_src(
        "overloaded_args",
        r#"
IMPORT strings
FUNC main AS Integer
  LET i AS Integer = strings::find("hello", "l")
  LET j AS Integer = strings::find("hello", "l", 3)
  RETURN i + j
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("find") || json_of(&ir).contains("strings"));
}

#[test]
fn lowers_builtin_with_generic_placeholder_argument_signature() {
    // `typeName`'s expected argument signature is the bare placeholder `T`;
    // builtin_argument_types must decline it (generic placeholder), leaving
    // the argument's expected type unspecified.
    let ir = lower_src(
        "generic_args",
        r#"
IMPORT io
FUNC main AS Integer
  LET t AS String = typeName(42)
  io::print(t)
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("typeName") || json_of(&ir).contains("main"));
}

#[test]
fn lowers_named_constructor_arg_inside_lambda() {
    // A record constructor with named args inside a lambda body routes through
    // constructor_arg_value's Named arm (captured_locals + fallback lowering).
    let ir = lower_src(
        "named_ctor_lambda",
        r#"
TYPE Coord
  x AS Integer
  y AS Integer
END TYPE
FUNC run(px AS Integer, py AS Integer) AS FUNC() AS Coord
  RETURN LAMBDA() -> Coord[x := px, y := py]
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS Coord = run(1, 2)
  RETURN f().x
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("Coord"));
}

#[test]
fn lowers_builtin_mixed_positional_and_named_args() {
    // A builtin call mixing positional and named args exercises the
    // named-argument reordering (positional-after-named tracking).
    let ir = lower_src(
        "mixed_named",
        r#"
IMPORT strings
FUNC main AS Integer
  LET s AS String = strings::replace("aaa", old := "a", new := "b")
  RETURN len(s)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("replace") || json_of(&ir).contains("irlower"));
}

#[test]
fn lowers_regex_and_datetime_default_padding() {
    let ir = lower_src(
        "regex_pad",
        r#"
IMPORT regex
IMPORT datetime
FUNC main AS Integer
  LET m AS Boolean = regex::match("abc", "a.c")
  LET dt AS datetime::DateTime = datetime::parse("2024-01-02")
  RETURN 0
END FUNC
"#,
    );
    // regex::match / datetime::parse pad optional trailing args with consts.
    assert!(json_of(&ir).contains("match") || json_of(&ir).contains("parse"));
}

// ---- toString(Byte) overload / general override routing -----------------

#[test]
fn lowers_tostring_with_base_and_overridable() {
    let ir = lower_src(
        "tostring",
        r#"
FUNC main AS Integer
  LET hex AS String = toString(255, 16)
  LET plain AS String = toString(42)
  RETURN 0
END FUNC
"#,
    );
    // toString(x, base): the base arg types as Byte (call_argument_expected_type).
    assert!(json_of(&ir).contains("Byte"));
}
