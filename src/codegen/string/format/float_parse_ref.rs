//! The reference specification for `_mfb_rt_string_to_float` (plan-120-F).
//!
//! `float_parse.rs` emits this algorithm as NIR. That emission cannot be
//! exercised from `cargo test` — it only runs after a full build, on a target
//! machine — so the *logic* is written once here in ordinary Rust and pinned
//! against `str::parse::<f64>()`, which is itself correctly rounded. The emitted
//! helper is then a transliteration of a function that is already known to be
//! right, and the corpus fixture
//! (`tests/rt-behavior/conversions/tofloat-correct-rounding-corpus-rt`) checks
//! the transliteration rather than the algorithm.
//!
//! Splitting it that way matters: a defect in a 500-instruction hand-written
//! NIR routine is otherwise indistinguishable from a defect in the algorithm,
//! and each full verify cycle costs a rebuild plus a target run.
//!
//! This module is `#[cfg(test)]`: it is a specification and an oracle, never
//! shipped code.

use super::float_parse_table::{entry, Q_MAX, Q_MIN};

/// f64 shape constants, named as in the Eisel–Lemire literature.
const MANTISSA_EXPLICIT_BITS: u32 = 52;
const MINIMUM_EXPONENT: i32 = -1023;
const INFINITE_POWER: i32 = 0x7FF;
const MIN_EXPONENT_ROUND_TO_EVEN: i32 = -4;
const MAX_EXPONENT_ROUND_TO_EVEN: i32 = 23;

/// What the scanner extracts. Mirrors exactly what the emitted scanner must
/// leave in registers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scanned {
    pub negative: bool,
    /// The first ≤19 significant digits as an integer.
    pub mantissa: u64,
    /// Decimal exponent to apply to `mantissa`.
    pub exponent: i32,
    /// Whether any significant digit was dropped past the 19th.
    pub many_digits: bool,
    /// Every significant digit, in order, for the exact fallback. Empty when
    /// the value is zero.
    pub digits: Vec<u8>,
    /// Decimal exponent to apply to `digits` read as one integer.
    pub digits_exponent: i32,
}

/// Scan the grammar `[+-]? (digit | '.')* ([eE] [+-]? digit+)?` requiring at
/// least one digit and at most one dot — byte for byte the grammar
/// `emit_parse_decimal_string_to_double` accepted, so no program's set of
/// accepted strings changes.
pub(crate) fn scan(text: &str) -> Option<Scanned> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = 0usize;
    let mut negative = false;
    if bytes[0] == b'-' || bytes[0] == b'+' {
        negative = bytes[0] == b'-';
        index += 1;
        if index >= bytes.len() {
            return None;
        }
    }

    let mut mantissa: u64 = 0;
    let mut digit_count = 0u32;
    let mut digits: Vec<u8> = Vec::new();
    let mut many_digits = false;
    let mut seen_digit = false;
    let mut dot_seen = false;
    let mut fractional_digits = 0i32;
    // Leading zeros are not significant and must not consume mantissa room.
    let mut seen_significant = false;
    // Trailing zeros dropped past the 19-digit window still shift the exponent.
    let mut dropped = 0i32;

    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'.' => {
                if dot_seen {
                    return None;
                }
                dot_seen = true;
            }
            b'e' | b'E' => break,
            b'0'..=b'9' => {
                seen_digit = true;
                let digit = byte - b'0';
                if dot_seen {
                    fractional_digits += 1;
                }
                if digit != 0 {
                    seen_significant = true;
                }
                if seen_significant {
                    digits.push(digit);
                    if digit_count < 19 {
                        mantissa = mantissa * 10 + digit as u64;
                        digit_count += 1;
                    } else {
                        many_digits = true;
                        dropped += 1;
                    }
                }
            }
            _ => return None,
        }
        index += 1;
    }

    let mut exponent = 0i32;
    if index < bytes.len() {
        // `e` or `E`. The old scanner required a digit before it.
        if !seen_digit {
            return None;
        }
        index += 1;
        if index >= bytes.len() {
            return None;
        }
        let mut exponent_negative = false;
        if bytes[index] == b'-' {
            exponent_negative = true;
            index += 1;
        } else if bytes[index] == b'+' {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        let mut seen_exponent_digit = false;
        let mut value: i32 = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if !byte.is_ascii_digit() {
                return None;
            }
            seen_exponent_digit = true;
            // Clamp rather than wrap: past this magnitude every finite mantissa
            // already overflows or underflows, so further digits cannot change
            // the answer, and the old scanner clamped for the same reason.
            if value < 100_000 {
                value = value * 10 + (byte - b'0') as i32;
            }
            index += 1;
        }
        if !seen_exponent_digit {
            return None;
        }
        exponent = if exponent_negative { -value } else { value };
    }

    if !seen_digit {
        return None;
    }

    // `exponent` so far is the literal `eNN`. Fold in the decimal point and the
    // digits the 19-digit window dropped.
    let scale = exponent - fractional_digits;
    Some(Scanned {
        negative,
        mantissa,
        exponent: scale + dropped,
        many_digits,
        digits_exponent: scale,
        digits,
    })
}

