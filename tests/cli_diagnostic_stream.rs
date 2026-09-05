//! bug-505 (audit-3 FE-03) and FE-04: the diagnostic renderer printed every
//! located diagnostic with a three-line source echo, re-reading the whole
//! source file for each one, so a source provoking one error per line cost
//! O(errors × filesize) and turned 240 KB of input into 10 GB of stderr — and
//! the echoed line was written raw, so an ESC/CSI sequence in a hostile source
//! line recolored or erased the developer's terminal.
//!
//! Drives the real `mfb` binary: past `rules::MAX_RENDERED_DIAGNOSTICS` the
//! rest are counted (`... and N more`), so stderr stays bounded by the cap and
//! not by the input; a small erroneous program still renders every diagnostic;
//! and the echoed line has its terminal-unsafe bytes escaped as `\u{XXXX}`.

use std::process::{Command, Output};

mod common;
use common::*;

/// A `main` with `errors` bindings whose initializer type mismatches — one
/// located `TYPE_BINDING_MISMATCH` per line.
fn erroneous_program(errors: usize) -> String {
    let mut source = String::from("FUNC main() AS Integer\n");
    for index in 0..errors {
        source.push_str(&format!("  LET v{index} AS Integer = \"s\"\n"));
    }
    source.push_str("  RETURN 0\nEND FUNC\n");
    source
}

fn build(name: &str, source: &str) -> Output {
    let project = temp_project(name, source);
    Command::new(mfb_exe())
        .args(["build", "-ast", "-ir"])
        .arg(&project)
        .output()
        .expect("run mfb build")
}

fn count_headers(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|line| line.contains(" error["))
        .count()
}

#[test]
fn diagnostic_stream_is_capped_and_bounded_on_a_hostile_source() {
    // ~250 KB of source, one error per line: pre-fix this was every one of the
    // 8 000 diagnostics rendered with three echoed lines each.
    let errors = 8_000;
    let source = erroneous_program(errors);
    assert!(source.len() > 200_000, "source is {} bytes", source.len());
    let output = build("bug505_hostile_stream", &source);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "status: {}", output.status);
    assert_eq!(
        count_headers(&stderr),
        100,
        "exactly the first 100 diagnostics render; got:\n{}",
        stderr.lines().take(8).collect::<Vec<_>>().join("\n")
    );
    assert!(
        stderr.contains(&format!(
            "... and {} more diagnostics not shown",
            errors - 100
        )),
        "the withheld count must be reported once, got tail:\n{}",
        stderr.lines().rev().take(4).collect::<Vec<_>>().join("\n")
    );
    // Bounded by the cap, not by the input: 100 renderings of ~5 short lines.
    assert!(
        stderr.len() < 64 * 1024,
        "stderr is {} bytes for a {} byte source",
        stderr.len(),
        source.len()
    );
}

#[test]
fn every_diagnostic_of_a_small_program_still_renders() {
    // Well under the cap: nothing is dropped and no "more" line appears.
    let output = build("bug505_small_stream", &erroneous_program(22));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "status: {}", output.status);
    assert_eq!(count_headers(&stderr), 22, "{stderr}");
    assert!(!stderr.contains("more diagnostics not shown"), "{stderr}");
    assert!(!stderr.contains("more diagnostic not shown"), "{stderr}");
}

#[test]
fn one_diagnostic_past_the_cap_is_reported_in_the_singular() {
    let output = build("bug505_cap_plus_one", &erroneous_program(101));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(count_headers(&stderr), 100, "{stderr}");
    assert!(
        stderr.contains("... and 1 more diagnostic not shown"),
        "{stderr}"
    );
}

#[test]
fn echoed_source_line_is_terminal_safe() {
    // FE-04: the offending line carries an ANSI colour sequence, a BEL, a bare
    // CR and a bidi override. The echo must escape every one of them and
    // contain none of the raw bytes.
    let source = "FUNC main() AS Integer\n  LET bad AS Integer = \"\u{1b}[31mRED\u{1b}[0m\u{7}\r\u{202e}\"\n  RETURN 0\nEND FUNC\n";
    let output = build("bug505_terminal_safe_echo", source);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "status: {}", output.status);
    let echoed = stderr
        .lines()
        .find(|line| line.contains("LET bad AS Integer"))
        .unwrap_or_else(|| panic!("the offending line must be echoed:\n{stderr}"));
    assert!(
        !output.stderr.contains(&b'\x1b') && !output.stderr.contains(&b'\x07'),
        "raw ESC/BEL reached stderr:\n{stderr:?}"
    );
    assert!(
        !echoed.contains('\u{202e}') && !echoed.contains('\r'),
        "raw CR/RLO reached the echo: {echoed:?}"
    );
    for escaped in ["\\u{001b}[31mRED", "\\u{0007}", "\\u{000d}", "\\u{202e}"] {
        assert!(echoed.contains(escaped), "missing {escaped:?} in: {echoed}");
    }
}
