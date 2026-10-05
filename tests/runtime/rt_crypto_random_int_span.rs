//! `crypto::randomInt(min AS Integer, max AS Integer)` keeps its page's range
//! contract (bug-814): a span whose `max - min` overflows an `Integer` raises
//! `ErrInvalidArgument`, and every other one is valid — the widest included.
//!
//! The body computed `max - min` and `span + 1` with the checked operators, so it
//! raised `ErrOverflow` (77050010), which the member does not declare, in both
//! places: instead of `ErrInvalidArgument` (77050002) for a span that overflows,
//! and instead of a value for a span of exactly `9223372036854775807` (`[0,
//! 9223372036854775807]`, `[-9223372036854775808, -1]`), whose count of outcomes,
//! 2^63, is the one an `Integer` cannot hold.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

#[test]
fn random_int_keeps_its_span_contract() {
    let source = r#"IMPORT io
IMPORT crypto

FUNC r(label AS String, lo AS Integer, hi AS Integer) AS Nothing
  LET v AS Integer = crypto::randomInt(lo, hi)
  IF v >= lo AND v <= hi THEN
    io::print(label & " in range")
  ELSE
    io::print(label & " OUT OF RANGE")
  END IF
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC main() AS Nothing
  r("full", -9223372036854775807 - 1, 9223372036854775807)
  r("min-to-zero", -9223372036854775807 - 1, 0)
  r("past-max", -5, 9223372036854775807)
  r("span-max", -1, 9223372036854775806)
  r("widest", 0, 9223372036854775807)
  r("widest-negative", -9223372036854775807 - 1, -1)
  r("reversed", 5, 1)
END FUNC
"#;
    let project = common::temp_project("rt_crypto_random_int_span", source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(60),
        "seven randomInt draws must finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        [
            "full raised 77050002",
            "min-to-zero raised 77050002",
            "past-max raised 77050002",
            "span-max in range",
            "widest in range",
            "widest-negative in range",
            "reversed raised 77050002",
        ],
        "\n{stdout}"
    );
}
