//! bug-831: `toFixed(String)` converts the decimal text exactly — the nearest
//! Q32.32 carrier, ties away from zero, then the range check — so it stores the
//! value `LET x AS Fixed = <same text>` stores. It used to parse through binary64
//! first, which misrounded near the top of the range and rejected in-range text
//! (`2147483647.9999999`) with `ErrOverflow`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// `(text read at run time, the same value written as a Fixed literal)`.
const EXACT: &[(&str, &str)] = &[
    ("2147483647.99", "2147483647.99"),
    ("2147483647.9999999", "2147483647.9999999"),
    (
        "2147483647.99999999976716935634613037109375",
        "2147483647.99999999976716935634613037109375",
    ),
    ("-2147483648", "-2147483648.0"),
    ("-2147483648.00000000000000000001", "-2147483648.0"),
    (
        "-2147483647.99999999976716935634613037109375",
        "-2147483647.99999999976716935634613037109375",
    ),
    // Exactly half a quantum (2^-33): ties round away from zero.
    (
        "0.000000000116415321826934814453125",
        "0.000000000116415321826934814453125",
    ),
    (
        "-0.000000000116415321826934814453125",
        "-0.000000000116415321826934814453125",
    ),
    ("0.000000000116415321826934814453124", "0.0"),
    ("0.1", "0.1"),
    ("1.25", "1.25"),
    (".5", "0.5"),
    ("5.", "5.0"),
    ("+3.75", "3.75"),
    ("-0", "0.0"),
    ("000123.4500", "123.45"),
    (
        "0.1234567890123456789012345678901234567890123",
        "0.1234567890123456789012345678901234567890123",
    ),
    // Exponent forms are exact too.
    ("2.5e2", "250.0"),
    ("1e-3", "0.001"),
    ("21474836.4799e2", "2147483647.99"),
    ("214748364799e-2", "2147483647.99"),
    ("1.5E+1", "15.0"),
    ("100e-2", "1.0"),
    ("0e99999", "0.0"),
    ("1e-99999", "0.0"),
    (
        "123456789012345678901234567890e-28",
        "12.3456789012345678901234567890",
    ),
];

/// `(text, error code)`.
const ERRORS: &[(&str, &str)] = &[
    ("2147483648", "77050010"),
    ("2147483647.9999999999", "77050010"),
    ("-2147483649", "77050010"),
    ("1e10", "77050010"),
    ("9999999999999999999999", "77050010"),
    ("1e99999", "77050010"),
    ("", "77050003"),
    ("abc", "77050003"),
    ("1.2.3", "77050003"),
    ("e5", "77050003"),
    ("1e", "77050003"),
    ("1e+", "77050003"),
    ("--1", "77050003"),
    ("-", "77050003"),
    (".", "77050003"),
    ("NaN", "77050003"),
    ("Infinity", "77050003"),
    (" 1", "77050003"),
    ("1 ", "77050003"),
    ("1e2.5", "77050003"),
];

fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

#[test]
fn to_fixed_string_matches_the_fixed_literal_and_the_declared_errors() {
    let mut body = String::new();
    for (i, (text, literal)) in EXACT.iter().enumerate() {
        body.push_str(&format!(
            "  LET l{i} AS Fixed = {literal}\n  io::print(\"{i} \" & tx({}) & \" \" & toString(l{i}, toByte(12)))\n",
            quoted(text)
        ));
    }
    for (i, (text, _)) in ERRORS.iter().enumerate() {
        body.push_str(&format!("  io::print(\"e{i} \" & tx({}))\n", quoted(text)));
    }
    let source = format!(
        "IMPORT io
IMPORT os

FUNC tx(text AS String) AS String
  LET s AS String = os::getEnvOr(\"MFB_UNSET_TOFIXED_ARG\", text)
  RETURN toString(toFixed(s), toByte(12))
  TRAP(e)
    RETURN \"err \" & toString(e.code)
  END TRAP
END FUNC

FUNC main() AS Integer
{body}  RETURN 0
END FUNC
"
    );
    let project = common::temp_project("tofixed_string_exact", &source);
    let exe = common::build_project(&project);
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    assert!(out.status.success(), "program failed");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = stdout.lines();
    for (i, (text, _)) in EXACT.iter().enumerate() {
        let line = lines.next().expect("a line per exact case");
        let rest = line
            .strip_prefix(&format!("{i} "))
            .unwrap_or_else(|| panic!("case {i} ({text}): {line}"));
        let (converted, literal) = rest.split_once(' ').expect("two values");
        assert_eq!(converted, literal, "toFixed({text:?}) vs the literal");
    }
    for (i, (text, code)) in ERRORS.iter().enumerate() {
        let line = lines.next().expect("a line per error case");
        assert_eq!(line, format!("e{i} err {code}"), "toFixed({text:?})");
    }
}
