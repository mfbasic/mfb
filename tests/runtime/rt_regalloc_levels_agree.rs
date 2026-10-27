//! bug-760: every optimization level computes the same value for a
//! register-pressure-heavy Float loop.
//!
//! The benchmark's `math simd` row — sixteen Float `collections::sum` terms and
//! three `toFloat(collections::sum(<Integer list>))` terms accumulated in one
//! loop — printed 111035761.957 at -O0/-O1 (and in C and Python) but
//! 109802161.957 at -O2/-O3 after the linear-scan allocator started sharing a
//! register between vregs whose live runs do not overlap: exactly the three
//! Integer-sum terms went missing (1544 + 2568 + 2056 per rep × 200). run.sh's
//! cross-level check caught it; this pins it without the benchmark.
//!
//! The -O0 build is the oracle (no optimizer, no register sharing); the literal
//! is the value C and Python compute for the same loop, so -O0 itself is checked.
//!
//! plan-171-A adds a second program, `tests/rt-behavior/functions/frame-pair-saves`:
//! its frames save seven callee-saved registers in `stp` pairs at every level, and
//! from -O1 the pairing row merges body accesses too, so -O0 (frame pairs only)
//! and -O1..-O3 (frame and body pairs) must print the same checksums.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

const SOURCE: &str = include_str!("regalloc_levels/math_simd_loop.mfb");
const EXPECTED: &str = "111035761.957";

fn run_at(project: &Path, level: &str) -> String {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-O")
        .arg(level)
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let exe = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable");
    let run = Command::new(exe).output().expect("run program");
    assert!(run.status.success(), "-O{level} program failed");
    String::from_utf8(run.stdout)
        .expect("utf8 program output")
        .trim()
        .to_owned()
}

#[test]
fn a_float_loop_under_register_pressure_agrees_at_every_level() {
    let project = common::temp_project("bug760_regalloc_levels", SOURCE);
    let mut results = Vec::new();
    for level in ["0", "1", "2", "3"] {
        results.push((level, run_at(&project, level)));
    }
    let _ = std::fs::remove_dir_all(&project);
    let wrong: Vec<String> = results
        .iter()
        .filter(|(_, got)| got != EXPECTED)
        .map(|(level, got)| format!("-O{level}: {got}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "bug-760: the math-simd loop must print {EXPECTED} at every level (C and Python \
         agree); got:\n  {}",
        wrong.join("\n  ")
    );
}

const FRAME_PAIR_SAVES: &str =
    include_str!("../rt-behavior/functions/frame-pair-saves/src/main.mfb");
/// The program's output, as `golden/build.log` records it (and Python computes).
const FRAME_PAIR_SAVES_OUTPUT: &str = "65\n1714\n288228\n204026";

#[test]
fn frame_and_body_pairs_agree_at_every_level() {
    let project = common::temp_project("frame_pair_saves_levels", FRAME_PAIR_SAVES);
    let mut results = Vec::new();
    for level in ["0", "1", "2", "3"] {
        results.push((level, run_at(&project, level)));
    }
    let _ = std::fs::remove_dir_all(&project);
    let wrong: Vec<String> = results
        .iter()
        .filter(|(_, got)| got != FRAME_PAIR_SAVES_OUTPUT)
        .map(|(level, got)| format!("-O{level}: {got:?}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "plan-171-A: frame-pair-saves must print {FRAME_PAIR_SAVES_OUTPUT:?} at every level; got:\n  {}",
        wrong.join("\n  ")
    );
}

/// plan-171-B: the compare-and-branch fixture — every compare with zero, sign
/// test and single-bit test, plus a bit test whose AArch64 `tbz` must be
/// relaxed past ±32 KiB — runs identically at every level (`-O0` forms
/// nothing, `-O1`+ forms `cbz`/`tbz`).
const BRANCH_TESTS: &str =
    include_str!("../rt-behavior/control-flow/branch-zero-and-bit-tests/src/main.mfb");
/// The program's output, as `golden/build.log` records it (the base compiler's).
const BRANCH_TESTS_OUTPUT: &str = "0 z+nonneg [!11]
1 nz+ne+nonneg [0!11]
-1 nz+ne+neg [01030316263]
1024 nz+ne+nonneg [10!11]
2048 nz+ne+nonneg []
3073 nz+ne+nonneg [010]
1073741824 nz+ne+nonneg [!1130]
2147483648 nz+ne+nonneg [!1131]
4611686018427387904 nz+ne+nonneg [!1162]
-9223372036854775808 nz+ne+neg [!1163]
9223372036854775807 nz+ne+nonneg [010303162]
21320
0";

#[test]
fn compare_and_branch_forms_agree_at_every_level() {
    let project = common::temp_project("branch_zero_and_bit_tests_levels", BRANCH_TESTS);
    let mut results = Vec::new();
    for level in ["0", "1", "2", "3"] {
        results.push((level, run_at(&project, level)));
    }
    let _ = std::fs::remove_dir_all(&project);
    let wrong: Vec<String> = results
        .iter()
        .filter(|(_, got)| got != BRANCH_TESTS_OUTPUT)
        .map(|(level, got)| format!("-O{level}: {got:?}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "plan-171-B: branch-zero-and-bit-tests must print the same at every level; got:\n  {}",
        wrong.join("\n  ")
    );
}
