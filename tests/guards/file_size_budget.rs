//! Guard: the file-size budget (plan-176).
//!
//! Counted in non-comment, non-blank lines of `.rs` and `.mfb` files under
//! `src/`, `packages/`, `examples/`, `repository/` and `tests/`:
//!
//! - a code file holds at most 1000 code lines (`code`) and 250 inline test
//!   lines (`inline`: `#[cfg(test)]` items, MFBASIC `TESTING … END TESTING`);
//! - a test-only file holds at most 1000 lines (`test`).
//!
//! Over 1000 code lines → split the file into modules. Over 250 inline test
//! lines → move the tests into their own test-only file. A test-only file over
//! 1000 → split it. Generated files (`GENERATED` in a comment on one of the
//! first three lines) are exempt; nothing else is.
//!
//! `tests/guards/file_size_ceilings.txt` lists the files still over a limit,
//! each at its current count. This test NEVER fails: it writes a banner to
//! stderr naming every file over a limit without a ceiling (NEW), past its
//! ceiling (GREW), every ceiling that can be lowered or removed (TIGHTEN), and
//! every ceiling row that names no in-scope file or does not parse (STALE).
//! `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` lowers ceilings and
//! drops the ones no longer needed; it never adds or raises one.
//!
//! The counting rules are implemented twice — here and in
//! `tools/loc-census/census.py` — and the two must agree. How to split a file:
//! `.ai/build-tooling.md` § "Splitting an over-budget file (plan-176)".

use std::io::Write;
use std::path::{Path, PathBuf};

#[path = "file_size_budget/ceilings.rs"]
mod ceilings;
#[path = "file_size_budget/classify.rs"]
mod classify;
#[path = "file_size_budget/lex_cases.rs"]
mod lex_cases;
#[path = "file_size_budget/mfb_lex.rs"]
mod mfb_lex;
#[path = "file_size_budget/rust_lex.rs"]
mod rust_lex;

use ceilings::Row;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ceiling_path(root: &Path) -> PathBuf {
    root.join("tests/guards/file_size_ceilings.txt")
}

/// One `##`-framed banner line, padded to 80 columns.
fn framed(text: &str) -> String {
    let width = 80usize.saturating_sub(4);
    let pad = width.saturating_sub(text.chars().count() + 2);
    format!("##  {text}{}##\n", " ".repeat(pad))
}

/// The banner for `rows`, or nothing when there are none.
fn banner(rows: &[Row]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let count = |f: fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
    let new = count(|r| matches!(r, Row::New { .. }));
    let grew = count(|r| matches!(r, Row::Grew { .. }));
    let stale = count(|r| matches!(r, Row::Stale { .. }));
    let tighten = count(|r| matches!(r, Row::Tighten { .. }));
    let rule = "#".repeat(80) + "\n";
    let mut out = String::new();
    out.push_str(&rule);
    out.push_str(&framed(&format!(
        "FILE SIZE BUDGET (plan-176): {new} over, {grew} grew, {stale} stale, {tighten} can tighten"
    )));
    out.push_str(&framed(
        "code ≤ 1000, inline tests ≤ 250, test-only file ≤ 1000 non-comment LOC.",
    ));
    out.push_str(&framed(
        "Split into a new module; move inline tests out to a test-only file.",
    ));
    out.push_str(&rule);
    for row in rows {
        out.push_str(&row.render());
        out.push('\n');
    }
    if tighten > 0 || stale > 0 {
        out.push_str(
            "  run `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` to lower ceilings.\n",
        );
    }
    out.push_str(&rule);
    out
}

#[test]
fn file_size_budget() {
    let root = repo_root();
    let counts = classify::census(&root);
    let path = ceiling_path(&root);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if std::env::var("MFB_LOC_TIGHTEN").as_deref() == Ok("1") && path.exists() {
        let lowered = ceilings::tighten(&ceilings::parse(&text), &counts);
        if lowered != text {
            if let Err(e) = std::fs::write(&path, &lowered) {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "file_size_budget: cannot write {}: {e}",
                    path.display()
                );
            }
            text = lowered;
        }
    }
    let rows = ceilings::compare(&ceilings::parse(&text), &counts);
    // Raw stderr, not `eprintln!`: libtest captures `eprintln!` output of a
    // passing test, and this banner exists to be seen.
    let _ = std::io::stderr().lock().write_all(banner(&rows).as_bytes());
}
