//! A bare general built-in passed as a `collections` callback (bug-368): its
//! callable type is derived from the list's element type, so
//!
//! - a collection argument that is not a list has no element to derive it
//!   from, and the call is a mismatch naming the predicate;
//! - a derived type the member still rejects (`isEmpty` returns a Boolean,
//!   `forEach` needs a `Nothing` callback) is a mismatch naming the derived
//!   types.

use crate::testutil::check_src_details;

fn found(src: &str) -> Vec<(String, String)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.detail))
        .collect()
}

fn mismatches(src: &str) -> Vec<String> {
    found(src)
        .into_iter()
        .filter(|(rule, _)| rule == "TYPE_CALL_ARGUMENT_MISMATCH")
        .map(|(_, detail)| detail)
        .collect()
}

#[test]
fn a_collection_that_is_not_a_list_names_the_predicate() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  LET a = collections::filter(5, isEmpty)
  RETURN 0
END FUNC
";
    assert_eq!(
        mismatches(src),
        [
            "Call to `collections.filter` has argument type(s) (Integer, isEmpty), \
          expected List OF T, FUNC(T) AS Boolean."
                .to_string()
        ]
    );
}

#[test]
fn a_derived_predicate_type_the_member_rejects_names_the_derived_types() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  collections::forEach([\"a\"], isEmpty)
  RETURN 0
END FUNC
";
    assert_eq!(
        mismatches(src),
        ["Call to `collections.forEach` has argument type(s) \
          (List OF String, FUNC(String) AS Boolean), expected List OF T, FUNC(T) AS Nothing."
            .to_string()]
    );
}

/// A call through a local that is not a function value has no signature to
/// check arguments against: only the unknown result is reported, never an
/// arity or argument rule.
#[test]
fn a_call_through_a_non_function_local_checks_no_arguments() {
    let src = "FUNC main() AS Integer\n  LET x = 1\n  RETURN x(2, 3)\nEND FUNC\n";
    assert_eq!(super::shape_codes(src), ["TYPE_UNKNOWN_VALUE"]);
}

/// Rule, line and detail of every TYPE_CALL_ARGUMENT_MISMATCH for `src`.
fn located_mismatches(src: &str) -> Vec<(String, usize, String)> {
    check_src_details(src)
        .into_iter()
        .filter(|d| d.rule == "TYPE_CALL_ARGUMENT_MISMATCH")
        .map(|d| (d.rule, d.line, d.detail))
        .collect()
}

/// A general built-in that is no predicate (`toString`, `len`) has no callable
/// type, so passing it as a callback is the bug-368 mismatch naming it — not an
/// `Unknown` argument the callback parameter silently accepted, leaving only
/// the binding's TYPE_UNKNOWN_VALUE (or, unbound, a native-lowering failure).
#[test]
fn a_non_predicate_builtin_callback_is_a_mismatch_naming_it() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  LET ys = collections::filter([1, 2], toString)
  collections::forEach([1, 2], toString)
  LET n AS Integer = collections::reduce([1, 2], 0, len)
  RETURN 0
END FUNC
";
    let mismatch = |line: usize, detail: &str| {
        (
            "TYPE_CALL_ARGUMENT_MISMATCH".to_string(),
            line,
            detail.to_string(),
        )
    };
    assert_eq!(
        located_mismatches(src),
        [
            mismatch(
                3,
                "Call to `collections.filter` has argument type(s) (List OF Integer, toString), \
                 expected List OF T, FUNC(T) AS Boolean."
            ),
            mismatch(
                4,
                "Call to `collections.forEach` has argument type(s) (List OF Integer, toString), \
                 expected List OF T, FUNC(T) AS Nothing."
            ),
            mismatch(
                5,
                "Call to `collections.reduce` has argument type(s) (List OF Integer, Integer, len), \
                 expected List OF T, U, FUNC(U, T) AS U."
            ),
        ]
    );
}