/// `floor(log2(10^q)) + 63`, the table's binary exponent, by the standard
/// fixed-point approximation (exact over the table's range).
fn power(q: i32) -> i32 {
    ((q as i64 * (152_170 + 65536)) >> 16) as i32 + 63
}

fn full_multiplication(a: u64, b: u64) -> (u64, u64) {
    let product = a as u128 * b as u128;
    (product as u64, (product >> 64) as u64)
}

/// The 128-bit approximate product `w * 10^q`, taking a second multiplication
/// only when the first leaves the result ambiguous.
fn compute_product_approx(q: i32, w: u64, precision: u32) -> (u64, u64) {
    let (table_hi, table_lo) = entry(q);
    let mask = if precision < 64 {
        0xFFFF_FFFF_FFFF_FFFFu64 >> precision
    } else {
        0xFFFF_FFFF_FFFF_FFFFu64
    };
    let (mut first_lo, mut first_hi) = full_multiplication(w, table_hi);
    if first_hi & mask == mask {
        let (_, second_hi) = full_multiplication(w, table_lo);
        let (sum, carried) = first_lo.overflowing_add(second_hi);
        first_lo = sum;
        if carried {
            first_hi += 1;
        }
    }
    (first_lo, first_hi)
}

/// A biased mantissa/exponent pair, or the `DECLINED` sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BiasedFp {
    pub f: u64,
    pub e: i32,
}

pub(crate) const DECLINED: i32 = -1;

/// Eisel–Lemire. Returns `e == DECLINED` when the 128-bit approximation cannot
/// decide the rounding, which routes the caller to the exact fallback.
pub(crate) fn lemire(q: i32, w: u64) -> BiasedFp {
    let zero = BiasedFp { f: 0, e: 0 };
    if w == 0 || q < Q_MIN {
        return zero;
    }
    if q > Q_MAX {
        return BiasedFp {
            f: 0,
            e: INFINITE_POWER,
        };
    }
    let lz = w.leading_zeros();
    let w = w << lz;
    let (lo, hi) = compute_product_approx(q, w, MANTISSA_EXPLICIT_BITS + 3);
    if lo == 0xFFFF_FFFF_FFFF_FFFF {
        // The truncated table entry could round the wrong way here. Outside the
        // window where that is provably harmless, decline.
        let inside_safe_exponent = (-27..=55).contains(&q);
        if !inside_safe_exponent {
            return BiasedFp { f: 0, e: DECLINED };
        }
    }
    let upperbit = (hi >> 63) as i32;
    let shift = upperbit + 64 - MANTISSA_EXPLICIT_BITS as i32 - 3;
    let mut mantissa = hi >> shift;
    let mut power2 = power(q) + upperbit - lz as i32 - MINIMUM_EXPONENT;
    if power2 <= 0 {
        if -power2 + 1 >= 64 {
            // More than 64 bits below the minimum exponent: nothing survives.
            return zero;
        }
        mantissa >>= -power2 + 1;
        mantissa += mantissa & 1;
        mantissa >>= 1;
        power2 = (mantissa >= (1u64 << MANTISSA_EXPLICIT_BITS)) as i32;
        return BiasedFp {
            f: mantissa,
            e: power2,
        };
    }
    // An exact tie must round to even. `lo <= 1` with the shifted mantissa
    // reproducing `hi` exactly means the discarded part is exactly one half.
    if lo <= 1
        && (MIN_EXPONENT_ROUND_TO_EVEN..=MAX_EXPONENT_ROUND_TO_EVEN).contains(&q)
        && mantissa & 3 == 1
        && (mantissa << shift) == hi
    {
        mantissa &= !1u64;
    }
    mantissa += mantissa & 1;
    mantissa >>= 1;
    if mantissa >= (2u64 << MANTISSA_EXPLICIT_BITS) {
        mantissa = 1u64 << MANTISSA_EXPLICIT_BITS;
        power2 += 1;
    }
    mantissa &= !(1u64 << MANTISSA_EXPLICIT_BITS);
    if power2 >= INFINITE_POWER {
        return BiasedFp {
            f: 0,
            e: INFINITE_POWER,
        };
    }
    BiasedFp {
        f: mantissa,
        e: power2,
    }
}

