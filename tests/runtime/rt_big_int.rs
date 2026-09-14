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

/// plan-127-B Phase 1: a carry out of every byte, a borrow through a run of zeros, and
/// three ways to reach zero, each canonical (never a negative zero).
#[test]
fn additive_carry_and_borrow_chains() {
    let lines = run(
        "big_b_carry_borrow",
        r#"IMPORT io
IMPORT big
IMPORT collections

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

SUB main()
  LET allOnes AS List OF Byte = [255, 255, 255, 255, 255, 255, 255, 255, 255]
  LET twoTo72 AS List OF Byte = [0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
  LET one AS big::Int = big::fromInteger(1)
  io::print(show(big::toBytes(big::add(big::fromBytes(allOnes, FALSE), one))))
  io::print(show(big::toBytes(big::subtract(big::fromBytes(twoTo72, FALSE), one))))
  io::print(show(big::toBytes(big::add(one, big::fromBytes(allOnes, FALSE)))))
  io::print(show(big::toBytes(big::subtract(big::fromBytes(allOnes, TRUE), one))))
  LET x AS big::Int = big::fromBytes(allOnes, TRUE)
  LET zero AS big::Int = big::add(x, big::negate(x))
  io::print(toString(len(zero.magnitude)) & " " & toString(zero.negative))
  LET same AS big::Int = big::subtract(x, x)
  io::print(toString(len(same.magnitude)) & " " & toString(same.negative))
  LET oneMinusOne AS big::Int = big::add(one, big::fromInteger(-1))
  io::print(toString(len(oneMinusOne.magnitude)) & " " & toString(oneMinusOne.negative))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "[0,0,0,0,0,0,0,0,0,1]",
            "[255,255,255,255,255,255,255,255,255]",
            "[0,0,0,0,0,0,0,0,0,1]",
            "[0,0,0,0,0,0,0,0,0,1]",
            "0 FALSE",
            "0 FALSE",
            "0 FALSE"
        ]
    );
}

/// plan-127-B Phase 1: `add`/`subtract` agree with `Integer` arithmetic on every pair of a
/// signed spread, and commutativity, antisymmetry, `(a + b) - b = a` and associativity
/// hold on a spread that leaves the `Integer` range.
#[test]
fn additive_identities_and_integer_oracle() {
    let lines = run(
        "big_b_identities",
        r#"IMPORT io
IMPORT big
IMPORT collections

SUB main()
  LET small AS List OF Integer = [-70000, -65536, -300, -256, -255, -1, 0, 1, 255, 256, 300, 65535, 70000]
  MUT mismatches AS Integer = 0
  MUT checked AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(small)
    MUT j AS Integer = 0
    WHILE j < len(small)
      LET x AS Integer = collections::get(small, i)
      LET y AS Integer = collections::get(small, j)
      LET bx AS big::Int = big::fromInteger(x)
      LET by AS big::Int = big::fromInteger(y)
      IF big::toInteger(big::add(bx, by)) <> x + y THEN
        mismatches = mismatches + 1
      END IF
      IF big::toInteger(big::subtract(bx, by)) <> x - y THEN
        mismatches = mismatches + 1
      END IF
      checked = checked + 2
      j = j + 1
    END WHILE
    i = i + 1
  END WHILE
  io::print("integer oracle: " & toString(mismatches) & " mismatches of " & toString(checked))

  LET wide AS List OF Byte = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
  LET ones AS List OF Byte = [255, 255, 255, 255, 255, 255, 255, 255, 255, 255]
  LET spread AS List OF big::Int = [big::fromBytes(wide, TRUE), big::fromBytes(ones, FALSE), big::fromInteger(-1), big::fromInteger(0), big::fromInteger(1), big::fromInteger(9223372036854775807), big::fromInteger(-9223372036854775807 - 1), big::fromBytes(ones, TRUE), big::fromBytes(wide, FALSE)]
  MUT failures AS Integer = 0
  MUT cases AS Integer = 0
  MUT p AS Integer = 0
  WHILE p < len(spread)
    MUT q AS Integer = 0
    WHILE q < len(spread)
      LET a AS big::Int = collections::get(spread, p)
      LET b AS big::Int = collections::get(spread, q)
      IF NOT big::equals(big::add(a, b), big::add(b, a)) THEN
        failures = failures + 1
      END IF
      IF NOT big::equals(big::subtract(a, b), big::negate(big::subtract(b, a))) THEN
        failures = failures + 1
      END IF
      IF NOT big::equals(big::subtract(big::add(a, b), b), a) THEN
        failures = failures + 1
      END IF
      MUT r AS Integer = 0
      WHILE r < len(spread)
        LET c AS big::Int = collections::get(spread, r)
        IF NOT big::equals(big::add(big::add(a, b), c), big::add(a, big::add(b, c))) THEN
          failures = failures + 1
        END IF
        cases = cases + 1
        r = r + 1
      END WHILE
      cases = cases + 3
      q = q + 1
    END WHILE
    p = p + 1
  END WHILE
  io::print("identities: " & toString(failures) & " failures of " & toString(cases))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "integer oracle: 0 mismatches of 338",
            "identities: 0 failures of 972"
        ]
    );
}

