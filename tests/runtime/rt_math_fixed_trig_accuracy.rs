//! `math::sin`, `math::cos` and `math::tan` on a `Fixed` argument are within one
//! Q32.32 unit (2^-32) of the true value at every representable angle, and `tan`
//! raises `ErrOverflow` when its true value is outside the `Fixed` range
//! (bug-615).
//!
//! The Q32.32 kernel reduced the angle by `k * pi/2` with `pi/2` itself rounded to
//! Q32.32, so the reduction lost `k * 0.26` units: `sin(toFixed(2000000000.0))`
//! came out 0.9432 against a true 0.9147, and at `math::pi2Fixed` (0.26 units below
//! pi/2) the reduced angle collapsed to zero. The 31-step Q32.32 CORDIC then left a
//! residue of a few units on a cosine that is truly 0.26 units, `tan` divided `sin`
//! by `-3` units, and returned -1431655767.67 where the true tangent is +1.65e10
//! with no error. Even at small angles CORDIC was 4-6 units off (`sin(1.0)`).
//!
//! **The oracle is an exact big-integer evaluation.** Each argument's raw Q32.32
//! value is read back from the program, pi comes from Machin's formula, and sin/cos
//! are Taylor series, all in 336-bit fixed point, so the truth carries no host
//! floating-point error at all — near a pole, where the tangent needs ~95 bits of
//! the reduced angle, that is the only oracle good enough.

#[path = "../common/mod.rs"]
mod common;

use num_bigint::BigInt;
use std::time::Duration;

const ERR_OVERFLOW: i64 = 77050010;
/// Working precision of the oracle, in bits after the binary point.
const W: u32 = 336;

fn big(v: i64) -> BigInt {
    BigInt::from(v)
}

fn pow2(bits: u32) -> BigInt {
    BigInt::from(1) << bits
}

/// Floor division for a positive divisor (`BigInt`'s `/` truncates toward zero).
fn div_floor(a: &BigInt, b: &BigInt) -> BigInt {
    let q = a / b;
    if (a % b) < big(0) {
        q - 1
    } else {
        q
    }
}

/// `atan(1/n)` scaled by 2^W.
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

/// `pi/2` scaled by 2^W (Machin: pi = 16 atan(1/5) - 4 atan(1/239)).
fn pi_over_2() -> BigInt {
    (big(16) * atan_inverse(5) - big(4) * atan_inverse(239)) / big(2)
}

/// `(sin r, cos r)` scaled by 2^W for a scaled `|r| <= pi/4`.
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

struct Truth {
    sin: BigInt,
    cos: BigInt,
    tan: BigInt,
}

/// The exact sin/cos/tan of the Q32.32 value `raw`, scaled by 2^W.
fn truth(raw: i64, pio2: &BigInt) -> Truth {
    let x = big(raw) << (W - 32);
    let k = div_floor(&(&x * 2 + pio2), &(pio2 * 2));
    let r = &x - &k * pio2;
    let (s, c) = sin_cos_small(&r);
    let quadrant = i64::try_from(((k % 4) + 4) % 4).unwrap();
    let (sin, cos) = match quadrant {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    };
    let tan = (&sin << W) / &cos;
    Truth { sin, cos, tan }
}

/// Parse `toString(fixed, 32)` — the exact decimal expansion — back to its raw value.
fn parse_fixed(text: &str) -> i64 {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (int_part, frac_part) = digits.split_once('.').expect("fixed has a fraction");
    assert_eq!(frac_part.len(), 32, "32 fractional digits in {text:?}");
    let int_value: BigInt = int_part.parse().unwrap();
    let frac_value: BigInt = frac_part.parse().unwrap();
    let ten32 = BigInt::from(10u128.pow(32));
    let scaled_frac = &frac_value << 32u32;
    assert_eq!(
        &scaled_frac % &ten32,
        big(0),
        "{text:?} is not an exact Q32.32 expansion"
    );
    let mut raw = (int_value << 32u32) + scaled_frac / ten32;
    if negative {
        raw = -raw;
    }
    i64::try_from(raw).expect("raw fits i64")
}

/// `|actual_raw - truth|` in Q32.32 units, as a `BigInt` scaled by 2^(W-32).
fn error_units(actual_raw: i64, truth: &BigInt) -> BigInt {
    let diff = (big(actual_raw) << (W - 32)) - truth;
    if diff < big(0) {
        -diff
    } else {
        diff
    }
}

