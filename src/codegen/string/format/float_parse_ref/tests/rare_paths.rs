//! The reference parser's rare paths, each reached on purpose.
//!
//! `float_parse_ref.rs` is the specification the emitted `_mfb_rt_string_to_float`
//! transliterates, and the sweeps in `tests/mod.rs` pin its answers against
//! `str::parse::<f64>`. A sweep only visits a branch its inputs happen to reach,
//! and the branches that matter most are the ones inputs almost never reach: the
//! 128-bit product whose low word is all ones (the table entry's truncation could
//! round the wrong way, so Eisel–Lemire DECLINES), a result more than 64 bits below
//! the smallest subnormal, an approximation seed one ULP above the answer. Each is
//! either solved for or constructed below, and each asserts the correctly rounded
//! answer, because a rare path that rounds wrongly is a wrong `toFloat` that no
//! ordinary input exposes.

use super::super::*;
use crate::codegen::string::format::float_parse_table::{entry, Q_MAX, Q_MIN};

/// The inverse of an odd `a` modulo 2^64, by Newton's iteration.
fn inverse_mod_2_64(a: u64) -> u64 {
    let mut x = a;
    for _ in 0..6 {
        x = x.wrapping_mul(2u64.wrapping_sub(a.wrapping_mul(x)));
    }
    x
}

/// A mantissa `w` (top bit set, so no normalizing shift) whose first product with
/// the table entry for `q` has an all-ones low word, if the entry's high word is odd
/// and the solution has its top bit set.
fn declining_mantissa(q: i32) -> Option<u64> {
    let (table_hi, _) = entry(q);
    if table_hi & 1 == 0 {
        return None;
    }
    let w = inverse_mod_2_64(table_hi).wrapping_neg();
    (w >> 63 == 1).then_some(w)
}

/// Outside the window where the truncated table entry is provably harmless, an
/// all-ones low word makes Eisel–Lemire decline, and the exact fallback must still
/// return the correctly rounded double — seeded, because `lemire` declined, by the
/// scaled multiply instead.
#[test]
fn a_declined_product_falls_back_to_the_exact_answer() {
    let mut declined = 0;
    for q in Q_MIN..=Q_MAX {
        // Outside the safe window; and below 10^289 so the scaled seed stays finite.
        if (-27..=55).contains(&q) || q > 280 {
            continue;
        }
        let Some(w) = declining_mantissa(q) else {
            continue;
        };
        // At most 19 digits, so the scanner keeps every one of them as the mantissa.
        if w >= 10_000_000_000_000_000_000 || lemire(q, w).e != DECLINED {
            continue;
        }
        declined += 1;
        let text = format!("{w}e{q}");
        let want: f64 = text.parse().unwrap();
        let digits: Vec<u8> = w.to_string().bytes().map(|b| b - b'0').collect();
        let scanned = Scanned {
            negative: false,
            mantissa: w,
            exponent: q,
            many_digits: false,
            digits,
            digits_exponent: q,
        };
        assert_eq!(
            finish_inner(&scanned, false).to_bits(),
            want.to_bits(),
            "{text}: Eisel–Lemire declines here, the exact fallback must round correctly"
        );
        assert!(
            takes_exact_path(&text),
            "{text} must be routed to the fallback"
        );
        assert_eq!(parse(&text).unwrap().to_bits(), want.to_bits(), "{text}");
    }
    assert!(
        declined >= 3,
        "the construction must find declining inputs across the table; found {declined}"
    );
}

/// A seed one ULP above the answer is walked DOWN to it; a negative input keeps its
/// sign through the walk.
#[test]
fn the_exact_walk_corrects_a_seed_one_ulp_too_high() {
    // `mantissa`/`exponent` seed the walk at 1.0000000000000002; the digits are the
    // exact value, 1.
    for negative in [false, true] {
        let scanned = Scanned {
            negative,
            mantissa: 10_000_000_000_000_002,
            exponent: -16,
            many_digits: false,
            digits: vec![1],
            digits_exponent: 0,
        };
        let want = if negative { -1.0 } else { 1.0 };
        assert_eq!(
            exact(&scanned).to_bits(),
            f64::to_bits(want),
            "negative={negative}: the walk must land on the double whose interval holds 1"
        );
    }
}

/// An exact tie between two doubles rounds to the even one: `1 + 2^-53` is exactly
/// the midpoint above 1.0 and must give 1.0, while `1 + 3 * 2^-53` gives the odd
/// neighbour's even successor.
#[test]
fn the_exact_walk_breaks_ties_to_even() {
    // 1 + 2^-53 = 1.00000000000000011102230246251565404236316680908203125
    let tie = "1.00000000000000011102230246251565404236316680908203125";
    let scanned = scan(tie).unwrap();
    assert_eq!(exact(&scanned).to_bits(), 1.0f64.to_bits());
    assert_eq!(
        parse(tie).unwrap().to_bits(),
        tie.parse::<f64>().unwrap().to_bits()
    );
}

