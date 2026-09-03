//! The reference specification for `_mfb_rt_float_to_string_sci` and for
//! `__json_stringifyNumber`'s new body (plan-120-G).
//!
//! Same split as plan-120-F's `float_parse_ref.rs`, for the same reason: the
//! emitted NIR cannot be exercised from `cargo test`, so the algorithm is
//! written once here in ordinary Rust and pinned, and the emitted helper is
//! then a transliteration of something already known to be right.
//!
//! Two things are specified:
//!
//! 1. **`sci_digits(v, p)`** — the `p` most significant decimal digits of a
//!    finite non-zero double, correctly rounded, with its decimal exponent.
//!    The rounding is **half-to-EVEN**, which is load-bearing: at an exact tie
//!    `toExponential` rounds half-away-from-zero and disagrees with what
//!    `JSON.stringify` needs. `2188699164681338.2` is the worked example — its
//!    exact value is `2188699164681338.25`, a tie at 17 digits, where Node's
//!    `toExponential(16)` gives `...8383` and half-to-even gives `...8382`.
//!
//! 2. **`stringify_number(v)`** — the whole rendering: integer form if it round
//!    trips, else the shortest `p` in `1..=17` whose scientific rendering reads
//!    back as `v`, then ECMAScript's placement rules (plain decimal for
//!    `1e-6 <= |v| < 1e21`, exponential outside).
//!
//! The oracle is Node's `JSON.stringify`. Rather than shell out per value, the
//! tests reconstruct what Node must print from Rust's own shortest-round-trip
//! formatting plus the placement rules, and separately check the curated table
//! that was captured from Node verbatim.
//!
//! This module is `#[cfg(test)]`: a specification and an oracle, never shipped.

/// The `p` most significant decimal digits of `value` (finite, non-zero,
/// magnitude only), correctly rounded half-to-even, and the decimal exponent
/// `e` such that the value is `0.d1d2... * 10^(e+1)`, i.e. `d1.d2d3... * 10^e`.
///
/// Exact throughout: the digit stream comes from `m * 2^e2` by the same
/// integer/limb passes the native formatter uses, so there is no intermediate
/// rounding to reason about.
pub(crate) fn sci_digits(value: f64, p: u32) -> (Vec<u8>, i32) {
    assert!(value.is_finite() && value > 0.0, "magnitude only, non-zero");
    assert!((1..=17).contains(&p), "p out of range: {p}");

    let bits = value.to_bits();
    let biased = (bits >> 52) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (m, e2) = if biased == 0 {
        (fraction, -1074i32)
    } else {
        (fraction | (1u64 << 52), biased - 1075)
    };

    // The exact value as a decimal digit stream. `int_digits` are the integer
    // part most-significant first; `frac` is the remaining fraction, held as a
    // big fraction that yields one digit per multiply by ten.
    let (int_digits, mut frac) = split(m, e2);

    let mut digits: Vec<u8> = Vec::with_capacity(p as usize + 1);
    let exponent: i32;

    if !int_digits.is_empty() {
        exponent = int_digits.len() as i32 - 1;
        digits.extend_from_slice(&int_digits);
    } else {
        // Skip leading zeros of the fraction; the first non-zero digit sets the
        // exponent. Only the digits from there are ever stored, which is why
        // the emitted helper needs a buffer of p+1 rather than one big enough
        // for a subnormal's 300-odd leading zeros.
        let mut zeros = 0i32;
        loop {
            let digit = frac.next_digit();
            if digit != 0 {
                digits.push(digit);
                break;
            }
            zeros += 1;
            assert!(zeros < 400, "a finite double cannot have this many zeros");
        }
        exponent = -(zeros + 1);
    }

    // Extend to p+1 digits so the rounding digit is available, then note
    // whether anything beyond it is non-zero.
    while digits.len() < p as usize + 1 {
        digits.push(frac.next_digit());
    }
    let round_digit = digits[p as usize];
    // Sticky must cover EVERY discarded digit, not just the fraction. A large
    // value's integer part can run to 300-odd digits, all of which sit in
    // `digits` past the rounding position; ignoring them turned a round-up into
    // a tie and cost a digit of shortness.
    let tail_nonzero = digits[(p as usize + 1)..].iter().any(|&d| d != 0);
    digits.truncate(p as usize);
    let sticky = tail_nonzero || frac.any_nonzero_remaining();

    let round_up = match round_digit.cmp(&5) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            if sticky {
                true
            } else {
                // Exact tie: to even.
                digits[p as usize - 1] % 2 == 1
            }
        }
    };

    let mut exponent = exponent;
    if round_up {
        let mut index = digits.len();
        loop {
            if index == 0 {
                // Every digit was 9: the carry grows a place, and the exponent
                // moves with it. This is the `9.99...e+N -> 1e+(N+1)` ripple.
                digits.insert(0, 1);
                digits.truncate(p as usize);
                exponent += 1;
                break;
            }
            index -= 1;
            if digits[index] == 9 {
                digits[index] = 0;
            } else {
                digits[index] += 1;
                break;
            }
        }
    }

    (digits, exponent)
}