fn assemble(negative: bool, fp: BiasedFp) -> f64 {
    let bits = fp.f | ((fp.e as u64) << MANTISSA_EXPLICIT_BITS) | ((negative as u64) << 63);
    f64::from_bits(bits)
}

/// The full parse: scan, Clinger fast path, Eisel–Lemire, then the exact
/// fallback for the cases Lemire declines or a truncated mantissa makes
/// ambiguous.
pub(crate) fn parse(text: &str) -> Option<f64> {
    let scanned = scan(text)?;
    Some(finish(&scanned))
}

pub(crate) fn finish(scanned: &Scanned) -> f64 {
    // Clinger's exactly-representable fast path: both the mantissa and the
    // power of ten are exact in binary64, so one multiply or divide is
    // correctly rounded by IEEE 754 itself.
    if !scanned.many_digits && scanned.mantissa < (1u64 << 53) && scanned.exponent.abs() <= 22 {
        let value = scanned.mantissa as f64;
        let power = 10f64.powi(scanned.exponent.abs());
        let result = if scanned.exponent >= 0 {
            value * power
        } else {
            value / power
        };
        return if scanned.negative { -result } else { result };
    }

    let mut fp = lemire(scanned.exponent, scanned.mantissa);
    if scanned.many_digits && fp.e != DECLINED {
        // The mantissa was truncated, so the true value lies between `mantissa`
        // and `mantissa + 1` scaled. If both round the same way the truncation
        // is harmless; if not, only the exact comparison can decide.
        let upper = lemire(scanned.exponent, scanned.mantissa + 1);
        if upper.e == DECLINED || upper != fp {
            fp.e = DECLINED;
        }
    }
    if fp.e == DECLINED {
        return exact(scanned);
    }
    assemble(scanned.negative, fp)
}

// ---------------------------------------------------------------------------
// The exact fallback.
//
// Decide between the two candidate doubles by comparing the input's exact value
// against the midpoint between them. Both sides are rational with only powers
// of 2 and 10 in play, so cross-multiplying clears every denominator and the
// comparison is between two big integers — no division, and no accumulated
// error to reason about.
// ---------------------------------------------------------------------------

/// Minimal big natural, little-endian 32-bit limbs. Same shape as the table
/// generator's, kept separate because this one needs comparison and shifting
/// rather than division.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Nat(Vec<u32>);

impl Nat {
    fn zero() -> Self {
        Nat(vec![])
    }

    fn from_u64(value: u64) -> Self {
        let mut limbs = vec![value as u32, (value >> 32) as u32];
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Nat(limbs)
    }

    fn mul_add_small(&mut self, multiplier: u32, addend: u32) {
        let mut carry = addend as u64;
        for limb in self.0.iter_mut() {
            let product = *limb as u64 * multiplier as u64 + carry;
            *limb = product as u32;
            carry = product >> 32;
        }
        while carry != 0 {
            self.0.push(carry as u32);
            carry >>= 32;
        }
    }

