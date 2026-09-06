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
fn the_two_factorings_agree() {
    // The emitted code truncates to 18 digits natively and rounds in
    // MFBASIC; the direct implementation rounds the exact stream at each
    // `p`. They must be indistinguishable, or the factoring that keeps
    // rounding out of assembly is not sound.
    let mut state: u64 = 0xC0FF_EE00_1234_5678;
    let mut checked = 0u32;
    for _ in 0..20_000 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let value = f64::from_bits(state);
        if !value.is_finite() {
            continue;
        }
        assert_eq!(
            stringify_number_via_18(value),
            stringify_number(value),
            "value {value:e} ({:#018x})",
            value.to_bits()
        );
        checked += 1;
    }
    assert!(checked > 15_000, "too few finite samples: {checked}");
    // And the curated shapes.
    for value in [
        0.0,
        -0.0,
        1.0,
        -1.5,
        1e20,
        1e21,
        1e-6,
        1e-7,
        5e-324,
        1.7976931348623157e308,
        0.1,
        877566786661990.25,
        2188699164681338.2,
        9.999999999999999e22,
    ] {
        assert_eq!(
            stringify_number_via_18(value),
            stringify_number(value),
            "value {value:e}"
        );
    }
}

#[test]
fn the_search_finds_the_shortest_form() {
    // A value needing few digits must not be padded out to 17.
    assert_eq!(stringify_number(0.5), "0.5");
    assert_eq!(stringify_number(1.5), "1.5");
    assert_eq!(stringify_number(1234.0), "1234");
    assert_eq!(stringify_number(1e100), "1e+100");
}
