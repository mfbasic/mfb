//! bug-775: `big::` arithmetic works a 64-bit word (or, where a quotient digit must fit a
//! single `udiv`, a 32-bit limb) at a time, converts text a chunk of digits per pass, and
//! squares only up to the exponent's top bit.
//!
//! Each test compares two operations **in the same process**, timed with
//! `datetime::monotonicNanos` and interleaved over seven rounds; the medians are compared
//! (the `rt_compress_bounds.rs::decode_time_is_linear_in_output_size` measure). Every pair
//! is chosen so that both sides run the same kind of loop — a `udiv` chain against a `udiv`
//! chain, a `mul`/`umulh` pass against a `mul`/`umulh` pass — so the ratio reads the
//! algorithm's structure, not the machine's relative speed of memory and arithmetic:
//!
//! - **modPow follows the exponent's bit length.** `65537` has 17 significant bits and
//!   `2^23 + 1` has 24; both are three bytes. Looping over every bit of every byte costs
//!   them the same (ratio 0.96–1.00 measured before the fix); looping to the top set bit,
//!   with no squaring after it, makes the 17-bit one cost under half (0.46 after).
//! - **toString is not one digit per pass.** `toString(2000!)` (5,736 digits) against one
//!   short division of `2000!` by 10 — one pass of the same `udiv` chain. One digit per
//!   pass costs about one such pass per digit, halved because the copy shrinks (2,603×
//!   measured before); nine digits per pass costs a ninth of that (267× after).
//! - **parse is not one digit per pass.** `parse` of the same 5,736 digits against one
//!   multiplication of `2000!` by 10 — one `mul`/`umulh` pass over the same magnitude
//!   (1,292× before, 63× after: 19 digits fold in per pass).
//! - **multiply is word-wide.** `251 * x` and `9223372036854775783 * x` for an 8 KiB `x`:
//!   one byte against eight bytes of multiplier. Byte limbs make the second eight rows to
//!   the first's one (5.7× before); 64-bit words make both one row (1.00× after).
//!
//! Every bound sits at least 1.5× from both measurements (macOS aarch64, release `mfb`,
//! `-O1`; see the bug-775 Resolution for the commands).

#[path = "../common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

const ROUNDS: usize = 7;

const PROGRAM: &str = r#"IMPORT big
IMPORT datetime
IMPORT io

FUNC modPowTime(e AS big::Int, m AS big::Int) AS Integer
  MUT nonZero AS Integer = 0
  LET started AS Integer = datetime::monotonicNanos()
  FOR k = 1 TO 4
    IF NOT big::isZero(big::modPow(big::fromInteger(3), e, m)) THEN
      nonZero = nonZero + 1
    END IF
  NEXT
  LET elapsed AS Integer = datetime::monotonicNanos() - started
  IF nonZero <> 4 THEN
    RETURN -1
  END IF
  RETURN elapsed / 4
END FUNC

FUNC textTime(value AS big::Int) AS Integer
  LET started AS Integer = datetime::monotonicNanos()
  LET text AS String = big::toString(value)
  LET elapsed AS Integer = datetime::monotonicNanos() - started
  IF len(text) <> 5736 THEN
    RETURN -1
  END IF
  RETURN elapsed
END FUNC

FUNC divideTime(value AS big::Int) AS Integer
  LET ten AS big::Int = big::fromInteger(10)
  MUT bits AS Integer = 0
  LET started AS Integer = datetime::monotonicNanos()
  FOR k = 1 TO 100
    bits = bits + big::bitLength(big::divide(value, ten))
  NEXT
  LET elapsed AS Integer = datetime::monotonicNanos() - started
  IF bits = 0 THEN
    RETURN -1
  END IF
  RETURN elapsed / 100
END FUNC

FUNC parseTime(text AS String, expected AS big::Int) AS Integer
  LET started AS Integer = datetime::monotonicNanos()
  LET value AS big::Int = big::parse(text)
  LET elapsed AS Integer = datetime::monotonicNanos() - started
  IF NOT big::equals(value, expected) THEN
    RETURN -1
  END IF
  RETURN elapsed
END FUNC

FUNC scaleTime(value AS big::Int) AS Integer
  LET ten AS big::Int = big::fromInteger(10)
  MUT bits AS Integer = 0
  LET started AS Integer = datetime::monotonicNanos()
  FOR k = 1 TO 100
    bits = bits + big::bitLength(big::multiply(ten, value))
  NEXT
  LET elapsed AS Integer = datetime::monotonicNanos() - started
  IF bits = 0 THEN
    RETURN -1
  END IF
  RETURN elapsed / 100
END FUNC

