//! `math::sin`, `math::cos` and `math::tan` on a `Float` are within one ULP of the
//! true value at every finite angle, including angles so large that adjacent
//! `Float` values are more than 2*pi apart (bug-618).
//!
//! The kernel reduced `x` by `q * pi/2` with fdlibm's three-part Cody-Waite
//! constants, which are exact only while `q` fits the ~33 bits of `PIO2_1`
//! (|x| < 2^20 * pi/2). Past that the reduced angle drifted — `sin(2^53)` was off in
//! its eighth digit — and once `q` no longer fit the quadrant conversion the
//! "reduced" angle was not in `[-pi/4, pi/4]` at all, so the polynomial ran far
//! outside its interval: `sin(1e20)` returned 1.96e29.
//!
//! **The oracle is an exact big-integer evaluation.** A finite `Float` is exactly
//! `m * 2^e`, so the test reduces it by pi/2 (Machin's formula) and sums the sin/cos
//! Taylor series in 1400-bit fixed point: enough for `f64::MAX`, and for the
//! double closest to a multiple of pi/2 (`6381956970095103 * 2^797`, whose cosine
//! is about 4.7e-19). No host libm is involved, so a host whose libm is itself a
//! ULP off cannot move the verdict.

#[path = "../common/mod.rs"]
mod common;

use num_bigint::BigInt;
use std::time::Duration;

/// Working precision of the oracle, in bits after the binary point.
const W: u32 = 1400;

fn big(v: i64) -> BigInt {
    BigInt::from(v)
}

fn pow2(bits: u32) -> BigInt {
    BigInt::from(1) << bits
}

fn div_floor(a: &BigInt, b: &BigInt) -> BigInt {
    let q = a / b;
    if (a % b) < big(0) {
        q - 1
    } else {
        q
    }
}

fn atan_inverse(n: i64) -> BigInt {
    let n2 = big(n * n);
    let mut power = pow2(W) / big(n);
    let mut sum = big(0);
    let mut k = 0i64;
    while power != big(0) {
        let term = &power / big(2 * k + 1);
        if k % 2 == 0 {
            sum += term;
        } else {
            sum -= term;
        }
        power /= &n2;
        k += 1;
    }
    sum
}

fn pi_over_2() -> BigInt {
    (big(16) * atan_inverse(5) - big(4) * atan_inverse(239)) / big(2)
}

fn sin_cos_small(r: &BigInt) -> (BigInt, BigInt) {
    let r2 = (r * r) >> W;
    let mut sin = r.clone();
    let mut term = r.clone();
    let mut k = 1i64;
    loop {
        term = -((&term * &r2) >> W) / big((2 * k) * (2 * k + 1));
        if term == big(0) {
            break;
        }
        sin += &term;
        k += 1;
    }
    let mut cos = pow2(W);
    let mut term = pow2(W);
    let mut k = 1i64;
    loop {
        term = -((&term * &r2) >> W) / big((2 * k - 1) * (2 * k));
        if term == big(0) {
            break;
        }
        cos += &term;
        k += 1;
    }
    (sin, cos)
}

/// `v` exactly, scaled by 2^W.
fn scaled(v: f64) -> BigInt {
    assert!(v.is_finite());
    if v == 0.0 {
        return big(0);
    }
    let bits = v.to_bits();
    let exponent = ((bits >> 52) & 0x7FF) as i32;
    let fraction = (bits & ((1u64 << 52) - 1)) as i64;
    let (mantissa, e) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1i64 << 52), exponent - 1075)
    };
    let shift = e + W as i32;
    assert!(shift >= 0, "{v} is below the oracle's precision");
    let magnitude = big(mantissa) << shift as u32;
    if v < 0.0 {
        -magnitude
    } else {
        magnitude
    }
}

/// `(sin x, cos x, tan x)`, each scaled by 2^W.
fn truth(x: f64, pio2: &BigInt) -> (BigInt, BigInt, BigInt) {
    let xs = scaled(x);
    let k = div_floor(&(&xs * 2 + pio2), &(pio2 * 2));
    let r = &xs - &k * pio2;
    let (s, c) = sin_cos_small(&r);
    let quadrant = i64::try_from(((k % 4) + 4) % 4).unwrap();
    let (sin, cos) = match quadrant {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    };
    let tan = (&sin << W) / &cos;
    (sin, cos, tan)
}

/// One ULP of `v` (the gap to the next double away from zero), scaled by 2^W.
fn ulp_scaled(v: f64) -> BigInt {
    let a = v.abs();
    scaled(f64::from_bits(a.to_bits() + 1)) - scaled(a)
}

/// Whether `actual` is within one ULP of `exact` (both judged at `actual`'s ULP).
fn within_one_ulp(actual: f64, exact: &BigInt) -> bool {
    let diff = scaled(actual) - exact;
    let diff = if diff < big(0) { -diff } else { diff };
    diff <= ulp_scaled(actual)
}

