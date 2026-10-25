//! bug-769: building a `Set` with `toSet` costs about one insert per element, and
//! `toSet` / `mapValues` allocate their result once.
//!
//! `toSet` grew its result from empty although `len(value)` bounds it — 9
//! allocations, 8 copies and 8 entropy-scrubbed frees for 300 elements — and every
//! freed byte went through a latency-bound single-chain PCG64 fill (~65% of an
//! insert). `mapValues` grew the same way to an exactly known size.
//!
//! `to_set_and_map_values_allocate_their_result_once`: under `--debug`, `R` more
//! calls add exactly `R` arena allocations.
//!
//! `to_set_costs_within_4x_a_probe_hit_add`: in-process timing, best of seven,
//! `toSet` per element at N = 300 against a same-binary control — an `add` of an
//! element already present (a probe hit, no write, no grow).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

fn run_program(name: &str, source: &str, debug: bool) -> (String, String) {
    let project = common::temp_project(name, source);
    let mut build = Command::new(common::mfb_exe());
    build.arg("build");
    if debug {
        build.arg("--debug");
    }
    let output = build.arg(&project).output().expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name}: build failed:\n{stdout}\n{}\n--- source ---\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with(common::host_executable_suffix()))
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
    (text, report)
}

const HELPERS: &str = "IMPORT io
IMPORT collections
IMPORT datetime

FUNC buildL(n AS Integer, dup AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, i / dup)
  NEXT
  RETURN xs
END FUNC

FUNC buildM(n AS Integer) AS Map OF Integer TO Integer
  MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {}
  FOR i = 0 TO n - 1
    m = collections::set(m, i * 7, i)
  NEXT
  RETURN m
END FUNC
";

/// `(name, setup, call)`: `call` adds one result's checksum to `acc`.
const ALLOC_CASES: &[(&str, &str, &str)] = &[
    (
        "to_set_distinct",
        "LET xs AS List OF Integer = buildL(300, 1)",
        "LET s AS Set OF Integer = collections::toSet(xs)
    acc = acc + len(s)",
    ),
    (
        "to_set_duplicates",
        "LET xs AS List OF Integer = buildL(300, 3)",
        "LET s AS Set OF Integer = collections::toSet(xs)
    acc = acc + len(s)",
    ),
    (
        "map_values",
        "LET m AS Map OF Integer TO Integer = buildM(300)",
        "LET r AS Map OF Integer TO Integer = collections::mapValues(m, LAMBDA(v AS Integer) -> v * 2)
    acc = acc + len(r) + collections::getOr(r, 7 * 299, 0)",
    ),
];

fn alloc_program(setup: &str, call: &str, reps: u32) -> String {
    format!(
        "{HELPERS}
FUNC main() AS Integer
  {setup}
  MUT acc AS Integer = 0
  FOR k = 1 TO {reps}
    {call}
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
"
    )
}

#[test]
fn to_set_and_map_values_allocate_their_result_once() {
    let count = |report: &str| -> u64 {
        report
            .lines()
            .find_map(|line| line.strip_prefix("arena.0.alloc_calls "))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("no arena.0.alloc_calls in\n{report}"))
    };
    let mut failures = Vec::new();
    for (name, setup, call) in ALLOC_CASES {
        let (text_a, a) = run_program(
            &format!("b769_{name}_10"),
            &alloc_program(setup, call, 10),
            true,
        );
        let (text_b, b) = run_program(
            &format!("b769_{name}_20"),
            &alloc_program(setup, call, 20),
            true,
        );
        let (sum_a, sum_b): (u64, u64) = (
            text_a.trim().parse().expect("a checksum"),
            text_b.trim().parse().expect("a checksum"),
        );
        if sum_b != 2 * sum_a {
            failures.push(format!(
                "{name}: checksums {sum_a} / {sum_b} are not per-call"
            ));
            continue;
        }
        let per_call = (count(&b) - count(&a)) as f64 / 10.0;
        if per_call > 1.0 {
            failures.push(format!(
                "{name}: {per_call} arena allocations per call (want 1: the result, once)"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-769 G1:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn to_set_costs_within_4x_a_probe_hit_add() {
    let source = format!(
        "{HELPERS}
FUNC viaToSet(xs AS List OF Integer, reps AS Integer) AS Integer
  MUT acc AS Integer = 0
  FOR k = 0 TO reps - 1
    acc = acc + len(collections::toSet(xs))
  NEXT
  RETURN acc
END FUNC

FUNC viaHits(xs AS List OF Integer, reps AS Integer) AS Integer
  MUT s AS Set OF Integer = collections::toSet(xs)
  MUT acc AS Integer = 0
  FOR k = 0 TO reps - 1
    FOR EACH x IN xs
      s = collections::add(s, x)
    NEXT
    acc = acc + len(s)
  NEXT
  RETURN acc
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Integer = buildL(300, 1)
  MUT bestToSet AS Integer = 9223372036854775807
  MUT bestHits AS Integer = 9223372036854775807
  MUT a AS Integer = 0
  MUT b AS Integer = 0
  FOR r = 0 TO 6
    LET t0 AS Integer = datetime::monotonicNanos()
    a = viaToSet(xs, 1000)
    LET t1 AS Integer = datetime::monotonicNanos()
    b = viaHits(xs, 1000)
    LET t2 AS Integer = datetime::monotonicNanos()
    IF t1 - t0 < bestToSet THEN
      bestToSet = t1 - t0
    END IF
    IF t2 - t1 < bestHits THEN
      bestHits = t2 - t1
    END IF
  NEXT
  io::print(toString(a) & \" \" & toString(b))
  io::print(toString(bestToSet) & \" \" & toString(bestHits))
  RETURN 0
END FUNC
"
    );
    let (text, _) = run_program("b769_to_set_cost", &source, false);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("300000 300000"), "checksums:\n{text}");
    let times: Vec<f64> = lines
        .next()
        .expect("a timing line")
        .split_whitespace()
        .map(|t| t.parse().expect("ns"))
        .collect();
    let ratio = times[0] / times[1];
    eprintln!(
        "toSet {:.1} ns/elt, probe-hit add {:.1} ns/elt, ×{ratio:.2}",
        times[0] / 300_000.0,
        times[1] / 300_000.0
    );
    assert!(
        ratio <= 4.0,
        "bug-769: toSet of 300 Integers cost ×{ratio:.2} a probe-hit add per element (bound ×4)"
    );
}