/// A value with no significant digit is a zero of the input's sign.
#[test]
fn the_exact_path_of_no_digits_is_a_signed_zero() {
    for negative in [false, true] {
        let scanned = Scanned {
            negative,
            mantissa: 0,
            exponent: 0,
            many_digits: false,
            digits: Vec::new(),
            digits_exponent: 0,
        };
        let zero = exact(&scanned);
        assert_eq!(zero, 0.0);
        assert_eq!(zero.is_sign_negative(), negative);
    }
}

/// More than 64 bits below the smallest subnormal nothing survives: zero.
#[test]
fn a_value_far_below_the_smallest_subnormal_is_zero() {
    for q in Q_MIN..Q_MIN + 8 {
        let fp = lemire(q, 1);
        assert_eq!(
            (fp.f, fp.e),
            (0, 0),
            "q={q}: nothing survives this far down"
        );
    }
    assert_eq!(parse("1e-400").unwrap().to_bits(), 0.0f64.to_bits());
    assert_eq!(parse("-1e-400").unwrap().to_bits(), (-0.0f64).to_bits());
}

/// The 128-bit product keeps the full 64-bit mask when asked for 64 or more bits of
/// precision (no shift of the mask).
#[test]
fn a_full_precision_product_uses_the_whole_mask() {
    let narrow = compute_product_approx(10, u64::MAX, 55);
    let full = compute_product_approx(10, u64::MAX, 64);
    assert_eq!(
        narrow, full,
        "the product does not depend on the precision asked for"
    );
}

/// The scanner rejects a grammar the old scanner rejected: an exponent with no
/// mantissa digit before it, and an exponent letter with nothing after it.
#[test]
fn the_scanner_rejects_a_malformed_exponent() {
    for text in [
        "e5", ".e5", "-e5", "1e", "1e+", "1e-", "1e5x", "", "-", "+", ".",
    ] {
        assert!(scan(text).is_none(), "{text:?} is not a number");
        assert!(parse(text).is_none(), "{text:?} is not a number");
    }
}

/// The big-integer helpers are no-ops on zero and on a zero shift.
#[test]
fn shifting_zero_or_by_zero_changes_nothing() {
    let mut zero = Nat::zero();
    zero.shl(7);
    assert_eq!(zero, Nat::zero());
    let mut one = Nat::from_u64(1);
    one.shl(0);
    assert_eq!(one, Nat::from_u64(1));
    let mut compared = Nat::from_u64(5);
    compared.mul_pow10(0);
    assert_eq!(compared, Nat::from_u64(5));
    assert_eq!(
        Nat::from_u64(1).cmp(&Nat::from_u64(u64::MAX)),
        std::cmp::Ordering::Less,
        "a shorter limb vector is the smaller number"
    );
}

/// Inside the window where the truncated table entry is provably harmless, an
/// all-ones low word does NOT make Eisel–Lemire decline.
#[test]
fn an_all_ones_low_word_inside_the_safe_window_is_not_declined() {
    let mut inside = 0;
    for q in -27..=55 {
        let Some(w) = declining_mantissa(q) else {
            continue;
        };
        inside += 1;
        assert_ne!(
            lemire(q, w).e,
            DECLINED,
            "q={q}: the entry's truncation cannot matter this close to 1"
        );
    }
    assert!(
        inside >= 3,
        "the construction must find inputs in the window; found {inside}"
    );
}

/// Text that is not a number is not routed to the fallback either.
#[test]
fn text_that_is_not_a_number_takes_no_exact_path() {
    assert!(!takes_exact_path("not a number"));
}

/// Shifting carries into a new limb, and the exact comparison scales whichever side
/// needs it: every sign of the decimal exponent against every sign of the binary one.
#[test]
fn the_big_integer_comparison_scales_either_side() {
    let mut top = Nat::from_u64(u64::MAX);
    top.shl(1);
    assert_eq!(
        top,
        Nat(vec![0xFFFF_FFFE, 0xFFFF_FFFF, 1]),
        "the carry becomes a third limb"
    );

    use std::cmp::Ordering::{Equal, Greater, Less};
    // d × 10^q against m × 2^e.
    for (d, q, midpoint, want) in [
        (5, 1, (50, 0), Equal),
        (5, -1, (1, -1), Equal),
        (3, 0, (3, 0), Equal),
        (1, 0, (1, 2), Less),
        (3, 0, (1, -2), Greater),
        (5, 2, (125, 2), Equal),
        (5, -2, (25, -2), Less),
    ] {
        assert_eq!(
            compare_scaled(&Nat::from_u64(d), q, &midpoint),
            want,
            "{d} x 10^{q} against {} x 2^{}",
            midpoint.0,
            midpoint.1
        );
    }
}
