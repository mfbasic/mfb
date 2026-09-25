# bug-700: a `--debug` build spends tens of seconds writing its exit report once a program allocates a few hundred thousand times

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: LOW
Class: Other (tooling performance)

Status: FIXED (30d64579e)
Regression Test: tests/runtime/rt_debug_report_exit_time.rs

`mfb build --debug` produces a program that prints a measurement report to stderr
when it exits. Once the program has made a few hundred thousand allocations, the
report takes tens of seconds to write. The program's own work is long finished:
the process sits at 100% CPU on one thread between the report's header and its
statistics, and the window of an `--app` build stays open, unresponsive, for the
whole time. A developer profiling a real program (an animation that allocates
every frame, for example) sees it as a hang on quit.

**The single correct behavior a fix produces:** the time to write the report
grows no faster than n log n in the number of recorded samples. For the
reproduction below, that means it finishes in well under a second, not 26 s.

References:

- `src/codegen/builtins/perf/perf.rs:emit_write_stats`. Its doc comment records
  the design choice this bug trips: "an insertion sort (Open Decision — simple,
  exact; O(n²), upgradeable to heapsort if a profiled program's per-name sample
  count makes exit slow) yields the median."
- plan-67-E (`planning/completed/plan-67-E-perf-stats-table.md`): the statistics
  table and its sort scratch.
- Found while measuring `examples/water` under `--debug`. After Esc the frame loop
  had stopped (no new frames in the `MFB_CANVAS_STATS` log), but the process took
  more than 6 s and less than 70 s to exit. Its report showed
  `perf.mfb_alloc.count 349304` and `perf.overflow 101863133`.

## Failing Reproduction

```
' /tmp/perfrepro/src/main.mfb   (project.json: console executable)
IMPORT io

FUNC main AS Integer
  MUT total AS Integer = 0
  FOR i = 1 TO 350000
    LET s AS String = "n" & toString(i)
    total = total + len(s)
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
```

```
mfb build /tmp/perfrepro          && /usr/bin/time -p ./build/perfrepro.out
mfb build --debug /tmp/perfrepro  && /usr/bin/time -p ./build/perfrepro.out 2>report.txt
```

Measured on macOS aarch64, 2026-09-25:

| Build | Loop count | `real` | `perf.mfb_alloc.count` | `perf.overflow` |
| --- | --- | --- | --- | --- |
| release | 350000 | 0.17 s | — | — |
| `--debug` | 25000 | 0.45 s | 50001 | 0 |
| `--debug` | 50000 | 1.43 s | 100001 | 0 |
| `--debug` | 100000 | 4.50 s | 200001 | 0 |
| `--debug` | 175000 | 13.60 s | 349000 | 2003 |
| `--debug` | 350000 | 26.10 s | 349000 | 702003 |

- Observed: at 3 s into the 350000 run, `report.txt` holds only the three header
  lines (`mfb.debug.begin`, `.target`, `.build`), and a `sample` of the process
  shows one thread spinning in a tight loop inside the program image. The program
  body is done and all the remaining time goes to the report.
- Expected: the report is written in a small fraction of a second.

Below the sample-log cap, doubling the samples multiplies the time by about 3.2,
which fits the O(n²) sort. **The last two rows do not fit a sort-only explanation.**
Both sort the same 349000 samples per name, yet one takes twice as long as the
other, and the difference tracks `perf.overflow`. So there are two costs, and
Phase 1 must separate them before the fix is chosen.

## Root Cause

**Phase 1 result: hypothesis 1 (the O(n²) insertion sort) confirmed as the sole
cause; hypothesis 2 (extra cost tracking `perf.overflow`) is NOT confirmed.**

1. **The O(n²) insertion sort in `emit_write_stats`, confirmed.** Reproducing
   the 100000-loop row (below the log cap, `perf.mfb_alloc.count 200001`, plus
   an equal-sized `perf.mfb_free` span sharing the same log) on the machine
   used for this fix measured 2.9–4.5 s real time, consistent with the table.
   `sample`(1) profiling the 350000-loop run for 3 s captured **100% of
   samples inside a single ~300-byte hot loop** in the built executable,
   matching the insertion sort's compiled body (compare-and-shift over
   `PERF_SORT_OFFSET` scratch) — no time in any other code path. This is
   decisive: the O(n²) sort is the cost, not a proxy for it.
