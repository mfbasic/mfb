# bug-1020: `collections::reduce` / `reduceRight` return the caller's initial-value block when no element replaces it (use after free, double free)

Last updated: 2026-10-10
Effort: small (<1h)
Severity: HIGH
Class: Memory-safety

Status: Fixed (bs18-agent-j: 437e4d219 RED, 4d7d92c37 fix, 0290042c0 clippy cleanup)
Regression Test: tests/rt-behavior/collections/reduce-result-is-seed-rt (and `rt_reduce_collection_accumulator_frees`)

When the accumulator is still the initial value after the loop (an empty list, or a reducer that
returns its accumulator unchanged), `reduce` returned the accumulator slot itself: the unowned seed
block that the argument temporary or the source local also owns. The result was then freed twice,
or read after its owner freed it.

- Repro on main: `LET r AS Point = collections::reduce(emptyList, Point[1, 2], keep)` followed by
  more allocations prints the wrong point (the fixture prints `empty: 21,22`). With a union seed,
  `--debug` showed 8 frees for 7 allocs and `double_free_skips 1` (agent-j's measurement).
- Coordinator check (2026-10-10): the fixture fails with main's code
  (`/tmp/main-dbg-target/release/mfb`) and passes with bs18-agent-j's
  (`scripts/test-accept.sh … reduce-result-is-seed-rt`).
- Cause: `lower_collection_reduce_impl` returned the accumulator slot even when it was still the
  seed. Fix: a freeable-flat accumulator that is still the seed after the loop is copied once.
- Not covered: recursive (graph-owning) accumulators; agent-j did not test them.

Found by bs18-agent-j while making `reduce` accept a union variant as its initial value (bug-988).
