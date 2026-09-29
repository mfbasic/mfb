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
