pub(crate) const TYPE_BYTE: &str = "Byte";
pub(crate) const TYPE_FIXED: &str = "Fixed";
pub(crate) const TYPE_FLOAT: &str = "Float";
pub(crate) const TYPE_INTEGER: &str = "Integer";
/// `Money`: a 64-bit signed integer carrier interpreted as a base-10 fixed-point
/// value scaled to 5 decimal places (SCALE = 100000). One unit = 0.00001;
/// `1.00000` is raw i64 `100000` (plan-29-A). It is a *dimensioned* numeric —
/// same-dimension add/subtract, scalar scaling, `M/M` ratio — with every
/// dimensionally-invalid pairing rejected at compile time (see `money_result_type`).
pub(crate) const TYPE_MONEY: &str = "Money";

/// The base-10 scale of a `Money` raw i64: the value is `raw / MONEY_SCALE`, so
/// `1.00000` is `100000` and `0.00001` is `1` (plan-29-B).
pub(crate) const MONEY_SCALE: i64 = 100_000;

/// The literal type a numeric-literal string classifies to. Distinct from the
/// runtime numeric-type constants above: this is only the *literal* lattice
/// (`Integer`/`Float`/`Fixed`/`Money`) that `classify_literal` decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralType {
    Integer,
    Float,
    Fixed,
    Money,
}

/// Classify a *canonical* numeric-literal string (as emitted by the lexer:
/// digit separators already stripped, any radix prefix already decoded to
/// decimal, optional trailing `.`-fraction / `e`-exponent / `f`|`F` suffix)
/// into its literal type and a suffix-free, parse-ready value string.
///
/// This is the single source of truth for numeric-literal typing, replacing the
/// scattered `value.contains('.')` checks (plan-28-A §4.1). A trailing `f`
/// forces `Float` and `F` forces `Fixed` (plan-28-B); otherwise a `.` or an
/// `e`/`E` exponent is `Float`; otherwise `Integer`. The returned value string
/// has any suffix removed so every `parse::<i64>()`/`parse::<f64>()` consumer
/// keeps working.
pub(crate) fn classify_literal(text: &str) -> (String, LiteralType) {
    if let Some(stripped) = text.strip_suffix('f') {
        return (stripped.to_string(), LiteralType::Float);
    }
    if let Some(stripped) = text.strip_suffix('F') {
        return (stripped.to_string(), LiteralType::Fixed);
    }
    // `m`/`M` forces `Money` (plan-29-A §4.4). There is only one money type, so —
    // unlike `f`/`F` — the case is not load-bearing; both map to `Money`.
    if let Some(stripped) = text.strip_suffix('m') {
        return (stripped.to_string(), LiteralType::Money);
    }
    if let Some(stripped) = text.strip_suffix('M') {
        return (stripped.to_string(), LiteralType::Money);
    }
    if text.contains('.') || text.contains('e') || text.contains('E') {
        (text.to_string(), LiteralType::Float)
    } else {
        (text.to_string(), LiteralType::Integer)
    }
}

/// Longest plain-decimal expansion `expand_scientific_notation` will materialize.
/// The widest legal expansion of an `f64` literal is ~325 characters (the
/// smallest subnormal, `5e-324`) and ~310 for the largest finite magnitude, so
/// this budget leaves an order of magnitude of headroom while bounding the work
/// an adversarial exponent (`1e-1000000000`) can demand (bug-11).
const MAX_EXPANDED_DIGITS: usize = 8192;

