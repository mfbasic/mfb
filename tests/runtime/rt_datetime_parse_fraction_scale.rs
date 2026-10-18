//! bug-827: a `datetime::parse` `f` run scales the fraction by the number of
//! digits actually READ (owner decision 2026-10-05), as `parseIso` does:
//! `.12` under `fff` is 120 ms and `.1` under nine `f`s is 100 ms. It used to
//! scale by the run length, so `.12` under `fff` was 12 ms. A fraction with more
//! than nine digits keeps its first nine, as `parseIso` does.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// `(text, pattern, nanoseconds rendered by toIso(.., 9))`.
const CASES: &[(&str, &str, &str)] = &[
    ("10:00:00.12", "HH:mm:ss.fff", "10:00:00.120000000Z"),
    ("10:00:00.1", "HH:mm:ss.fffffffff", "10:00:00.100000000Z"),
    ("10:00:00.120", "HH:mm:ss.fff", "10:00:00.120000000Z"),
    ("10:00:00.5", "HH:mm:ss.f", "10:00:00.500000000Z"),
    ("10:00:00.05", "HH:mm:ss.ffff", "10:00:00.050000000Z"),
    (
        "10:00:00.123456789",
        "HH:mm:ss.fffffffff",
        "10:00:00.123456789Z",
    ),
    (
        "10:00:00.12345",
        "HH:mm:ss.fffffffff",
        "10:00:00.123450000Z",
    ),
    // Longer than nine digits: the first nine are kept.
    (
        "10:00:00.0123456789",
        "HH:mm:ss.ffffffffff",
        "10:00:00.012345678Z",
    ),
    (
        "10:00:00.1234567891",
        "HH:mm:ss.ffffffffff",
        "10:00:00.123456789Z",
    ),
];

#[test]
fn fraction_is_scaled_by_the_digits_read() {
    let mut body = String::new();
    for (text, pattern, _) in CASES {
        body.push_str(&format!(
            "  io::print(datetime::toIso(datetime::parse(\"{text}\", \"{pattern}\"), 9))\n"
        ));
    }
    body.push_str(
        "  io::print(datetime::toIso(datetime::parseIso(\"1970-01-01T10:00:00.12Z\"), 9))\n",
    );
    let source = format!(
        "IMPORT io
IMPORT datetime

FUNC main() AS Nothing
{body}END FUNC
"
    );
    let project = common::temp_project("datetime_parse_fraction_scale", &source);
    let exe = common::build_project(&project);
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    assert!(out.status.success(), "program failed");
    let mut expected = String::new();
    for (_, _, rendered) in CASES {
        expected.push_str(&format!("1970-01-01T{rendered}\n"));
    }
    expected.push_str("1970-01-01T10:00:00.120000000Z\n");
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}