/// An MFBASIC Float literal that parses to exactly `v` (the exact decimal
/// expansion; the lexer has no exponent notation).
fn literal(v: f64) -> String {
    if v.abs() >= 2f64.powi(60) {
        format!("{v:.1}")
    } else {
        format!("{v:.80}")
    }
}

fn corpus() -> Vec<f64> {
    let mut xs = vec![
        0.5,
        1.0,
        3.0,
        100.0,
        1.0e6,
        1.6e6,
        1.65e6,
        2f64.powi(21),
        1.0e7,
        1.0e9,
        1.0e12,
        2f64.powi(52),
        2f64.powi(53),
        1.0e20,
        1.0e22,
        1.0e100,
        1.0e300,
        f64::MAX,
        2f64.powi(1023),
        1125899906842624.8,
        // The double closest to a multiple of pi/2 (cos ~ -4.7e-19).
        6381956970095103.0 * 2f64.powi(797),
    ];
    // Angles where the medium path itself was more than one ULP out, because it
    // discarded the low half of its own reduction (measured at 798870ec2: 1.29,
    // 1.39, 1.52 and 2.17 ULP) — well inside the range the spec claims at <=1 ULP.
    xs.extend([
        16.223429914714487,
        842522.8010648803,
        417085.1813766881,
        413441.44719405076,
    ]);
    // 2^20 * pi/2 and its neighbourhood, where the medium reduction stops being exact.
    let edge = 2f64.powi(20) * std::f64::consts::FRAC_PI_2;
    xs.extend([edge, edge * 1.001, edge * 2.0, edge * 64.0]);
    // Pseudo-random doubles with exponents spread over [2^20, 2^1024).
    let mut state = 0x618u64;
    for i in 0..360 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let exponent = 20 + (state >> 40) % 1004;
        let fraction = state & ((1u64 << 52) - 1);
        let v = f64::from_bits(((exponent + 1023) << 52) | fraction);
        xs.push(if i % 2 == 0 { v } else { -v });
    }
    let negatives: Vec<f64> = xs.iter().take(25).map(|v| -v).collect();
    xs.extend(negatives);
    xs
}

/// One argument's results: scalar sin/cos/tan, then the same three through a
/// two-lane `List OF Float` call (`[x, -x]`, first lane). Each is `Ok(value)` or
/// `Err(line)` when the call raised.
struct Row {
    x: f64,
    results: [Result<f64, String>; 6],
}

const NAMES: [&str; 6] = ["sin", "cos", "tan", "sin(List)", "cos(List)", "tan(List)"];

fn run(xs: &[f64]) -> Vec<Row> {
    let list = xs
        .iter()
        .map(|v| literal(*v))
        .collect::<Vec<_>>()
        .join(", ");
    let mut funcs = String::new();
    for f in ["sin", "cos", "tan"] {
        funcs.push_str(&format!(
            r#"
FUNC {f}Of(x AS Float) AS String
  RETURN toString(math::{f}(x), toByte(255))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC {f}ListOf(x AS Float) AS String
  LET pair AS List OF Float = [x, 0.0 - x]
  RETURN toString(collections::get(math::{f}(pair), 0), toByte(255))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC
"#
        ));
    }
    let source = format!(
        r#"IMPORT io
IMPORT math
IMPORT collections
{funcs}
SUB main()
  LET xs AS List OF Float = [{list}]
  MUT i AS Integer = 0
  WHILE i < len(xs)
    LET x AS Float = collections::get(xs, i)
    io::print(toString(x, toByte(255)) & "|" & sinOf(x) & "|" & cosOf(x) & "|" & tanOf(x) & "|" & sinListOf(x) & "|" & cosListOf(x) & "|" & tanListOf(x))
    i = i + 1
  END WHILE
END SUB
"#
    );
    let project = common::temp_project("math_float_trig_large_angles", &source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(120),
        "Float sin/cos/tan over the corpus must finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status)
    );
    let _ = std::fs::remove_dir_all(&project);
    let rows: Vec<Row> = stdout
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            assert_eq!(parts.len(), 7, "row {line:?}");
            let x = parts[0]
                .parse::<f64>()
                .unwrap_or_else(|_| panic!("argument {:?}", parts[0]));
            let parse = |p: &str| p.parse::<f64>().map_err(|_| p.to_string());
            Row {
                x,
                results: [
                    parse(parts[1]),
                    parse(parts[2]),
                    parse(parts[3]),
                    parse(parts[4]),
                    parse(parts[5]),
                    parse(parts[6]),
                ],
            }
        })
        .collect();
    assert_eq!(rows.len(), xs.len(), "one row per argument");
    for (row, x) in rows.iter().zip(xs) {
        assert_eq!(
            row.x.to_bits(),
            x.to_bits(),
            "the program's literal is the argument"
        );
    }
    rows
}