/// plan-127-B Phase 2: products at 8, 64, 512 and 4096 bits against products computed by
/// Python and committed as literals; the four sign quadrants; zero; `sum`/`product`
/// against folding `add`/`multiply`; the empty-list identities; a zero inside a product.
#[test]
fn multiply_matches_an_independent_oracle() {
    let lines = run(
        "big_b_multiply_oracle",
        r#"IMPORT io
IMPORT big
IMPORT collections

FUNC operand(n AS Integer, mul AS Integer, add AS Integer) AS List OF Byte
  MUT b AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < n
    b = collections::append(b, toByte((i * mul + add) MOD 256))
    i = i + 1
  END WHILE
  RETURN b
END FUNC

SUB main()
  LET a1 AS big::Int = big::fromBytes(operand(1, 131, 7), FALSE)
  LET b1 AS big::Int = big::fromBytes(operand(1, 197, 3), FALSE)
  LET e1Bytes AS List OF Byte = [21]
  LET e1 AS big::Int = big::fromBytes(e1Bytes, FALSE)
  LET p1 AS big::Int = big::multiply(a1, b1)
  io::print("1: " & toString(big::equals(p1, e1)) & " " & toString(len(p1.magnitude)))
  LET a8 AS big::Int = big::fromBytes(operand(8, 131, 7), FALSE)
  LET b8 AS big::Int = big::fromBytes(operand(8, 197, 3), FALSE)
  LET e8Bytes AS List OF Byte = [21, 22, 217, 135, 17, 35, 179, 234, 47, 239, 78, 99, 23, 228, 148, 62]
  LET e8 AS big::Int = big::fromBytes(e8Bytes, FALSE)
  LET p8 AS big::Int = big::multiply(a8, b8)
  io::print("8: " & toString(big::equals(p8, e8)) & " " & toString(len(p8.magnitude)))
  LET a64 AS big::Int = big::fromBytes(operand(64, 131, 7), FALSE)
  LET b64 AS big::Int = big::fromBytes(operand(64, 197, 3), FALSE)
  LET e64Bytes AS List OF Byte = [21, 22, 217, 135, 17, 35, 179, 234, 185, 204, 26, 204, 209, 214, 204, 101, 15, 117, 135, 249, 55, 239, 14, 75, 15, 9, 39, 23, 207, 120, 3, 28, 185, 3, 236, 29, 145, 109, 164, 224, 19, 240, 223, 207, 110, 178, 196, 148, 206, 106, 144, 49, 247, 213, 124, 92, 29, 178, 204, 218, 135, 195, 66, 112, 208, 29, 183, 110, 26, 72, 5, 37, 124, 153, 137, 32, 50, 78, 71, 232, 69, 243, 192, 121, 51, 128, 49, 17, 54, 49, 213, 177, 212, 15, 57, 222, 12, 152, 84, 209, 26, 5, 100, 199, 1, 222, 179, 85, 84, 187, 94, 18, 102, 102, 230, 186, 114, 226, 209, 88, 7, 177, 30, 103, 27, 14, 9, 34]
  LET e64 AS big::Int = big::fromBytes(e64Bytes, FALSE)
  LET p64 AS big::Int = big::multiply(a64, b64)
  io::print("64: " & toString(big::equals(p64, e64)) & " " & toString(len(p64.magnitude)))
  LET a512 AS big::Int = big::fromBytes(operand(512, 131, 7), FALSE)
  LET b512 AS big::Int = big::fromBytes(operand(512, 197, 3), FALSE)
  LET e512Bytes AS List OF Byte = [21, 22, 217, 135, 17, 35, 179, 234, 185, 204, 26, 204, 209, 214, 204, 101, 15, 117, 135, 249, 55, 239, 14, 75, 15, 9, 39, 23, 207, 120, 3, 28, 185, 3, 236, 29, 145, 109, 164, 224, 19, 240, 223, 207, 110, 178, 196, 148, 206, 106, 144, 49, 247, 213, 124, 92, 29, 178, 204, 218, 135, 195, 66, 112, 250, 205, 155, 86, 43, 6, 150, 208, 223, 178, 245, 160, 219, 151, 126, 132, 88, 107, 101, 57, 153, 239, 40, 243, 69, 73, 238, 221, 13, 44, 170, 46, 175, 218, 33, 45, 238, 24, 25, 156, 142, 162, 201, 49, 197, 53, 118, 179, 217, 150, 226, 228, 142, 137, 202, 255, 154, 66, 236, 70, 195, 9, 12, 126, 200, 217, 92, 70, 69, 202, 123, 80, 246, 222, 176, 96, 159, 219, 191, 58, 253, 249, 93, 19, 205, 124, 80, 50, 211, 39, 91, 91, 211, 183, 183, 67, 2, 234, 168, 176, 167, 130, 242, 101, 135, 69, 81, 154, 142, 214, 103, 241, 227, 229, 236, 168, 137, 55, 164, 125, 185, 129, 194, 45, 182, 136, 143, 124, 67, 16, 207, 45, 30, 82, 55, 118, 4, 145, 140, 157, 185, 144, 146, 103, 1, 14, 131, 138, 13, 255, 15, 176, 134, 136, 94, 4, 157, 28, 45, 201, 21, 4, 65, 195, 180, 2, 92, 175, 178, 206, 177, 75, 81, 45, 139, 93, 85, 226, 171, 166, 123, 38, 202, 90, 130, 59, 171, 194, 46, 230, 18, 159, 122, 82, 31, 7, 252, 166, 5, 58, 56, 169, 136, 251, 243, 27, 102, 133, 229, 50, 92, 25, 210, 52, 48, 122, 125, 230, 164, 101, 33, 254, 237, 153, 255, 64, 82, 220, 218, 114, 150, 239, 112, 205, 109, 63, 239, 119, 254, 116, 131, 40, 134, 145, 240, 154, 62, 76, 107, 142, 106, 106, 58, 200, 204, 175, 32, 11, 33, 86, 213, 139, 38, 160, 29, 145, 162, 80, 189, 221, 87, 35, 238, 40, 123, 215, 242, 52, 139, 161, 115, 38, 173, 172, 31, 177, 212, 45, 180, 23, 200, 108, 247, 29, 75, 44, 172, 127, 151, 33, 7, 250, 239, 18, 80, 83, 24, 196, 73, 78, 204, 110, 169, 31, 201, 85, 53, 15, 213, 60, 174, 23, 34, 197, 174, 80, 77, 159, 243, 188, 158, 143, 64, 33, 218, 89, 82, 183, 180, 50, 230, 193, 242, 97, 192, 4, 89, 171, 164, 60, 33, 196, 199, 38, 142, 112, 113, 135, 99, 117, 101, 34, 94, 9, 145, 155, 33, 208, 26, 163, 98, 7, 4, 253, 229, 107, 135, 95, 224, 188, 231, 142, 153, 187, 232, 77, 213, 44, 71, 215, 73, 69, 194, 109, 187, 76, 27, 213, 236, 6, 23, 202, 24, 42, 231, 67, 241, 95, 52, 102, 155, 212, 46, 162, 213, 200, 155, 65, 102, 2, 63, 8, 14, 62, 81, 172, 255, 57, 19, 243, 132, 188, 74, 160, 97, 134, 180, 236, 75, 202, 14, 25, 9, 210, 31, 235, 92, 94, 182, 98, 179, 118, 195, 228, 241, 179, 66, 236, 192, 146, 114, 179, 101, 217, 156, 6, 34, 70, 252, 155, 51, 19, 205, 177, 212, 133, 213, 145, 209, 223, 211, 118, 224, 94, 2, 157, 63, 61, 166, 202, 59, 15, 146, 216, 174, 43, 155, 16, 91, 142, 251, 179, 6, 131, 126, 6, 111, 68, 221, 69, 209, 14, 82, 172, 110, 168, 40, 5, 139, 206, 155, 9, 100, 190, 231, 243, 51, 185, 210, 15, 198, 2, 26, 155, 155, 105, 21, 113, 143, 191, 21, 222, 170, 207, 90, 159, 42, 82, 36, 241, 74, 132, 172, 23, 210, 173, 190, 82, 125, 11, 19, 226, 136, 217, 230, 255, 56, 222, 129, 119, 205, 214, 32, 1, 134, 255, 2, 162, 45, 183, 141, 66, 39, 77, 4, 221, 42, 253, 162, 175, 116, 3, 173, 128, 79, 43, 101, 13, 244, 44, 7, 146, 160, 68, 207, 81, 27, 189, 136, 143, 33, 207, 235, 134, 239, 184, 52, 116, 200, 67, 118, 179, 11, 204, 137, 148, 254, 19, 107, 83, 223, 94, 226, 57, 121, 240, 172, 131, 132, 1, 7, 107, 61, 208, 51, 184, 237, 39, 119, 40, 209, 192, 9, 248, 33, 215, 41, 106, 168, 180, 165, 138, 178, 175, 212, 50, 27, 156, 134, 238, 35, 54, 246, 118, 9, 187, 95, 8, 8, 110, 138, 236, 233, 144, 50, 94, 104, 96, 149, 153, 191, 24, 246, 101, 57, 132, 150, 127, 16, 92, 179, 35, 129, 223, 80, 38, 109, 247, 216, 96, 159, 100, 196, 15, 83, 99, 80, 110, 204, 185, 198, 72, 77, 38, 98, 86, 18, 228, 94, 212, 86, 55, 131, 12, 231, 97, 141, 57, 122, 160, 184, 151, 77, 46, 73, 237, 173, 159, 17, 19, 117, 75, 230, 82, 101, 46, 2, 239, 68, 147, 48, 42, 209, 180, 42, 65, 71, 207, 43, 111, 233, 168, 126, 127, 250, 254, 93, 43, 183, 14, 6, 174, 90, 26, 61, 82, 176, 100, 192, 85, 57, 187, 231, 147, 208, 240, 3, 89, 128, 208, 85, 98, 132, 18, 27, 236, 24, 244, 142, 58, 4, 190, 124, 142, 3, 172, 156, 37, 83, 250, 43, 59, 55, 111, 115, 154, 240, 199, 174, 251, 188, 64, 29, 100, 104, 56, 36, 188, 85, 254, 6, 255, 60, 206, 1, 106, 90, 228, 87, 195, 248, 11, 76, 200, 81, 253, 25, 181, 162, 244, 253, 204, 177, 60, 195, 82, 60, 15, 34, 130, 126, 169, 86, 151, 187, 212, 115, 238, 81, 237, 82, 215, 136, 181, 240, 141, 157, 112, 20, 92, 91, 97, 123, 125, 122, 194, 97, 45, 55, 209, 10, 51, 219, 88, 185, 75, 162, 17, 168, 179, 199, 55, 20, 174, 18, 21, 204, 69, 209, 255, 38, 85, 222, 203, 244, 104, 123, 54, 112, 57, 228, 123, 213, 2, 86, 223, 235, 14, 156, 162, 112, 152, 110, 1, 160, 219, 9, 57, 189, 159, 183, 20, 10, 162, 178, 76, 194, 30, 55, 32]
  LET e512 AS big::Int = big::fromBytes(e512Bytes, FALSE)
  LET p512 AS big::Int = big::multiply(a512, b512)
  io::print("512: " & toString(big::equals(p512, e512)) & " " & toString(len(p512.magnitude)))
  LET na AS big::Int = big::negate(a8)
  LET nb AS big::Int = big::negate(b8)
  io::print(toString(big::equals(big::multiply(na, b8), big::negate(e8))) & " " & toString(big::equals(big::multiply(a8, nb), big::negate(e8))) & " " & toString(big::equals(big::multiply(na, nb), e8)))
  LET z AS big::Int = big::multiply(na, big::fromInteger(0))
  io::print(toString(len(z.magnitude)) & " " & toString(z.negative))
  LET values AS List OF big::Int = [a64, nb, big::fromInteger(-3), a8, b64]
  MUT folded AS big::Int = big::fromInteger(0)
  MUT multiplied AS big::Int = big::fromInteger(1)
  MUT k AS Integer = 0
  WHILE k < len(values)
    folded = big::add(folded, collections::get(values, k))
    multiplied = big::multiply(multiplied, collections::get(values, k))
    k = k + 1
  END WHILE
  io::print(toString(big::equals(big::sum(values), folded)) & " " & toString(big::equals(big::product(values), multiplied)))
  LET none AS List OF big::Int = []
  LET emptySum AS big::Int = big::sum(none)
  LET emptyProduct AS big::Int = big::product(none)
  io::print(toString(len(emptySum.magnitude)) & " " & toString(emptySum.negative) & " " & toString(big::toInteger(emptyProduct)))
  LET withZero AS List OF big::Int = [na, big::fromInteger(0), b64]
  LET zeroProduct AS big::Int = big::product(withZero)
  io::print(toString(len(zeroProduct.magnitude)) & " " & toString(zeroProduct.negative))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "1: TRUE 1",
            "8: TRUE 16",
            "64: TRUE 128",
            "512: TRUE 1024",
            "TRUE TRUE TRUE",
            "0 FALSE",
            "TRUE TRUE",
            "0 FALSE 1",
            "0 FALSE"
        ]
    );
}

