//! bug-700: a `--debug` build's exit report must not take tens of seconds once a
//! program allocates a few hundred thousand times. `perf.rs:emit_write_stats`
//! insertion-sorts a name's collected durations to find the median — O(n²) —
//! which dominates the report-writing time far past the point where the
//! program's own work is finished (the process sits at 100% CPU on one thread
//! between the report's header and its statistics).
//!
//! The `perf` section is macOS-only (plan-67-B), so both tests here are
//! macOS-only; off macOS the report carries no `perf.` lines at all
//! (`tests/runtime/rt_debug_report.rs`'s `assert_block`).
//!
//! Phase 1 root-cause note (see `bugs/bug-700-*.md` for the full record): the
//! reproduction's two names (`mfb_alloc`, `mfb_free`) both saturate the shared
//! sample-log cap at the same count for a 175000 and a 350000-iteration run, so
//! hypothesis 1 (the O(n²) sort, over identical `n` in both rows) predicts near
//! -equal report times for those two rows. Hypothesis 2 (extra cost tracking
//! `perf.overflow`) does not hold up under inspection: `perf.end`'s overflow
//! branch is three O(1) instructions (load counter, increment, store) with no
//! scan or sort, so it cannot cost seconds regardless of how many samples
//! overflow. A `sample`(1) profile of the reproduction at 350000 iterations
//! shows 100% of captured time inside a single ~300-byte hot loop matching the
//! insertion sort's body. The documented "last two rows don't fit a sort-only
//! explanation" reading is measurement noise from a shared/contended machine:
//! re-measured on a machine busy with sibling builds, the 175000 and 350000
//! rows' wall times came out in the OPPOSITE order from the bug doc's table
//! (12.62s vs 10.36s here). The fix is the O(n²) insertion sort; no separate
//! overflow-path fix is needed.

#![cfg(target_os = "macos")]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;
use std::time::{Duration, Instant};

/// Build `source` as project `name` with `--debug` and return the executable.
fn build_debug(name: &str, source: &str) -> std::path::PathBuf {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    assert!(
        output.status.success(),
        "{name}: mfb build --debug failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    std::path::PathBuf::from(path)
}

/// The value of `perf.<name>.<stat>` from a `--debug` run's stderr, as a `u64`.
fn perf_value(case: &str, stderr: &str, key: &str) -> u64 {
    let prefix = format!("{key} ");
    stderr
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("{case}: no `{key} <n>` line in report:\n{stderr}"))
        .parse()
        .unwrap_or_else(|err| panic!("{case}: `{key}` value did not parse as u64: {err}"))
}

