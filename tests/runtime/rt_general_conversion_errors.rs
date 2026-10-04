//! The `general` conversions raise exactly the errors their pages declare
//! (bug-807).
//!
//! `toString` and `toInt` declared no errors while their lowering raised four:
//! `toString(<List OF Byte>)` decodes UTF-8 and raises `ErrEncoding`, and
//! `toInt` raises `ErrInvalidFormat`/`ErrOverflow` from its `String` parses and
//! `ErrOverflow` from a `Float` outside the `Integer` range. These pin each raise
//! the descriptors now list, per overload.
//!
//! The second half pins a sibling defect the per-overload measurement exposed:
//! `toInt`, `toFixed`, `toMoney` and the `isZero`/`isPositive`/`isNegative`
//! predicates did not treat a `Float` argument as an observation boundary
//! (spec `04_types.md`: "passed as an argument" includes every built-in's
//! argument). An arithmetic infinity reached `toInt` and came back as
//! `ErrInvalidFormat`, and `isNegative(0.0 / 0.0)` answered `TRUE`. They now raise
//! `ErrFloatOverflow` (77050015) / `ErrFloatNaN` (77050013) at the call, as
//! `toString`, `math::abs` and a user `FUNC` argument always did.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

fn run(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(60),
        "a handful of conversions must finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

fn assert_lines(actual: &[String], expected: &[&str]) {
    assert_eq!(
        actual,
        expected,
        "\nactual:\n{}\nexpected:\n{}",
        actual.join("\n"),
        expected.join("\n")
    );
}

/// One raise per declared code, through the overload that raises it.
#[test]
fn to_string_and_to_int_raise_what_they_declare() {
    let source = r#"IMPORT io

FUNC bytes(label AS String, b AS List OF Byte) AS Nothing
  io::print(label & " " & toString(b))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC text(label AS String, t AS String) AS Nothing
  io::print(label & " " & toString(toInt(t)))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC radix(label AS String, t AS String, base AS Integer) AS Nothing
  io::print(label & " " & toString(toInt(t, base)))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC number(label AS String, f AS Float) AS Nothing
  io::print(label & " " & toString(toInt(f)))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC main() AS Nothing
  bytes("utf8", [104, 105])
  bytes("bad-utf8", [255])
  text("parse", "42")
  text("not-a-number", "4x2")
  text("too-big", "99999999999999999999")
  radix("hex", "ff", 16)
  radix("bad-base", "ff", 1)
  radix("hex-too-big", "ffffffffffffffffff", 16)
  number("truncate", -1.9)
  number("float-too-big", 1.0e300)
END FUNC
"#;
    assert_lines(
        &run("general_conversion_raises", source),
        &[
            "utf8 hi",
            "bad-utf8 raised 77020004",
            "parse 42",
            "not-a-number raised 77050003",
            "too-big raised 77050010",
            "hex 255",
            "bad-base raised 77050003",
            "hex-too-big raised 77050010",
            "truncate -1",
            "float-too-big raised 77050010",
        ],
    );
}

/// A non-finite arithmetic `Float` argument is observed at the call.
#[test]
fn a_non_finite_float_argument_is_observed_at_the_conversion() {
    let source = r#"IMPORT io

FUNC probe(label AS String, x AS Float, y AS Float) AS Nothing
  IF label = "toInt" THEN io::print(label & " " & toString(toInt(x / y)))
  IF label = "toFixed" THEN io::print(label & " " & toString(toFixed(x / y)))
  IF label = "toMoney" THEN io::print(label & " " & toString(toMoney(x / y)))
  IF label = "isZero" THEN io::print(label & " " & toString(isZero(x / y)))
  IF label = "isPositive" THEN io::print(label & " " & toString(isPositive(x / y)))
  IF label = "isNegative" THEN io::print(label & " " & toString(isNegative(x / y)))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC main() AS Nothing
  FOR EACH l IN ["toInt", "toFixed", "toMoney", "isZero", "isPositive", "isNegative"]
    probe(l, 1.0, 0.0)
    probe(l, 0.0, 0.0)
    probe(l, 3.0, 2.0)
  NEXT
END FUNC
"#;
    assert_lines(
        &run("general_float_observation", source),
        &[
            "toInt raised 77050015",
            "toInt raised 77050013",
            "toInt 1",
            "toFixed raised 77050015",
            "toFixed raised 77050013",
            "toFixed 1.50",
            "toMoney raised 77050015",
            "toMoney raised 77050013",
            "toMoney 1.50",
            "isZero raised 77050015",
            "isZero raised 77050013",
            "isZero FALSE",
            "isPositive raised 77050015",
            "isPositive raised 77050013",
            "isPositive TRUE",
            "isNegative raised 77050015",
            "isNegative raised 77050013",
            "isNegative FALSE",
        ],
    );
}
