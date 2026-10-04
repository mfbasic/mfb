//! bug-772: `strings::toScalars` / `strings::fromScalars` make exactly one arena
//! allocation per call (their result) and free nothing per scalar.
//!
//! They were MFBASIC bodies three layers deep over `encoding::utf32Encode` /
//! `utf32Decode`: ~1 allocation per scalar for `toScalars` (an inline-`TRAP`
//! `Result` block each) and ~2 for `fromScalars` (a one-byte list and a one-scalar
//! `String` each), plus two lists' geometric regrows. Measured with the `--debug`
//! arena counters over a loop of N calls of one operation: the allocation count
//! must be exactly N above the fixed program overhead, at every string length.
//! Values are checked too: ASCII, 2-, 3- and 4-byte scalars, and the empty string.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

fn source(op: &str) -> String {
    format!(
        r#"IMPORT io
IMPORT strings
IMPORT os

FUNC main() AS Nothing
  LET n AS Integer = toInt(os::getEnv("N"))
  LET text AS String = os::getEnv("TEXT")
  LET scalars AS List OF Scalar = strings::toScalars(text)
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < n
    {op}
    i = i + 1
  END WHILE
  io::print(toString(total))
  io::print(strings::fromScalars(scalars))
  io::print(toString(len(scalars)))
END FUNC
"#
    )
}

fn counter(stats: &str, key: &str) -> u64 {
    stats
        .lines()
        .find_map(|line| line.strip_prefix(&format!("arena.0.{key} ")))
        .unwrap_or_else(|| panic!("no arena.0.{key} in:\n{stats}"))
        .trim()
        .parse()
        .expect("numeric counter")
}

/// Runs the probe for `op` with `text` at N and 2N and returns the allocations
/// per call, checking the round trip on the way.
fn allocations_per_call(name: &str, op: &str, text: &str) -> u64 {
    let project = common::temp_project(name, &source(op));
    let built = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    assert!(
        built.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let executable = project.join("build").join(format!(
        "{name}{}",
        if cfg!(windows) { ".exe" } else { ".out" }
    ));
    let run = |n: u64| {
        let output = Command::new(&executable)
            .env("N", n.to_string())
            .env("TEXT", text)
            .output()
            .expect("run the probe");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stats = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "probe failed:\n{stats}");
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines[1], text, "fromScalars(toScalars(s)) = s");
        assert_eq!(
            lines[2],
            text.chars().count().to_string(),
            "one Scalar per Unicode scalar value"
        );
        counter(&stats, "alloc_calls")
    };
    let small = run(200);
    let large = run(400);
    (large - small) / 200
}

const TEXTS: [&str; 3] = [
    "The Quick Brown Fox Jumps Over The Lazy Dog",
    "a\u{e9}\u{4e2d}\u{1f600}z \u{10ffff}\u{80}",
    "",
];

#[test]
fn to_scalars_allocates_once_per_call() {
    for (index, text) in TEXTS.iter().enumerate() {
        let per_call = allocations_per_call(
            &format!("bug772_to_{index}"),
            "LET l AS List OF Scalar = strings::toScalars(text)\n    total = total + len(l)",
            text,
        );
        assert_eq!(per_call, 1, "toScalars of {text:?}");
    }
}

#[test]
fn from_scalars_allocates_once_per_call() {
    for (index, text) in TEXTS.iter().enumerate() {
        let per_call = allocations_per_call(
            &format!("bug772_from_{index}"),
            "LET s AS String = strings::fromScalars(scalars)\n    total = total + len(s)",
            text,
        );
        assert_eq!(per_call, 1, "fromScalars of {text:?}");
    }
}
