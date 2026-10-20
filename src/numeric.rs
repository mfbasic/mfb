use crate::operators::BinaryOp;
use crate::types::ParameterType;

/// plan-106-E: the five names below are reached only by the name-domain
/// pinning harness in this file's tests (see [`binary_result_type`]); production
/// promotion is entirely typed.
///
/// plan-111-G: `TYPE_FIXED`/`TYPE_FLOAT` carried no `#[cfg(test)]` while the
/// other three did, so they compiled into the release binary as dead constants
/// — two more `&str` type spellings than the census counted. Gated with the
/// rest.
#[cfg(test)]
pub(crate) const TYPE_BYTE: &str = "Byte";
#[cfg(test)]
pub(crate) const TYPE_FIXED: &str = "Fixed";
#[cfg(test)]
pub(crate) const TYPE_FLOAT: &str = "Float";
#[cfg(test)]
pub(crate) const TYPE_INTEGER: &str = "Integer";
/// `Money`: a 64-bit signed integer carrier interpreted as a base-10 fixed-point
/// value scaled to 5 decimal places (SCALE = 100000). One unit = 0.00001;
/// `1.00000` is raw i64 `100000` (plan-29-A). It is a *dimensioned* numeric —
/// same-dimension add/subtract, scalar scaling, `M/M` ratio — with every
/// dimensionally-invalid pairing rejected at compile time (see `money_result_type`).
#[cfg(test)]
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
/// after the decimal point (`builder_strings/numeric_text.rs` stores `Byte 2` into the
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
pub(crate) fn default_to_string_text(
    type_: &crate::types::ParameterType,
    literal: &str,
) -> Option<String> {
    match type_ {
        crate::types::ParameterType::Float => {
            let value: f64 = literal.parse().ok()?;
            value.is_finite().then(|| format!("{value:.2}"))
        }
        crate::types::ParameterType::Fixed => fixed_raw_from_decimal(literal)
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
pub(crate) fn fixed_default_to_string_text(raw: i64) -> String {
    // Half a 2^-32 ULP at two decimal places, rounded up so an exactly
    // representable boundary value (`0.125`) lands on the away-from-zero side.
    const HALF: u64 = (1_u64 << 31).div_ceil(100);
    let negative = raw < 0;
    // The magnitude is at most 2^63 and the bias at most 2^31, so the unsigned
    // sum cannot overflow: every magnitude rounds, the maximum and minimum
    // Fixed included (bug-818).
    let magnitude = raw.unsigned_abs() + HALF;
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

/// For a string [`expand_scientific_notation`] left unexpanded (its exponent
/// does not fit an i32 or is beyond the digit budget; a zero mantissa was
/// already folded): `Some(true)` when it is a well-formed nonzero mantissa with
/// a negative exponent — a magnitude below 10^-8192, which every exact decimal
/// converter rounds to zero — and `Some(false)` otherwise (a huge magnitude, or
/// malformed text). `None` when `expanded` has no exponent marker. Deciding
/// from the text is what keeps an adversarial exponent O(1) (bug-11).
fn unexpanded_exponent_is_tiny(expanded: &str) -> Option<bool> {
    let (mantissa, exponent) = expanded.split_once(['e', 'E'])?;
    let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let well_formed_mantissa = mantissa.bytes().filter(|byte| *byte == b'.').count() <= 1
        && mantissa.bytes().any(|byte| byte.is_ascii_digit())
        && mantissa
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.');
    let negative_exponent = exponent.strip_prefix('-').is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    });
    Some(well_formed_mantissa && negative_exponent)
}

/// Why a decimal string has no `Fixed` carrier: it is not a decimal number at
/// all, or its rounded value lies outside the signed 32.32 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixedDecimalError {
    Invalid,
    OutOfRange,
}

