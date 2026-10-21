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
fn lemire_alone_is_sufficient() {
    // The emitted helper omits Clinger's fast path, because it is the only
    // part of the algorithm that needs f64 arithmetic and a hand-written
    // NIR helper may not name a physical FP register. That is only safe if
    // the shortcut is an optimization rather than a correctness crutch, so
    // run every vector both ways and require identical bits.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..20_000 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let value = f64::from_bits(state);
        if !value.is_finite() {
            continue;
        }
        let text = format!("{value:e}");
        let with = parse(&text).expect("accepted");
        let without = parse_without_fast_path(&text).expect("accepted");
        assert_eq!(
            with.to_bits(),
            without.to_bits(),
            "{text}: fast path {:#018x} vs Lemire {:#018x}",
            with.to_bits(),
            without.to_bits()
        );
    }
    // And the exactly-representable range the shortcut exists to serve.
    for text in [
        "1",
        "10",
        "100",
        "1e22",
        "1e-22",
        "0.5",
        "0.25",
        "123456789",
        "9007199254740992",
        "1.5",
        "2.5",
        "1e15",
        "1e-15",
        "3",
        "7",
        "0.1",
        "0.2",
        "0.3",
    ] {
        let with = parse(text).expect("accepted");
        let without = parse_without_fast_path(text).expect("accepted");
        assert_eq!(with.to_bits(), without.to_bits(), "{text}");
        let std: f64 = text.parse().unwrap();
        assert_eq!(without.to_bits(), std.to_bits(), "{text} vs std");
    }
}

#[test]
fn the_original_corpus_never_reached_the_exact_fallback() {
    // Measured, not assumed. Every vector the corpus carried before
    // plan-120-F's Phase 2 is settled by Lemire alone, so the cross-arch
    // runs would have exercised the 128-bit multiply and none of the
    // big-integer comparison — which is why the midpoint vectors below were
    // added to the fixture. If a future vector does reach the fallback this
    // assertion fails, and the right response is to relax it, not to drop
    // the midpoints.
    let mut reached = Vec::new();
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
        "9007199254740993",
        "123456789012345678901234567890",
        "1.000000000000000011102230246251565404236316680908203125",
        "3.141592653589793238462643383279",
        "1.7976931348623157e308",
    ] {
        if takes_exact_path(text) {
            reached.push(text);
        }
    }
    assert!(
        reached.is_empty(),
        "these corpus vectors DO reach the exact fallback: {reached:?}"
    );
}

#[test]
fn the_exact_fallback_is_reachable_and_correct() {
    // Midpoints are the constructive way in: the exact decimal expansion of
    // the value halfway between two doubles has far more than 19 significant
    // digits, so the truncated mantissa cannot decide it and only the
    // big-integer comparison can. Each one also exercises the tie-to-even
    // branch, which nothing else does.
    let mut reached = 0;
    for bits in [
        0x3FF0_0000_0000_0000u64, // 1.0
        0x3FF0_0000_0000_0001,
        0x0000_0000_0000_0001, // smallest subnormal
        0x000F_FFFF_FFFF_FFFF, // largest subnormal
        0x0010_0000_0000_0000, // smallest normal
        0x4024_0000_0000_0000, // 10.0
        0x7FEF_FFFF_FFFF_FFFF, // largest finite
        0x3FB9_999A_0000_0000,
    ] {
        let lower = f64::from_bits(bits);
        let upper = f64::from_bits(bits + 1);
        // The midpoint as an exact decimal string, via the halfway integer.
        let text = midpoint_decimal(lower, upper);
        if takes_exact_path(&text) {
            reached += 1;
        }
        let want: f64 = text.parse().expect("std parses the midpoint");
        let have = parse(&text).expect("accepted");
        assert_eq!(
            have.to_bits(),
            want.to_bits(),
            "{text}: want {:#018x}, got {:#018x}",
            want.to_bits(),
            have.to_bits()
        );
    }
    assert!(
        reached >= 6,
        "only {reached} midpoints reached the exact fallback; the corpus \
         would not be covering it"
    );
}

/// The exact decimal expansion of the midpoint between two adjacent doubles.
/// Both are dyadic, so their mean is dyadic too and terminates in decimal.
fn midpoint_decimal(lower: f64, upper: f64) -> String {
    // (lower + upper) / 2 == (m*2 + 1) * 2^(e-1) with (m, e) from `lower`.
    let bits = lower.to_bits();
    let biased = (bits >> 52) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (m, e) = if biased == 0 {
        (fraction, -1074i32)
    } else {
        (fraction | (1u64 << 52), biased - 1075)
    };
    debug_assert_eq!(upper.to_bits(), bits + 1);
    exact_dyadic(m as u128 * 2 + 1, e - 1)
}

/// `value * 2^exponent` as an exact decimal string.
fn exact_dyadic(value: u128, exponent: i32) -> String {
    if exponent >= 0 {
        let mut digits = vec![value];
        let _ = &mut digits;
        let mut result = num_to_string(value);
        for _ in 0..exponent {
            result = mul_decimal_by_two(&result);
        }
        return result;
    }
    // Multiplying by 5^k and shifting the point k places is exact.
    let mut result = num_to_string(value);
    for _ in 0..(-exponent) {
        result = mul_decimal_by_five(&result);
    }
    let k = (-exponent) as usize;
    let mut digits = result;
    while digits.len() <= k {
        digits.insert(0, '0');
    }
    let split = digits.len() - k;
    format!("{}.{}", &digits[..split], &digits[split..])
}

fn num_to_string(value: u128) -> String {
    value.to_string()
}

fn mul_decimal_by_two(text: &str) -> String {
    mul_decimal_small(text, 2)
}

fn mul_decimal_by_five(text: &str) -> String {
    mul_decimal_small(text, 5)
}

fn mul_decimal_small(text: &str, factor: u32) -> String {
    let mut carry = 0u32;
    let mut out: Vec<u8> = Vec::with_capacity(text.len() + 1);
    for byte in text.bytes().rev() {
        let digit = (byte - b'0') as u32 * factor + carry;
        out.push(b'0' + (digit % 10) as u8);
        carry = digit / 10;
    }
    while carry > 0 {
        out.push(b'0' + (carry % 10) as u8);
        carry /= 10;
    }
    out.reverse();
    String::from_utf8(out).expect("digits")
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

mod rare_paths;
