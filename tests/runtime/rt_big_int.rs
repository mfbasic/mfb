//! `big::Int` runtime behavior (plan-127).
//!
//! Every assertion here runs a real release-built program: lowering a member is not
//! proof it computes the right bytes. Expected values are written out by hand (or, for
//! large magnitudes, computed outside MFB and committed as literals) — never produced
//! by the code under test.
//!
//! A byte list handed to a `big` member is bound `AS List OF Byte` first: an integer
//! list literal in argument position types as `List OF Integer` and is rejected.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

fn run(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(60),
        &format!("{name} did not finish"),
    );
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

/// A `show(List OF Byte)` helper every byte-level program below shares.
const SHOW_BYTES: &str = r#"
FUNC show(bytes AS List OF Byte) AS String
  MUT s AS String = "["
  MUT i AS Integer = 0
  WHILE i < len(bytes)
    IF i > 0 THEN
      s = s & ","
    END IF
    s = s & toString(collections::get(bytes, i))
    i = i + 1
  END WHILE
  RETURN s & "]"
END FUNC
"#;

/// plan-127-A Phase 2: a `big::Int` declared without an initializer is canonical
/// zero — an empty magnitude and a `FALSE` sign — with no initialization seam. The
/// declaration is `MUT`: an immutable `LET` must have an initializer (Corrections C4).
#[test]
fn a_defaulted_int_is_canonical_zero() {
    let lines = run(
        "big_default_zero",
        r#"IMPORT io
IMPORT big

SUB main()
  MUT x AS big::Int
  io::print(toString(len(x.magnitude)))
  io::print(toString(x.negative))
END SUB
"#,
    );
    assert_eq!(lines, vec!["0", "FALSE"]);
}

/// plan-127-A Phase 4: `toInteger(fromInteger(x)) = x` at 0, ±1 and both `Integer`
/// extremes; `ErrOverflow` exactly one past each end.
#[test]
fn integer_round_trip_and_overflow_edges() {
    let lines = run(
        "big_integer_round_trip",
        r#"IMPORT io
IMPORT big

FUNC back(x AS big::Int) AS String
  RETURN toString(big::toInteger(x))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  LET twoTo63 AS List OF Byte = [0, 0, 0, 0, 0, 0, 0, 128]
  LET pastMinimum AS List OF Byte = [1, 0, 0, 0, 0, 0, 0, 128]
  LET twoTo64 AS List OF Byte = [0, 0, 0, 0, 0, 0, 0, 0, 1]
  io::print(back(big::fromInteger(0)))
  io::print(back(big::fromInteger(1)))
  io::print(back(big::fromInteger(-1)))
  io::print(back(big::fromInteger(9223372036854775807)))
  io::print(back(big::fromInteger(-9223372036854775807 - 1)))
  io::print(back(big::fromBytes(twoTo63, FALSE)))
  io::print(back(big::fromBytes(twoTo63, TRUE)))
  io::print(back(big::fromBytes(pastMinimum, TRUE)))
  io::print(back(big::fromBytes(twoTo64, FALSE)))
END SUB
"#,
    );
    assert_eq!(
        &lines[..5],
        &[
            "0",
            "1",
            "-1",
            "9223372036854775807",
            "-9223372036854775808"
        ],
        "{lines:?}"
    );
    // 2^63 does not fit; -2^63 is `Integer`'s minimum; -(2^63 + 1) and 2^64 do not fit.
    // 77050010 is `ErrOverflow` (`errorcode/mod.rs`).
    assert_eq!(lines[5], "raised 77050010", "2^63: {lines:?}");
    assert_eq!(lines[6], "-9223372036854775808", "-2^63: {lines:?}");
    assert_eq!(lines[7], "raised 77050010", "-(2^63+1): {lines:?}");
    assert_eq!(lines[8], "raised 77050010", "2^64: {lines:?}");
}

/// plan-127-A Phase 4: `toBytes(fromBytes(b, n, e), e) = b` for canonical `b` in both
/// orders, the `Little` default, and normalization of non-canonical input.
#[test]
fn byte_round_trips_in_both_orders() {
    let source = format!(
        r#"IMPORT io
IMPORT big
IMPORT collections
{SHOW_BYTES}
SUB main()
  LET ascending AS List OF Byte = [1, 2, 3]
  LET descending AS List OF Byte = [3, 2, 1]
  LET trailingZeros AS List OF Byte = [5, 0, 0]
  LET zeros AS List OF Byte = [0, 0]
  LET emptyBytes AS List OF Byte = []
  io::print(show(big::toBytes(big::fromBytes(ascending, FALSE, big::Endian.Little), big::Endian.Little)))
  io::print(show(big::toBytes(big::fromBytes(descending, FALSE, big::Endian.Big), big::Endian.Big)))
  io::print(show(big::toBytes(big::fromBytes(ascending, TRUE))))
  io::print(show(big::toBytes(big::fromBytes(descending, FALSE, big::Endian.Big))))
  io::print(show(big::toBytes(big::fromBytes(trailingZeros, FALSE))))
  LET trailing AS big::Int = big::fromBytes(trailingZeros, FALSE)
  io::print(toString(len(trailing.magnitude)))
  LET negativeZero AS big::Int = big::fromBytes(zeros, TRUE)
  io::print(toString(len(negativeZero.magnitude)) & " " & toString(negativeZero.negative))
  io::print(show(big::toBytes(big::fromBytes(emptyBytes, FALSE))))
  MUT b AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < 32
    b = collections::append(b, toByte(i * 7 + 1))
    i = i + 1
  END WHILE
  io::print(show(big::toBytes(big::fromBytes(b, FALSE, big::Endian.Big), big::Endian.Big)))
  io::print(show(big::toBytes(big::fromBytes(b, TRUE))))
  io::print(show(b))
END SUB
"#
    );
    let lines = run("big_byte_round_trip", &source);
    let thirty_two: Vec<String> = (0..32).map(|i| (i * 7 + 1).to_string()).collect();
    let thirty_two = format!("[{}]", thirty_two.join(","));
    assert_eq!(
        lines,
        vec![
            "[1,2,3]".to_string(),
            "[3,2,1]".to_string(),
            "[1,2,3]".to_string(),
            // A big-endian [3,2,1] read back in the default little-endian order.
            "[1,2,3]".to_string(),
            // Trailing zero bytes are not part of the value.
            "[5]".to_string(),
            "1".to_string(),
            "0 FALSE".to_string(),
            "[]".to_string(),
            thirty_two.clone(),
            thirty_two.clone(),
            thirty_two,
        ]
    );
}