/// The exact integer digits and remaining fraction of `m * 2^e2`.
fn split(m: u64, e2: i32) -> (Vec<u8>, Fraction) {
    if e2 >= 0 {
        // A whole number: no fraction at all.
        let mut value = Big::from_u64(m);
        value.shl(e2 as usize);
        (value.to_decimal_digits(), Fraction::zero())
    } else {
        let k = (-e2) as usize;
        let integer = if k > 63 { 0 } else { m >> k };
        let mask = if k >= 64 { u64::MAX } else { (1u64 << k) - 1 };
        let fraction = m & mask;
        let int_digits = if integer == 0 {
            Vec::new()
        } else {
            Big::from_u64(integer).to_decimal_digits()
        };
        (int_digits, Fraction::new(fraction, k))
    }
}

/// The fraction `value / 2^k`, as limbs pre-shifted so each multiply by ten
/// carries one decimal digit out of the top. Mirrors the native formatter's
/// representation exactly.
pub(crate) struct Fraction {
    limbs: Vec<u32>,
}

impl Fraction {
    fn zero() -> Self {
        Fraction { limbs: Vec::new() }
    }

    fn new(value: u64, k: usize) -> Self {
        if value == 0 {
            return Fraction::zero();
        }
        let n = k.div_ceil(32);
        let shift = n * 32 - k;
        let mut limbs = vec![0u32; n];
        // The payload spans at most three limbs: the value is below 2^53 and
        // the pre-shift is under 32, so everything above limb 2 is zero. The
        // native formatter relies on the same bound when it places F.
        let shifted = (value as u128) << shift;
        for index in 0..n.min(4) {
            limbs[index] = ((shifted >> (32 * index)) & 0xFFFF_FFFF) as u32;
        }
        Fraction { limbs }
    }

    fn next_digit(&mut self) -> u8 {
        if self.limbs.is_empty() {
            return 0;
        }
        let mut carry: u64 = 0;
        for limb in self.limbs.iter_mut() {
            let product = *limb as u64 * 10 + carry;
            *limb = (product & 0xFFFF_FFFF) as u32;
            carry = product >> 32;
        }
        carry as u8
    }

    fn any_nonzero_remaining(&self) -> bool {
        self.limbs.iter().any(|&limb| limb != 0)
    }
}

/// Minimal big natural for the integer side.
struct Big(Vec<u32>);

impl Big {
    fn from_u64(value: u64) -> Self {
        let mut limbs = vec![value as u32, (value >> 32) as u32];
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Big(limbs)
    }

    fn shl(&mut self, shift: usize) {
        if self.0.is_empty() {
            return;
        }
        for _ in 0..shift {
            let mut carry = 0u32;
            for limb in self.0.iter_mut() {
                let next = *limb >> 31;
                *limb = (*limb << 1) | carry;
                carry = next;
            }
            if carry != 0 {
                self.0.push(carry);
            }
        }
    }

    /// Decimal digits, most significant first.
    fn to_decimal_digits(&self) -> Vec<u8> {
        if self.0.is_empty() {
            return Vec::new();
        }
        let mut limbs = self.0.clone();
        let mut digits = Vec::new();
        while limbs.iter().any(|&l| l != 0) {
            let mut remainder: u64 = 0;
            for limb in limbs.iter_mut().rev() {
                let current = (remainder << 32) | *limb as u64;
                *limb = (current / 10) as u32;
                remainder = current % 10;
            }
            digits.push(remainder as u8);
            while limbs.last() == Some(&0) {
                limbs.pop();
            }
        }
        digits.reverse();
        digits
    }
}

