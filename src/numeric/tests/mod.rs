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
            default_to_string_text(&crate::types::ParameterType::Float, literal).as_deref(),
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
        // The minimum Fixed's magnitude is 2^63.
        ("-2147483648.0", "-2147483648.00"),
        // Next to the maximum the bias carries into an integer part of 2^31
        // (bug-818: these were truncated to `…47.99`).
        ("2147483647.996", "2147483648.00"),
        ("-2147483647.996", "-2147483648.00"),
        (
            "2147483647.99999999976716935634613037109375",
            "2147483648.00",
        ),
        ("2147483647.994", "2147483647.99"),
    ] {
        assert_eq!(
            default_to_string_text(&crate::types::ParameterType::Fixed, literal).as_deref(),
            Some(expected),
            "Fixed {literal}"
        );
    }
    // A literal the conversion rejects is not folded. (`1e-1000000000` was the
    // example until bug-813 made it round to 0, like `1e-400`.)
    assert_eq!(
        default_to_string_text(&crate::types::ParameterType::Fixed, "1e9999999999"),
        None
    );
    assert_eq!(
        default_to_string_text(&crate::types::ParameterType::Float, "not-a-number"),
        None
    );
}

/// bug-820: a `Float`-suffixed literal in a `Fixed` slot converts its
/// binary64 value, as `toFixed(Float)` does, and an out-of-range or
/// non-finite one keeps a spelling the range check rejects.
#[test]
fn fixed_constant_from_float_literal_converts_the_binary64_value() {
    assert_eq!(fixed_constant_from_float_literal("1.5"), "1.5");
    assert_eq!(fixed_constant_from_float_literal("0"), "0");
    assert_eq!(fixed_constant_from_float_literal("1e2"), "100");
    // binary64 0.1 is slightly above 0.1; both round to raw 429496730.
    let tenth = fixed_constant_from_float_literal("0.1");
    assert_eq!(fixed_raw_from_decimal(&tenth), Ok(429_496_730));
    // A half-quantum binary64 rounds away from zero.
    let half_quantum = fixed_constant_from_float_literal("0.000000000116415321826934814453125");
    assert_eq!(fixed_raw_from_decimal(&half_quantum), Ok(1));
    // The decimal spelling is inside the Fixed range, but the nearest
    // binary64 is exactly 2^31, which is not: the Float value decides.
    assert!(fixed_raw_from_decimal("2147483647.99999994").is_ok());
    let past_max = fixed_constant_from_float_literal("2147483647.99999994");
    assert_eq!(past_max, "2.147483648e9");
    assert!(fixed_raw_from_decimal(&past_max).is_err());
    // Its negation is the Fixed minimum (the lowering's bug-07 fold).
    assert_eq!(
        fixed_raw_from_decimal(&format!("-{past_max}")),
        Ok(i64::MIN)
    );
    assert_eq!(fixed_constant_from_float_literal("3e9"), "3e9");
    assert!(fixed_raw_from_decimal(&fixed_constant_from_float_literal("3e9")).is_err());
    assert_eq!(fixed_constant_from_float_literal("1e400"), "1e400");
    // Out of range, not malformed, so the verifier reports it (a whole
    // part too long for i128 used to classify as Invalid).
    assert_eq!(
        fixed_conversion_from_decimal("1e400"),
        Err(FixedDecimalError::OutOfRange)
    );
    assert_eq!(
        fixed_conversion_from_decimal("-1e400"),
        Err(FixedDecimalError::OutOfRange)
    );
    assert_eq!(
        fixed_conversion_from_decimal("1x400"),
        Err(FixedDecimalError::Invalid)
    );
}

#[test]
fn fixed_decimal_from_raw_round_trips_through_the_parser() {
    let mut raws = vec![
        0,
        1,
        -1,
        1 << 32,
        -(1 << 32),
        (1 << 32) + 1,
        i64::MAX,
        i64::MIN,
    ];
    for literal in ["0.1", "-2.5", "123.456", "0.0000000001", "-7"] {
        raws.push(fixed_raw_from_decimal(literal).expect("literal parses"));
    }
    for raw in raws {
        let text = fixed_decimal_from_raw(raw);
        assert_eq!(
            fixed_raw_from_decimal(&text),
            Ok(raw),
            "raw {raw} spelled `{text}` must parse back to itself"
        );
    }
    assert_eq!(fixed_decimal_from_raw(-(3 << 31)), "-1.5");
    assert_eq!(fixed_decimal_from_raw(5 << 32), "5");
}