/// plan-127-A Phase 5: `compare` is a total order over a spread covering both signs,
/// zero, and magnitudes past `Integer`; `equals`, `isZero` and `sign` agree with it.
#[test]
fn compare_is_a_total_order_and_the_predicates_agree() {
    // Ascending. 2^64 and -(2^64) are one byte past an `Integer`.
    let lines = run(
        "big_compare_order",
        r#"IMPORT io
IMPORT big
IMPORT collections

SUB main()
  LET twoTo64 AS List OF Byte = [0, 0, 0, 0, 0, 0, 0, 0, 1]
  LET values AS List OF big::Int = [big::fromBytes(twoTo64, TRUE), big::fromInteger(-70000), big::fromInteger(-256), big::fromInteger(-1), big::fromInteger(0), big::fromInteger(1), big::fromInteger(255), big::fromInteger(256), big::fromInteger(70000), big::fromBytes(twoTo64, FALSE)]
  MUT i AS Integer = 0
  WHILE i < len(values)
    MUT j AS Integer = 0
    MUT row AS String = ""
    WHILE j < len(values)
      LET a AS big::Int = collections::get(values, i)
      LET b AS big::Int = collections::get(values, j)
      MUT eq AS String = "n"
      IF big::equals(a, b) THEN
        eq = "y"
      END IF
      row = row & toString(big::compare(a, b)) & eq & " "
      j = j + 1
    END WHILE
    LET v AS big::Int = collections::get(values, i)
    io::print(row & "| " & toString(big::sign(v)) & " " & toString(big::isZero(v)))
    i = i + 1
  END WHILE
END SUB
"#,
    );
    let n = 10usize;
    let zero_index = 4usize;
    let expected: Vec<String> = (0..n)
        .map(|i| {
            let row: String = (0..n)
                .map(|j| {
                    let (order, eq) = match i.cmp(&j) {
                        std::cmp::Ordering::Less => ("-1", "n"),
                        std::cmp::Ordering::Equal => ("0", "y"),
                        std::cmp::Ordering::Greater => ("1", "n"),
                    };
                    format!("{order}{eq} ")
                })
                .collect();
            let sign = match i.cmp(&zero_index) {
                std::cmp::Ordering::Less => "-1",
                std::cmp::Ordering::Equal => "0",
                std::cmp::Ordering::Greater => "1",
            };
            let is_zero = if i == zero_index { "TRUE" } else { "FALSE" };
            format!("{row}| {sign} {is_zero}")
        })
        .collect();
    assert_eq!(lines, expected);
}

/// plan-127-A Phase 5: `negate`/`abs` never produce a negative zero, and a
/// hand-built non-canonical record reads as the number it spells.
#[test]
fn negate_abs_and_the_total_decoder() {
    let source = format!(
        r#"IMPORT io
IMPORT big
IMPORT collections
{SHOW_BYTES}
SUB main()
  LET x AS big::Int = big::fromInteger(-70000)
  io::print(toString(big::equals(big::negate(big::negate(x)), x)))
  LET negatedZero AS big::Int = big::negate(big::fromInteger(0))
  io::print(toString(len(negatedZero.magnitude)) & " " & toString(negatedZero.negative))
  io::print(toString(big::toInteger(big::abs(x))) & " " & toString(big::abs(x).negative))
  io::print(toString(big::toInteger(big::abs(big::fromInteger(12)))))
  io::print(toString(big::toInteger(big::negate(big::fromInteger(12)))))
  LET spelled AS big::Int = big::Int[[7, 0], FALSE]
  io::print(toString(big::equals(spelled, big::fromInteger(7))) & " " & toString(big::compare(spelled, big::fromInteger(7))))
  LET minusZero AS big::Int = big::Int[[], TRUE]
  io::print(toString(big::equals(minusZero, big::fromInteger(0))) & " " & toString(big::isZero(minusZero)) & " " & toString(big::sign(minusZero)))
  LET zeroBytes AS big::Int = big::Int[[0, 0, 0], TRUE]
  io::print(toString(big::isZero(zeroBytes)) & " " & toString(big::compare(zeroBytes, big::fromInteger(-1))))
  LET cleaned AS big::Int = big::negate(big::negate(spelled))
  io::print(show(cleaned.magnitude) & " " & toString(cleaned.negative))
END SUB
"#
    );
    let lines = run("big_negate_abs_decoder", &source);
    assert_eq!(
        lines,
        vec![
            "TRUE",
            "0 FALSE",
            "70000 FALSE",
            "12",
            "-12",
            "TRUE 0",
            "TRUE TRUE 0",
            "TRUE 1",
            // A member's result is canonical even from a non-canonical input.
            "[7] FALSE",
        ]
    );
}
