//! bug-760 B1: removing the oldest key of a `Map` / `Set` costs the same at any
//! size.
//!
//! bug-732 finding 2 kept the hash index across `removeKey` / Set `remove`, but
//! `map_mutate/mod.rs:lower_map_remove_key_in_place` still compacts the entries
//! after every removal (`map_bucket_remove.rs:lower_map_compact_from_helper`),
//! and now re-hashes and re-probes every entry it moves. Removing the oldest key
//! — the FIFO / sliding-window shape of the benchmark's `map intchurn` and
//! `mapchurn churn` — moves every remaining entry, so each step costs the window
//! size (12× the pre-bug-732 constant on 300 front removals). The fix leaves a
//! removed entry as a tombstone and compacts lazily, so a step is O(1) amortized.
//!
//! **The measure.** The program times, in-process, the same number of FIFO steps
//! (insert the next key, remove the oldest) over a window of `W` and of `8W`
//! keys, best of three each. Flat per-step cost gives a ratio near ×1; a
//! per-removal shift gives ×8. The bound ×3 sits between. The program also
//! checks the surviving keys — count, order and values — so a fast path that
//! loses or reorders entries fails here too.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const W: u64 = 500;
const SCALE: u64 = 8;
const STEPS: u64 = 20_000;
const MAX_RATIO: f64 = 3.0;

fn program() -> String {
    format!(
        "IMPORT collections\nIMPORT io\nIMPORT datetime\n\n\
         FUNC mapFifo(w AS Integer) AS Integer\n  \
         MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {{}}\n  \
         FOR i = 0 TO w - 1\n    m = collections::set(m, i, i * 3)\n  NEXT\n  \
         FOR i = w TO w + {STEPS} - 1\n    \
         m = collections::set(m, i, i * 3)\n    \
         m = collections::removeKey(m, i - w)\n  NEXT\n  \
         LET ks AS List OF Integer = collections::keys(m)\n  \
         MUT bad AS Integer = 0\n  \
         FOR j = 0 TO len(ks) - 1\n    \
         LET k AS Integer = collections::get(ks, j)\n    \
         IF k <> {STEPS} + j OR collections::get(m, k) <> k * 3 THEN\n      bad = bad + 1\n    END IF\n  \
         NEXT\n  \
         IF len(m) <> w OR collections::hasKey(m, {STEPS} - 1) THEN\n    bad = bad + 1\n  END IF\n  \
         RETURN bad\n\
         END FUNC\n\n\
         FUNC setFifo(w AS Integer) AS Integer\n  \
         MUT s AS Set OF Integer = Set OF Integer {{ }}\n  \
         FOR i = 0 TO w - 1\n    s = collections::add(s, i)\n  NEXT\n  \
         FOR i = w TO w + {STEPS} - 1\n    \
         s = collections::add(s, i)\n    \
         s = collections::remove(s, i - w)\n  NEXT\n  \
         LET ks AS List OF Integer = collections::toList(s)\n  \
         MUT bad AS Integer = 0\n  \
         FOR j = 0 TO len(ks) - 1\n    \
         IF collections::get(ks, j) <> {STEPS} + j THEN\n      bad = bad + 1\n    END IF\n  \
         NEXT\n  \
         IF len(s) <> w OR collections::contains(s, {STEPS} - 1) THEN\n    bad = bad + 1\n  END IF\n  \
         RETURN bad\n\
         END FUNC\n\n\
         FUNC best(which AS String, w AS Integer) AS Integer\n  \
         MUT bestNs AS Integer = 9223372036854775807\n  \
         FOR r = 0 TO 2\n    \
         LET t0 AS Integer = datetime::monotonicNanos()\n    \
         MUT bad AS Integer = 0\n    \
         IF which = \"map\" THEN\n      bad = mapFifo(w)\n    ELSE\n      bad = setFifo(w)\n    END IF\n    \
         LET ns AS Integer = datetime::monotonicNanos() - t0\n    \
         IF bad <> 0 THEN\n      io::print(\"BAD \" & which & \" \" & toString(w) & \" \" & toString(bad))\n    END IF\n    \
         IF ns < bestNs THEN\n      bestNs = ns\n    END IF\n  \
         NEXT\n  \
         RETURN bestNs\n\
         END FUNC\n\n\
         FUNC main AS Integer\n  \
         io::print(\"map \" & toString(best(\"map\", {W})) & \" \" & toString(best(\"map\", {W} * {SCALE})))\n  \
         io::print(\"set \" & toString(best(\"set\", {W})) & \" \" & toString(best(\"set\", {W} * {SCALE})))\n  \
         RETURN 0\n\
         END FUNC\n"
    )
}

#[test]
fn removing_the_oldest_key_is_flat_in_the_window_size() {
    let project = common::temp_project("b760_fifo_removal", &program());
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "program failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !stdout.contains("BAD"),
        "bug-760 B1: FIFO churn lost, reordered or corrupted entries:\n{stdout}"
    );
    let mut failures = Vec::new();
    for line in stdout.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [kind, small, large] = fields[..] else {
            panic!("unexpected line {line:?} in:\n{stdout}");
        };
        let small: f64 = small.parse().expect("small ns");
        let large: f64 = large.parse().expect("large ns");
        let ratio = large / small;
        eprintln!(
            "{kind}: W={W}: {:.2} ms; W={}: {:.2} ms; ×{ratio:.1}",
            small / 1e6,
            W * SCALE,
            large / 1e6
        );
        if ratio > MAX_RATIO {
            failures.push(format!(
                "{kind}: {STEPS} FIFO steps took {:.2} ms at a window of {W} and {:.2} ms at \
                 {}: ×{ratio:.1}",
                small / 1e6,
                large / 1e6,
                W * SCALE
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-760 B1: removing the oldest key costs the window size (flat is ≤ ×{MAX_RATIO}; \
         a per-removal compaction is ×{SCALE}):\n  {}",
        failures.join("\n  ")
    );
    let _ = std::fs::remove_dir_all(&project);
}
