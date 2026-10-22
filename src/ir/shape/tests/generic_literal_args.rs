//! A numeric literal at a GENERIC built-in parameter adopts the slot type the
//! call's other arguments fix, as it does at a typed binding or a user `FUNC`
//! parameter (`mfb spec architecture type-inference`, "Numeric literals at a
//! generic parameter"). `collections::append(bytes, 7)` was rejected because
//! selection saw `7` as an `Integer` against the pinned `T = Byte`.

use crate::testutil::check_src_details;

fn found(src: &str) -> Vec<(String, usize, String)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.line, d.detail))
        .collect()
}

#[test]
fn byte_literals_fit_every_pinned_collections_slot() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT xs AS List OF Byte = [1, 2]
  xs = collections::append(xs, 7)
  xs = collections::append(xs, [8, 255])
  xs = collections::prepend(xs, 0)
  xs = collections::insert(xs, 1, 9)
  xs = collections::set(xs, 0, 3)
  LET has AS Boolean = collections::contains(xs, 9)
  MUT m AS Map OF String TO Byte = Map OF String TO Byte { \"a\" := 1 }
  m = collections::set(m, \"b\", 2)
  LET got AS Byte = collections::getOr(m, \"z\", 5)
  RETURN 0
END FUNC
";
    assert_eq!(found(src), []);
}

#[test]
fn fixed_and_money_literals_fit_their_pinned_slots() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT fx AS List OF Fixed = [1.5]
  fx = collections::append(fx, 2)
  fx = collections::append(fx, 2.25)
  fx = collections::append(fx, -3)
  fx = collections::append(fx, 0.5f)
  MUT mo AS List OF Money = [1.50]
  mo = collections::append(mo, 2.25)
  mo = collections::append(mo, 4)
  RETURN 0
END FUNC
";
    assert_eq!(found(src), []);
}

#[test]
fn a_literal_outside_its_slot_is_still_an_argument_mismatch() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT xs AS List OF Byte = [1, 2]
  xs = collections::append(xs, 300)
  xs = collections::append(xs, -1)
  MUT mo AS List OF Money = [1.50]
  mo = collections::append(mo, 2.5f)
  RETURN 0
END FUNC
";
    let expected = |line: usize, actual: &str| {
        (
            "TYPE_CALL_ARGUMENT_MISMATCH".to_string(),
            line,
            format!(
                "Call to `collections.append` has argument type(s) ({actual}), \
                 expected List OF T, T or List OF T, List OF T."
            ),
        )
    };
    assert_eq!(
        found(src),
        [
            expected(4, "List OF Byte, Integer"),
            expected(5, "List OF Byte, Integer"),
            expected(7, "List OF Money, Float"),
        ]
    );
}

#[test]
fn an_out_of_range_list_literal_element_reports_the_byte_overflow() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT xs AS List OF Byte = [1, 2]
  xs = collections::append(xs, [4, 256])
  RETURN 0
END FUNC
";
    let rules: Vec<(String, usize)> = found(src)
        .into_iter()
        .map(|(rule, line, _)| (rule, line))
        .collect();
    assert_eq!(
        rules,
        [
            ("TYPE_CALL_ARGUMENT_MISMATCH".to_string(), 4),
            ("TYPE_BYTE_LITERAL_OVERFLOW".to_string(), 4),
        ]
    );
}

#[test]
fn a_literal_against_an_integer_list_is_not_retyped() {
    // The retry runs only when exact selection fails, so a call that already
    // selects keeps its meaning: `T = Integer`, no coercion.
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT xs AS List OF Integer = [1, 2]
  xs = collections::append(xs, 300)
  LET s AS String = \"x\"
  xs = collections::append(xs, s)
  RETURN 0
END FUNC
";
    let rules: Vec<(String, usize)> = found(src)
        .into_iter()
        .map(|(rule, line, _)| (rule, line))
        .collect();
    assert_eq!(rules, [("TYPE_CALL_ARGUMENT_MISMATCH".to_string(), 6)]);
}

#[test]
fn a_concrete_builtin_overload_set_keeps_exact_selection() {
    // `math::cos` is `Float` | `Fixed`, no type variable: the literal is not
    // re-typed, so the call stays the mismatch plus its Unknown-initializer
    // cascade (`syntax/math/func_math_cos_invalid`) — no `Fixed` result type
    // leaking into a TYPE_BINDING_MISMATCH.
    let src = "\
IMPORT math
FUNC main() AS Integer
  LET x AS Float = math::cos(1)
  RETURN 0
END FUNC
";
    let rules: Vec<(String, usize)> = found(src)
        .into_iter()
        .map(|(rule, line, _)| (rule, line))
        .collect();
    assert_eq!(
        rules,
        [
            ("TYPE_CALL_ARGUMENT_MISMATCH".to_string(), 3),
            ("TYPE_UNKNOWN_VALUE".to_string(), 3),
        ]
    );
}

#[test]
fn only_plain_numeric_literals_are_retyped() {
    // A suffixed Fixed/Money literal keeps its type, a negated variable is no
    // literal, and a list literal with any non-literal element is not re-typed
    // element-wise: each stays the argument mismatch exact selection gives.
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT xs AS List OF Byte = [1, 2]
  LET n AS Integer = 3
  xs = collections::append(xs, 2F)
  xs = collections::append(xs, 2m)
  xs = collections::append(xs, -n)
  xs = collections::append(xs, [n, 2])
  RETURN 0
END FUNC
";
    let expected = |line: usize, actual: &str| {
        (
            "TYPE_CALL_ARGUMENT_MISMATCH".to_string(),
            line,
            format!(
                "Call to `collections.append` has argument type(s) ({actual}), \
                 expected List OF T, T or List OF T, List OF T."
            ),
        )
    };
    assert_eq!(
        found(src),
        [
            expected(5, "List OF Byte, Fixed"),
            expected(6, "List OF Byte, Money"),
            expected(7, "List OF Byte, Integer"),
            expected(8, "List OF Byte, List OF Integer"),
        ]
    );
}