/// Convert a decimal `Fixed` literal string into its 32.32 fixed-point `i64` raw
/// value, rounding the exact decimal to the nearest carrier with ties away from
/// zero, then range-checking the **signed, rounded** carrier. Handles a leading
/// `-` and scientific notation. This is the single source of truth for `Fixed`
/// constants: native codegen (`native_immediate_value`), the fold in `ir::lower`
/// (bug-07: the minimum `Fixed` has no positive-magnitude literal), `.mfp`
/// constant encoding, and the verifier's literal range check (bug-813), so the
/// literal a program is allowed to write is exactly the literal that lowers.
pub(crate) fn fixed_raw_from_decimal(value: &str) -> Result<i64, String> {
    fixed_conversion_from_decimal(value).map_err(|error| match error {
        FixedDecimalError::Invalid => format!("invalid Fixed constant `{value}`"),
        FixedDecimalError::OutOfRange => format!("Fixed constant `{value}` is out of range"),
    })
}

/// [`fixed_raw_from_decimal`] with a structured error, for callers that must
/// tell a malformed spelling from an out-of-range value.
pub(crate) fn fixed_conversion_from_decimal(value: &str) -> Result<i64, FixedDecimalError> {
    const SCALE: i128 = 1_i128 << 32;
    // A half quantum, 2^-33, needs 33 fractional decimal digits, so 33 digits
    // decide the rounding exactly: with `F` the first 33 digits as an integer,
    // `F / 10^33 * 2^32 = F / (2 * 5^33)`, and since that divisor is even, the
    // digits past the 33rd (a value below 1 in units of `F`) can never move the
    // doubled remainder across it. Truncating earlier misrounds near a tie
    // (bug-813: `0.000000000116415321826934814453125` is exactly half a quantum).
    const DECIDING_DIGITS: usize = 33;
    const DIVISOR: i128 = 2 * 5_i128.pow(DECIDING_DIGITS as u32);

    let expanded = expand_scientific_notation(value);
    // The expansion only leaves an exponent marker behind when the exponent does
    // not fit an i32 or is too extreme to expand within the digit budget (the
    // zero mantissa having been folded) — decide it in O(1) rather than parse
    // the exponential text (bug-11). A positive exponent is far outside the
    // 32.32 range; a negative one is a magnitude below 10^-8192, far under half
    // a quantum, so it rounds to exactly 0 (bug-813), like `1e-39` (bug-91).
    if let Some(tiny) = unexpanded_exponent_is_tiny(&expanded) {
        return if tiny {
            Ok(0)
        } else {
            Err(FixedDecimalError::OutOfRange)
        };
    }
    let value = expanded.as_str();
    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (whole, fractional) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fractional.is_empty() {
        return Err(FixedDecimalError::Invalid);
    }
    // A whole part of decimal digits too large for i128 is a well-formed number
    // far outside the 32.32 range, not a malformed one (`1e400` expands to 401
    // digits): reporting it Invalid skipped the verifier's range diagnostic and
    // failed later, in lowering, with no source location (bug-820).
    let mut whole_value = if whole.is_empty() {
        0_i128
    } else {
        whole.parse::<i128>().map_err(|_| {
            if whole.bytes().all(|digit| digit.is_ascii_digit()) {
                FixedDecimalError::OutOfRange
            } else {
                FixedDecimalError::Invalid
            }
        })?
    };
    // Every fractional digit must be a decimal digit, even past the deciding
    // ones (bug-91: a long spelling rounds rather than being rejected).
    if !fractional.bytes().all(|digit| digit.is_ascii_digit()) {
        return Err(FixedDecimalError::Invalid);
    }
    // The first 33 fractional digits, zero-padded on the right (< 10^33, which
    // fits i128 with room to spare).
    let mut deciding = 0_i128;
    for index in 0..DECIDING_DIGITS {
        let digit = fractional
            .as_bytes()
            .get(index)
            .map_or(0, |byte| byte - b'0');
        deciding = deciding * 10 + i128::from(digit);
    }
    // Round the magnitude half away from zero; the sign is applied afterward.
    let mut fractional_value = deciding / DIVISOR;
    if (deciding % DIVISOR) * 2 >= DIVISOR {
        fractional_value += 1;
    }
    if fractional_value == SCALE {
        whole_value += 1;
        fractional_value = 0;
    }
    let raw = whole_value
        .checked_mul(SCALE)
        .and_then(|current| current.checked_add(fractional_value))
        .ok_or(FixedDecimalError::OutOfRange)?;
    let raw = if negative { -raw } else { raw };
    i64::try_from(raw).map_err(|_| FixedDecimalError::OutOfRange)
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

/// The exact decimal spelling of a `Fixed` raw 32.32 value — the inverse of
/// [`fixed_raw_from_decimal`], which maps it back to the same raw (plan-136-B: an
/// imported package's literal `Fixed` default is decoded from its `CONST_POOL` raw).
///
/// `frac / 2^32` is a finite decimal of at most 32 places (`frac * 5^32` digits),
/// so no rounding happens here; trailing zeros are trimmed.
pub(crate) fn fixed_decimal_from_raw(raw: i64) -> String {
    const FRACTION_BITS: u32 = 32;
    const FIVE_POW_32: u128 = 23_283_064_365_386_962_890_625;
    let magnitude = u128::from(raw.unsigned_abs());
    let whole = magnitude >> FRACTION_BITS;
    let fraction = magnitude & ((1_u128 << FRACTION_BITS) - 1);
    decimal_text(raw < 0, whole, fraction * FIVE_POW_32, 32)
}

/// The `Fixed` constant spelling of a `Float`-suffixed literal (`1.5f`) placed in
/// a `Fixed` slot (bug-820). The literal is a `Float`, so it converts exactly as
/// `toFixed(Float)` does at run time (`mfb spec language types`, "Promotion
/// failures and equality"): the binary64 value — not the decimal spelling — is
/// scaled by `2^32` and rounded to nearest, ties away from zero. `0.1f` therefore
/// lowers to the carrier nearest the binary64 `0.1`, which can differ from the
/// carrier nearest the decimal `0.1` that an unsuffixed literal takes.
///
/// `value` is the suffix-free spelling from [`classify_literal`]. An in-range
/// result is the exact decimal of the carrier, so [`fixed_raw_from_decimal`]
/// maps it back to that raw. An out-of-range binary64 keeps an out-of-range
/// spelling (its shortest round-trip scientific form, which is itself out of
/// range because every binary64 inside the Fixed range rounds into it), and a
/// non-finite one keeps the source spelling, so the verifier's literal range
/// check reports `TYPE_FIXED_LITERAL_OVERFLOW` in both cases.
pub(crate) fn fixed_constant_from_float_literal(value: &str) -> String {
    const SCALE: f64 = 4_294_967_296.0;
    // 2^63: the first magnitude past the i64 raw range; -2^63 itself fits.
    const RAW_LIMIT: f64 = 9_223_372_036_854_775_808.0;
    let Ok(float) = value.parse::<f64>() else {
        return value.to_string();
    };
    if !float.is_finite() {
        return value.to_string();
    }
    // Multiplying by a power of two is exact; `round` is ties-away-from-zero.
    let scaled = (float * SCALE).round();
    if (-RAW_LIMIT..RAW_LIMIT).contains(&scaled) {
        fixed_decimal_from_raw(scaled as i64)
    } else {
        format!("{float:e}")
    }
}

/// The exact decimal spelling of a `Money` raw value (`raw / MONEY_SCALE`) — the
/// inverse of [`money_raw_from_decimal`] (plan-136-B).
pub(crate) fn money_decimal_from_raw(raw: i64) -> String {
    let magnitude = u128::from(raw.unsigned_abs());
    let scale = MONEY_SCALE as u128;
    decimal_text(raw < 0, magnitude / scale, magnitude % scale, 5)
}

/// `[-]whole[.fraction]`, the fraction zero-padded to `places` digits and then
/// stripped of trailing zeros.
fn decimal_text(negative: bool, whole: u128, fraction: u128, places: usize) -> String {
    let sign = if negative && (whole != 0 || fraction != 0) {
        "-"
    } else {
        ""
    };
    let digits = format!("{fraction:0places$}");
    let digits = digits.trim_end_matches('0');
    if digits.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{digits}")
    }
}