/// ECMAScript's Number-to-String placement, given the significant digits and
/// the decimal exponent. Plain decimal when the exponent is in `[-7, 20]` as
/// ECMAScript counts it (that is, `1e-6 <= |v| < 1e21`), exponential otherwise,
/// with no zero padding on the exponent.
pub(crate) fn place(digits: &[u8], exponent: i32, negative: bool) -> String {
    let text: String = digits.iter().map(|d| (b'0' + d) as char).collect();
    let n = digits.len() as i32;
    // ECMAScript works in terms of `k` digits and `n` where the value is
    // `s * 10^(n-k)`; `n` here is `exponent + 1`.
    let n_ecma = exponent + 1;
    let body = if n_ecma >= 1 && n_ecma <= 21 {
        if n >= n_ecma {
            // Point sits inside or at the end of the digits.
            let (head, tail) = text.split_at(n_ecma as usize);
            if tail.is_empty() {
                head.to_string()
            } else {
                format!("{head}.{tail}")
            }
        } else {
            // Pad with zeros out to the point.
            let mut out = text.clone();
            out.push_str(&"0".repeat((n_ecma - n) as usize));
            out
        }
    } else if n_ecma <= 0 && n_ecma > -6 {
        format!("0.{}{}", "0".repeat((-n_ecma) as usize), text)
    } else {
        // Exponential. ECMAScript writes the exponent with an explicit sign and
        // no padding: `1e+21`, `1e-7`.
        let e = n_ecma - 1;
        let head = &text[..1];
        let tail = &text[1..];
        let mantissa = if tail.is_empty() {
            head.to_string()
        } else {
            format!("{head}.{tail}")
        };
        if e >= 0 {
            format!("{mantissa}e+{e}")
        } else {
            format!("{mantissa}e-{}", -e)
        }
    };
    if negative {
        format!("-{body}")
    } else {
        body
    }
}

