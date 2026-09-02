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

- date: 2026-09-02 (run timestamp `20260902-093747`)
- commit: `c4bed25a7`
- host: Apple M2 Max, macOS 15.7.7 (arm64)
- command: `./benchmark/run.sh 10`

Timings are host-specific — compare a new run against this baseline only on
comparable hardware, and prefer the median column.