/// plan-127-B Phase 3: `bitLength` at the byte boundaries; `shiftLeft` against `multiply`
/// by 256 and by 2^n for n in 1..64; `shiftRight(shiftLeft(x, n), n) = x`; truncation
/// toward zero; `testBit` against a subtraction; each negative argument raising
/// `ErrInvalidArgument`.
#[test]
fn bit_operations() {
    let lines = run(
        "big_b_bits",
        r#"IMPORT io
IMPORT big
IMPORT collections

FUNC tryShiftLeft(a AS big::Int, n AS Integer) AS String
  RETURN toString(big::bitLength(big::shiftLeft(a, n)))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryShiftRight(a AS big::Int, n AS Integer) AS String
  RETURN toString(big::bitLength(big::shiftRight(a, n)))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryTestBit(a AS big::Int, index AS Integer) AS String
  RETURN toString(big::testBit(a, index))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(toString(big::bitLength(big::fromInteger(0))) & " " & toString(big::bitLength(big::fromInteger(1))) & " " & toString(big::bitLength(big::fromInteger(255))) & " " & toString(big::bitLength(big::fromInteger(256))) & " " & toString(big::bitLength(big::fromInteger(-256))))

  LET twoTo64 AS List OF Byte = [0, 0, 0, 0, 0, 0, 0, 0, 1]
  LET ones AS List OF Byte = [255, 255, 255, 255, 255, 255, 255, 255, 255, 255]
  LET spread AS List OF big::Int = [big::fromInteger(1), big::fromInteger(255), big::fromInteger(-70000), big::fromBytes(twoTo64, FALSE), big::fromBytes(ones, TRUE), big::fromInteger(9223372036854775807)]
  LET two AS big::Int = big::fromInteger(2)
  LET oneHundredFiftySix AS big::Int = big::fromInteger(256)

  MUT byteShiftMismatches AS Integer = 0
  MUT shiftMismatches AS Integer = 0
  MUT roundTripMismatches AS Integer = 0
  MUT shiftChecks AS Integer = 0
  MUT bitMismatches AS Integer = 0
  MUT bitChecks AS Integer = 0
  MUT v AS Integer = 0
  WHILE v < len(spread)
    LET x AS big::Int = collections::get(spread, v)
    IF NOT big::equals(big::shiftLeft(x, 8), big::multiply(x, oneHundredFiftySix)) THEN
      byteShiftMismatches = byteShiftMismatches + 1
    END IF
    MUT power AS big::Int = big::fromInteger(1)
    MUT n AS Integer = 1
    WHILE n <= 64
      power = big::multiply(power, two)
      IF NOT big::equals(big::shiftLeft(x, n), big::multiply(x, power)) THEN
        shiftMismatches = shiftMismatches + 1
      END IF
      IF NOT big::equals(big::shiftRight(big::shiftLeft(x, n), n), x) THEN
        roundTripMismatches = roundTripMismatches + 1
      END IF
      shiftChecks = shiftChecks + 1
      n = n + 1
    END WHILE
    LET magnitude AS big::Int = big::abs(x)
    MUT i AS Integer = 0
    WHILE i < big::bitLength(x) + 8
      LET upper AS big::Int = big::shiftRight(magnitude, i)
      LET bit AS big::Int = big::subtract(upper, big::shiftLeft(big::shiftRight(magnitude, i + 1), 1))
      LET expected AS Boolean = NOT big::isZero(bit)
      IF big::testBit(x, i) <> expected THEN
        bitMismatches = bitMismatches + 1
      END IF
      bitChecks = bitChecks + 1
      i = i + 1
    END WHILE
    v = v + 1
  END WHILE
  io::print("shiftLeft 8 = multiply 256: " & toString(byteShiftMismatches) & " mismatches")
  io::print("shiftLeft n = multiply 2^n: " & toString(shiftMismatches) & " of " & toString(shiftChecks))
  io::print("shiftRight(shiftLeft): " & toString(roundTripMismatches) & " of " & toString(shiftChecks))
  io::print("testBit: " & toString(bitMismatches) & " of " & toString(bitChecks))

  io::print(toString(big::toInteger(big::shiftRight(big::fromInteger(-7), 1))) & " " & toString(big::toInteger(big::shiftRight(big::fromInteger(7), 1))))
  LET gone AS big::Int = big::shiftRight(big::fromInteger(-7), 1000)
  io::print(toString(len(gone.magnitude)) & " " & toString(gone.negative))
  LET zeroShifted AS big::Int = big::shiftLeft(big::fromInteger(0), 1000000)
  io::print(toString(len(zeroShifted.magnitude)) & " " & toString(zeroShifted.negative))

  LET one AS big::Int = big::fromInteger(1)
  io::print(tryShiftLeft(one, -1) & " | " & tryShiftRight(one, -1) & " | " & tryTestBit(one, -1))
  io::print(tryShiftLeft(one, 0) & " | " & tryShiftRight(one, 0) & " | " & tryTestBit(one, 0))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "0 1 8 9 9",
            "shiftLeft 8 = multiply 256: 0 mismatches",
            "shiftLeft n = multiply 2^n: 0 of 384",
            "shiftRight(shiftLeft): 0 of 384",
            "testBit: 0 of 282",
            "-3 3",
            "0 FALSE",
            "0 FALSE",
            "raised 77050002 | raised 77050002 | raised 77050002",
            "1 | 1 | TRUE"
        ]
    );
}

