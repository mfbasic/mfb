# Baseline results

Committed reference numbers for the benchmark suite — the output of a single
full `./benchmark/run.sh 10` (10 iterations per test, so the median/average/
min/max columns are meaningful). Regenerate by re-running that command and
moving the six `<target>-<ts>.log` / `.sums` pairs here, dropping the
timestamp from each name.

| file | target |
|------|--------|
| `mfb-O1.log` | MFBASIC at `-O1` (the default level) |
| `mfb-O2.log` | MFBASIC at `-O2` |
| `mfb-O3.log` | MFBASIC at `-O3` |
| `c-O0.log`   | C at `-O0` |
| `c-O2.log`   | C at `-O2` |
| `python.log` | CPython |

The matching `.sums` files hold each target's per-row `test_<name> = <checksum>`
lines. They are the correctness record, not timings: run.sh cross-validates
them at the end of every run, and this baseline passed with **363 checksum
keys, all 363 shared across ≥2 targets, 0 mismatched** — every row does the
same observable work in every language (see "Work equivalence" in the parent
README).

Provenance:

- date: 2026-09-03 (run timestamp `20260903-084033`)
- commit: `3ccc68297`
- host: Apple M2 Max, macOS 15.7.7 (arm64)
- command: `./benchmark/run.sh 10`

Rank these numbers with `./benchmark/rank.py` (see `benchmark/RANKING.md`).
Note that it ranks on the **`min`** column, not the median: for a *ratio*
between two columns `min` is measurably the more stable estimator
(`./benchmark/rank.py --calibrate`).

Timings are host-specific — compare a new run against this baseline only on
comparable hardware, and prefer the median column.

**This run was taken on a loaded machine** (load average ~8–16 throughout; a
second build was running on the same host). Absolute times are therefore inflated
and should not be compared against an earlier baseline taken on a quiet one. The
*ratios* between columns are what `rank.py` grades and they are unaffected, since
all six targets ran under the same conditions within the one run — but do not
read the raw millisecond figures as a machine-independent record.

plan-121-E additionally changed what three rows measure. `isSubset`/`isSuperset`/
`isDisjoint` now use a **TRUE** predicate (a full scan in every language) instead
of a FALSE one that let each language early-exit at whatever point its own
iteration order met the single counterexample, and the C peer gained a compact
element index so ITERATING a set is O(n) there as it already was in mfb and
Python. Both changes make the row measure the operation; neither flatters any
language. Their checksums changed from `0` to `k_pred` as a result — a checksum of
0 on a predicate row was the signal that the predicate never held.