FUNC factorTime(factor AS big::Int, value AS big::Int) AS Integer
  MUT bits AS Integer = 0
  LET started AS Integer = datetime::monotonicNanos()
  FOR k = 1 TO 100
    bits = bits + big::bitLength(big::multiply(factor, value))
  NEXT
  LET elapsed AS Integer = datetime::monotonicNanos() - started
  IF bits = 0 THEN
    RETURN -1
  END IF
  RETURN elapsed / 100
END FUNC

FUNC main() AS Nothing
  LET m AS big::Int = big::subtract(big::pow(big::fromInteger(2), 2048), big::fromInteger(1557))
  LET short AS big::Int = big::fromInteger(65537)
  LET long AS big::Int = big::fromInteger(8388609)
  LET factorial AS big::Int = big::factorial(2000)
  LET digits AS String = big::toString(factorial)
  LET wide AS big::Int = big::subtract(big::pow(big::fromInteger(3), 41000), big::fromInteger(1))
  LET byteFactor AS big::Int = big::fromInteger(251)
  LET wordFactor AS big::Int = big::fromInteger(9223372036854775783)
  MUT round AS Integer = 0
  WHILE round < 7
    io::print("modpow17 " & toString(modPowTime(short, m)))
    io::print("modpow24 " & toString(modPowTime(long, m)))
    io::print("text " & toString(textTime(factorial)))
    io::print("divide " & toString(divideTime(factorial)))
    io::print("parse " & toString(parseTime(digits, factorial)))
    io::print("scale " & toString(scaleTime(factorial)))
    io::print("byte " & toString(factorTime(byteFactor, wide)))
    io::print("word " & toString(factorTime(wordFactor, wide)))
    round = round + 1
  END WHILE
END FUNC
"#;

/// Build and run [`PROGRAM`] once; every test reads the medians of its rounds.
fn medians() -> &'static HashMap<String, u64> {
    static MEDIANS: OnceLock<HashMap<String, u64>> = OnceLock::new();
    MEDIANS.get_or_init(|| {
        let project = common::temp_project("big_int_cost", PROGRAM);
        let binary = common::build_project(&project);
        let (status, stdout) = common::run_bounded(
            &binary,
            Duration::from_secs(240),
            "the big::Int cost program did not finish",
        );
        let _ = std::fs::remove_dir_all(&project);
        assert!(
            status.success(),
            "{}:\n{stdout}",
            common::exit_description(&status)
        );
        let mut samples: HashMap<String, Vec<u64>> = HashMap::new();
        for line in stdout.lines() {
            let (label, nanos) = line.split_once(' ').expect("`<label> <nanoseconds>`");
            let nanos: i64 = nanos.trim().parse().expect("nanoseconds");
            // A -1 is the program's own check failing: the operation computed a wrong
            // value, so its time measures nothing.
            let nanos = u64::try_from(nanos)
                .unwrap_or_else(|_| panic!("{label} computed a wrong value:\n{stdout}"));
            samples.entry(label.to_string()).or_default().push(nanos);
        }
        samples
            .into_iter()
            .map(|(label, mut runs)| {
                assert_eq!(runs.len(), ROUNDS, "{label}: {runs:?}");
                runs.sort_unstable();
                (label, runs[runs.len() / 2])
            })
            .collect()
    })
}

/// `numerator / denominator` of the two medians, with both medians in the message.
fn ratio(numerator: &str, denominator: &str) -> (f64, String) {
    let medians = medians();
    let (top, bottom) = (medians[numerator], medians[denominator]);
    assert!(bottom > 0, "{denominator} median is 0 ns: {medians:?}");
    (
        top as f64 / bottom as f64,
        format!("{numerator} {top} ns, {denominator} {bottom} ns"),
    )
}

#[test]
fn mod_pow_cost_follows_bit_length() {
    let (ratio, detail) = ratio("modpow17", "modpow24");
    assert!(
        ratio <= 0.8,
        "modPow(3, 65537, m) took {ratio:.2}x modPow(3, 2^23 + 1, m) (limit 0.8): {detail}"
    );
}

#[test]
fn to_string_is_not_one_digit_per_pass() {
    let (ratio, detail) = ratio("text", "divide");
    assert!(
        ratio <= 800.0,
        "toString(2000!) took {ratio:.0}x one short division of it (limit 800): {detail}"
    );
}

#[test]
fn parse_is_not_one_digit_per_pass() {
    let (ratio, detail) = ratio("parse", "scale");
    assert!(
        ratio <= 250.0,
        "parse of 2000!'s 5,736 digits took {ratio:.0}x one multiplication of it by 10 \
         (limit 250): {detail}"
    );
}

#[test]
fn multiply_is_word_wide() {
    let (ratio, detail) = ratio("word", "byte");
    assert!(
        ratio <= 2.0,
        "an 8-byte multiplier took {ratio:.2}x a 1-byte one over an 8 KiB operand \
         (limit 2.0): {detail}"
    );
}