/// plan-127-B Phase 4: `toString`/`toRadixString` against Python's text for a spread past
/// the `Integer` range; `parse` round trips in every radix; the rejected texts
/// (`ErrInvalidFormat`) and radixes (`ErrInvalidArgument`); a 4096-bit product's decimal.
#[test]
fn text_matches_an_independent_oracle() {
    let lines = run(
        "big_b_text",
        r#"IMPORT io
IMPORT big
IMPORT collections

FUNC operand(n AS Integer, mul AS Integer, add AS Integer) AS List OF Byte
  MUT b AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < n
    b = collections::append(b, toByte((i * mul + add) MOD 256))
    i = i + 1
  END WHILE
  RETURN b
END FUNC

FUNC tryParse(text AS String, radix AS Integer) AS String
  RETURN big::toString(big::parse(text, radix))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryRadix(value AS big::Int, radix AS Integer) AS String
  RETURN big::toRadixString(value, radix)
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  LET twoTo64 AS List OF Byte = [0, 0, 0, 0, 0, 0, 0, 0, 1]
  LET spread AS List OF big::Int = [big::fromInteger(0), big::fromInteger(1), big::fromInteger(-1), big::fromInteger(9), big::fromInteger(10), big::fromInteger(-255), big::fromInteger(9223372036854775807), big::fromInteger(-9223372036854775807 - 1), big::fromBytes(twoTo64, FALSE), big::fromBytes(operand(40, 131, 7), TRUE)]
  LET decimals AS List OF String = ["0", "1", "-1", "9", "10", "-255", "9223372036854775807", "-9223372036854775808", "18446744073709551616", "-2106587317222212391832297503250513450982309751508901136347145804653738540457117282751973428136455"]
  LET hexadecimals AS List OF String = ["0", "1", "-1", "9", "a", "-ff", "7fffffffffffffff", "-8000000000000000", "10000000000000000", "-fc79f673f06dea67e461de5bd855d24fcc49c643c03dba37b431ae2ba825a21f9c199613900d8a07"]
  MUT decimalMismatches AS Integer = 0
  MUT hexMismatches AS Integer = 0
  MUT roundTrips AS Integer = 0
  MUT radixTrips AS Integer = 0
  MUT checks AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(spread)
    LET x AS big::Int = collections::get(spread, i)
    LET text AS String = big::toString(x)
    IF text <> collections::get(decimals, i) THEN
      decimalMismatches = decimalMismatches + 1
      io::print("decimal " & toString(i) & ": " & text)
    END IF
    IF big::toRadixString(x, 16) <> collections::get(hexadecimals, i) THEN
      hexMismatches = hexMismatches + 1
      io::print("hex " & toString(i) & ": " & big::toRadixString(x, 16))
    END IF
    IF NOT big::equals(big::parse(text), x) THEN
      roundTrips = roundTrips + 1
    END IF
    MUT radix AS Integer = 2
    WHILE radix <= 36
      IF NOT big::equals(big::parse(big::toRadixString(x, radix), radix), x) THEN
        radixTrips = radixTrips + 1
      END IF
      radix = radix + 1
    END WHILE
    checks = checks + 1
    i = i + 1
  END WHILE
  io::print("toString vs Python: " & toString(decimalMismatches) & " of " & toString(checks))
  io::print("toRadixString 16 vs Python: " & toString(hexMismatches) & " of " & toString(checks))
  io::print("parse(toString): " & toString(roundTrips) & " of " & toString(checks))
  io::print("parse(toRadixString) radix 2..36: " & toString(radixTrips) & " of " & toString(checks * 35))
  io::print(big::toString(big::fromInteger(0)) & " " & big::toString(big::fromInteger(-42)) & " " & big::toRadixString(big::fromInteger(255), 16) & " " & big::toRadixString(big::fromInteger(-5), 2) & " " & big::toRadixString(big::fromInteger(35), 36))
  io::print(tryParse("", 10) & " | " & tryParse("-", 10) & " | " & tryParse("12x", 10) & " | " & tryParse("+-1", 10) & " | " & tryParse("+5", 10))
  io::print(tryParse("007", 10) & " | " & tryParse("-0", 10) & " | " & tryParse("FF", 16) & " | " & tryParse("zZ", 36) & " | " & tryParse("2", 2))
  io::print(tryRadix(big::fromInteger(5), 1) & " | " & tryRadix(big::fromInteger(5), 37) & " | " & tryParse("5", 1) & " | " & tryParse("5", 37))
  LET a4096 AS big::Int = big::fromBytes(operand(512, 131, 7), FALSE)
  LET b4096 AS big::Int = big::fromBytes(operand(512, 197, 3), FALSE)
  LET productText AS String = big::toString(big::multiply(a4096, b4096))
  io::print("4096-bit product decimal matches Python: " & toString(productText = "137260908971024161284171554128698218510424693699068168217209054903607064674892954272753896430108198557065944290285244839606924389661937473320268737691625558143403268237540196126858144944442780116840549191228479297432396251798933009654900977786981538456825059922839483418611485451145946118587790743283146030999966457480128324879132926068052768579695968171344601509218684373885799666259763728197255492399961843097336780809713755935027137563762141236497910319157305052696111977809523325194880485126510328071646415471244067467561847817419868517887359299767048269463175280986639878655556673683898011958816820822864898760917077381202716021714817856723214899969940851153211172661837843242545426336746781874367991959385773838100907291051181680546053601254186248208862736933984952140244654621368450157431401551940894057739902902343662351456216032333961624184915440024267532728091427056600621341662726859211299687224077640493511256435459455897878857422581043724177110035631281128548715863115329372136114919831422336456713853905692220378436428135012622834440474860547806331456784795278021762472857546727686799316885665615194252569528369723504454306670226819832122520928407050631280902149353681216937829533905733322576673538109597974532017975432400965358481758146834147630740467580018160704022333240874040779216662257925342865971950993656953157649465534066829588644079804727174268312775414036584140597622908519122272990414638664698699546328334718294654407218875037577430204548427227995568382335009197353062223468390277664522395911491772227328130947394853178326641174649132820585650622377100285698685821524822211520991868644732644231352034109693732174798793167759515065209508278713223998837930661972631416461806688055723626089399902123291001055124161453170999049179945030126012733836810048458289279947447089161163343843376989324993382827791066080991322730149534006414162483512066638751628048510004732379854699185197325256318626930600275494760236144198303521846953357029179303261607289919506570516343101408318247273694906522076182061872584687906592424787739775518640581626316755535567731160287841756018147793584644300999775422227526739247142051938385670886147271317037320334753902387828870852645001526879095772921519957271245259921498357776968642494480331637022795608239511316855893825742444325514800278640994935075020976732566174306248447234490944158803516919812520006536454685057332258968449275439963331050256845387997786873662530149567027497591447972886923212398716491738453525"))
  io::print("4096-bit product parses back: " & toString(big::equals(big::parse(productText), big::multiply(a4096, b4096))))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "toString vs Python: 0 of 10",
            "toRadixString 16 vs Python: 0 of 10",
            "parse(toString): 0 of 10",
            "parse(toRadixString) radix 2..36: 0 of 350",
            "0 -42 ff -101 z",
            "raised 77050003 | raised 77050003 | raised 77050003 | raised 77050003 | raised 77050003",
            "7 | 0 | 255 | 1295 | raised 77050003",
            "raised 77050002 | raised 77050002 | raised 77050002 | raised 77050002",
            "4096-bit product decimal matches Python: TRUE",
            "4096-bit product parses back: TRUE"
        ]
    );
}