/// The single correct behavior a fix produces (bug-700's Goal): the report for a
/// few-hundred-thousand-sample run finishes in well under a second once the sort
/// is O(n log n), so a generous multi-second bound is still a meaningful RED gate
/// today (pre-fix: ~4.5 s measured for this loop count in the bug doc's table;
/// this run reproduced 10-12 s under machine load) and leaves wide margin for a
/// slow/contended CI machine after the fix. Loop count is 100000 — the doc's own
/// Phase 1 example, chosen to stay below the sample-log capacity (698000) so no
/// sample is dropped and the sort sees the full O(n²) shape.
#[test]
fn report_exit_time_is_not_quadratic_in_sample_count() {
    let source = concat!(
        "IMPORT io\n",
        "\n",
        "FUNC main AS Integer\n",
        "  MUT total AS Integer = 0\n",
        "  FOR i = 1 TO 100000\n",
        "    LET s AS String = \"n\" & toString(i)\n",
        "    total = total + len(s)\n",
        "  NEXT\n",
        "  io::print(toString(total))\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let exe = build_debug("dbg_exit_time", source);
    let start = Instant::now();
    let output = Command::new(&exe).output().expect("run built program");
    let elapsed = start.elapsed();
    assert!(
        output.status.success(),
        "dbg_exit_time: program failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("mfb.debug.end 1"),
        "dbg_exit_time: report did not complete:\n{stderr}"
    );
    let count = perf_value("dbg_exit_time", &stderr, "perf.mfb_alloc.count");
    assert!(
        count > 100_000,
        "dbg_exit_time: expected a six-figure sample count below the log cap, got {count}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "dbg_exit_time: report took {elapsed:?} for {count} samples — \
         the O(n\u{b2}) insertion sort in emit_write_stats must not dominate \
         exit time (bug-700)"
    );
}

/// Exactness guard for Phase 2 (bug-700 Goal: "the median printed is unchanged,
/// exactly, for every name"). For exactly 3 samples, the median (the middle
/// value once sorted) equals `sum - min - max` for ANY multiset of size 3,
/// regardless of duplicates — one value is the min, one is the max, and by
/// elimination the third is the median, computed here from the report's own
/// trusted `sum`/`min`/`max` lines without needing to control (or read) the
/// raw per-sample durations. Three distinct `LET ... AS String = a & b`
/// concatenations force exactly 3 `mfb_alloc`/`mfb_free` spans, deterministic by
/// construction (compiled shape), independent of actual timing values.
#[test]
fn median_is_exact_for_a_three_sample_name() {
    let source = concat!(
        "IMPORT io\n",
        "\n",
        "FUNC main AS Integer\n",
        "  LET a AS String = \"a\" & \"b\"\n",
        "  LET b AS String = \"c\" & \"d\"\n",
        "  LET c AS String = \"e\" & \"f\"\n",
        "  io::print(a & b & c)\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let exe = build_debug("dbg_exit_median3", source);
    let output = Command::new(&exe).output().expect("run built program");
    assert!(
        output.status.success(),
        "dbg_exit_median3: program failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    for prefix in ["perf.mfb_alloc", "perf.mfb_free"] {
        let count = perf_value("dbg_exit_median3", &stderr, &format!("{prefix}.count"));
        assert_eq!(
            count, 3,
            "dbg_exit_median3: expected exactly 3 {prefix} samples"
        );
        let min = perf_value("dbg_exit_median3", &stderr, &format!("{prefix}.min"));
        let max = perf_value("dbg_exit_median3", &stderr, &format!("{prefix}.max"));
        let sum = perf_value("dbg_exit_median3", &stderr, &format!("{prefix}.sum"));
        let median = perf_value("dbg_exit_median3", &stderr, &format!("{prefix}.median"));
        assert_eq!(
            median,
            sum - min - max,
            "{prefix}: median {median} != sum-min-max ({sum}-{min}-{max}) for 3 samples"
        );
    }
}

/// A minimal, non-allocating program still records the whole-program span
/// (`perf.program`, count 1 always) — a trivial n=1 exactness check: every
/// statistic collapses to the single sample's value.
#[test]
fn median_is_exact_for_a_single_sample() {
    let source = "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
    let exe = build_debug("dbg_exit_median1", source);
    let output = Command::new(&exe).output().expect("run built program");
    assert!(
        output.status.success(),
        "dbg_exit_median1: program failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        perf_value("dbg_exit_median1", &stderr, "perf.program.count"),
        1
    );
    let avg = perf_value("dbg_exit_median1", &stderr, "perf.program.avg");
    let median = perf_value("dbg_exit_median1", &stderr, "perf.program.median");
    let min = perf_value("dbg_exit_median1", &stderr, "perf.program.min");
    let max = perf_value("dbg_exit_median1", &stderr, "perf.program.max");
    let sum = perf_value("dbg_exit_median1", &stderr, "perf.program.sum");
    assert_eq!(avg, median, "dbg_exit_median1: avg != median for n=1");
    assert_eq!(median, min, "dbg_exit_median1: median != min for n=1");
    assert_eq!(min, max, "dbg_exit_median1: min != max for n=1");
    assert_eq!(max, sum, "dbg_exit_median1: max != sum for n=1");
}
