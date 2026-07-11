pub(crate) const TYPE_BYTE: &str = "Byte";
pub(crate) const TYPE_FIXED: &str = "Fixed";
pub(crate) const TYPE_FLOAT: &str = "Float";
pub(crate) const TYPE_INTEGER: &str = "Integer";

/// The literal type a numeric-literal string classifies to. Distinct from the
/// runtime numeric-type constants above: this is only the *literal* lattice
/// (`Integer`/`Float`/`Fixed`) that `classify_literal` decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralType {
    Integer,
    Float,
    Fixed,
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
/// materializing gigabytes of zeros). Used to keep the exact Fixed conversion
/// precise for exponent literals and to fold a scientific-notation literal's
/// `toString` to a plain decimal (plan-28-B).
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
        return if negative { "-0".to_string() } else { "0".to_string() };
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

/// The plain-decimal text of a scientific-notation `Float`/`Fixed` literal, for
/// the constant `toString` fold (plan-28-B). `None` when the exponent is too
/// extreme to expand ([`MAX_EXPANDED_DIGITS`]) — the fold is then skipped and the
/// runtime formatter handles the value, rather than folding to the raw
/// exponential text or materializing gigabytes of zeros (bug-11).
pub(crate) fn expanded_literal_text(value: &str) -> Option<String> {
    let expanded = expand_scientific_notation(value);
    (!expanded.contains(['e', 'E'])).then_some(expanded)
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

pub(crate) fn binary_result_type(operator: &str, left: &str, right: &str) -> Option<&'static str> {
    if !is_numeric_type(left) || !is_numeric_type(right) {
        return None;
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

fn is_numeric_type(type_: &str) -> bool {
    matches!(type_, TYPE_BYTE | TYPE_FIXED | TYPE_FLOAT | TYPE_INTEGER)
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
        assert_eq!(
            expand_scientific_notation("1e-1000000000"),
            "1e-1000000000"
        );
        assert_eq!(expand_scientific_notation("1e2147483647"), "1e2147483647");
        assert_eq!(expand_scientific_notation("1e-2147483648"), "1e-2147483648");
        // The widest f64 literal still expands (325 characters).
        assert_eq!(expand_scientific_notation("5e-324").len(), 326);
        // The fold helper declines rather than yielding exponential text.
        assert_eq!(expanded_literal_text("2.5e2").as_deref(), Some("250"));
        assert_eq!(expanded_literal_text("1e-1000000000"), None);
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
    fn is_numeric_type_accepts_only_the_four_numerics() {
        for t in [TYPE_BYTE, TYPE_FIXED, TYPE_FLOAT, TYPE_INTEGER] {
            assert!(is_numeric_type(t), "{t} should be numeric");
        }
        for t in ["String", "Boolean", "Nothing", ""] {
            assert!(!is_numeric_type(t), "{t} should not be numeric");
        }
    }
}