/// plan-127-B Validation Plan runtime proof: a 1000-term `big::sum` of mixed-sign 192-bit
/// values, printed through `toString`, against the total Python computes from the same
/// operand formula, and against folding `add`.
#[test]
fn a_thousand_term_sum_prints_the_independent_total() {
    let lines = run(
        "big_b_sum_1000",
        r#"IMPORT io
IMPORT big
IMPORT collections

FUNC operand(n AS Integer, mul AS Integer, add AS Integer) AS List OF Byte
  MUT b AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < n
    b = collections::append(b, toByte((i * mul + add) MOD 256))
    i = i + 1
  END WHILE
  RETURN b
END FUNC

SUB main()
  MUT terms AS List OF big::Int = []
  MUT folded AS big::Int = big::fromInteger(0)
  MUT k AS Integer = 0
  WHILE k < 1000
    LET term AS big::Int = big::fromBytes(operand(24, 131 + k MOD 50, k MOD 256), k MOD 3 = 0)
    terms = collections::append(terms, term)
    folded = big::add(folded, term)
    k = k + 1
  END WHILE
  LET total AS big::Int = big::sum(terms)
  io::print(toString(len(terms)) & " terms")
  io::print(big::toString(total))
  io::print(toString(big::equals(total, folded)))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "1000 terms",
            "1046487506421243044961144144988157756223431887292684862494154",
            "TRUE"
        ]
    );
}