#[test]
fn money_decimal_from_raw_round_trips_through_the_parser() {
    let mut raws = vec![0, 1, -1, 150_000, -150_000, i64::MAX, i64::MIN];
    for literal in ["1.23457", "-0.00001", "42"] {
        raws.push(money_raw_from_decimal(literal).expect("literal parses"));
    }
    for raw in raws {
        let text = money_decimal_from_raw(raw);
        assert_eq!(
            money_raw_from_decimal(&text),
            Ok(raw),
            "raw {raw} spelled `{text}` must parse back to itself"
        );
    }
    assert_eq!(money_decimal_from_raw(150_000), "1.5");
    assert_eq!(money_decimal_from_raw(-1), "-0.00001");
}

#[test]
fn fixed_raw_from_decimal_rejects_extreme_exponents_in_o1() {
    for literal in ["1e2147483647", "1e9999999999", "-1e9999999999"] {
        let error = fixed_raw_from_decimal(literal).expect_err("must be out of range");
        assert!(error.contains("is out of range"), "{literal}: {error}");
    }
    // bug-813: an extreme NEGATIVE exponent is a magnitude below 10^-8192,
    // far under half a quantum (2^-33), so it rounds to exactly 0 — still in
    // O(1), without expanding — like `1e-39` (bug-91) and `1e-400`.
    for literal in [
        "1e-1000000000",
        "1e-2147483648",
        "-1e-1000000000",
        "1e-9999999999",
    ] {
        assert_eq!(fixed_raw_from_decimal(literal), Ok(0), "{literal}");
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
    // A whole part too large to fit an i128 is a well-formed number out of
    // range, not a malformed one (bug-820: reporting it malformed skipped
    // the verifier's TYPE_FIXED_LITERAL_OVERFLOW).
    let huge_whole = format!("1{}", "0".repeat(39));
    assert!(fixed_raw_from_decimal(&huge_whole)
        .unwrap_err()
        .contains("out of range"));
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

/// bug-813: the converter is the single range authority for Fixed
/// literals, so its rounding must be exact at every boundary, including
/// spellings longer than the 33 fractional digits a half quantum (2^-33)
/// needs. Every expected raw was computed independently with Python's
/// `fractions.Fraction`: `round_half_away(Fraction(s) * 2**32)`.
#[test]
fn fixed_raw_from_decimal_rounds_exactly_at_boundaries() {
    for (literal, raw) in [
        // The exact maximum and minimum carriers.
        ("2147483647.99999999976716935634613037109375", i64::MAX),
        ("-2147483648", i64::MIN),
        ("-2147483647.99999999976716935634613037109375", -i64::MAX),
        ("2.14748364799999999976716935634613037109375e9", i64::MAX),
        // Just inside the rounding boundary beside each end.
        ("2147483647.999999999883584678173065185546874", i64::MAX),
        (
            "2147483647.9999999998835846781730651855468749999999",
            i64::MAX,
        ),
        ("-2147483648.000000000116415321826934814453124", i64::MIN),
        // Exact half-quantum ties round away from zero, which needs the
        // 29th..33rd fractional digits.
        ("0.000000000116415321826934814453125", 1),
        ("-0.000000000116415321826934814453125", -1),
        ("0.000000000814907252788543701171875", 4),
        // Just below a tie stays down, however long the spelling.
        ("0.00000000011641532182693481445312499", 0),
        ("0.0000000001164153218269348144531249999", 0),
    ] {
        assert_eq!(fixed_raw_from_decimal(literal), Ok(raw), "{literal}");
    }
    // Rounding past either end is out of range.
    for literal in [
        "2147483647.999999999883584678173065185546875",
        "-2147483648.000000000116415321826934814453125",
        "2147483648",
        "-2147483648.0000000002",
    ] {
        let error = fixed_raw_from_decimal(literal).expect_err("must be out of range");
        assert!(error.contains("is out of range"), "{literal}: {error}");
    }
}

/// A Money literal with an extreme negative exponent is a nonzero magnitude
/// below 10^-8192: it rounds to 0 and reports the lost precision (the
/// `TYPE_MONEY_LITERAL_PRECISION` warning), exactly as `1e-400` does, rather
/// than being reported out of range. Still decided in O(1) (bug-11).
#[test]
fn money_conversion_rounds_extreme_negative_exponents_to_zero() {
    let tiny = MoneyConversion {
        raw: 0,
        lost_precision: true,
    };
    assert_eq!(money_conversion_from_decimal("1e-400"), Ok(tiny));
    for literal in ["1e-1000000000", "-1e-2147483648", "2.5e-9999999999"] {
        assert_eq!(
            money_conversion_from_decimal(literal),
            Ok(tiny),
            "{literal}"
        );
    }
    for literal in ["1e2147483647", "1e9999999999"] {
        assert!(money_conversion_from_decimal(literal)
            .unwrap_err()
            .contains("is out of range"));
    }
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
    assert_eq!(
        binary_result_type(BinaryOp::Add, "String", TYPE_INTEGER),
        None
    );
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_INTEGER, "Boolean"),
        None
    );
    assert_eq!(binary_result_type(BinaryOp::Add, "String", "String"), None);
}