/// The same holds at a user function's callable parameter, where the
/// `Unknown` built-in used to pass the shape pass and fail native lowering
/// ("NIR local reference 'toString' does not resolve"); a predicate whose
/// element type it rejects (`isEmpty` over `Integer`) is the same mismatch.
/// A predicate that fits is still accepted.
#[test]
fn a_builtin_that_does_not_fit_a_callable_parameter_is_a_mismatch_naming_it() {
    let src = "\
FUNC useP(p AS FUNC(Integer) AS Boolean) AS Integer
  RETURN 0
END FUNC
FUNC main() AS Integer
  LET a AS Integer = useP(toString)
  LET b AS Integer = useP(isEmpty)
  LET c AS Integer = useP(isEven)
  RETURN 0
END FUNC
";
    let mismatch = |line: usize, name: &str| {
        (
            "TYPE_CALL_ARGUMENT_MISMATCH".to_string(),
            line,
            format!("Argument 1 for `useP` has type {name}, expected FUNC(Integer) AS Boolean."),
        )
    };
    assert_eq!(
        located_mismatches(src),
        [mismatch(5, "toString"), mismatch(6, "isEmpty")]
    );
}

/// A bare general built-in passed to a package member's plain (non-callable)
/// parameter is a mismatch naming the built-in in place of its type: through
/// the shared package table (`math::sqrt`) as through `collections`.
#[test]
fn a_builtin_passed_to_a_package_members_plain_parameter_is_named() {
    let src = "\
IMPORT math
IMPORT collections
FUNC main() AS Integer
  LET f AS Float = math::sqrt(toString)
  LET xs AS List OF Integer = collections::append([1], toString)
  RETURN 0
END FUNC
";
    assert_eq!(
        located_mismatches(src),
        [
            (
                "TYPE_CALL_ARGUMENT_MISMATCH".to_string(),
                4,
                "Call to `math.sqrt` has argument type(s) (toString), expected Float | Fixed."
                    .to_string()
            ),
            (
                "TYPE_CALL_ARGUMENT_MISMATCH".to_string(),
                5,
                "Call to `collections.append` has argument type(s) (List OF Integer, toString), \
                 expected List OF T, T or List OF T, List OF T."
                    .to_string()
            ),
        ]
    );
}

/// No general built-in shares its name with a registry member that takes a
/// unary callback. Lowering's typing of a general call relies on it: the
/// bare-predicate derivation (`filter(xs, isEven)`) belongs to the collections
/// members alone, so a general call never needs it.
#[test]
fn no_general_builtin_is_a_callback_member() {
    for name in [
        "error",
        "len",
        "typeName",
        "toString",
        "toInt",
        "toFloat",
        "toFixed",
        "toByte",
        "toMoney",
        "toScalar",
        "isNumeric",
        "isEven",
        "isOdd",
        "isPositive",
        "isNegative",
        "isZero",
        "isEmpty",
        "isNotEmpty",
    ] {
        assert!(
            crate::codegen::builtins::general::is_general_call(name),
            "{name} is a general built-in"
        );
        assert!(
            !crate::codegen::registry::callback_member_bare(name),
            "{name} is also a callback member"
        );
    }
    // The control: a collections callback member is one.
    assert!(crate::codegen::registry::callback_member_bare("filter"));
}

/// The cascade a mistyped initializer draws is the same under an inline
/// TRAP as without it, and a non-predicate built-in bound where a predicate
/// type is declared has no known type (it is no predicate), not an accepted
/// `Unknown`.
#[test]
fn unknown_initializers_cascade_under_an_inline_trap_and_for_a_non_predicate() {
    let src = "\
IMPORT math
FUNC main() AS Integer
  LET x AS Float = math::cos(1) TRAP(e)
    RECOVER 0.0
  END TRAP
  LET f AS FUNC(Integer) AS Boolean = toString
  RETURN 0
END FUNC
";
    let unknown = |binding: &str| {
        (
            "TYPE_UNKNOWN_VALUE".to_string(),
            format!("Initializer for binding `{binding}` does not have a known type."),
        )
    };
    assert_eq!(
        found(src),
        [
            (
                "TYPE_CALL_ARGUMENT_MISMATCH".to_string(),
                "Call to `math.cos` has argument type(s) (Integer), expected Float | Fixed."
                    .to_string()
            ),
            unknown("x"),
            unknown("f"),
        ]
    );
}

/// A bare general built-in passed to a general built-in (`len(toString)`) is
/// named as written, as it is at a package member's parameter, not reported as
/// an "(Unknown)" argument: lowering has no value type for the bare name.
#[test]
fn a_bare_builtin_at_a_general_builtin_is_named() {
    let src = "\
FUNC main() AS Integer
  LET n AS Integer = len(toString)
  RETURN n
END FUNC
";
    assert_eq!(
        mismatches(src),
        ["Call to `len` has argument type(s) (toString), \
          expected String, List OF T, Set OF T, or Map OF K TO V."]
    );
}
