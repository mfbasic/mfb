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

- date: 2026-09-28 (run timestamp `20260928-055319`)
- commit: `9c1ea5568`
- host: Apple M2 Max, macOS 15.7.7 (arm64), load average ~2.7–3.5 at start
- command: `./benchmark/run.sh 10`

Rank these numbers with `./benchmark/rank.py` (see `benchmark/RANKING.md`).
Note that it ranks on the **`min`** column, not the median: for a *ratio*
between two columns `min` is measurably the more stable estimator
(`./benchmark/rank.py --calibrate`).

Timings are host-specific — compare a new run against this baseline only on
comparable hardware, and prefer the median column.

This run replaced the 2026-09-03 baseline (`3ccc68297`), which had been taken on
a loaded machine (load average ~8–16) and so inflated every absolute time; the C
and Python columns ran ~7% faster here under the same hardware. Two mfb-only
checksums moved against that baseline, both from deliberate `Fixed` accuracy
fixes rather than a work change: `math_fixed` (197572.096519 → 197572.096513;
correctly-rounded trig, bug-615, and `exp`/`pow` reduction, bug-659) and
`vector_fixed` (1959779.201230 → 1959779.331315; `angle`/`slerp`/`project`/
`reject`/`clamp_length` rounded once, bug-654).

plan-121-E (landed in the previous baseline) changed what three rows measure. `isSubset`/`isSuperset`/
`isDisjoint` now use a **TRUE** predicate (a full scan in every language) instead
of a FALSE one that let each language early-exit at whatever point its own
iteration order met the single counterexample, and the C peer gained a compact
element index so ITERATING a set is O(n) there as it already was in mfb and
Python. Both changes make the row measure the operation; neither flatters any
language. Their checksums changed from `0` to `k_pred` as a result — a checksum of
0 on a predicate row was the signal that the predicate never held.

This baseline **includes** the performance regressions filed as bug-760 (measured
against the previous baseline: record-field `groupby`/`sortBy`, plain map
`removeKey`/set `remove`, `float nbody`, and the `-O1`-only list `sort*`/`find`
and `width` rows). A run after that fix should show those rows improve, not match.