#[test]
fn div_always_yields_float() {
    // DIV promotes to Float regardless of operand types (even Byte/Byte).
    assert_eq!(
        binary_result_type(BinaryOp::IntDiv, TYPE_BYTE, TYPE_BYTE),
        Some(TYPE_FLOAT)
    );
    assert_eq!(
        binary_result_type(BinaryOp::IntDiv, TYPE_INTEGER, TYPE_INTEGER),
        Some(TYPE_FLOAT)
    );
}

#[test]
fn fixed_dominates_all_other_numerics() {
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_FIXED, TYPE_INTEGER),
        Some(TYPE_FIXED)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Multiply, TYPE_FLOAT, TYPE_FIXED),
        Some(TYPE_FIXED)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Subtract, TYPE_BYTE, TYPE_FIXED),
        Some(TYPE_FIXED)
    );
}

#[test]
fn float_dominates_integer_and_byte() {
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_FLOAT, TYPE_INTEGER),
        Some(TYPE_FLOAT)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Multiply, TYPE_BYTE, TYPE_FLOAT),
        Some(TYPE_FLOAT)
    );
}

#[test]
fn byte_pair_stays_byte_but_mixed_widens_to_integer() {
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_BYTE, TYPE_BYTE),
        Some(TYPE_BYTE)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_BYTE, TYPE_INTEGER),
        Some(TYPE_INTEGER)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_INTEGER, TYPE_INTEGER),
        Some(TYPE_INTEGER)
    );
}

#[test]
fn is_numeric_type_accepts_only_the_five_numerics() {
    for t in [TYPE_BYTE, TYPE_FIXED, TYPE_FLOAT, TYPE_INTEGER, TYPE_MONEY] {
        assert!(legacy_is_numeric_type(t), "{t} should be numeric");
        let variant = numeric_variant(t).unwrap_or_else(|| panic!("{t} has no variant"));
        assert!(is_numeric(&variant), "{t} should be numeric");
        assert_eq!(numeric_variant_name(&variant), Some(t));
    }
    for t in ["String", "Boolean", "Nothing", ""] {
        assert!(!legacy_is_numeric_type(t), "{t} should not be numeric");
        assert_eq!(numeric_variant(t), None, "{t} should have no variant");
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
        binary_result_type(BinaryOp::Add, TYPE_MONEY, TYPE_MONEY),
        Some(TYPE_MONEY)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Subtract, TYPE_MONEY, TYPE_MONEY),
        Some(TYPE_MONEY)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Multiply, TYPE_MONEY, TYPE_MONEY),
        None
    );
    assert_eq!(
        binary_result_type(BinaryOp::Divide, TYPE_MONEY, TYPE_MONEY),
        Some(TYPE_FLOAT)
    );
    assert_eq!(
        binary_result_type(BinaryOp::IntDiv, TYPE_MONEY, TYPE_MONEY),
        Some(TYPE_FLOAT)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Mod, TYPE_MONEY, TYPE_MONEY),
        Some(TYPE_MONEY)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Power, TYPE_MONEY, TYPE_MONEY),
        None
    );

    for k in scalars {
        // M , k
        assert_eq!(
            binary_result_type(BinaryOp::Add, TYPE_MONEY, k),
            None,
            "M+{k}"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Subtract, TYPE_MONEY, k),
            None,
            "M-{k}"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Multiply, TYPE_MONEY, k),
            Some(TYPE_MONEY),
            "M*{k}"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Divide, TYPE_MONEY, k),
            Some(TYPE_MONEY),
            "M/{k}"
        );
        assert_eq!(
            binary_result_type(BinaryOp::IntDiv, TYPE_MONEY, k),
            Some(TYPE_FLOAT),
            "M DIV {k}"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Mod, TYPE_MONEY, k),
            None,
            "M MOD {k}"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Power, TYPE_MONEY, k),
            None,
            "M^{k}"
        );

        // k , M
        assert_eq!(
            binary_result_type(BinaryOp::Add, k, TYPE_MONEY),
            None,
            "{k}+M"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Subtract, k, TYPE_MONEY),
            None,
            "{k}-M"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Multiply, k, TYPE_MONEY),
            Some(TYPE_MONEY),
            "{k}*M"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Divide, k, TYPE_MONEY),
            None,
            "{k}/M"
        );
        assert_eq!(
            binary_result_type(BinaryOp::IntDiv, k, TYPE_MONEY),
            None,
            "{k} DIV M"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Mod, k, TYPE_MONEY),
            None,
            "{k} MOD M"
        );
        assert_eq!(
            binary_result_type(BinaryOp::Power, k, TYPE_MONEY),
            None,
            "{k}^M"
        );
    }
}