/// Deterministic pseudo-random raws (64-bit LCG) masked to `bits` signed bits.
fn random_raws(seed: u64, count: usize, bits: u32) -> Vec<i64> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let v = (state ^ (state >> 29)) as i64;
            if bits >= 64 {
                v
            } else {
                v >> (64 - bits)
            }
        })
        .collect()
}

/// Raws on both sides of the odd multiples of pi/2 that come closest to a Q32.32
/// value, found with 128-bit wrapping arithmetic over two windows of `m`.
fn near_pole_raws(pio2: &BigInt) -> Vec<i64> {
    // A = pi/2 * 2^(32+96), keeping only the low 128 bits: (2m+1)*A mod 2^128 has
    // the fractional part of (2m+1)*pi/2 in Q32.32 units in its low 96 bits.
    let a_big = (pio2 << 128u32) >> (W - 32);
    let mask128 = (big(1) << 128u32) - 1;
    let a = u128::try_from(a_big & mask128).unwrap();
    let low96 = (1u128 << 96) - 1;
    let max_m = ((1i64 << 31) as f64 / std::f64::consts::PI) as u64;
    let mut best: Vec<(u128, u64)> = Vec::new();
    for range in [0u64..(1 << 21), (max_m - (1 << 21))..max_m] {
        let mut window: Vec<(u128, u64)> = Vec::new();
        for m in range {
            let f = (u128::from(2 * m + 1)).wrapping_mul(a) & low96;
            let distance = f.min((1u128 << 96) - f);
            if window.len() < 6 || distance < window[5].0 {
                window.push((distance, m));
                window.sort();
                window.truncate(6);
            }
        }
        best.extend(window);
    }
    let mut raws = Vec::new();
    for m in (0u64..8).chain(best.iter().map(|(_, m)| *m)) {
        let centre = big(2 * m as i64 + 1) * pio2;
        let floor_raw = i64::try_from(centre >> (W - 32)).unwrap();
        for delta in -2..=2 {
            for sign in [1i64, -1] {
                if let Some(v) = floor_raw.checked_add(delta) {
                    raws.push(sign * v);
                }
            }
        }
    }
    raws
}

fn corpus(pio2: &BigInt) -> Vec<i64> {
    let one = 1i64 << 32;
    let mut raws = vec![
        0,
        1,
        -1,
        one,
        -one,
        one / 2,
        3 * one / 4,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
    ];
    // The filed table, and the probe's large angles.
    for f in [
        1.5,
        1.57,
        1.5707,
        1.57079,
        1.570796,
        1.5707963,
        -1.5707963,
        1000.0,
        1.0e6,
        1.0e8,
        2.0e9,
        -2147483000.5,
    ] {
        raws.push((f * one as f64).round() as i64);
    }
    raws.extend(near_pole_raws(pio2));
    raws.extend(random_raws(0x615, 160, 64));
    raws.extend(random_raws(0x616, 120, 37));
    raws.extend(random_raws(0x617, 60, 30));
    raws
}

const PROGRAM_HEAD: &str = r#"IMPORT io
IMPORT math
IMPORT collections

FUNC sinOf(x AS Fixed) AS String
  RETURN "ok " & toString(math::sin(x), toByte(32))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC cosOf(x AS Fixed) AS String
  RETURN "ok " & toString(math::cos(x), toByte(32))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tanOf(x AS Fixed) AS String
  RETURN "ok " & toString(math::tan(x), toByte(32))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC
"#;

/// Run sin/cos/tan over `raws`; one `(raw, sin, cos, tan)` row per argument.
fn run(raws: &[i64]) -> Vec<(i64, String, String, String)> {
    let his = raws
        .iter()
        .map(|r| (r >> 32).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let los = raws
        .iter()
        .map(|r| format!("{}.0", r & 0xFFFF_FFFF))
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!(
        r#"{PROGRAM_HEAD}
SUB main()
  LET his AS List OF Integer = [{his}]
  LET los AS List OF Float = [{los}]
  MUT i AS Integer = 0
  WHILE i < len(his)
    LET x AS Fixed = toFixed(collections::get(his, i)) + toFixed(collections::get(los, i) / 4294967296.0)
    io::print(toString(x, toByte(32)) & "|" & sinOf(x) & "|" & cosOf(x) & "|" & tanOf(x))
    i = i + 1
  END WHILE
END SUB
"#
    );
    let project = common::temp_project("math_fixed_trig_accuracy", &source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(120),
        "Fixed sin/cos/tan over the corpus must finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status)
    );
    let _ = std::fs::remove_dir_all(&project);
    let rows: Vec<(i64, String, String, String)> = stdout
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            assert_eq!(parts.len(), 4, "row {line:?}");
            (
                parse_fixed(parts[0]),
                parts[1].to_string(),
                parts[2].to_string(),
                parts[3].to_string(),
            )
        })
        .collect();
    assert_eq!(rows.len(), raws.len(), "one row per argument");
    for (row, raw) in rows.iter().zip(raws) {
        assert_eq!(row.0, *raw, "the program built the argument it was given");
    }
    rows
}