/// Like [`money_raw_from_decimal`] but also reports whether excess fractional
/// precision was rounded away, for the literal-precision diagnostic.
pub(crate) fn money_conversion_from_decimal(value: &str) -> Result<MoneyConversion, String> {
    const SCALE: i128 = MONEY_SCALE as i128; // 5 decimal places
    const FRAC_DIGITS: usize = 5;

    let expanded = expand_scientific_notation(value);
    // An unexpanded exponent marker is decided in O(1) (bug-11): a positive
    // exponent is far outside the Money range; a negative one is a nonzero
    // magnitude below 10^-8192, which rounds to 0 with lost precision, as
    // `1e-400` does through the expansion.
    if let Some(tiny) = unexpanded_exponent_is_tiny(&expanded) {
        return if tiny {
            Ok(MoneyConversion {
                raw: 0,
                lost_precision: true,
            })
        } else {
            Err(format!("Money constant `{value}` is out of range"))
        };
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

/// The name-keyed adapter over [`typed_binary_result_type`], for the callers
/// that still hold type *names* (the AST and wire domains). Maps each operand
/// name onto its scalar variant, runs the one algebra, and renders the closed
/// result set back to its `&'static str` spelling — no parse, no second table.
/// The name-domain view of the promotion algebra.
///
/// plan-106-E: production has **no** string promotion caller left — codegen's
/// last two (`static_type_name` and its pre-pass twin) went typed with the NIR
/// walks. This survives as `#[cfg(test)]` because ~30 assertions in this file
/// state the promotion table in NAME form, which is how the table is legible and
/// how it is pinned against the frozen `legacy_*` copies. It is an adapter over
/// [`typed_binary_result_type`], not a second implementation: promotion has
/// exactly one algebra.
#[cfg(test)]
pub(crate) fn binary_result_type(
    operator: BinaryOp,
    left: &str,
    right: &str,
) -> Option<&'static str> {
    let left = numeric_variant(left)?;
    let right = numeric_variant(right)?;
    numeric_variant_name(&typed_binary_result_type(operator, &left, &right)?)
}

/// The scalar numeric lattice as a [`ParameterType`]: `Byte`/`Fixed`/`Float`/
/// `Integer`/`Money`. The lattice gate for [`typed_binary_result_type`] — a
/// variant match, not a name compare (plan-106-A replaced the name-keyed
/// `is_numeric_type` this grew from; the name domain now goes through
/// [`numeric_variant`]).
pub(crate) fn is_numeric(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
    )
}