/// Expand a decimal scientific-notation string (`2.5e2`, `1e-3`) into a plain
/// decimal string (`250`, `0.001`) by shifting the decimal point — exact digit
/// arithmetic, no `f64` rounding. A string with no `e`/`E` is returned unchanged,
/// as is one whose exponent is so extreme that the expansion would exceed
/// [`MAX_EXPANDED_DIGITS`] (callers reject the still-exponential text rather than
/// materializing gigabytes of zeros). Used to keep the exact Fixed/Money
/// conversions precise for exponent literals (plan-28-B).
pub(crate) fn expand_scientific_notation(value: &str) -> String {
    let Some((mantissa, exponent_text)) = value.split_once(['e', 'E']) else {
        return value.to_string();
    };
    let Ok(exponent) = exponent_text.parse::<i32>() else {
        return value.to_string();
    };
    let (negative, mantissa) = mantissa
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, mantissa));
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{int_part}{frac_part}");
    // A zero mantissa is zero at every exponent — fold it without shifting, so
    // `0e-1000000000` costs nothing.
    if !digits.is_empty() && digits.bytes().all(|digit| digit == b'0') {
        return if negative {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    // The decimal point starts after `int_part.len()` significant digits; the
    // exponent shifts it right by `exponent`. Compute in i64: `int_part.len() as
    // i32 + exponent` overflows for an exponent near `i32::MAX`.
    let point = int_part.len() as i64 + exponent as i64;
    // Zeros shifted in on either side, and thus the whole expansion, grow with
    // |point|. Refuse to build one past the budget.
    let zeros = if point <= 0 {
        (-point) as u64
    } else {
        (point as u64).saturating_sub(digits.len() as u64)
    };
    if zeros > MAX_EXPANDED_DIGITS as u64 {
        return value.to_string();
    }
    let point = point as isize;
    let mut result = String::new();
    if negative {
        result.push('-');
    }
    if point <= 0 {
        result.push_str("0.");
        for _ in 0..(-point) {
            result.push('0');
        }
        result.push_str(&digits);
    } else if (point as usize) >= digits.len() {
        result.push_str(&digits);
        for _ in 0..(point as usize - digits.len()) {
            result.push('0');
        }
    } else {
        result.push_str(&digits[..point as usize]);
        result.push('.');
        result.push_str(&digits[point as usize..]);
    }
    result
}

/// The text the runtime `toString(value)` helper produces for a `Float`/`Fixed`
/// constant when no precision argument is supplied — the default is two digits
/// after the decimal point (`builder_strings.rs` stores `Byte 2` into the
/// precision slot). The constant fold must match the runtime byte-for-byte, or
/// the same value prints two different ways depending on whether the compiler
/// could see it (bug-358).
///
/// - `Float`: the literal parses to the same `f64` the immediate encoder
///   materializes (`native_immediate_value`), and Rust's fixed-precision
///   formatting is the exact correctly-rounded `%.2f` (ties-to-even) that
///   `float_format.rs` computes at runtime.
/// - `Fixed`: the literal converts through [`fixed_raw_from_decimal`] — the
///   same single source of truth the immediate encoder uses — and the Q32.32
///   raw renders exactly as `emit_fixed_to_string_value` does at precision 2.
///
/// Scientific-notation literals go through the same conversions, so `2.5e2`
/// still reads the same as the equivalent plain literal (plan-28-B). `None`
/// when the literal does not convert (e.g. a `Fixed` exponent outside the
/// 32.32 range) — the fold is then skipped and the runtime formatter handles
/// the value.
pub(crate) fn default_to_string_text(type_: &str, literal: &str) -> Option<String> {
    match type_ {
        TYPE_FLOAT => {
            let value: f64 = literal.parse().ok()?;
            value.is_finite().then(|| format!("{value:.2}"))
        }
        TYPE_FIXED => fixed_raw_from_decimal(literal)
            .ok()
            .map(fixed_default_to_string_text),
        _ => None,
    }
}

/// Render a Q32.32 `Fixed` raw at the default precision (2), mirroring
/// `emit_fixed_to_string_value` exactly: strip the sign, pre-round the
/// magnitude half-away-from-zero by `ceil(2^31 / 10^2)` (bug-312 K1), split at
/// the radix point, and emit each fraction digit as the carry of a truncating
/// ×10 step.
fn fixed_default_to_string_text(raw: i64) -> String {
    // Half a 2^-32 ULP at two decimal places, rounded up so an exactly
    // representable boundary value (`0.125`) lands on the away-from-zero side.
    const HALF: u64 = (1_u64 << 31).div_ceil(100);
    let negative = raw < 0;
    let mut magnitude = raw.unsigned_abs();
    // The runtime guard is a *signed* compare of the magnitude register against
    // `i64::MAX - half`, so the minimum Fixed (magnitude 2^63, which reads as
    // negative) still takes the bump and the logical shift below sees the
    // carried integer part — reproduce that exactly.
    if (magnitude as i64) <= i64::MAX - HALF as i64 {
        magnitude = magnitude.wrapping_add(HALF);
    }
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    text.push_str(&(magnitude >> 32).to_string());
    text.push('.');
    let mut fraction = magnitude & u64::from(u32::MAX);
    for _ in 0..2 {
        fraction *= 10;
        text.push(char::from(b'0' + (fraction >> 32) as u8));
        fraction &= u64::from(u32::MAX);
    }
    text
}

/// Convert a decimal `Fixed` literal string into its 32.32 fixed-point `i64` raw
/// value (round-half-up on the fractional part). Handles a leading `-` and
/// scientific notation. `Err` when the value is malformed or out of the `i64`
/// raw range. This is the single source of truth for `Fixed` constant lowering,
/// shared by native codegen (`native_immediate_value`) and the fold in
/// `ir::lower` (bug-07: the minimum `Fixed` has no positive-magnitude literal).
pub(crate) fn fixed_raw_from_decimal(value: &str) -> Result<i64, String> {
    const SCALE: i128 = 1_i128 << 32;

    let expanded = expand_scientific_notation(value);
    // The expansion only leaves an exponent marker behind when the exponent does
    // not fit an i32 or is too extreme to expand within the digit budget. Either
    // way (the zero mantissa having been folded) the magnitude is far outside the
    // 32.32 range — reject in O(1) rather than parse the exponential text
    // (bug-11).
    if expanded.contains(['e', 'E']) {
        return Err(format!("Fixed constant `{value}` is out of range"));
    }
    let value = expanded.as_str();
    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (whole, fractional) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fractional.is_empty() {
        return Err(format!("invalid Fixed constant `{value}`"));
    }
    let mut whole_value = if whole.is_empty() {
        0_i128
    } else {
        whole
            .parse::<i128>()
            .map_err(|_| format!("invalid Fixed constant `{value}`"))?
    };
    let mut fractional_value = 0_i128;
    if !fractional.is_empty() {
        // The 32.32 layout resolves only 2^-32 (~2.3e-10), so fractional digits
        // past ~28 places sit far below one ULP and cannot change the
        // round-half-up result. Cap accumulation to keep `fractional_value * SCALE`
        // inside i128 (10^28 * 2^32 ≈ 4.3e37 < i128::MAX) instead of rejecting a
        // long literal outright (bug-91: `1e-39F` must round to 0, not error).
        const MAX_FRACTIONAL_DIGITS: usize = 28;
        let mut denominator = 1_i128;
        for digit in fractional.bytes().take(MAX_FRACTIONAL_DIGITS) {
            if !digit.is_ascii_digit() {
                return Err(format!("invalid Fixed constant `{value}`"));
            }
            fractional_value = fractional_value * 10 + (digit - b'0') as i128;
            denominator *= 10;
        }
        // The remaining digits are below ULP but must still be well-formed.
        for digit in fractional.bytes().skip(MAX_FRACTIONAL_DIGITS) {
            if !digit.is_ascii_digit() {
                return Err(format!("invalid Fixed constant `{value}`"));
            }
        }
        let scaled = fractional_value * SCALE;
        fractional_value = scaled / denominator;
        if (scaled % denominator) * 2 >= denominator {
            fractional_value += 1;
        }
        if fractional_value == SCALE {
            whole_value += 1;
            fractional_value = 0;
        }
    }
    let raw = whole_value
        .checked_mul(SCALE)
        .and_then(|current| current.checked_add(fractional_value))
        .ok_or_else(|| format!("Fixed constant `{value}` is out of range"))?;
    let raw = if negative { -raw } else { raw };
    i64::try_from(raw).map_err(|_| format!("Fixed constant `{value}` is out of range"))
}

/// The outcome of converting a decimal `Money` literal to its raw i64: the raw
/// value plus whether digits beyond the 5th fractional place changed it (which
/// drives the `TYPE_MONEY_LITERAL_PRECISION` warning, plan-29-B §Open Decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MoneyConversion {
    pub raw: i64,
    /// `true` when rounding at the 6th fractional digit changed the stored value
    /// (`1.234567` → `1.23457`); `false` when the literal was exactly representable
    /// (`1.250000`), so the warning stays silent.
    pub lost_precision: bool,
}

