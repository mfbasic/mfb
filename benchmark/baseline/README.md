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

The matching `.sums` files hold each target's per-row `<key> = <checksum>`
lines. They are the correctness record, not timings: run.sh cross-validates
them at the end of every run, and this baseline passed with **515 checksum
keys, all 515 shared across ≥2 targets, 0 mismatched, 4 documented
approximations** — every row does the same observable work in every language
(see "Work equivalence" in the parent README), and every key agrees across
mfb -O1/-O2/-O3 and across C -O0/-O2.

Provenance:

- date: 2026-09-28 (run timestamp `20260928-201348`)
- commit: the bug-760 fix (branch `worktree-B-760` at `1b5faaded`, merged to main)
- host: Apple M2 Max, macOS 15.7.7 (arm64)
- command: `./benchmark/run.sh 10`

Rank these numbers with `./benchmark/rank.py` (see `benchmark/RANKING.md`).
Note that it ranks on the **`min`** column, not the median: for a *ratio*
between two columns `min` is measurably the more stable estimator
(`./benchmark/rank.py --calibrate`).

Timings are host-specific — compare a new run against this baseline only on
comparable hardware, and prefer the median column.

This run replaced the 2026-09-28 morning baseline (`9c1ea5568`), which included
the performance regressions filed as bug-760. Against it, with C and Python flat
(geomean 1.00), mfb's geomean `min` ratio is 0.777 at -O1, 0.838 at -O2 and 0.804
at -O3; every bug-760 row is back within 1.25× of the 2026-09-03 numbers. The only
`.sums` changes are the added P1 rows; no existing checksum moved. Three -O1
`distinct` rows and three -O2 rows (`list (Fixed) sum`, `map (Record-Dynamic)
hasKey`, `list (Record-Dynamic) forEach`) read 1.27–1.49× the previous baseline
here but match the pre-fix compiler when measured standalone (and the `distinct`
helper's code is smaller) — an in-process context effect, not a codegen change;
all six are at or below their 2026-09-03 times.

That morning baseline in turn replaced the 2026-09-03 one (`3ccc68297`), taken on a
loaded machine (load average ~8–16). Two mfb-only checksums moved against it, both
from deliberate `Fixed` accuracy fixes rather than a work change: `math_fixed`
(197572.096519 → 197572.096513; correctly-rounded trig, bug-615, and `exp`/`pow`
reduction, bug-659) and `vector_fixed` (1959779.201230 → 1959779.331315;
`angle`/`slerp`/`project`/`reject`/`clamp_length` rounded once, bug-654).

plan-121-E (landed in the previous baseline) changed what three rows measure. `isSubset`/`isSuperset`/
`isDisjoint` now use a **TRUE** predicate (a full scan in every language) instead
of a FALSE one that let each language early-exit at whatever point its own
iteration order met the single counterexample, and the C peer gained a compact
element index so ITERATING a set is O(n) there as it already was in mfb and
Python. Both changes make the row measure the operation; neither flatters any
language. Their checksums changed from `0` to `k_pred` as a result — a checksum of
0 on a predicate row was the signal that the predicate never held.