2. **Work that scales with the overflow count — NOT confirmed.** Code
   inspection of `perf.end`'s overflow branch (same file) shows it is three
   O(1) instructions (load the `PERF_OVERFLOW_OFFSET` counter, increment,
   store) with no scan, no sort, no per-sample work — it cannot cost seconds
   regardless of how many samples overflow (2003 vs. 702003 in the two rows).
   `emit_write_stats`'s own per-name linear scan over the log
   (`PERF_LOG_COUNT_OFFSET`, capped at `PERF_LOG_CAPACITY` once the log is
   full) is also unaffected by overflow count, since overflowed attempts never
   increment `log_count`. Re-measuring the 175000- and 350000-loop rows
   directly on this fix's development machine (shared with four sibling
   agents running concurrent heavy builds) gave `real` times of **12.62 s and
   10.36 s respectively — the OPPOSITE order from this doc's original table**,
   which is the same magnitude of "extra" time the table attributed to
   `perf.overflow`. That reversal is strong evidence the original table's
   "last two rows do not fit a sort-only explanation" reading was measurement
   noise from a contended machine, not a second, overflow-tracking cost. No
   overflow-path change was made; `perf_end` is untouched by this fix.

## Goal

- The report for the 350000 reproduction is written in under 1 s on the machine
  above. The `--debug` total `real` is then dominated by the program body.
- The median printed is unchanged, exactly, for every name (same definition: the
  middle value, or the floor-mean of the two middle values for an even count).

### Non-goals (must NOT change)

- The report format, names, column order and every other statistic.
- The sample-log capacity and the overflow accounting (`perf.overflow`). The cap is
  a documented bound, not part of this bug.
- Release builds: `perf` instrumentation is `--debug`-only and nothing outside it
  changes.
- **Tempting wrong fix:** report an approximate median (a histogram, or a sample
  of the log). The median is documented as exact, so this is forbidden unless the
  spec and the report documentation are changed deliberately, which is a separate
  decision.

## Blast Radius

- `src/codegen/builtins/perf/perf.rs:emit_write_stats`: the insertion sort. Fixed
  by this bug.
- `perf_end`'s overflow path (same file): suspect under hypothesis 2. **Not
  changed** — Phase 1 showed it does not cost anything (see Root Cause).
- Every `--debug` build on every target (the emitter is target-neutral codegen).
  They all share the defect, and the fix applies to all of them.
- `mfb test --debug` / benchmark harnesses that parse the report: unaffected by
  the format (Non-goals), and they only gain speed.

## Fix Design

Replace the insertion sort with an in-place **heapsort** over the same scratch
buffer. The emitter's comment already names it as the upgrade. It is O(n log n)
worst case, needs no extra memory (the scratch is sized exactly for n i64s), and
has a simple loop structure to hand-emit, in the same style as the existing
`emit_load_elem`/`emit_store_elem` helpers. For 698000 samples that is about
1.4e7 compares instead of about 1.2e11.

An alternative is **quickselect** for the median alone. It is O(n) on average,
but O(n²) in the worst case unless median-of-medians is emitted as well. For an
even count it needs two order statistics, so it is more code for no gain at
these sizes. Rejected.

Phase 1 did not confirm hypothesis 2, so the overflow path is untouched (see
Root Cause and Blast Radius).

## Phases

### Phase 1: failing test and measurement (no behavior change)

- [x] `tests/runtime/rt_debug_report_exit_time.rs`: build the reproduction with
      `--debug`, and assert that the report is complete and the process exits
      within a bound (3 s) at a loop count below the log cap (100000). RED
      before the fix: 4.0 s measured on this machine (report_exit_time_is_not_
      quadratic_in_sample_count).
- [x] Also assert the printed medians for a small, fixed input that is known to
      be deterministic, as the exactness guard for Phase 2
      (median_is_exact_for_a_three_sample_name, using the closed-form
      `median = sum - min - max` that holds for any 3-element multiset;
      median_is_exact_for_a_single_sample for the trivial n=1 case).
- [x] Separate the two hypotheses as described under Root Cause, and record the
      result here. Done — see Root Cause above.

Acceptance: the timing test fails today for the documented reason, and the
hypotheses are resolved in this file.
Commit: 30d64579e (Phase 1 tests landed together with Phase 2 in one commit —
serial single-root-cause fix, no independent Phase 1-only commit).

### Phase 2: heapsort (and the overflow path, if implicated)