/// The whole of `__json_stringifyNumber`'s new behaviour.
pub(crate) fn stringify_number(value: f64) -> String {
    assert!(value.is_finite(), "non-finite has no JSON form");
    if value == 0.0 {
        // Covers -0.0 too: plan-120-C's rule that it serializes as `0`.
        return "0".to_string();
    }
    let negative = value < 0.0;
    let magnitude = value.abs();

    // Shortest `p` whose scientific rendering reads back as the same double.
    // 17 always suffices for binary64, so this always terminates with a
    // round-tripping answer.
    for p in 1..=17u32 {
        let (digits, exponent) = sci_digits(magnitude, p);
        let candidate = place(&digits, exponent, negative);
        if candidate.parse::<f64>() == Ok(value) {
            return candidate;
        }
    }
    unreachable!("17 significant digits always round-trip a binary64")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_node_oracle_table() {
        // Captured verbatim from Node v24.12.0 during plan-120-A's execution
        // and carried in the plan's References.
        for (value, want) in [
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1e-21, "1e-21"),
            (1e-30, "1e-30"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (-0.0, "0"),
            (0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (100.0, "100"),
            (0.1, "0.1"),
            (1e-5, "0.00001"),
        ] {
            assert_eq!(stringify_number(value), want, "value {value:e}");
        }
    }

    #[test]
    fn the_placement_boundaries_are_exact() {
        // The four exponents the rules turn on. Each side of each boundary.
        assert_eq!(stringify_number(1e20), "100000000000000000000");
        assert_eq!(stringify_number(1e21), "1e+21");
        assert_eq!(stringify_number(1e-6), "0.000001");
        assert_eq!(stringify_number(1e-7), "1e-7");
    }

    #[test]
    fn the_all_nines_ripple_carries_into_the_exponent() {
        // Rounding 9.99...  up at p digits must produce 1 and bump the
        // exponent, not a 10-digit mantissa.
        let (digits, exponent) = sci_digits(9.999999999999999e22, 3);
        assert_eq!(digits, vec![1, 0, 0]);
        assert_eq!(exponent, 23, "the carry must move the exponent");
        let (digits, exponent) = sci_digits(0.99999, 2);
        assert_eq!(digits, vec![1, 0]);
        assert_eq!(exponent, 0);
    }

    #[test]
    fn the_tie_breaks_to_even() {
        // 2188699164681338.25 exactly: a tie at 17 significant digits.
        // Half-to-even gives ...8382; toExponential's half-away-from-zero
        // gives ...8383 and would put ~0.03% of values out of step with Node.
        let (digits, exponent) = sci_digits(2188699164681338.2, 17);
        let rendered: String = digits.iter().map(|d| (b'0' + d) as char).collect();
        assert_eq!(rendered, "21886991646813382");
        assert_eq!(exponent, 15);
    }

    #[test]
    fn every_rendering_is_the_shortest_one() {
        // Rust's `{:e}` is NOT a usable oracle here, which cost a debugging
        // round to learn. Where two equally-short forms both read back exactly,
        // Rust picks the half-away-from-zero one and ECMA-262 picks the even
        // one — `877566786661990.25` renders `...990.3` in Rust and `...990.2`
        // in Node, and Node is what this must match. So the fuzz asserts the
        // two properties that define the output instead of deferring to another
        // language's formatter: it reads back exactly, and nothing shorter
        // does. Agreement with Node itself is checked by the curated table
        // above and by the runtime fixture.
        let mut state: u64 = 0x0DDB_1A5E_5BAD_5EED;
        let mut checked = 0u32;
        for _ in 0..20_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let value = f64::from_bits(state);
            if !value.is_finite() || value == 0.0 {
                continue;
            }
            let text = stringify_number(value);
            let back: f64 = text.parse().expect("our own output must parse");
            assert_eq!(
                back.to_bits(),
                value.to_bits(),
                "{text} did not read back as {value:e}"
            );
            // Nothing shorter round-trips: try every smaller significant-digit
            // count and require all of them to fail.
            let used = significant_digits(&text);
            for shorter in 1..used {
                let (digits, exponent) = sci_digits(value.abs(), shorter);
                let candidate = place(&digits, exponent, value < 0.0);
                assert!(
                    candidate.parse::<f64>() != Ok(value),
                    "{candidate} ({shorter} digits) also round-trips, so \
                     {text} is not the shortest form"
                );
            }
            checked += 1;
        }
        assert!(checked > 15_000, "too few finite samples: {checked}");
    }

    /// How many significant digits a rendering carries.
    fn significant_digits(text: &str) -> u32 {
        let mantissa = text.split(['e', 'E']).next().unwrap_or(text);
        let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
        let trimmed = digits.trim_start_matches('0');
        // Trailing zeros of an integer form are placeholders, not significant.
        let trimmed = if mantissa.contains('.') {
            trimmed.to_string()
        } else {
            trimmed.trim_end_matches('0').to_string()
        };
        trimmed.len().max(1) as u32
    }

    #[test]
    fn every_rendering_reads_back_as_the_same_double() {
        // The property that actually matters for interop, asserted directly
        // rather than inferred from agreeing with an oracle.
        let mut state: u64 = 0xF00D_FACE_1234_5678;
        for _ in 0..20_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let value = f64::from_bits(state);
            if !value.is_finite() {
                continue;
            }
            let text = stringify_number(value);
            let back: f64 = text.parse().expect("our own output must parse");
            assert_eq!(
                back.to_bits(),
                value.to_bits(),
                "{text} did not read back as {value:e}"
            );
        }
    }

    /// Write `<bits-hex> <rendering>` for a large sample so Node can check the
    /// whole algorithm against the only authority for it.
    ///
    /// Ignored by default because it needs a file and an external program. Run
    /// it, then verify, with:
    ///
    /// The test prints the path it wrote (on macOS `temp_dir()` is under
    /// `/var/folders`, not `/tmp`), so pass that path to the checker:
    ///
    /// ```text
    /// cargo test --bin mfb -- --ignored write_node_cross_check_sample
    /// node -e 'const fs=require("fs");let bad=0,n=0;
    ///   for (const line of fs.readFileSync(process.argv[1],"utf8").trim().split("\n")) {
    ///     const [hex, got] = line.split(" ");
    ///     const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt("0x"+hex));
    ///     const want = JSON.stringify(b.readDoubleLE());
    ///     n++; if (want !== got) { bad++; if (bad < 10) console.log(hex, "want", want, "got", got); }
    ///   }
    ///   console.log("checked", n, "mismatches", bad);' <printed-path>
    /// ```
    ///
    /// Last run: **checked 50018, mismatches 0** against Node v24.12.0.
    #[test]
    #[ignore = "writes a file and needs Node; the on-demand cross-check"]
    fn write_node_cross_check_sample() {
        use std::fmt::Write as _;
        let mut out = String::new();
        let mut state: u64 = 0x5DEE_CE66_D1B0_1EAF;
        let mut written = 0u32;
        while written < 50_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let value = f64::from_bits(state);
            if !value.is_finite() {
                continue;
            }
            writeln!(out, "{:016x} {}", state, stringify_number(value)).expect("write");
            written += 1;
        }
        // Plus the shapes a random sweep will not reach on its own.
        for value in [
            0.0,
            -0.0,
            1.0,
            -1.0,
            1e20,
            1e21,
            1e-6,
            1e-7,
            5e-324,
            -5e-324,
            f64::MAX,
            f64::MIN_POSITIVE,
            0.1,
            0.3,
            1e100,
            1e-100,
            877566786661990.25,
            2188699164681338.2,
        ] {
            writeln!(out, "{:016x} {}", value.to_bits(), stringify_number(value)).expect("write");
        }
        let path = std::env::temp_dir().join("mfb-sci-sample.txt");
        std::fs::write(&path, out).expect("write sample");
        eprintln!("wrote {} lines to {}", written + 18, path.display());
    }

    #[test]
    fn the_search_finds_the_shortest_form() {
        // A value needing few digits must not be padded out to 17.
        assert_eq!(stringify_number(0.5), "0.5");
        assert_eq!(stringify_number(1.5), "1.5");
        assert_eq!(stringify_number(1234.0), "1234");
        assert_eq!(stringify_number(1e100), "1e+100");
    }
}