/// plan-127-C Phase 1: Algorithm D's quiet branches on pinned operands — a pair whose D3
/// estimate correction runs and a pair whose D6 add-back runs, both found by an
/// instrumented simulation and checked against Python's `divmod` — plus the normalization
/// boundary (divisor top byte 127 and 128), a divisor longer than the dividend, a divisor
/// equal to the dividend and to its negation, and a one-byte divisor.
#[test]
fn division_targeted_cases() {
    let lines = run(
        "big_c_division_cases",
        r#"IMPORT io
IMPORT big

FUNC show(parts AS big::DivResult) AS String
  RETURN big::toString(parts.quotient) & " " & big::toString(parts.remainder)
END FUNC

SUB main()
  LET correctU AS List OF Byte = [15, 137, 103, 105]
  LET correctV AS List OF Byte = [50, 128]
  io::print(show(big::divMod(big::fromBytes(correctU, FALSE), big::fromBytes(correctV, FALSE))))
  LET addBackU AS List OF Byte = [38, 17, 133, 176]
  LET addBackV AS List OF Byte = [167, 28, 129]
  io::print(show(big::divMod(big::fromBytes(addBackU, FALSE), big::fromBytes(addBackV, FALSE))))
  LET dividend AS List OF Byte = [255, 255, 255, 255, 255, 255]
  LET below AS List OF Byte = [255, 127]
  LET at AS List OF Byte = [0, 128]
  io::print(show(big::divMod(big::fromBytes(dividend, FALSE), big::fromBytes(below, FALSE))))
  io::print(show(big::divMod(big::fromBytes(dividend, FALSE), big::fromBytes(at, FALSE))))
  io::print(show(big::divMod(big::fromInteger(-12345), big::fromBytes(dividend, FALSE))))
  LET wide AS big::Int = big::fromBytes(dividend, TRUE)
  io::print(show(big::divMod(wide, wide)))
  io::print(show(big::divMod(wide, big::negate(wide))))
  io::print(show(big::divMod(big::fromBytes(dividend, FALSE), big::fromInteger(7))))
END SUB
"#,
    );
    // Expected values computed by Python `divmod` (non-negative operands) and by the
    // measured truncating rule for the signed ones — never by this code.
    assert_eq!(
        lines,
        vec![
            "53884 27863",
            "349 8454523",
            "8590196744 7",
            "8589934591 32767",
            "0 -12345",
            "1 0",
            "-1 0",
            "40210710958665 0",
        ]
    );
}

/// plan-127-C Phase 1: the defining property over 10,000 seeded random pairs spanning 1 to
/// 512 bytes of magnitude and all four sign quadrants — `a = b * q + r`, `|r| < |b|`, the
/// remainder takes the dividend's sign and the quotient truncates toward zero. Quotient and
/// remainder are unique under those rules, so the property is a complete check; the seed
/// is fixed so a failure reproduces.
#[test]
fn division_property_over_ten_thousand_seeded_pairs() {
    let project = common::temp_project(
        "big_c_division_property",
        r#"IMPORT io
IMPORT big
IMPORT collections
IMPORT math

FUNC randomMagnitude(length AS Integer) AS List OF Byte
  MUT bytes AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < length
    bytes = collections::append(bytes, toByte(math::rand(0, 255)))
    i = i + 1
  END WHILE
  RETURN bytes
END FUNC

SUB main()
  math::seed(127)
  MUT identityFailures AS Integer = 0
  MUT boundFailures AS Integer = 0
  MUT signFailures AS Integer = 0
  MUT quadrants AS List OF Integer = [0, 0, 0, 0]
  MUT shorterDivisors AS Integer = 0
  MUT pair AS Integer = 0
  WHILE pair < 10000
    MUT a AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 512)), FALSE)
    MUT b AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 512)), FALSE)
    IF big::isZero(b) THEN
      b = big::fromInteger(1)
    END IF
    LET quadrant AS Integer = math::rand(0, 3)
    IF quadrant = 1 OR quadrant = 3 THEN
      a = big::negate(a)
    END IF
    IF quadrant = 2 OR quadrant = 3 THEN
      b = big::negate(b)
    END IF
    quadrants = collections::set(quadrants, quadrant, collections::get(quadrants, quadrant) + 1)
    IF len(b.magnitude) < len(a.magnitude) THEN
      shorterDivisors = shorterDivisors + 1
    END IF
    LET parts AS big::DivResult = big::divMod(a, b)
    IF NOT big::equals(big::add(big::multiply(b, parts.quotient), parts.remainder), a) THEN
      identityFailures = identityFailures + 1
    END IF
    IF big::compare(big::abs(parts.remainder), big::abs(b)) >= 0 THEN
      boundFailures = boundFailures + 1
    END IF
    IF NOT big::isZero(parts.remainder) AND big::sign(parts.remainder) <> big::sign(a) THEN
      signFailures = signFailures + 1
    END IF
    IF NOT big::isZero(parts.quotient) AND big::sign(parts.quotient) <> big::sign(a) * big::sign(b) THEN
      signFailures = signFailures + 1
    END IF
    pair = pair + 1
  END WHILE
  io::print("pairs " & toString(pair))
  io::print("shorter " & toString(shorterDivisors))
  io::print("quadrants " & toString(collections::get(quadrants, 0)) & " " & toString(collections::get(quadrants, 1)) & " " & toString(collections::get(quadrants, 2)) & " " & toString(collections::get(quadrants, 3)))
  io::print("identity " & toString(identityFailures))
  io::print("bound " & toString(boundFailures))
  io::print("sign " & toString(signFailures))
