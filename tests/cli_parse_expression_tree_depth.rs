//! bug-501 (audit-3 FE-01): the expression grammar's depth guard bounded the
//! parser's OWN recursion, but a left-associative chain (`1+1+1+…`), a postfix
//! member chain (`a.b.c…`), nested groups each carrying a short chain, and a
//! `|>` pipeline all build a deep tree WITHOUT recursing in the parser — and
//! every pass after it walks that tree recursively, so a 40 KB `1+1+…` source
//! aborted the compiler with a native stack overflow (SIGABRT) instead of a
//! diagnostic. Separately (bug-501 B), a `|>` right-hand side with several `_`
//! COPIED the left operand once per placeholder, doubling the tree per stage:
//! sixteen stages of a 200-byte source cost 720 MB and 100 s.
//!
//! Drives the real `mfb` binary with each hostile shape and asserts a clean
//! `exit 1` carrying the located diagnostic — never a signal — and that a chain
//! exactly at the cap still compiles: the guard admits exactly what `ir::verify`
//! admitted before it, so no program that built before stops building.

use std::process::{Command, Output};

mod common;
use common::*;

/// A `main` whose one binding is initialised by `expression`.
fn program(expression: &str) -> String {
    format!("FUNC main() AS Integer\n  LET x AS Integer = {expression}\n  RETURN 0\nEND FUNC\n")
}

fn build(name: &str, source: &str) -> Output {
    let project = temp_project(name, source);
    Command::new(mfb_exe())
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build")
}

fn assert_clean_rejection(output: &Output, expected_detail: &str, shape: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{shape}: mfb must exit 1 with a diagnostic, not die by signal.\n\
         status: {}\nstderr:\n{stderr}",
        output.status
    );
    assert!(
        stderr.contains("MFB_PARSE_UNEXPECTED_TOKEN") && stderr.contains(expected_detail),
        "{shape}: expected the located `{expected_detail}` diagnostic, got:\n{stderr}"
    );
}

const TOO_DEEP: &str = "Expression nesting is too deep.";

#[test]
fn operator_chain_of_20000_terms_is_rejected_cleanly() {
    // The FE-01 spike (`spikes/audit-3/FE-01/gen.py`): a flat 40 KB `1+1+…`.
    let source = program(&format!("1{}", "+1".repeat(20_000)));
    let output = build("bug501_operator_chain", &source);
    assert_clean_rejection(&output, TOO_DEEP, "20000-term `+` chain");
}

#[test]
fn nested_groups_of_short_chains_are_rejected_cleanly() {
    // Every group nests 250 deep and every chain is 20 long — each well under
    // the 256 cap on its own — but the BUILT tree's left spine is 5 000 deep.
    // Charging chain length alone cannot see this; tracking tree depth can.
    let mut expression = String::from("1");
    for _ in 0..250 {
        expression = format!("({expression}{})", "+1".repeat(20));
    }
    let output = build("bug501_nested_groups", &program(&expression));
    assert_clean_rejection(&output, TOO_DEEP, "250 groups of 20-term chains");
}

#[test]
fn member_chain_is_rejected_cleanly() {
    let source = program(&format!("a{}", ".b".repeat(20_000)));
    let output = build("bug501_member_chain", &source);
    assert_clean_rejection(&output, TOO_DEEP, "20000-member `.` chain");
}

#[test]
fn pipeline_placeholder_copies_are_refused() {
    // Two placeholders per stage double the tree: 20 stages would be a
    // million-node expression. The copy is refused before it is made, at the
    // stage whose copy would exceed the budget.
    let source = format!(
        "FUNC f(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n{}",
        program(&format!("1{}", " |> f(_, _)".repeat(20)))
    );
    let output = build("bug501_pipeline_copies", &source);
    assert_clean_rejection(
        &output,
        "Pipeline placeholder substitution is too large",
        "20 stages of `|> f(_, _)`",
    );
}

#[test]
fn chain_at_the_cap_still_compiles() {
    // 256 operators is the deepest chain `ir::verify` accepted before the parser
    // guard existed (root at depth 0, rejected past 256). The parser must agree
    // exactly, or the guard would be a language-surface change.
    let source = program(&format!("1{}", "+1".repeat(256)));
    let project = temp_project("bug501_chain_at_cap", &source);
    build_project(&project);
}