/// Convert a decimal `Money` literal string into its scaled raw `i64`
/// (SCALE = 100000, round-half-away-from-zero beyond 5 fractional digits, exact
/// integer arithmetic — no `f64`). This is the single source of truth for Money
/// constant lowering, shared by IR lowering, native immediate emission, and
/// `.mfp` encoding (plan-29-B). `Err` when the value is malformed or outside the
/// i64 raw range. Handles a leading `-` and scientific notation.
pub(crate) fn money_raw_from_decimal(value: &str) -> Result<i64, String> {
    Ok(money_conversion_from_decimal(value)?.raw)
}

/// Like [`money_raw_from_decimal`] but also reports whether excess fractional
/// precision was rounded away, for the literal-precision diagnostic.
pub(crate) fn money_conversion_from_decimal(value: &str) -> Result<MoneyConversion, String> {
    const SCALE: i128 = MONEY_SCALE as i128; // 5 decimal places
    const FRAC_DIGITS: usize = 5;

    let expanded = expand_scientific_notation(value);
    // An unexpanded exponent marker means the magnitude is far outside the Money
    // range (the zero mantissa having been folded) — reject in O(1) (bug-11).
    if expanded.contains(['e', 'E']) {
        return Err(format!("Money constant `{value}` is out of range"));
    }
    let value = expanded.as_str();
    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (whole, fractional) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fractional.is_empty() {
        return Err(format!("invalid Money constant `{value}`"));
    }
    let whole_value = if whole.is_empty() {
        0_i128
    } else {
        whole
            .parse::<i128>()
            .map_err(|_| format!("invalid Money constant `{value}`"))?
    };
    // Validate every fractional digit is a decimal digit (both the significant
    // prefix and the below-scale tail must be well-formed).
    for digit in fractional.bytes() {
        if !digit.is_ascii_digit() {
            return Err(format!("invalid Money constant `{value}`"));
        }
    }
    // The first 5 fractional digits, zero-padded on the right to exactly 5.
    let mut fractional_value = 0_i128;
    for index in 0..FRAC_DIGITS {
        let digit = fractional
            .as_bytes()
            .get(index)
            .map(|byte| (byte - b'0') as i128)
            .unwrap_or(0);
        fractional_value = fractional_value * 10 + digit;
    }
    // The 6th fractional digit (if present) drives round-half-away-from-zero; any
    // nonzero digit at or past the 6th place means the literal was not exactly
    // representable at 5 places.
    let mut lost_precision = false;
    let sixth = fractional
        .as_bytes()
        .get(FRAC_DIGITS)
        .map(|byte| byte - b'0')
        .unwrap_or(0);
    if fractional.len() > FRAC_DIGITS
        && fractional.as_bytes()[FRAC_DIGITS..]
            .iter()
            .any(|b| *b != b'0')
    {
        lost_precision = true;
    }
    let mut whole_value = whole_value;
    if sixth >= 5 {
        fractional_value += 1;
        if fractional_value == SCALE {
            whole_value += 1;
            fractional_value = 0;
        }
    }
    let raw = whole_value
        .checked_mul(SCALE)
        .and_then(|current| current.checked_add(fractional_value))
        .ok_or_else(|| format!("Money constant `{value}` is out of range"))?;
    let raw = if negative { -raw } else { raw };
    let raw =
        i64::try_from(raw).map_err(|_| format!("Money constant `{value}` is out of range"))?;
    Ok(MoneyConversion {
        raw,
        lost_precision,
    })
}