    fn shl(&mut self, shift: usize) {
        if self.0.is_empty() || shift == 0 {
            return;
        }
        let limb_shift = shift / 32;
        let bit_shift = shift % 32;
        if bit_shift != 0 {
            let mut carry: u32 = 0;
            for limb in self.0.iter_mut() {
                let next = *limb >> (32 - bit_shift);
                *limb = (*limb << bit_shift) | carry;
                carry = next;
            }
            if carry != 0 {
                self.0.push(carry);
            }
        }
        if limb_shift != 0 {
            let mut prefix = vec![0u32; limb_shift];
            prefix.extend_from_slice(&self.0);
            self.0 = prefix;
        }
    }

    fn mul_pow10(&mut self, exponent: u32) {
        // Nine digits at a time: 10^9 is the largest power of ten below 2^32,
        // so each pass is one small multiply rather than nine.
        let mut remaining = exponent;
        while remaining >= 9 {
            self.mul_add_small(1_000_000_000, 0);
            remaining -= 9;
        }
        if remaining > 0 {
            self.mul_add_small(10u32.pow(remaining), 0);
        }
    }

    fn cmp(&self, other: &Nat) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if self.0.len() != other.0.len() {
            return self.0.len().cmp(&other.0.len());
        }
        for index in (0..self.0.len()).rev() {
            match self.0[index].cmp(&other.0[index]) {
                Ordering::Equal => continue,
                ordering => return ordering,
            }
        }
        Ordering::Equal
    }
}

/// The exact value of the digit string as `digits × 10^digits_exponent`,
/// compared against candidate midpoints to pick the correctly-rounded double.
fn exact(scanned: &Scanned) -> f64 {
    if scanned.digits.is_empty() {
        return if scanned.negative { -0.0 } else { 0.0 };
    }

    // Build the digit string as one integer D, so the value is D × 10^q.
    let mut d = Nat::zero();
    for &digit in &scanned.digits {
        d.mul_add_small(10, digit as u32);
    }
    let q = scanned.digits_exponent;

    // Bracket the answer: start from the approximate value and walk to the
    // double whose interval contains D × 10^q. `lemire` is within one ULP even
    // when it declines to certify, so a small search settles it; a bare
    // `parse::<f64>` is deliberately NOT used here since that is the oracle.
    let approx = approximate(scanned);
    let mut candidate = approx;
    if candidate.is_infinite() {
        // Compare against the largest finite double's upper midpoint.
        candidate = f64::MAX;
    }
    if candidate == 0.0 {
        candidate = f64::from_bits(1);
    }

    // Walk down while D is below this candidate's lower midpoint, and up while
    // it is at or above the upper one. At most a couple of steps.
    loop {
        let lower = midpoint_below(candidate);
        if let Some(lower) = lower {
            if compare_scaled(&d, q, &lower) == std::cmp::Ordering::Less {
                candidate = previous(candidate);
                continue;
            }
        }
        let upper = midpoint_above(candidate);
        match compare_scaled(&d, q, &upper) {
            std::cmp::Ordering::Greater => {
                candidate = next(candidate);
                continue;
            }
            std::cmp::Ordering::Equal => {
                // Exactly the upper midpoint: ties-to-even.
                if candidate.to_bits() & 1 == 1 {
                    candidate = next(candidate);
                }
                break;
            }
            std::cmp::Ordering::Less => break,
        }
    }
    if scanned.negative {
        -candidate
    } else {
        candidate
    }
}

/// An approximation within a few ULP, used only to seed the exact search.
fn approximate(scanned: &Scanned) -> f64 {
    let fp = lemire(scanned.exponent, scanned.mantissa);
    if fp.e != DECLINED {
        return assemble(false, fp);
    }
    // Lemire declined; a plain scaled multiply is close enough to seed from.
    let mut value = scanned.mantissa as f64;
    let mut exponent = scanned.exponent;
    while exponent > 0 && value.is_finite() {
        value *= 10.0;
        exponent -= 1;
    }
    while exponent < 0 && value != 0.0 {
        value /= 10.0;
        exponent += 1;
    }
    value
}

