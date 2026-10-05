//! bug-821: a newline inside an open `(`, `[` or `{` is not a statement
//! separator, so literals, constructors, calls and parameter lists span lines.
//! `mfb fmt` must keep those layouts — never join the lines or re-indent them to
//! the block level — rewrite a leading-comma layout (`⏎, "B"`) into the
//! trailing-comma form, and be idempotent.
//!
//! This drives the real `mfb fmt` over the `literal-spans-lines-valid` fixture
//! (every multi-line form, including leading-comma lists and maps), then proves:
//! the only edits are moved commas; no leading comma survives; a second run
//! changes nothing (`--check` passes); and the formatted program still builds
//! and prints the same verdicts.

use std::path::Path;
use std::process::Command;

#[path = "../common/mod.rs"]
mod common;
use common::*;

fn fixture_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rt-behavior/lexical/literal-spans-lines-valid/src/main.mfb");
    std::fs::read_to_string(&path).expect("read the literal-spans-lines-valid fixture")
}

/// The source with every comma and every whitespace character removed: equal
/// before and after formatting iff the formatter changed only comma placement
/// and whitespace, and in particular kept every token in order.
fn tokens_without_commas(source: &str) -> String {
    source
        .chars()
        .filter(|c| *c != ',' && !c.is_whitespace())
        .collect()
}

#[test]
fn fmt_keeps_multi_line_brackets_and_moves_leading_commas() {
    let source = fixture_source();
    assert!(
        source
            .lines()
            .any(|line| line.trim_start().starts_with(", ")),
        "the fixture must exercise a leading-comma layout"
    );
    let project = temp_project("fmt_multiline_brackets", &source);
    let file = project.join("src/main.mfb");

    let output = Command::new(mfb_exe())
        .arg("fmt")
        .arg(&project)
        .output()
        .expect("run mfb fmt");
    assert!(
        output.status.success(),
        "mfb fmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&file).expect("read back");

    // Layout kept: no physical line was joined or dropped (the fixture has no
    // comma-only line, the one kind the formatter removes)...
    assert_eq!(
        formatted.lines().count(),
        source.lines().count(),
        "mfb fmt joined or split lines:\n{formatted}"
    );
    // ...every multi-line bracket body keeps its own indentation (the owner's
    // column-0 continuation lines are not re-indented)...
    assert!(
        formatted.contains("\n\"AOA\"") || formatted.contains("\n\"ANG\""),
        "a column-0 continuation line was re-indented:\n{formatted}"
    );
    // ...only commas moved...
    assert_eq!(
        tokens_without_commas(&formatted),
        tokens_without_commas(&source)
    );
    // ...and every leading comma became a trailing one.
    assert!(
        !formatted
            .lines()
            .any(|line| line.trim_start().starts_with(',')),
        "a leading comma survived:\n{formatted}"
    );

    // Idempotent: formatting the output again changes nothing.
    let check = Command::new(mfb_exe())
        .arg("fmt")
        .arg("--check")
        .arg(&project)
        .output()
        .expect("run mfb fmt --check");
    assert!(
        check.status.success(),
        "a second mfb fmt would change the file:\n{}",
        String::from_utf8_lossy(&check.stdout)
    );

    // The formatted program still parses and computes the same values.
    let exe = build_project(&project);
    let run = Command::new(&exe).output().expect("run formatted program");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "formatted program failed:\n{stdout}");
    assert!(
        stdout.starts_with("count=180 first=AED last=ZWG\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("MISMATCH"), "{stdout}");
    assert_eq!(
        stdout.lines().filter(|l| l.ends_with(" ok")).count(),
        16,
        "{stdout}"
    );

    let _ = std::fs::remove_dir_all(&project);
}