- [x] Replace the insertion sort in `emit_write_stats` with an emitted heapsort
      over the scratch buffer.
- [x] Update the doc comment's Open Decision to record the change.
- [x] Hypothesis 2 was not confirmed (Phase 1), so no overflow-path change.

Acceptance: the Phase 1 tests pass, the medians are identical, and the 350000
reproduction's report takes under 1 s.
Commit: 30d64579e

### Phase 3: full validation

- [x] Targeted tests green (full suite deferred to the coordinator landing all
      five concurrent bug fixes together — see the note below): `cargo test
      --test rt_debug_report_exit_time` (3 passed), `cargo test --test
      rt_debug_report` (9 passed, unaffected by this change), `cargo test
      --bin mfb perf::` (9 passed, including the structural
      `perf_end_counts_every_way_a_sample_can_be_lost` and
      `no_perf_body_calls_an_arena_helper` guards — confirms `perf_end` and the
      arena-free invariant are unchanged), `cargo test --test
      test_targets_registered` (4 passed, confirms the new `[[test]]` Cargo.toml
      stanza). No golden contains a timing (plan-130-B) and this change does not
      touch any byte-identity-covered path outside `--debug` builds, so no
      golden was regenerated.
- [x] Re-run the reproduction table above and record the new times (below).
- [x] Spec: `mfb spec tooling debug-report` (`src/docs/spec/tooling/09_debug-
      report.md`) describes only the printed `perf.<span>.<stat>` columns, not
      the sort algorithm; no spec text describes `emit_write_stats`'s algorithm
      (confirmed: `grep -rln emit_write_stats src/docs/spec/` finds nothing), so
      no spec update is needed.

Acceptance: full suite green, and the table shows sub-second report times.
Commit: 30d64579e

**Post-fix reproduction (same machine, `--debug` build):**

| Loop count | `real` (pre-fix, this doc's table) | `real` (post-fix) |
| --- | --- | --- |
| 350000 | 26.10 s | 0.24 s |

`perf.mfb_alloc.count`/`perf.mfb_free.count`/`perf.overflow` are unchanged
(349000/349000/702003), confirming the fix changed only the sort's cost, not
its accounting.

## Validation Plan

- Regression tests: `tests/runtime/rt_debug_report_exit_time.rs` (time bound +
  exact medians) — done, all 3 green post-fix.
- Runtime proof: the reproduction table re-measured (350000-loop: 26.10 s →
  0.24 s). `examples/water` under `--debug`, quit with Esc, was not re-run on
  this machine (no GUI session available); the fix is in the same
  `emit_write_stats` path the fixture's `mfb_alloc`/`mfb_free` spans exercise,
  and is not expected to behave differently for that program.
- Doc sync: `emit_write_stats` doc comment updated (both the function doc and
  the `PERF_SORT_OFFSET` field comment). No spec entry describes the median
  algorithm (confirmed by grep), so no spec change was needed.
- Full suite: **deferred by coordinator instruction** — five bugs (564, 690,
  695, 697, 700) are being fixed concurrently in sibling worktrees on this
  machine; running `cargo test` once per worktree would 5x the load and each
  run would fight the others for `target/debug/mfb`/`target/release/mfb`
  build locks (`.ai/testing-gates.md`'s gate-lock and stale-binary hazards).
  The coordinator runs the full suite once after all five land. This bug's own
  targeted tests (`rt_debug_report_exit_time`, `rt_debug_report`, `--bin mfb
  perf::`, `test_targets_registered`) are green — see Phase 3.

## Open Decisions

- Heapsort (recommended) vs. quickselect: resolved — heapsort landed, per the
  Fix Design's reasoning (no extra memory, no median-of-medians complexity for
  the even-count case).

## Summary

Fixed. The O(n²) insertion sort in `emit_write_stats` (`src/codegen/builtins/
perf/perf.rs`) was the sole cost — confirmed by a `sample`(1) profile showing
100% of captured time in the sort's hot loop, and by code inspection showing
the overflow path is O(1) per event. The fix is local: one hand-emitted
heapsort loop over the same scratch buffer, in `--debug`-only code, guarded by
an exact-median test that holds regardless of actual (non-deterministic)
sample durations. `perf_end`'s overflow path is untouched — Phase 1 found no
cost there; the original table's "last two rows don't fit a sort-only
explanation" reading was traced to measurement noise from a shared, contended
machine (re-measuring reproduced the two rows in the opposite order).