fn check(rows: &[Row], pio2: &BigInt) -> Vec<String> {
    let mut wrong = Vec::new();
    for row in rows {
        let x = row.x;
        let (sin, cos, tan) = truth(x, pio2);
        let exact = [&sin, &cos, &tan, &sin, &cos, &tan];
        for (i, result) in row.results.iter().enumerate() {
            let name = NAMES[i];
            match result {
                Ok(actual) if within_one_ulp(*actual, exact[i]) => {}
                Ok(actual) => wrong.push(format!(
                    "{name}({x:e}) = {actual:e}, more than one ULP from the true value"
                )),
                Err(line) => wrong.push(format!("{name}({x:e}) must not raise: {line}")),
            }
        }
        for i in 0..3 {
            if let (Ok(scalar), Ok(listed)) = (&row.results[i], &row.results[i + 3]) {
                if scalar.to_bits() != listed.to_bits() {
                    wrong.push(format!(
                        "{}({x:e}): scalar {scalar:e} and List {listed:e} differ",
                        NAMES[i]
                    ));
                }
            }
        }
    }
    wrong
}

#[test]
fn trig_is_within_one_ulp_at_every_large_finite_angle() {
    let pio2 = pi_over_2();
    let xs = corpus();
    let rows = run(&xs);
    let wrong = check(&rows, &pio2);
    assert!(
        wrong.is_empty(),
        "{} wrong results over {} angles:\n{}",
        wrong.len(),
        rows.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_oracle_agrees_with_host_libm_at_ordinary_angles() {
    // Guard the oracle itself: below 2^20 every libm is good to a ULP or so.
    let pio2 = pi_over_2();
    let mut state = 0x6180u64;
    for _ in 0..200 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = ((state >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 2.0e6;
        let (sin, cos, _) = truth(x, &pio2);
        for (name, host, exact) in [("sin", x.sin(), &sin), ("cos", x.cos(), &cos)] {
            let diff = scaled(host) - exact;
            let diff = if diff < big(0) { -diff } else { diff };
            assert!(
                diff <= ulp_scaled(host) * 2,
                "oracle {name}({x:e}) disagrees with libm {host:e}"
            );
        }
    }
}

/// IEEE 754 §6.3: a zero argument keeps its sign through `sin` and `tan`
/// (`sin(-0.0)` is `-0.0`), and `cos(±0.0)` is `+1.0`. The kernel used to return
/// `+0.0` for `sin(-0.0)` — the reduced angle is `-0.0` and `-0.0 * P` is `-0.0`,
/// but collapsing the double-double adds the `+0.0` low half and `(-0.0) + (+0.0)`
/// is `+0.0`. Pre-existing; found while fixing bug-618.
#[test]
fn a_zero_argument_keeps_its_sign() {
    let source = r#"IMPORT io
IMPORT math
IMPORT collections

SUB main()
  LET negz AS Float = (0.0 - 1.0) * 0.0
  LET posz AS Float = 0.0
  LET pair AS List OF Float = [negz, posz]
  LET p AS Byte = toByte(255)
  io::print(toString(math::sin(negz), p) & "|" & toString(math::cos(negz), p) & "|" & toString(math::tan(negz), p))
  io::print(toString(math::sin(posz), p) & "|" & toString(math::cos(posz), p) & "|" & toString(math::tan(posz), p))
  io::print(toString(collections::get(math::sin(pair), 0), p) & "|" & toString(collections::get(math::cos(pair), 0), p) & "|" & toString(collections::get(math::tan(pair), 0), p))
  io::print(toString(collections::get(math::sin(pair), 1), p) & "|" & toString(collections::get(math::cos(pair), 1), p) & "|" & toString(collections::get(math::tan(pair), 1), p))
END SUB
"#;
    let project = common::temp_project("math_float_trig_signed_zero", source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(60),
        "six trig calls on zero must finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status)
    );
    let _ = std::fs::remove_dir_all(&project);
    let rows: Vec<Vec<f64>> = stdout
        .lines()
        .map(|line| line.split('|').map(|p| p.parse::<f64>().expect("a Float")).collect())
        .collect();
    assert_eq!(rows.len(), 4, "four rows:\n{stdout}");
    // Rows 0 and 2 are -0.0 (scalar, then List); rows 1 and 3 are +0.0.
    for (row, negative) in rows.iter().zip([true, false, true, false]) {
        let expected_zero = if negative { -0.0f64 } else { 0.0f64 };
        assert_eq!(
            row[0].to_bits(),
            expected_zero.to_bits(),
            "sin of {}0.0 must be {}0.0, got {}",
            if negative { "-" } else { "+" },
            if negative { "-" } else { "+" },
            row[0]
        );
        assert_eq!(row[1].to_bits(), 1.0f64.to_bits(), "cos of a zero must be +1.0");
        assert_eq!(
            row[2].to_bits(),
            expected_zero.to_bits(),
            "tan of {}0.0 must be {}0.0, got {}",
            if negative { "-" } else { "+" },
            if negative { "-" } else { "+" },
            row[2]
        );
    }
}

