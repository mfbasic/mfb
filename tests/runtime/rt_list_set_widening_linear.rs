//! bug-627: a loop of length-changing `collections::set`s over a `List OF String`
//! runs in time linear in the list length.
//!
//! `lower_list_set_in_place` wrote a longer replacement where the old payload lay
//! and shifted every byte after it up, then fixed up every entry whose offset lay
//! past it (plan-121-F). That is O(N) per write, so widening each element of an
//! N-element list front to back cost O(N²): 0.58 s, 1.81 s and 6.69 s for N =
//! 25,000, 50,000 and 100,000, while the same loop with a same-size replacement
//! stayed at 0.17 s. Allocation grew only linearly, so only time shows it.
//!
//! **The measure.** Wall time at `N` and `8N`, the better of three runs each so a
//! loaded machine cannot fake a failure. Linear work gives at most ×8 (process
//! start-up only lowers it); the shift gives ×64 in the limit. The ×16 bound sits
//! between. The program also reads back every element and prints how many are
//! wrong, so a fast path that corrupts the list fails here too.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const N: u64 = 12_500;
const SCALE: u64 = 8;
const MAX_RATIO: f64 = 16.0;

fn program(n: u64) -> String {
    format!(
        "IMPORT collections\nIMPORT io\nIMPORT strings\n\n\
         FUNC main AS Integer\n  \
         MUT keep AS List OF String = strings::split(strings::repeat(\"x,\", {n}), \",\")\n  \
         FOR i = 0 TO {n} - 1\n    \
         LET s AS String = \"abcdefghijklmnopqrstuvwxyzabcdefghijkl\" & toString(i)\n    \
         keep = collections::set(keep, i, s)\n  \
         NEXT\n  \
         MUT bad AS Integer = 0\n  \
         FOR i = 0 TO {n} - 1\n    \
         IF collections::get(keep, i) <> \"abcdefghijklmnopqrstuvwxyzabcdefghijkl\" & toString(i) THEN\n      \
         bad = bad + 1\n    \
         END IF\n  \
         NEXT\n  \
         IF collections::get(keep, {n}) <> \"\" THEN\n    \
         bad = bad + 1\n  \
         END IF\n  \
         io::print(toString(len(keep)) & \" \" & toString(bad))\n  \
         RETURN 0\n\
         END FUNC\n"
    )
}

fn build(n: u64) -> PathBuf {
    let project = common::temp_project(&format!("b627_set_widen_{n}"), &program(n));
    common::build_project(&project)
}

/// One run: require the full, uncorrupted list and return the wall time.
fn run(exe: &PathBuf, n: u64) -> Duration {
    let start = Instant::now();
    let output = Command::new(exe).output().expect("run the program");
    let elapsed = start.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "N={n} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout.lines().next(),
        Some(format!("{} 0", n + 1).as_str()),
        "bug-627: N={n}: wrong length or corrupted elements (`<len> <bad>`)"
    );
    elapsed
}

#[test]
fn a_widening_set_loop_over_a_string_list_is_linear() {
    let small_exe = build(N);
    let large_exe = build(N * SCALE);
    // An untimed first run of each: a freshly written executable's first launch
    // pays a one-off start-up cost (measured 0.32 s against 0.16 s after) that
    // would inflate only the small side and hide the quadratic.
    run(&small_exe, N);
    run(&large_exe, N * SCALE);
    let small = (0..3).map(|_| run(&small_exe, N)).min().unwrap();
    let mut large = run(&large_exe, N * SCALE);
    if large.as_secs_f64() > small.as_secs_f64() * MAX_RATIO {
        for _ in 0..2 {
            large = large.min(run(&large_exe, N * SCALE));
        }
    }
    let ratio = large.as_secs_f64() / small.as_secs_f64();
    eprintln!("N={N}: {small:?}; N={}: {large:?}; ×{ratio:.1}", N * SCALE);
    assert!(
        ratio <= MAX_RATIO,
        "bug-627: {N} widening `set`s took {small:?}, {} took {large:?}: ×{ratio:.1} for \
         ×{SCALE} the work. Linear is ≤ ×{SCALE}; shifting the data tail on every write is \
         quadratic.",
        N * SCALE
    );
}
