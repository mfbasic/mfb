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

- [ ] Implement the one-pass walk over each name's chunk chain in `perf_done`
      accumulating sum/min/max; compute avg = sum/count; guard count==0.
- [ ] Print `name  count  avg  min  max  sum` (median column added next), aligned.

Acceptance: a **debug** build over a fixture with known sample values prints
correct count/avg/min/max/sum (hand-verified); release byte-identical (`diffs=0`).
Commit: —

### Phase 2 — Median (materialize + sort)

- [ ] Reserve the sort scratch buffer in the region header; materialize each
      name's samples into it.
- [ ] Implement the i64 sort and median pick (even-count averaging); insert the
      `median` column between `avg` and `min` to match the header
      `name count avg median min max sum`.

Acceptance: a **debug** build over a fixture with a known, hand-sortable sample
set prints the correct median for both odd and even counts; all six columns
correct and aligned; release byte-identical (`diffs=0`, acceptance green).
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

<Filled in during execution.>

## Summary

Pure arithmetic over D's proven sample structure, made correct by
hand-checkable fixtures. The only lurking hazard is i64 sum overflow at
implausible sample counts — noted, not mitigated. After E the table matches the
requested `name count avg median min max sum`.