/// The **one** binary numeric-promotion algebra (plan-106-A): the result type of
/// `left <operator> right`, or `None` when the pairing is not a numeric
/// promotion — a non-numeric operand, or a dimensionally-invalid `Money`
/// pairing (see [`typed_money_result_type`]).
///
/// This is the single source of truth for the compiler. Before plan-106-A the
/// same table was reachable only through seven per-layer wrappers over the
/// name-keyed [`binary_result_type`]; every typed engine now calls this
/// directly and the name-keyed form is the adapter, not the primitive.
pub(crate) fn typed_binary_result_type(
    operator: BinaryOp,
    left: &ParameterType,
    right: &ParameterType,
) -> Option<ParameterType> {
    if !is_numeric(left) || !is_numeric(right) {
        return None;
    }
    // Money is a *dimensioned* numeric: any pairing that includes it obeys the
    // dimensional lattice (plan-29-A §4.2), not the ordinary promotion rules.
    let l_money = matches!(left, ParameterType::Money);
    let r_money = matches!(right, ParameterType::Money);
    if l_money || r_money {
        return typed_money_result_type(operator, l_money, r_money);
    }
    if operator == BinaryOp::IntDiv {
        Some(ParameterType::Float)
    } else if matches!(left, ParameterType::Fixed) || matches!(right, ParameterType::Fixed) {
        Some(ParameterType::Fixed)
    } else if matches!(left, ParameterType::Float) || matches!(right, ParameterType::Float) {
        Some(ParameterType::Float)
    } else if matches!(left, ParameterType::Byte) && matches!(right, ParameterType::Byte) {
        Some(ParameterType::Byte)
    } else {
        Some(ParameterType::Integer)
    }
}