END SUB
"#,
    );
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(600),
        "the 10,000-pair division property did not finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status)
    );
    let _ = std::fs::remove_dir_all(&project);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "pairs 10000", "{stdout}");
    // Both shapes of divisor were exercised, and every quadrant was drawn.
    let shorter: usize = lines[1].trim_start_matches("shorter ").parse().expect("shorter count");
    assert!(shorter > 1000 && shorter < 9000, "{stdout}");
    let quadrants: Vec<usize> = lines[2]
        .trim_start_matches("quadrants ")
        .split(' ')
        .map(|q| q.parse().expect("quadrant count"))
        .collect();
    assert!(quadrants.iter().all(|&q| q > 1000), "{stdout}");
    assert_eq!(&lines[3..], &["identity 0", "bound 0", "sign 0"], "{stdout}");
}

/// plan-127-C Phase 2: `divide`/`remainder` agree with the matching `divMod` field across a
/// seeded spread in every sign quadrant; the §4.2 truncating convention on `(±7, ±2)`, which
/// matches the measured `MOD` (`-7 MOD 2 = -1`, `7 MOD -2 = 1`); each division member raises
/// `ErrInvalidArgument` on a zero divisor.
#[test]
fn division_members_agree_and_raise() {
    let lines = run(
        "big_c_division_members",
        r#"IMPORT io
IMPORT big
IMPORT collections
IMPORT math

FUNC randomMagnitude(length AS Integer) AS List OF Byte
  MUT bytes AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < length
    bytes = collections::append(bytes, toByte(math::rand(0, 255)))
    i = i + 1
  END WHILE
  RETURN bytes
END FUNC

FUNC tryDivide(a AS big::Int, b AS big::Int) AS String
  RETURN big::toString(big::divide(a, b))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryRemainder(a AS big::Int, b AS big::Int) AS String
  RETURN big::toString(big::remainder(a, b))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryDivMod(a AS big::Int, b AS big::Int) AS String
  LET parts AS big::DivResult = big::divMod(a, b)
  RETURN big::toString(parts.quotient) & " " & big::toString(parts.remainder)
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  math::seed(128)
  MUT disagreements AS Integer = 0
  MUT pair AS Integer = 0
  WHILE pair < 500
    MUT a AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 64)), math::rand(0, 1) = 1)
    MUT b AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 64)), math::rand(0, 1) = 1)
    IF big::isZero(b) THEN
      b = big::fromInteger(3)
    END IF
    LET parts AS big::DivResult = big::divMod(a, b)
    IF NOT big::equals(big::divide(a, b), parts.quotient) THEN
      disagreements = disagreements + 1
    END IF
    IF NOT big::equals(big::remainder(a, b), parts.remainder) THEN
      disagreements = disagreements + 1
    END IF
    pair = pair + 1
  END WHILE
  io::print("agreement: " & toString(disagreements) & " of " & toString(pair * 2))
  LET seven AS big::Int = big::fromInteger(7)
  LET two AS big::Int = big::fromInteger(2)
  io::print(tryDivMod(seven, two) & " | " & tryDivMod(big::negate(seven), two) & " | " & tryDivMod(seven, big::negate(two)) & " | " & tryDivMod(big::negate(seven), big::negate(two)))
  LET zero AS big::Int = big::fromInteger(0)
  io::print(tryDivide(seven, zero) & " | " & tryRemainder(seven, zero) & " | " & tryDivMod(seven, zero))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "agreement: 0 of 1000",
            "3 1 | -3 -1 | -3 1 | 3 -1",
            "raised 77050002 | raised 77050002 | raised 77050002"
        ]
    );
}

