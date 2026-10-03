//! bug-773: what a `regex::*` call allocates — per compiled pattern literal, per
//! character class, per repeated call of one pattern, and per start position a
//! first-scalar set can reject.
//!
//! Every assertion counts `--debug` arena allocations (`arena.0.alloc_calls`),
//! never time, and compares two programs that differ in exactly one dimension:
//!
//! * `a_compiled_pattern_literal_costs_at_most_10_allocations`: every call gets a
//!   DISTINCT pattern (so no compile cache can help), 10 literals longer in the
//!   second program. Before bug-773 each literal cost ~29 allocations: every
//!   parse-node copy or drop ran the graph walker, which allocated a 1,552-byte
//!   work stack even with nothing to push.
//! * `a_character_class_costs_at_most_50_allocations_to_compile`: the same, one
//!   `[0-9]` class against one literal. Before: ~815 — the ASCII bitset built a
//!   String per code point.
//! * `a_repeated_pattern_compiles_once`: the same `findAll` K and 2K times. Before:
//!   every call recompiled (~888 allocations for `[0-9]+`).
//! * `a_rejected_start_position_allocates_nothing`: a no-match alternation over a
//!   subject of N and 2N `z`s. Before: 20 allocations per position — nothing
//!   filtered start positions, and each ran fresh engine state.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

fn alloc_calls(name: &str, source: &str) -> (String, u64) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name}: build failed:\n{stdout}\n{}\n--- source ---\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with("-glibc.out"))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{name}: program failed:\n{text}\n{report}"
    );
    let count = report
        .lines()
        .find_map(|line| line.strip_prefix("arena.0.alloc_calls "))
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("{name}: no arena.0.alloc_calls in\n{report}"));
    (text, count)
}

/// `calls` calls of `regex::match(subject, prefix & toString(10000 + k))` — a
/// distinct pattern per call.
fn distinct_patterns(prefix: &str, subject: &str, calls: u32) -> String {
    format!(
        "IMPORT io
IMPORT regex

FUNC main() AS Integer
  MUT hits AS Integer = 0
  FOR k = 1 TO {calls}
    IF regex::match(\"{subject}\", \"{prefix}\" & toString(10000 + k)) THEN
      hits = hits + 1
    END IF
  NEXT
  io::print(toString(hits))
  RETURN 0
END FUNC
"
    )
}

#[test]
fn a_compiled_pattern_literal_costs_at_most_10_allocations() {
    const CALLS: u32 = 20;
    let (short_text, short) = alloc_calls("b773_lit_short", &distinct_patterns("a", "", CALLS));
    let (long_text, long) = alloc_calls(
        "b773_lit_long",
        &distinct_patterns("abcdefghijk", "", CALLS),
    );
    assert_eq!((short_text.as_str(), long_text.as_str()), ("0\n", "0\n"));
    let per_literal = (long - short) as f64 / f64::from(CALLS * 10);
    assert!(
        per_literal <= 10.0,
        "bug-773: each compiled pattern literal cost {per_literal:.1} arena allocations \
         (bound 10) — a parse-node copy or drop allocating the walker's work stack"
    );
}

#[test]
fn a_character_class_costs_at_most_50_allocations_to_compile() {
    const CALLS: u32 = 20;
    let (lit_text, lit) = alloc_calls("b773_cls_lit", &distinct_patterns("a", "", CALLS));
    let (cls_text, cls) = alloc_calls("b773_cls_cls", &distinct_patterns("[0-9]", "", CALLS));
    assert_eq!((lit_text.as_str(), cls_text.as_str()), ("0\n", "0\n"));
    let per_class = (cls as f64 - lit as f64) / f64::from(CALLS);
    assert!(
        per_class <= 50.0,
        "bug-773: compiling one `[0-9]` class cost {per_class:.1} more arena allocations than \
         one literal (bound 50) — the ASCII bitset building a String per code point"
    );
}

#[test]
fn a_repeated_pattern_compiles_once() {
    let program = |calls: u32| {
        format!(
            "IMPORT io
IMPORT regex

FUNC main() AS Integer
  MUT total AS Integer = 0
  FOR k = 1 TO {calls}
    total = total + len(regex::findAll(\"\", \"[0-9]+\"))
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
"
        )
    };
    let (text_a, a) = alloc_calls("b773_rep_20", &program(20));
    let (text_b, b) = alloc_calls("b773_rep_40", &program(40));
    assert_eq!((text_a.as_str(), text_b.as_str()), ("0\n", "0\n"));
    // A call that does not compile still allocates its own state: the cached
    // program's copy, the result list, the match context and the matcher tables —
    // 14 measured. A compile adds ~100 more (`[0-9]+`: 110 per call with no cache,
    // 914 before bug-773's compile fixes), so 25 separates the two.
    let per_call = (b - a) as f64 / 20.0;
    assert!(
        per_call <= 25.0,
        "bug-773: repeating `regex::findAll(\"\", \"[0-9]+\")` cost {per_call:.1} arena \
         allocations per call (bound 25): every call recompiles the pattern"
    );
}

#[test]
fn a_rejected_start_position_allocates_nothing() {
    let program = |n: u32| {
        format!(
            "IMPORT io
IMPORT regex
IMPORT strings

FUNC main() AS Integer
  LET subject AS String = strings::repeat(\"z\", {n})
  io::print(toString(len(regex::findAll(subject, \"cat|dog|bird|fish|owl\"))))
  RETURN 0
END FUNC
"
        )
    };
    let (text_a, a) = alloc_calls("b773_pos_500", &program(500));
    let (text_b, b) = alloc_calls("b773_pos_1000", &program(1_000));
    assert_eq!((text_a.as_str(), text_b.as_str()), ("0\n", "0\n"));
    let per_position = (b as f64 - a as f64) / 500.0;
    assert!(
        per_position <= 1.0,
        "bug-773: a start position `cat|dog|bird|fish|owl` cannot begin (a `z`) cost \
         {per_position:.2} arena allocations (bound 1): no first-scalar filter, and fresh \
         engine state per position"
    );
}