/// The `FOR` induction variable's promoted type: `+` folded across the loop's
/// `start`, `end` and `step` operand types. A non-numeric operand degrades the
/// fold to `Integer`, reproducing the `unwrap_or(TYPE_INTEGER)` that each
/// per-layer `promote_loop_numeric_type_name` copy applied.
pub(crate) fn typed_promote_loop_numeric_type(
    start: &ParameterType,
    end: &ParameterType,
    step: &ParameterType,
) -> ParameterType {
    let first =
        typed_binary_result_type(BinaryOp::Add, start, end).unwrap_or(ParameterType::Integer);
    typed_binary_result_type(BinaryOp::Add, &first, step).unwrap_or(ParameterType::Integer)
}

/// The [`ParameterType`] a numeric type *name* denotes, or `None` for anything
/// outside the five-scalar lattice. A closed static match over this module's own
/// `TYPE_*` constants — deliberately NOT [`ParameterType::parse`], which owns the
/// type *grammar* (plan-106-A's one-parser invariant); this only maps a leaf name
/// the lattice already enumerates.
#[cfg(test)]
fn numeric_variant(name: &str) -> Option<ParameterType> {
    match name {
        TYPE_BYTE => Some(ParameterType::Byte),
        TYPE_FIXED => Some(ParameterType::Fixed),
        TYPE_FLOAT => Some(ParameterType::Float),
        TYPE_INTEGER => Some(ParameterType::Integer),
        TYPE_MONEY => Some(ParameterType::Money),
        _ => None,
    }
}

/// The `&'static str` spelling of a numeric scalar variant — the inverse of
/// [`numeric_variant`], and the render half of the name-keyed adapters above.
#[cfg(test)]
fn numeric_variant_name(type_: &ParameterType) -> Option<&'static str> {
    match type_ {
        ParameterType::Byte => Some(TYPE_BYTE),
        ParameterType::Fixed => Some(TYPE_FIXED),
        ParameterType::Float => Some(TYPE_FLOAT),
        ParameterType::Integer => Some(TYPE_INTEGER),
        ParameterType::Money => Some(TYPE_MONEY),
        _ => None,
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
///
/// plan-106-B deleted the name-keyed `money_result_type` adapter this used to
/// sit behind: `ir::verify` was its last caller, and it speaks
/// [`ParameterType`] now. This is the only definition of the table.
pub(crate) fn typed_money_result_type(
    operator: BinaryOp,
    l_money: bool,
    r_money: bool,
) -> Option<ParameterType> {
    match operator {
        BinaryOp::Add | BinaryOp::Subtract => (l_money && r_money).then_some(ParameterType::Money),
        BinaryOp::Multiply => {
            if l_money && r_money {
                None
            } else {
                Some(ParameterType::Money)
            }
        }
        BinaryOp::Divide => {
            if l_money && r_money {
                Some(ParameterType::Float)
            } else if l_money {
                Some(ParameterType::Money)
            } else {
                None
            }
        }
        BinaryOp::IntDiv => {
            if l_money {
                Some(ParameterType::Float)
            } else {
                None
            }
        }
        BinaryOp::Mod => (l_money && r_money).then_some(ParameterType::Money),
        // `^` and the comparison / logical / concatenation operators have no
        // Money form at all: exponentiating a Money is meaningless, and the
        // rest never reach the dimensional lattice.
        BinaryOp::Power
        | BinaryOp::Or
        | BinaryOp::Xor
        | BinaryOp::And
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::Less
        | BinaryOp::LessEqual
        | BinaryOp::Greater
        | BinaryOp::GreaterEqual
        | BinaryOp::Concat => None,
    }
}

#[cfg(test)]
mod tests;