/// plan-127-C Phase 3: `factorial` (0, 1, 20, 100), `pow`, `gcd` (signs, zero, a large
/// shared factor, consecutive Fibonacci numbers) and `modPow` against values computed by
/// Python; `modPow` against `remainder(pow)` over 390 small signed cases; `pow` against a
/// folded `multiply`; and the four `ErrInvalidArgument` cases.
#[test]
fn powers_gcd_factorial_and_mod_pow_match_an_independent_oracle() {
    let lines = run(
        "big_c_powers",
        r#"IMPORT io
IMPORT big

FUNC tryPow(n AS Integer) AS String
  RETURN big::toString(big::pow(big::fromInteger(2), n))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryFactorial(n AS Integer) AS String
  RETURN big::toString(big::factorial(n))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryModPow(e AS Integer, m AS Integer) AS String
  RETURN big::toString(big::modPow(big::fromInteger(3), big::fromInteger(e), big::fromInteger(m)))
  TRAP(err)
    RETURN "raised " & toString(err.code)
  END TRAP
END FUNC

SUB main()
  io::print(big::toString(big::factorial(0)))
  io::print(big::toString(big::factorial(1)))
  io::print(big::toString(big::factorial(20)))
  io::print(big::toString(big::factorial(100)))
  io::print(big::toString(big::pow(big::fromInteger(7), 0)))
  io::print(big::toString(big::pow(big::fromInteger(0), 0)))
  io::print(big::toString(big::pow(big::fromInteger(-13), 1)))
  io::print(big::toString(big::pow(big::fromInteger(2), 100)))
  io::print(big::toString(big::pow(big::fromInteger(-3), 41)))
  io::print(big::toString(big::pow(big::parse("123456789123456789"), 7)))
  io::print(big::toString(big::gcd(big::fromInteger(-12), big::fromInteger(18))))
  io::print(big::toString(big::gcd(big::fromInteger(0), big::fromInteger(-7))))
  io::print(big::toString(big::gcd(big::fromInteger(0), big::fromInteger(0))))
  io::print(big::toString(big::gcd(big::parse("47660691512487140583616282547"), big::parse("-131194280351434800284477681165901434922899313011950236555"))))
  io::print(big::toString(big::gcd(big::parse("581811569836004006491505558634099066259034153405766997246569401"), big::parse("359579325206583560961765665172189099052367214309267232255589801"))))
  io::print(big::toString(big::modPow(big::parse("3"), big::parse("1000000000000000000000000000000"), big::parse("1000000007"))))
  io::print(big::toString(big::modPow(big::parse("-2"), big::parse("3"), big::parse("5"))))
  io::print(big::toString(big::modPow(big::parse("-2"), big::parse("4"), big::parse("5"))))
  io::print(big::toString(big::modPow(big::parse("5"), big::parse("0"), big::parse("13"))))
  io::print(big::toString(big::modPow(big::parse("5"), big::parse("0"), big::parse("1"))))
  io::print(big::toString(big::modPow(big::parse("5"), big::parse("0"), big::parse("-1"))))
  io::print(big::toString(big::modPow(big::parse("12345678901234567890"), big::parse("987654321"), big::parse("-1000000000000000003"))))
  io::print(big::toString(big::modPow(big::parse("-7"), big::parse("18446744073709551617"), big::parse("170141183460469231731687303715884105727"))))
  MUT modPowMismatches AS Integer = 0
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  io::print("modPow vs remainder(pow): " & toString(modPowMismatches) & " of 390")
  MUT powMismatches AS Integer = 0
  MUT folded AS big::Int = big::fromInteger(1)
  MUT e AS Integer = 0
  WHILE e <= 40
    IF NOT big::equals(big::pow(big::fromInteger(-37), e), folded) THEN
      powMismatches = powMismatches + 1
    END IF
    folded = big::multiply(folded, big::fromInteger(-37))
    e = e + 1
  END WHILE
  io::print("pow vs folded multiply: " & toString(powMismatches) & " of 41")
  io::print(tryPow(-1) & " | " & tryFactorial(-1) & " | " & tryModPow(-1, 7) & " | " & tryModPow(2, 0))
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "1",
            "1",
            "2432902008176640000",
            "93326215443944152681699238856266700490715968264381621468592963895217599993229915608941463976156518286253697920827223758251185210916864000000000000000000000000",
            "1",
            "1",
            "-13",
            "1267650600228229401496703205376",
            "-36472996377170786403",
            "437124192680754946535812089517240575642356431671694191678580822912775854282944444159297631378312438244249145659881926429",
            "6",
            "7",
            "0",
            "618970019642690137449562111",
            "1",
            "965115194",
            "-3",
            "1",
            "1",
            "0",
            "0",
            "745465334772758213",
            "-100316993018521585394941311808528038793",
            "modPow vs remainder(pow): 0 of 390",
            "pow vs folded multiply: 0 of 41",
            "raised 77050002 | raised 77050002 | raised 77050002 | raised 77050002"
        ]
    );
}

/// plan-127-D Phase 1: the `big::Int` overload of `crypto::randomInt` — `max < min` raises
/// `ErrInvalidArgument`, `min = max` returns `min`, 1,000 draws over a ±2^128 range (a span
/// the `Integer` overload rejects) all land in range and nearly all past `Integer`, and
/// 10,000 draws over `0..6` hit every outcome with no bucket more than 25% from the expected
/// 1,429 — the check a modulo-reduction bias fails.
#[test]
fn crypto_random_int_big_overload_is_in_range_and_uniform() {
    let lines = run(
        "big_d_random_int",
        r#"IMPORT io
IMPORT big
IMPORT crypto
IMPORT collections

FUNC tryDraw(low AS big::Int, high AS big::Int) AS String
  RETURN big::toString(crypto::randomInt(low, high))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(tryDraw(big::fromInteger(5), big::fromInteger(4)))
  LET huge AS big::Int = big::parse("123456789012345678901234567890")
  io::print(tryDraw(huge, huge))

  LET low AS big::Int = big::parse("-340282366920938463463374607431768211456")
  LET high AS big::Int = big::parse("340282366920938463463374607431768211455")
  LET integerMax AS big::Int = big::fromInteger(9223372036854775807)
  MUT outside AS Integer = 0
  MUT pastInteger AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 1000
    LET x AS big::Int = crypto::randomInt(low, high)
    IF big::compare(x, low) < 0 OR big::compare(x, high) > 0 THEN
      outside = outside + 1
    END IF
    IF big::compare(big::abs(x), integerMax) > 0 THEN
      pastInteger = pastInteger + 1
    END IF
    i = i + 1
  END WHILE
  io::print("outside: " & toString(outside) & " of 1000")
  io::print("past Integer: " & toString(pastInteger > 990))

  LET zero AS big::Int = big::fromInteger(0)
  LET six AS big::Int = big::fromInteger(6)
  MUT counts AS List OF Integer = [0, 0, 0, 0, 0, 0, 0]
  MUT j AS Integer = 0
  WHILE j < 10000
    LET v AS Integer = big::toInteger(crypto::randomInt(zero, six))
    counts = collections::set(counts, v, collections::get(counts, v) + 1)
    j = j + 1
  END WHILE
  MUT everyOutcome AS Boolean = TRUE
  MUT withinTolerance AS Boolean = TRUE
  MUT total AS Integer = 0
  MUT shown AS String = ""
  MUT k AS Integer = 0
  WHILE k < 7
    LET c AS Integer = collections::get(counts, k)
    total = total + c
    shown = shown & " " & toString(c)
    IF c = 0 THEN
      everyOutcome = FALSE
    END IF
    IF c < 1072 OR c > 1786 THEN
      withinTolerance = FALSE
    END IF
    k = k + 1
  END WHILE
  io::print("draws: " & toString(total))
  io::print("every outcome: " & toString(everyOutcome))
  io::print("within 25% of 1429: " & toString(withinTolerance))
  io::print("counts:" & shown)
END SUB
"#,
    );
    assert_eq!(
        &lines[..7],
        &[
            "raised 77050002",
            "123456789012345678901234567890",
            "outside: 0 of 1000",
            "past Integer: TRUE",
            "draws: 10000",
            "every outcome: TRUE",
            "within 25% of 1429: TRUE"
        ],
        "{lines:?}"
    );
}