pub(crate) fn binary_result_type(operator: &str, left: &str, right: &str) -> Option<&'static str> {
    if !is_numeric_type(left) || !is_numeric_type(right) {
        return None;
    }
    // Money is a *dimensioned* numeric: any pairing that includes it obeys the
    // dimensional lattice (plan-29-A §4.2), not the ordinary promotion rules.
    let l_money = left == TYPE_MONEY;
    let r_money = right == TYPE_MONEY;
    if l_money || r_money {
        return money_result_type(operator, l_money, r_money);
    }
    if operator == "DIV" {
        Some(TYPE_FLOAT)
    } else if left == TYPE_FIXED || right == TYPE_FIXED {
        Some(TYPE_FIXED)
    } else if left == TYPE_FLOAT || right == TYPE_FLOAT {
        Some(TYPE_FLOAT)
    } else if left == TYPE_BYTE && right == TYPE_BYTE {
        Some(TYPE_BYTE)
    } else {
        Some(TYPE_INTEGER)
    }
}

/// The dimensional algebra for a binary operator with at least one `Money`
/// operand (plan-29-A §1). `k` denotes a dimensionless numeric (`Integer`,
/// `Byte`, `Float`, `Fixed`); `M` denotes `Money`. Any pairing not listed here
/// returns `None` and is rejected by the front end as `TYPE_MONEY_OPERATION_INVALID`.
///
/// | op | valid forms → result | rejected |
/// |----|----------------------|----------|
/// | `+` `-`   | `M,M → M`            | `M,k` `k,M` |
/// | `*`       | `M,k → M` `k,M → M`  | `M,M`       |
/// | `/`       | `M,k → M` `M,M → Float` | `k,M`    |
/// | `DIV`     | `M,M → Float` `M,k → Float` | `k,M` |
/// | `MOD`     | `M,M → M`            | `M,k` `k,M` |
/// | `^`       | —                    | any `M`     |
pub(crate) fn money_result_type(
    operator: &str,
    l_money: bool,
    r_money: bool,
) -> Option<&'static str> {
    match operator {
        "+" | "-" => (l_money && r_money).then_some(TYPE_MONEY),
        "*" => {
            if l_money && r_money {
                None
            } else {
                Some(TYPE_MONEY)
            }
        }
        "/" => {
            if l_money && r_money {
                Some(TYPE_FLOAT)
            } else if l_money {
                Some(TYPE_MONEY)
            } else {
                None
            }
        }
        "DIV" => {
            if l_money {
                Some(TYPE_FLOAT)
            } else {
                None
            }
        }
        "MOD" => (l_money && r_money).then_some(TYPE_MONEY),
        _ => None,
    }
}