/// The midpoint between `value` and the next double up, as `(M, E)` meaning
/// `M × 2^E`.
fn midpoint_above(value: f64) -> (u64, i32) {
    let (mantissa, exponent) = decompose(value);
    (mantissa * 2 + 1, exponent - 1)
}

/// The midpoint between `value` and the previous double, or `None` at zero.
fn midpoint_below(value: f64) -> Option<(u64, i32)> {
    let (mantissa, exponent) = decompose(value);
    if mantissa == 0 {
        return None;
    }
    Some((mantissa * 2 - 1, exponent - 1))
}

/// `value == mantissa × 2^exponent` with `mantissa` an integer.
fn decompose(value: f64) -> (u64, i32) {
    let bits = value.to_bits() & !(1u64 << 63);
    let biased = (bits >> MANTISSA_EXPLICIT_BITS) as i32;
    let fraction = bits & ((1u64 << MANTISSA_EXPLICIT_BITS) - 1);
    if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1u64 << MANTISSA_EXPLICIT_BITS), biased - 1075)
    }
}

fn next(value: f64) -> f64 {
    f64::from_bits(value.to_bits() + 1)
}

fn previous(value: f64) -> f64 {
    f64::from_bits(value.to_bits() - 1)
}

/// Compare `d × 10^q` against `m × 2^e`, exactly.
///
/// Cross-multiply to clear both negative exponents at once:
///   left  = d × 10^max(q,0) × 2^max(-e,0)
///   right = m × 10^max(-q,0) × 2^max(e,0)
fn compare_scaled(d: &Nat, q: i32, midpoint: &(u64, i32)) -> std::cmp::Ordering {
    let (m, e) = *midpoint;
    let mut left = d.clone();
    let mut right = Nat::from_u64(m);
    if q > 0 {
        left.mul_pow10(q as u32);
    } else if q < 0 {
        right.mul_pow10((-q) as u32);
    }
    if e > 0 {
        right.shl(e as usize);
    } else if e < 0 {
        left.shl((-e) as usize);
    }
    left.cmp(&right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(text: &str) {
        let expected: Result<f64, _> = text.parse::<f64>();
        let got = parse(text);
        match (expected, got) {
            (Ok(want), Some(have)) => assert_eq!(
                have.to_bits(),
                want.to_bits(),
                "{text}: want {want:?} ({:#018x}), got {have:?} ({:#018x})",
                want.to_bits(),
                have.to_bits()
            ),
            (Ok(want), None) => panic!("{text}: rejected, but std parsed it as {want:?}"),
            (Err(_), _) => { /* grammar differs from std's by design */ }
        }
    }

    #[test]
    fn the_corpus_vectors_are_correctly_rounded() {
        // The same vectors the rt fixture carries, including the five it
        // reported WRONG against the old repeated-multiply parser.
        for text in [
            "1e-7",
            "1e-30",
            "5e-324",
            "2.2250738585072011e-308",
            "2.2250738585072014e-308",
            "2.4703282292062327e-324",
            "2.4703282292062328e-324",
            "1e-323",
            "0.1",
            "0.5",
            "1.0",
            "9007199254740993",
            "9007199254740992",
            "123456789012345678901234567890",
            "1.7976931348623157e308",
            "4.9406564584124654e-324",
            "0",
            "-0",
            "1e22",
            "1e23",
            "1e-22",
            "1e-23",
            "3.141592653589793",
            "2.718281828459045",
            "1e308",
            "1e-308",
            "0.3",
        ] {
            check(text);
        }
    }

    #[test]
    fn the_classic_torture_cases_are_correctly_rounded() {
        for text in [
            "8.98846567431158e307",
            "1.7976931348623158e308",
            "2.225073858507201136057409796709131975934819546351645648023426109724822222021076945516529523908135087914149158913039621106870086438694594645527657207407820621743379988141063267329253552286881372149012981122451451889849057222307285255133155755015914397476397983411801999323962548289017107081850690630666655994938275772572015763062690663332647565300009245888316433037779791869612049497390377829704905051080609940730262937128958950003583799967207254304360284078895771796150945516748243471030702609144621572289880258182545180325707018860872113128079512233426288368622321503775666622503982534335974568884423900265498198385487948292206894721689831099698365846814022854243330660339850886445804001034933970427567186443383770486037861622771738545623065874679014086723327636718749999999999999999999999999999999999999e-308",
            "1e-45",
            "7.8459735791271921e65",
            "3.571e266",
        ] {
            check(text);
        }
    }

    #[test]
    fn matches_std_over_random_bit_patterns() {
        // Render random doubles at full precision and read them back: the
        // parser must land on the identical bit pattern. This is the strongest
        // single check — it walks the whole exponent range including
        // subnormals, and every rendering is a hard case by construction.
        //
        // Run at 200_000 samples while this was being written (90s, 0
        // mismatches); committed at 20_000 so it costs the shared suite ~9s
        // instead. Raise the bound locally when changing the algorithm.
        let mut state: u64 = 0x243F_6A88_85A3_08D3;
        let mut checked = 0u32;
        for _ in 0..20_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let bits = state;
            let value = f64::from_bits(bits);
            if !value.is_finite() {
                continue;
            }
            let text = format!("{value:e}");
            let want = text.parse::<f64>().expect("std reparses its own rendering");
            let have = parse(&text).unwrap_or_else(|| panic!("{text}: rejected"));
            assert_eq!(
                have.to_bits(),
                want.to_bits(),
                "{text}: want {:#018x}, got {:#018x}",
                want.to_bits(),
                have.to_bits()
            );
            checked += 1;
        }
        assert!(checked > 15_000, "too few finite samples: {checked}");
    }

    #[test]
    fn matches_std_over_many_digit_decimals() {
        // Long digit strings are where the 19-digit mantissa window truncates
        // and Lemire has to decline to the exact path.
        let mut state: u64 = 0x1319_8A2E_0370_7344;
        for _ in 0..4_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let digit_count = 20 + (state >> 59) as usize;
            let mut text = String::new();
            let mut local = state;
            for index in 0..digit_count {
                local = local.wrapping_mul(6364136223846793005).wrapping_add(1);
                let digit = (local >> 60) % 10;
                if index == 0 && digit == 0 {
                    text.push('1');
                } else {
                    text.push((b'0' + digit as u8) as char);
                }
            }
            let exponent = ((state >> 40) % 60) as i32 - 30;
            let text = format!("{text}e{exponent}");
            check(&text);
        }
    }

    #[test]
    fn the_scanner_accepts_and_rejects_what_the_old_one_did() {
        // Accepted shapes.
        for text in [
            "1", "+1", "-1", ".5", "5.", "1.5", "1e5", "1E5", "1e+5", "1e-5", "0.0",
        ] {
            assert!(scan(text).is_some(), "{text} should be accepted");
        }
        // Rejected shapes.
        for text in [
            "", "+", "-", ".", "1.2.3", "1e", "1e+", "abc", "1a", " 1", "1 ", "1e5x", "0x1",
        ] {
            assert!(scan(text).is_none(), "{text} should be rejected");
        }
    }

    #[test]
    fn zero_keeps_its_sign() {
        assert_eq!(parse("0").unwrap().to_bits(), 0.0f64.to_bits());
        assert_eq!(parse("-0").unwrap().to_bits(), (-0.0f64).to_bits());
        assert_eq!(parse("-0.000").unwrap().to_bits(), (-0.0f64).to_bits());
        assert_eq!(parse("0e999999").unwrap().to_bits(), 0.0f64.to_bits());
    }

    #[test]
    fn overflow_and_underflow_saturate() {
        assert!(parse("1e400").unwrap().is_infinite());
        assert!(parse("-1e400").unwrap().is_infinite());
        assert_eq!(parse("1e-400").unwrap(), 0.0);
        // The clamp must not wrap a huge exponent into a small one.
        assert!(parse("1e18446744073709551616").unwrap().is_infinite());
        assert_eq!(parse("1e-18446744073709551616").unwrap(), 0.0);
    }
}