fn fixed_text(raw: i64) -> String {
    format!("{} (raw {raw})", raw as f64 / 4294967296.0)
}

fn units(error: &BigInt) -> f64 {
    let whole = error >> (W - 32);
    let whole = i64::try_from(whole)
        .map(|v| v as f64)
        .unwrap_or(f64::INFINITY);
    whole.max(0.0)
}

#[test]
fn fixed_sin_cos_tan_are_within_one_unit_and_tan_overflow_raises() {
    let pio2 = pi_over_2();
    let raws = corpus(&pio2);
    let rows = run(&raws);
    let one_unit = pow2(W - 32);
    let max_raw = big(i64::MAX) << (W - 32);
    let min_raw = big(i64::MIN) << (W - 32);
    let mut wrong = Vec::new();

    for (raw, sin_line, cos_line, tan_line) in &rows {
        let t = truth(*raw, &pio2);
        for (name, line, exact) in [("sin", sin_line, &t.sin), ("cos", cos_line, &t.cos)] {
            match line.strip_prefix("ok ") {
                Some(text) => {
                    let error = error_units(parse_fixed(text), exact);
                    if error > one_unit {
                        wrong.push(format!(
                            "{name}({}) = {text}: {} units from the true value",
                            fixed_text(*raw),
                            units(&error)
                        ));
                    }
                }
                None => wrong.push(format!(
                    "{name}({}) must not raise: {line}",
                    fixed_text(*raw)
                )),
            }
        }

        // tan: a true value more than one unit outside the range must raise
        // ErrOverflow; one more than a unit inside must be returned within a unit.
        let beyond = &t.tan > &(&max_raw + &one_unit) || &t.tan < &(&min_raw - &one_unit);
        let inside = &t.tan < &(&max_raw - &one_unit) && &t.tan > &(&min_raw + &one_unit);
        match tan_line.strip_prefix("ok ") {
            Some(text) => {
                if beyond {
                    wrong.push(format!(
                        "tan({}) = {text}, but the true tangent {} is outside the Fixed range",
                        fixed_text(*raw),
                        (&t.tan >> (W - 32)) as BigInt
                    ));
                } else {
                    let error = error_units(parse_fixed(text), &t.tan);
                    if error > one_unit {
                        wrong.push(format!(
                            "tan({}) = {text}: {} units from the true value",
                            fixed_text(*raw),
                            units(&error)
                        ));
                    }
                }
            }
            None => {
                if tan_line != &format!("raised {ERR_OVERFLOW}") || inside {
                    wrong.push(format!(
                        "tan({}) {tan_line}, true tangent {} raw units",
                        fixed_text(*raw),
                        (&t.tan >> (W - 32)) as BigInt
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong of {} Fixed trig results:\n{}",
        wrong.len(),
        rows.len() * 3,
        wrong.join("\n")
    );
}

#[test]
fn the_oracle_agrees_with_host_floating_point_where_both_are_exact_enough() {
    // Guard the oracle itself: at ordinary angles an f64 libm result is good to
    // ~1e-16, far inside one Q32.32 unit, so any disagreement is an oracle bug.
    let pio2 = pi_over_2();
    for raw in random_raws(0x618, 200, 36) {
        let x = raw as f64 / 4294967296.0;
        let t = truth(raw, &pio2);
        let scale = 2f64.powi(32);
        for (name, exact, host) in [("sin", &t.sin, x.sin()), ("cos", &t.cos, x.cos())] {
            let exact_units = i64::try_from(exact >> (W - 32)).unwrap() as f64;
            assert!(
                (exact_units - host * scale).abs() < 2.0,
                "oracle {name}({x}) disagrees with libm: {} vs {}",
                exact_units / scale,
                host
            );
        }
    }
}
