# plan-67-E: Full statistics — count, avg, median, min, max, sum

Last updated: 2026-07-26
Effort (Human): medium
Effort (AI): medium
Depends on: plan-67-D (table A with per-name chunked sample sequences; `perf_done`
iterating A and printing `name count`; the decimal formatter)
Produces:
- Per-name statistics computed over each name's sample sequence at `perf_done`:
  **count, avg, median, min, max, sum**.
- A median implementation: copy a name's samples into a scratch buffer in the
  region and sort (insertion or heapsort in hand-emitted code), then pick the
  middle (average of the two middle values for even counts).
- Column-aligned table output matching the header from B:
  `name  count  avg  median  min  max  sum`.

This is the letter that makes the table match the spec the user gave.

References: `.ai/compiler.md`. Prerequisites: plan-67-A gate.

## 1. Goal

- A **debug** build compiles+runs a program and `perf_done` prints, for each name,
  a row with all six statistics correctly computed over its recorded durations —
  verified against a fixture with hand-checkable sample values. Release output
  unchanged.

### Non-goals

- No new instrumentation sites (F). E only enriches `perf_done`'s output.
- Perf code stays arena-free; the sort scratch buffer lives in the perf region.
- **macOS only** (see plan-67-B "Platform scope"): the stats logic lives in the
  macOS arm; Linux/Windows stay no-op stubs. "Debug build" below means a **debug
  macOS** build.

## 2. Current State

- After D: `perf_done` walks A's entries and, per name, can already reach the
  chunked sample sequence and a `count`. It prints `name  count`.
- The div-by-10 decimal formatter (from C, pattern at `entry.rs:1025-1044`) is
  available for each numeric column.

### Verified properties

- **All six stats are computable from the chunked sequence in one or two passes** —
  count (already tracked), sum/min/max/avg in a single walk; median needs the
  samples materialized and sorted. avg = sum / count (integer division; see Open
  Decisions on rounding). VERIFIED as arithmetic; the fixture check is the proof.

## 3. Design Overview

- **Single-pass accumulation:** walk the name's chunk chain once accumulating
  `sum`, `min`, `max`; `count` is the entry field; `avg = sum / count`.
- **Median:** materialize the `count` samples into a scratch region buffer (a
  bump-allocated temp, reused per name), sort ascending, then median =
  `n` odd → middle element; `n` even → `(a[n/2-1] + a[n/2]) / 2`. Use a simple
  in-place sort (insertion sort is fine for typical counts; if counts get large in
  F, switch to heapsort — decide in Open Decisions). Sorting is over i64.
- **Formatting/alignment:** print `name`, then each stat via the decimal
  formatter, separated by padding to fixed column widths so the table reads as
  columns. Widths can be fixed generous constants (numbers are nanoseconds).

**Correctness risk:** integer overflow of `sum` for many large-nanos samples
(i64 sum of many ~1e9 values overflows only past ~9.2e18 ≈ 9.2e9 samples — not a
practical risk, but note it). Median even-count averaging and empty-sequence
guard. All bounded and fixture-checkable. **Design uncertainty:** none material;
E is arithmetic over D's proven structure.

## 4. Detailed Design

- Add a `perf_stats` inline block in `perf_done` (`perf.rs`): per A entry, one walk
  for sum/min/max, one materialize+sort for median, then emit the row.