// ----- plan-106-A Phase 1: typed promotion equivalence ------------------
//
// `typed_binary_result_type` is now the single source of truth and
// `binary_result_type` delegates to it, so comparing the two would be
// vacuous. These tests instead pin the typed algebra against a **frozen
// verbatim copy** of the name-keyed table as it stood at plan-106-A's base
// (`94a38078b`), exhaustively over the whole scalar lattice × every
// operator the language spells. Any drift in the collapse — the exact
// "silent wrong value" class plan-106 exists to prevent — reds here.

/// The pre-plan-106-A `is_numeric_type` body, copied verbatim.
fn legacy_is_numeric_type(type_: &str) -> bool {
    matches!(
        type_,
        TYPE_BYTE | TYPE_FIXED | TYPE_FLOAT | TYPE_INTEGER | TYPE_MONEY
    )
}

/// The pre-plan-106-A `binary_result_type` body, copied verbatim. The
/// reference oracle; never call it from production code.
fn legacy_binary_result_type(operator: &str, left: &str, right: &str) -> Option<&'static str> {
    if !legacy_is_numeric_type(left) || !legacy_is_numeric_type(right) {
        return None;
    }
    let l_money = left == TYPE_MONEY;
    let r_money = right == TYPE_MONEY;
    if l_money || r_money {
        return legacy_money_result_type(operator, l_money, r_money);
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

/// The pre-plan-106-A `money_result_type` body, copied verbatim.
fn legacy_money_result_type(operator: &str, l_money: bool, r_money: bool) -> Option<&'static str> {
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

/// The pre-plan-106-A `promote_loop_numeric_type_name` body (the shape
/// `ir::lower` and `monomorph` each hand-copied), over the legacy oracle.
fn legacy_promote_loop_numeric_type_name(start: &str, end: &str, step: &str) -> String {
    let first = legacy_binary_result_type("+", start, end).unwrap_or(TYPE_INTEGER);
    legacy_binary_result_type("+", first, step)
        .unwrap_or(TYPE_INTEGER)
        .to_string()
}

/// Every type name the promotion lattice can be asked about: the five
/// numeric scalars plus non-numeric names that must fall out as `None`.
const LATTICE_NAMES: [&str; 10] = [
    TYPE_BYTE,
    TYPE_FIXED,
    TYPE_FLOAT,
    TYPE_INTEGER,
    TYPE_MONEY,
    "String",
    "Boolean",
    "Nothing",
    "List OF Integer",
    "Unknown",
];

/// The matching `ParameterType` for each `LATTICE_NAMES` entry, built
/// structurally (no parse) so the test never depends on the grammar.
fn lattice_types() -> Vec<(&'static str, ParameterType)> {
    vec![
        (TYPE_BYTE, ParameterType::Byte),
        (TYPE_FIXED, ParameterType::Fixed),
        (TYPE_FLOAT, ParameterType::Float),
        (TYPE_INTEGER, ParameterType::Integer),
        (TYPE_MONEY, ParameterType::Money),
        ("String", ParameterType::String),
        ("Boolean", ParameterType::Boolean),
        ("Nothing", ParameterType::Nothing),
        (
            "List OF Integer",
            ParameterType::ListOf(Box::new(ParameterType::Integer)),
        ),
        ("Unknown", ParameterType::Unknown),
    ]
}

/// Every binary operator the language spells, so the equivalence covers the
/// `DIV`/`MOD`/`^` arms and the comparison/logical operators that fall
/// through to the default `Integer`.
const OPERATORS: [BinaryOp; 17] = [
    BinaryOp::Add,
    BinaryOp::Subtract,
    BinaryOp::Multiply,
    BinaryOp::Divide,
    BinaryOp::IntDiv,
    BinaryOp::Mod,
    BinaryOp::Power,
    BinaryOp::Concat,
    BinaryOp::Equal,
    BinaryOp::NotEqual,
    BinaryOp::Less,
    BinaryOp::Greater,
    BinaryOp::LessEqual,
    BinaryOp::GreaterEqual,
    BinaryOp::And,
    BinaryOp::Or,
    BinaryOp::Xor,
];

#[test]
fn typed_binary_result_type_matches_the_legacy_table_exhaustively() {
    let types = lattice_types();
    assert_eq!(types.len(), LATTICE_NAMES.len());
    let mut checked = 0usize;
    for operator in OPERATORS {
        let spelling = operator.name();
        for (left_name, left) in &types {
            for (right_name, right) in &types {
                let typed = typed_binary_result_type(operator, left, right);
                let rendered = typed.as_ref().and_then(numeric_variant_name);
                // A `Some` result is always a scalar the name map covers.
                assert_eq!(
                    typed.is_some(),
                    rendered.is_some(),
                    "{left_name} {spelling} {right_name}: typed result escaped the scalar lattice ({typed:?})"
                );
                assert_eq!(
                    rendered,
                    legacy_binary_result_type(spelling, left_name, right_name),
                    "{left_name} {spelling} {right_name}"
                );
                // And the shipped adapter agrees with both.
                assert_eq!(
                    binary_result_type(operator, left_name, right_name),
                    rendered,
                    "adapter drift at {left_name} {spelling} {right_name}"
                );
                checked += 1;
            }
        }
    }
    // 17 operators x 10 x 10 operand pairs.
    assert_eq!(checked, 1_700);
}

#[test]
fn typed_promote_loop_matches_the_legacy_fold_exhaustively() {
    let types = lattice_types();
    let mut checked = 0usize;
    for (start_name, start) in &types {
        for (end_name, end) in &types {
            for (step_name, step) in &types {
                let typed = typed_promote_loop_numeric_type(start, end, step);
                let rendered = numeric_variant_name(&typed).unwrap_or_else(|| {
                    panic!("loop promotion escaped the scalar lattice: {typed:?}")
                });
                let legacy = legacy_promote_loop_numeric_type_name(start_name, end_name, step_name);
                assert_eq!(
                    rendered, legacy,
                    "FOR {start_name} TO {end_name} STEP {step_name}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 1_000);
}

#[test]
fn typed_money_result_type_matches_the_legacy_table() {
    for operator in OPERATORS {
        let spelling = operator.name();
        for l_money in [false, true] {
            for r_money in [false, true] {
                assert_eq!(
                    typed_money_result_type(operator, l_money, r_money)
                        .as_ref()
                        .and_then(numeric_variant_name),
                    legacy_money_result_type(spelling, l_money, r_money),
                    "{spelling} l_money={l_money} r_money={r_money}"
                );
            }
        }
    }
}

#[test]
fn is_numeric_agrees_with_the_name_keyed_predicate() {
    for (name, type_) in lattice_types() {
        assert_eq!(is_numeric(&type_), legacy_is_numeric_type(name), "{name}");
    }
    // Variants with no name in the lattice list are still non-numeric.
    assert!(!is_numeric(&ParameterType::AttributeString));
    assert!(!is_numeric(&ParameterType::SetOf(Box::new(
        ParameterType::Integer
    ))));
}

#[test]
fn non_money_lattice_is_unchanged_by_money_rules() {
    // The Money guard must not perturb any all-non-Money pairing.
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_FIXED, TYPE_INTEGER),
        Some(TYPE_FIXED)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Multiply, TYPE_FLOAT, TYPE_BYTE),
        Some(TYPE_FLOAT)
    );
    assert_eq!(
        binary_result_type(BinaryOp::Add, TYPE_BYTE, TYPE_BYTE),
        Some(TYPE_BYTE)
    );
    assert_eq!(
        binary_result_type(BinaryOp::IntDiv, TYPE_INTEGER, TYPE_INTEGER),
        Some(TYPE_FLOAT)
    );
}
