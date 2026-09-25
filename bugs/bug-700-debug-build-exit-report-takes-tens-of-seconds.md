# bug-700: a `--debug` build spends tens of seconds writing its exit report once a program allocates a few hundred thousand times

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: LOW
Class: Other (tooling performance)

Status: Open
Regression Test: tests/runtime/rt_debug_report_exit_time.rs (to be written, Phase 1)

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

Not yet established. Hypotheses, most likely first:

1. **The O(n²) insertion sort in `emit_write_stats`.** Each name's durations
   are copied into the sort scratch (up to `PERF_LOG_CAPACITY` = 698000
   entries) and insertion-sorted to find the median. Insertion sort costs
   O(n + inversions). Allocation durations are close to random, so that is
   about n²/4 compares and moves per name. This is consistent with the 3.2×
   growth per doubling below the cap. *Confirm:* time the report on a
   synthetic log with the sort replaced by a no-op (a local patched build), at
   the table's sizes.
2. **Work that scales with the overflow count.** The 175000 and 350000 rows
   sort identical n but differ by 12.5 s, matching 700000 more overflowed
   samples. Either `perf_end` does something expensive on the overflow path, or
   the inversion count of the first 349000 samples depends on what the rest of
   the run did (a warmer arena changes the duration distribution). *Confirm:*
   time the program body alone (a timestamp printed before `main` returns) for
   both loop counts, and count sort iterations in a patched build.

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
- `perf_end`'s overflow path (same file): suspect under hypothesis 2. It is fixed
  here if Phase 1 shows it is the cost.
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

If Phase 1 confirms hypothesis 2, fix the overflow path's cost in the same bug:
it sits on the same report path and has the same symptom.

## Phases

### Phase 1: failing test and measurement (no behavior change)

- [ ] `tests/runtime/rt_debug_report_exit_time.rs`: build the reproduction with
      `--debug`, and assert that the report is complete and the process exits
      within a bound (for example 3 s) at a loop count below the log cap
      (100000). It fails today (4.5 s), and the bound leaves wide margin for a
      slow CI machine after the fix.
- [ ] Also assert the printed medians for a small, fixed input that is known to
      be deterministic, as the exactness guard for Phase 2.
- [ ] Separate the two hypotheses as described under Root Cause, and record the
      result here.

Acceptance: the timing test fails today for the documented reason, and the
hypotheses are resolved in this file.
Commit: —

### Phase 2: heapsort (and the overflow path, if implicated)

- [ ] Replace the insertion sort in `emit_write_stats` with an emitted heapsort
      over the scratch buffer.
- [ ] Update the doc comment's Open Decision to record the change.
- [ ] If hypothesis 2 is confirmed, fix the overflow-path cost.

Acceptance: the Phase 1 tests pass, the medians are identical, and the 350000
reproduction's report takes under 1 s.
Commit: —

### Phase 3: full validation

- [ ] Full suite (`cargo test`), goldens unchanged. No golden should contain a
      timing, so any diff is a bug-hunt trigger.
- [ ] Re-run the reproduction table above and record the new times.
- [ ] Spec: if `mfb spec` describes how the report computes the median, update
      it (`scripts/spec-census.sh --citations` for the `emit_write_stats`
      citations).

Acceptance: full suite green, and the table shows sub-second report times.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_debug_report_exit_time.rs` (time bound +
  exact medians).
- Runtime proof: the reproduction table re-measured, plus `examples/water` built
  with `--debug`, quit with Esc, exits promptly.
- Doc sync: `emit_write_stats` doc comment; spec entry if one describes the median.
- Full suite: `cargo test`.

## Open Decisions

- Heapsort (recommended) vs. quickselect: see Fix Design.

## Summary

The risk is small and local: one hand-emitted sort loop in `--debug`-only code,
guarded by an exact-median test. The open question is whether the insertion sort
is the only cost; the last two rows of the table say it may not be, and Phase 1
settles that before anything changes.