- Reserve a scratch buffer offset in the region header for the sort area (size =
  max samples per name; if a name exceeds it, sort in chunks / fall back — but
  simplest is to size it to the region's per-name cap from D).
- Column widths as named constants; the header string from B must match the
  column order/spacing.

## Compatibility / Format Impact

Debug-only: `perf_done` output gains four columns. Release unchanged.

## Phases

> Checkboxes current in the same commit. Unticked = NOT DONE.

### Phase 1 — Single-pass stats (count, sum, avg, min, max)

- [x] Implemented `emit_write_stats`: one linear pass over the **flat sample log**
      (not a chunk chain — see plan-67-D) filtered by `namePtr`, accumulating
      sum/min/max (min seeds `i64::MAX`, max 0; durations are non-negative) and
      materializing the name's durations into the region sort scratch. `avg =
      sum / count` (integer floor). count>=1 by construction (an A entry exists only
      after a logged sample), and aarch64 `udiv`-by-0 is 0, so no divide guard is
      load-bearing.
- [x] Refactored the numeric formatter into `emit_write_i64(newline)` with
      `_line`/`_field` wrappers so a row prints as one line: `name  <count> <avg>
      <median> <min> <max> <sum>\n` (fields have no newline; `sum` closes the row).

Acceptance: see Phase 2 (both phases landed together; the columns are one row).
Commit: —

### Phase 2 — Median (materialize + sort)

- [x] Reserved the sort scratch as a dedicated i64 buffer at `PERF_SORT_OFFSET`
      (region tail, past the log; the log budget was reduced to `PERF_LOG_CAPACITY
      = 698000` so log 16 B + scratch 8 B per sample both fit). The one-pass log
      scan materializes each name's durations there.
- [x] Implemented an in-place i64 insertion sort + median pick (odd → middle; even
      → mean of the two middle), with `median` between `avg` and `min` to match the
      header order.

Acceptance: **debug macOS** runtime proofs — single sample: `program 1 6000 6000
6000 6000 6000` (all stats equal the one duration). Multi-sample (a temporary
4-pair injection, reverted after): `program 5 4200 0 0 21000 21000` — count 5,
sum 21000, avg 4200 (=21000/5), min 0, max 21000, **median 0** (correct middle of
sorted `[0,0,0,0,21000]`), proving the multi-element sort + odd-count median. Even
count + real varied data are exercised by plan-67-F's arena samples. Release
byte-identity: see acceptance below.
Commit: —

## Validation Plan

- Tests: runtime-proof fixtures with hand-computable stats — odd count, even
  count, single sample, and a many-sample case crossing a D chunk boundary.
- Coverage check: debug `.ncode` reflects the stats block; release unchanged.
- Runtime proof: the six-column table with correct values at exit under debug.
- Doc sync: document the column set + median definition in the perf-helper spec
  section.
- Acceptance: `cargo test`; `artifact-gate.sh target/release/mfb` (`diffs=0`);
  acceptance under release green.

## Open Decisions

- **avg rounding** — *(recommended)* integer floor (`sum / count`) vs. rounded
  (`(sum + count/2) / count`). Recommend rounded so the displayed avg is not
  biased low. (§3)
  Decision: integer floor
- **Sort algorithm** — *(recommended)* insertion sort for simplicity, upgrade to
  heapsort only if F produces names with very large sample counts. (§3)
  Decision: insertion sort
- **Even-count median** — *(recommended)* average of the two middle values
  (integer) vs. lower-middle. Recommend average. (§3)
  Decision: Recommend average

## Corrections

- **Stats scan the flat log, not a chunk chain.** plan-67-D replaced the chunked
  per-name sample lists with a single flat log, so `emit_write_stats` computes
  sum/min/max in one linear pass over the log filtered by `namePtr`, materializing
  that name's durations into the sort scratch as it goes.
- **Insertion sort (Open Decision), kept exact.** Chose insertion sort per the
  recommended Open Decision — simple and exact. It is O(n²); a program whose
  per-name arena-sample count is very large would make the exit-time sort slow. The
  documented upgrade path (heapsort) is unchanged; the sort is isolated in
  `emit_write_stats`, so the swap is local if a profiled program needs it. Not
  capped/approximated (that would be a silent inaccuracy).
- **One-line rows.** The plan implied per-column formatting; the formatter
  (`emit_write_i64`) gained a `newline` flag so a row is `name` + five space-prefixed
  fields + a final ` <sum>\n`, keeping the data aligned under the single-line header.
- **avg rounding = integer floor** (Open Decision), **even median = mean of the two
  middle** (Open Decision), min seeds at `i64::MAX` / max at 0 (durations are
  non-negative).

## Summary

Pure arithmetic over D's proven sample structure, made correct by
hand-checkable fixtures. The only lurking hazard is i64 sum overflow at
implausible sample counts — noted, not mitigated. After E the table matches the
requested `name count avg median min max sum`.
