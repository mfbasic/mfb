//! A record a program builds directly, with fields that do not describe a valid
//! value, raises `ErrInvalidArgument` naming the bad field — never the
//! `ErrIndexOutOfRange` of the member's own internal list or string read.
//!
//! The package records are exported so a program can build one without going
//! through the package's validating constructor (`datetime::date`,
//! `csv::parseStream`). bug-698 found two members whose body indexed an internal
//! list with such a field: `datetime::format`'s month-name tokens read
//! `names[month - 1]`, and `csv::readRow` walked `chars` up to the record's own
//! `count` from its own `index`. Both leaked `ErrIndexOutOfRange` (77050001), an
//! error that says nothing about which input was wrong. The owner decision: the
//! member checks the fields it relies on and raises `ErrInvalidArgument`
//! (77050002) instead.
//!
//! Each probe also renders/reads a VALID record, so the check cannot be
//! satisfied by rejecting everything.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

/// Build and run `source`, returning its stdout lines.
fn run(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(60),
        "a handful of record probes must finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

fn assert_lines(actual: &[String], expected: &[String]) {
    let wrong: Vec<String> = expected
        .iter()
        .zip(
            actual
                .iter()
                .map(String::as_str)
                .chain(std::iter::repeat("<missing>")),
        )
        .filter(|(e, a)| e.as_str() != *a)
        .map(|(e, a)| format!("expected {e:?}, got {a:?}"))
        .collect();
    assert!(
        wrong.is_empty() && actual.len() == expected.len(),
        "wrong output ({} lines, expected {}):\n{}\nfull output:\n{}",
        actual.len(),
        expected.len(),
        wrong.join("\n"),
        actual.join("\n")
    );
}

/// `datetime::format`'s `MMM`/`MMMM` tokens on a `DateTime` whose `month` is
/// outside `1 .. 12`. The numeric month tokens (`M`, `MM`) render the field as
/// it is and are unaffected.
#[test]
fn datetime_format_month_name_of_an_out_of_range_month_is_invalid_argument() {
    let source = r#"IMPORT datetime
IMPORT io

FUNC show(label AS String, month AS Integer, pattern AS String) AS Nothing
  LET dt AS datetime::DateTime = datetime::DateTime[datetime::Date[2026, month, 1], datetime::Time[0, 0, 0, 0], datetime::utc(), 0]
  io::print(label & " " & datetime::format(dt, pattern))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code) & " " & e.message)
  RETURN
END TRAP
END FUNC

FUNC main() AS Nothing
  show("m13-MMM", 13, "MMM")
  show("m13-MMMM", 13, "MMMM")
  show("m0-MMM", 0, "MMM")
  show("mneg-MMMM", -1, "yyyy MMMM")
  show("m13-MM", 13, "MM")
  show("m1-MMM", 1, "MMM")
  show("m12-MMMM", 12, "MMMM")
END FUNC
"#;
    let bad = "raised 77050002 datetime: the month field of the date is outside 1 .. 12";
    let expected: Vec<String> = vec![
        format!("m13-MMM {bad}"),
        format!("m13-MMMM {bad}"),
        format!("m0-MMM {bad}"),
        format!("mneg-MMMM {bad}"),
        "m13-MM 13".to_string(),
        "m1-MMM Jan".to_string(),
        "m12-MMMM December".to_string(),
    ];
    assert_lines(&run("hand_built_datetime_month", source), &expected);
}