pub(crate) fn is_numeric_type(type_: &str) -> bool {
    matches!(
        type_,
        TYPE_BYTE | TYPE_FIXED | TYPE_FLOAT | TYPE_INTEGER | TYPE_MONEY
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_scientific_notation_bounds_extreme_exponents() {
        // In-range literals expand exactly as before.
        assert_eq!(expand_scientific_notation("2.5e2"), "250");
        assert_eq!(expand_scientific_notation("1e-3"), "0.001");
        assert_eq!(expand_scientific_notation("-1.5e1"), "-15");
        // A zero mantissa is zero at any exponent — folded, never shifted.
        assert_eq!(expand_scientific_notation("0e-1000000000"), "0");
        assert_eq!(expand_scientific_notation("0.00e999999999"), "0");
        // Beyond the digit budget the text is returned unchanged: no multi-GB
        // string, and `point` is computed in i64 so `i32::MAX` cannot overflow.
        assert_eq!(expand_scientific_notation("1e-1000000000"), "1e-1000000000");
        assert_eq!(expand_scientific_notation("1e2147483647"), "1e2147483647");
        assert_eq!(expand_scientific_notation("1e-2147483648"), "1e-2147483648");
        // The widest f64 literal still expands (325 characters).
        assert_eq!(expand_scientific_notation("5e-324").len(), 326);
    }

    #[test]
    fn default_to_string_text_matches_the_runtime_default() {
        // Every expectation here is the observed output of the runtime helper
        // (`toString` through an identity function, default precision 2) on
        // macos-aarch64 at bug-358 — the fold must agree byte-for-byte.
        for (literal, expected) in [
            ("3.141592653589793", "3.14"),
            ("2.5", "2.50"),
            ("0.1", "0.10"),
            // An exact decimal half resolves ties-to-even on a Float…
            ("0.125", "0.12"),
            ("0.135", "0.14"),
            ("0.000000000000123", "0.00"),
            ("9.999", "10.00"),
            ("2.675", "2.67"),
            ("2.5e2", "250.00"),
            ("-2.5", "-2.50"),
        ] {
            assert_eq!(
                default_to_string_text(TYPE_FLOAT, literal).as_deref(),
                Some(expected),
                "Float {literal}"
            );
        }
        for (literal, expected) in [
            ("0.666", "0.67"),
            // …but half-away-from-zero on a Fixed (bug-312 K1).
            ("0.125", "0.13"),
            ("2.5", "2.50"),
            ("0.99", "0.99"),
            ("2.5e2", "250.00"),
            ("-0.005", "-0.01"),
            ("-0.001", "-0.00"),
            // The minimum Fixed exercises the signed overflow-guard compare.
            ("-2147483648.0", "-2147483648.00"),
        ] {
            assert_eq!(
                default_to_string_text(TYPE_FIXED, literal).as_deref(),
                Some(expected),
                "Fixed {literal}"
            );
        }
        // A literal the conversion rejects is not folded.
        assert_eq!(default_to_string_text(TYPE_FIXED, "1e-1000000000"), None);
        assert_eq!(default_to_string_text(TYPE_FLOAT, "not-a-number"), None);
    }

    #[test]
    fn fixed_raw_from_decimal_rejects_extreme_exponents_in_o1() {
        for literal in [
            "1e-1000000000",
            "1e2147483647",
            "1e-2147483648",
            "1e9999999999",
        ] {
            let error = fixed_raw_from_decimal(literal).expect_err("must be out of range");
            assert!(error.contains("is out of range"), "{literal}: {error}");
        }
        // A zero mantissa with an extreme exponent is still exactly zero.
        assert_eq!(fixed_raw_from_decimal("0e-1000000000").unwrap(), 0);
        // In-range scientific literals are unaffected.
        assert_eq!(fixed_raw_from_decimal("2.5e2").unwrap(), 250 << 32);
        assert_eq!(fixed_raw_from_decimal("-1e1").unwrap(), -(10 << 32));
    }

    #[test]
    fn fixed_raw_from_decimal_rounds_sub_ulp_literals_to_zero() {
        // bug-91: a literal with more fractional digits than the 32.32 layout can
        // resolve must round (here, to 0), not be rejected as "too many digits".
        assert_eq!(fixed_raw_from_decimal("1e-39").unwrap(), 0);
        assert_eq!(
            fixed_raw_from_decimal("0.0000000000000000000000000000000000001").unwrap(),
            0
        );
        // A value just above 0.5 ULP still rounds up to 1 raw, and long trailing
        // digits below the cap do not change that.
        assert_eq!(
            fixed_raw_from_decimal("0.500000000000000000000000000000001").unwrap(),
            1_i64 << 31
        );
        // Ordinary short fractional literals are byte-identical to before.
        assert_eq!(fixed_raw_from_decimal("0.5").unwrap(), 1_i64 << 31);
        assert_eq!(fixed_raw_from_decimal("0.25").unwrap(), 1_i64 << 30);
    }

    #[test]
    fn classify_literal_types_integers_and_floats() {
        assert_eq!(
            classify_literal("42"),
            ("42".to_string(), LiteralType::Integer)
        );
        assert_eq!(
            classify_literal("4.5"),
            ("4.5".to_string(), LiteralType::Float)
        );
        // Radix literals reach the classifier already canonicalized to decimal.
        assert_eq!(
            classify_literal("4095"),
            ("4095".to_string(), LiteralType::Integer)
        );
    }

    #[test]
    fn classify_literal_handles_exponent_and_suffixes() {
        // Exponent -> Float (plan-28-B); the value string is left parse-ready.
        assert_eq!(
            classify_literal("1e3"),
            ("1e3".to_string(), LiteralType::Float)
        );
        // f/F suffixes force Float/Fixed and are stripped from the value.
        assert_eq!(
            classify_literal("2f"),
            ("2".to_string(), LiteralType::Float)
        );
        assert_eq!(
            classify_literal("2F"),
            ("2".to_string(), LiteralType::Fixed)
        );
        assert_eq!(
            classify_literal("1.5F"),
            ("1.5".to_string(), LiteralType::Fixed)
        );
        // A suffix wins even with an exponent.
        assert_eq!(
            classify_literal("1e3F"),
            ("1e3".to_string(), LiteralType::Fixed)
        );
    }

    #[test]
    fn expand_scientific_notation_returns_input_on_unparseable_exponent() {
        // A non-numeric exponent cannot be parsed to i32, so the string is
        // returned unchanged rather than shifted (the `Err` early-out).
        assert_eq!(expand_scientific_notation("1eZ"), "1eZ");
        assert_eq!(expand_scientific_notation("2.5e+"), "2.5e+");
        // A value with no exponent marker at all is likewise returned as-is.
        assert_eq!(expand_scientific_notation("123"), "123");
    }

    #[test]
    fn fixed_raw_from_decimal_rejects_malformed_values() {
        // A bare decimal point has neither a whole nor a fractional part.
        assert!(fixed_raw_from_decimal(".")
            .unwrap_err()
            .contains("invalid Fixed constant"));
        // A whole part too large to fit an i128 is rejected as malformed.
        let huge_whole = format!("1{}", "0".repeat(39));
        assert!(fixed_raw_from_decimal(&huge_whole)
            .unwrap_err()
            .contains("invalid Fixed constant"));
        // A non-digit in the significant fractional prefix (first 28 places).
        assert!(fixed_raw_from_decimal("1.5x")
            .unwrap_err()
            .contains("invalid Fixed constant"));
        // A non-digit among the below-ULP trailing fractional digits (past 28).
        let bad_tail = format!("0.{}x", "0".repeat(28));
        assert!(fixed_raw_from_decimal(&bad_tail)
            .unwrap_err()
            .contains("invalid Fixed constant"));
    }

    #[test]
    fn fixed_raw_from_decimal_carries_rounding_into_the_whole_part() {
        // A fraction that rounds up to exactly 1.0 must carry into the whole part
        // (fractional_value == SCALE), giving a clean `1 << 32` raw with no
        // fractional remainder.
        assert_eq!(
            fixed_raw_from_decimal("0.9999999999999999999").unwrap(),
            1_i64 << 32
        );
    }

    #[test]
    fn fixed_raw_from_decimal_rejects_out_of_range_magnitudes() {
        // Expands to a plain decimal, but the 32.32 raw exceeds i64 range
        // (i64::try_from fails).
        assert!(fixed_raw_from_decimal("5000000000")
            .unwrap_err()
            .contains("is out of range"));
        assert!(fixed_raw_from_decimal("-5000000000")
            .unwrap_err()
            .contains("is out of range"));
        // A whole part so large that `whole * SCALE` overflows i128 itself.
        let overflow = "9".repeat(29);
        assert!(fixed_raw_from_decimal(&overflow)
            .unwrap_err()
            .contains("is out of range"));
    }

    #[test]
    fn expand_scientific_notation_shifts_the_point() {
        assert_eq!(expand_scientific_notation("1e3"), "1000");
        assert_eq!(expand_scientific_notation("1E3"), "1000");
        assert_eq!(expand_scientific_notation("2.5e2"), "250");
        assert_eq!(expand_scientific_notation("1e-3"), "0.001");
        assert_eq!(expand_scientific_notation("1e+3"), "1000");
        assert_eq!(expand_scientific_notation("10e10"), "100000000000");
        assert_eq!(expand_scientific_notation("-2.5e2"), "-250");
        assert_eq!(expand_scientific_notation("1.5e-2"), "0.015");
        // No exponent -> unchanged.
        assert_eq!(expand_scientific_notation("3.14"), "3.14");
        assert_eq!(expand_scientific_notation("42"), "42");
    }

    #[test]
    fn non_numeric_operand_has_no_result_type() {
        assert_eq!(binary_result_type("+", "String", TYPE_INTEGER), None);
        assert_eq!(binary_result_type("+", TYPE_INTEGER, "Boolean"), None);
        assert_eq!(binary_result_type("+", "String", "String"), None);
    }

    #[test]
    fn div_always_yields_float() {
        // DIV promotes to Float regardless of operand types (even Byte/Byte).
        assert_eq!(
            binary_result_type("DIV", TYPE_BYTE, TYPE_BYTE),
            Some(TYPE_FLOAT)
        );
        assert_eq!(
            binary_result_type("DIV", TYPE_INTEGER, TYPE_INTEGER),
            Some(TYPE_FLOAT)
        );
    }

    #[test]
    fn fixed_dominates_all_other_numerics() {
        assert_eq!(
            binary_result_type("+", TYPE_FIXED, TYPE_INTEGER),
            Some(TYPE_FIXED)
        );
        assert_eq!(
            binary_result_type("*", TYPE_FLOAT, TYPE_FIXED),
            Some(TYPE_FIXED)
        );
        assert_eq!(
            binary_result_type("-", TYPE_BYTE, TYPE_FIXED),
            Some(TYPE_FIXED)
        );
    }

    #[test]
    fn float_dominates_integer_and_byte() {
        assert_eq!(
            binary_result_type("+", TYPE_FLOAT, TYPE_INTEGER),
            Some(TYPE_FLOAT)
        );
        assert_eq!(
            binary_result_type("*", TYPE_BYTE, TYPE_FLOAT),
            Some(TYPE_FLOAT)
        );
    }

    #[test]
    fn byte_pair_stays_byte_but_mixed_widens_to_integer() {
        assert_eq!(
            binary_result_type("+", TYPE_BYTE, TYPE_BYTE),
            Some(TYPE_BYTE)
        );
        assert_eq!(
            binary_result_type("+", TYPE_BYTE, TYPE_INTEGER),
            Some(TYPE_INTEGER)
        );
        assert_eq!(
            binary_result_type("+", TYPE_INTEGER, TYPE_INTEGER),
            Some(TYPE_INTEGER)
        );
    }

    #[test]
    fn is_numeric_type_accepts_only_the_five_numerics() {
        for t in [TYPE_BYTE, TYPE_FIXED, TYPE_FLOAT, TYPE_INTEGER, TYPE_MONEY] {
            assert!(is_numeric_type(t), "{t} should be numeric");
        }
        for t in ["String", "Boolean", "Nothing", ""] {
            assert!(!is_numeric_type(t), "{t} should not be numeric");
        }
    }

    #[test]
    fn classify_literal_types_money_suffix() {
        // `m`/`M` both force Money; the value string is left parse-ready.
        assert_eq!(
            classify_literal("1.25m"),
            ("1.25".to_string(), LiteralType::Money)
        );
        assert_eq!(
            classify_literal("1.25M"),
            ("1.25".to_string(), LiteralType::Money)
        );
        assert_eq!(
            classify_literal("42m"),
            ("42".to_string(), LiteralType::Money)
        );
        // A Money suffix wins even with an exponent.
        assert_eq!(
            classify_literal("1e3m"),
            ("1e3".to_string(), LiteralType::Money)
        );
    }

    #[test]
    fn money_raw_from_decimal_scales_and_rounds() {
        assert_eq!(money_raw_from_decimal("1.25").unwrap(), 125_000);
        assert_eq!(money_raw_from_decimal("0").unwrap(), 0);
        assert_eq!(money_raw_from_decimal("-0.00001").unwrap(), -1);
        assert_eq!(money_raw_from_decimal("1.00000").unwrap(), 100_000);
        // Round-half-away at the 6th fractional digit.
        assert_eq!(money_raw_from_decimal("1.234565").unwrap(), 123_457);
        assert_eq!(money_raw_from_decimal("1.234564").unwrap(), 123_456);
        // Negative rounds away from zero too (magnitude rounds up).
        assert_eq!(money_raw_from_decimal("-1.234565").unwrap(), -123_457);
        // Rounding carries into the whole part.
        assert_eq!(money_raw_from_decimal("0.999995").unwrap(), 100_000);
        // Scientific notation expands exactly.
        assert_eq!(money_raw_from_decimal("1.5e2").unwrap(), 15_000_000);
    }

    #[test]
    fn money_raw_from_decimal_covers_the_full_i64_range() {
        assert_eq!(
            money_raw_from_decimal("92233720368547.75807").unwrap(),
            i64::MAX
        );
        assert!(money_raw_from_decimal("92233720368547.75808")
            .unwrap_err()
            .contains("is out of range"));
        // The min Money is representable only as a negated literal (bug-07 shape).
        assert_eq!(
            money_raw_from_decimal("-92233720368547.75808").unwrap(),
            i64::MIN
        );
        assert!(money_raw_from_decimal("-92233720368547.75809")
            .unwrap_err()
            .contains("is out of range"));
    }

    #[test]
    fn money_raw_from_decimal_rejects_malformed_values() {
        assert!(money_raw_from_decimal(".")
            .unwrap_err()
            .contains("invalid Money constant"));
        assert!(money_raw_from_decimal("1.5x")
            .unwrap_err()
            .contains("invalid Money constant"));
        let huge_whole = format!("1{}", "0".repeat(39));
        assert!(money_raw_from_decimal(&huge_whole)
            .unwrap_err()
            .contains("invalid Money constant"));
    }

    #[test]
    fn money_conversion_reports_lost_precision() {
        // Digits beyond the 5th that change the value flag lost precision.
        let converted = money_conversion_from_decimal("1.234567").unwrap();
        assert_eq!(converted.raw, 123_457);
        assert!(converted.lost_precision);
        // Exactly representable at 5 places -> silent.
        let exact = money_conversion_from_decimal("1.250000").unwrap();
        assert_eq!(exact.raw, 125_000);
        assert!(!exact.lost_precision);
        // Trailing zeros past the 5th place are not a loss.
        let padded = money_conversion_from_decimal("1.2500000000").unwrap();
        assert!(!padded.lost_precision);
        // A 6th digit that rounds but is nonzero counts as a loss even when it
        // rounds down.
        let rounded_down = money_conversion_from_decimal("1.2500004").unwrap();
        assert_eq!(rounded_down.raw, 125_000);
        assert!(rounded_down.lost_precision);
    }

    /// Exhaustively assert the plan-29-A §1 dimensional-lattice table for every
    /// operator, both operand orders, and each dimensionless scalar `k`.
    #[test]
    fn money_dimensional_lattice_table() {
        let scalars = [TYPE_INTEGER, TYPE_BYTE, TYPE_FLOAT, TYPE_FIXED];

        // M , M
        assert_eq!(
            binary_result_type("+", TYPE_MONEY, TYPE_MONEY),
            Some(TYPE_MONEY)
        );
        assert_eq!(
            binary_result_type("-", TYPE_MONEY, TYPE_MONEY),
            Some(TYPE_MONEY)
        );
        assert_eq!(binary_result_type("*", TYPE_MONEY, TYPE_MONEY), None);
        assert_eq!(
            binary_result_type("/", TYPE_MONEY, TYPE_MONEY),
            Some(TYPE_FLOAT)
        );
        assert_eq!(
            binary_result_type("DIV", TYPE_MONEY, TYPE_MONEY),
            Some(TYPE_FLOAT)
        );
        assert_eq!(
            binary_result_type("MOD", TYPE_MONEY, TYPE_MONEY),
            Some(TYPE_MONEY)
        );
        assert_eq!(binary_result_type("^", TYPE_MONEY, TYPE_MONEY), None);

        for k in scalars {
            // M , k
            assert_eq!(binary_result_type("+", TYPE_MONEY, k), None, "M+{k}");
            assert_eq!(binary_result_type("-", TYPE_MONEY, k), None, "M-{k}");
            assert_eq!(
                binary_result_type("*", TYPE_MONEY, k),
                Some(TYPE_MONEY),
                "M*{k}"
            );
            assert_eq!(
                binary_result_type("/", TYPE_MONEY, k),
                Some(TYPE_MONEY),
                "M/{k}"
            );
            assert_eq!(
                binary_result_type("DIV", TYPE_MONEY, k),
                Some(TYPE_FLOAT),
                "M DIV {k}"
            );
            assert_eq!(binary_result_type("MOD", TYPE_MONEY, k), None, "M MOD {k}");
            assert_eq!(binary_result_type("^", TYPE_MONEY, k), None, "M^{k}");

            // k , M
            assert_eq!(binary_result_type("+", k, TYPE_MONEY), None, "{k}+M");
            assert_eq!(binary_result_type("-", k, TYPE_MONEY), None, "{k}-M");
            assert_eq!(
                binary_result_type("*", k, TYPE_MONEY),
                Some(TYPE_MONEY),
                "{k}*M"
            );
            assert_eq!(binary_result_type("/", k, TYPE_MONEY), None, "{k}/M");
            assert_eq!(binary_result_type("DIV", k, TYPE_MONEY), None, "{k} DIV M");
            assert_eq!(binary_result_type("MOD", k, TYPE_MONEY), None, "{k} MOD M");
            assert_eq!(binary_result_type("^", k, TYPE_MONEY), None, "{k}^M");
        }
    }

    #[test]
    fn non_money_lattice_is_unchanged_by_money_rules() {
        // The Money guard must not perturb any all-non-Money pairing.
        assert_eq!(
            binary_result_type("+", TYPE_FIXED, TYPE_INTEGER),
            Some(TYPE_FIXED)
        );
        assert_eq!(
            binary_result_type("*", TYPE_FLOAT, TYPE_BYTE),
            Some(TYPE_FLOAT)
        );
        assert_eq!(
            binary_result_type("+", TYPE_BYTE, TYPE_BYTE),
            Some(TYPE_BYTE)
        );
        assert_eq!(
            binary_result_type("DIV", TYPE_INTEGER, TYPE_INTEGER),
            Some(TYPE_FLOAT)
        );
    }
}
