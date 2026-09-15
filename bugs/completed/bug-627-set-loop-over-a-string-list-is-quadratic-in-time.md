# bug-627: a `collections::set` loop over a `List OF String` is quadratic in time

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (time)

Status: FIXED (0864d5247)
Regression Test: tests/runtime/rt_list_set_widening_linear.rs, tests/codegen/codegen_string_set_shift.rs

A loop that replaces every entry of a same-function `MUT` `List OF String` in place,
`keep = collections::set(keep, i, s)` with `s` bound to a `LET`, takes time proportional to
the square of the list length. Doubling the entry count takes 3.5–3.7 times as long. Memory grows
only about linearly (allocated bytes ×2.2), and the output is correct, so only wall time shows it.

**The single correct behavior a fix produces:** the loop runs in time linear in the entry count,
so doubling the count about doubles the time.

References:

- Memory note `collection-set-in-place-only-for-same-function-local`: the in-place `set`
  shape on a same-function local was measured at 5 ms for 20,000 writes into a 200,000-byte
  `List OF Byte`.
- Found by plan-133-C Phase 1 while looking for a many-grows workload
  (`planning/plan-133-C-arena-memory-over-time.md` Corrections).
- Related but distinct: bug-626 (`append` of a builtin call result copies the list).

## Failing Reproduction

`/tmp/plan-133-c/setgen{50k,100k}` and `/tmp/plan-133-c/splitgen{100k,200k}`, built with
`target/release/mfb build --debug` on macOS (worktree `worktree-P-133` at `19345469e`), timed
with `/usr/bin/time -l`:

```
IMPORT collections
IMPORT io
IMPORT strings

FUNC main AS Integer
  MUT keep AS List OF String = strings::split(strings::repeat("x,", {N}), ",")
  FOR i = 0 TO {N} - 1
    LET s AS String = "abcdefghijklmnopqrstuvwxyzabcdefghijkl" & toString(i)
    keep = collections::set(keep, i, s)
  NEXT
  io::print(toString(len(keep)))
  RETURN 0
END FUNC
```

| Shape | N | wall time | `alloc_bytes` | `grow` |
|---|---:|---:|---:|---:|
| list from `strings::split`, then `set` (above) | 100,000 | 9.70 s | 69,300,320 | 12 |
| same | 200,000 | 35.51 s (×3.66) | 138,599,824 (×2.0) | 12 |
| list built by `append(keep, "")` N times, then the same `set` loop | 50,000 | 2.60 s | 23,262,272 | 18 |
| same | 100,000 | 9.09 s (×3.50) | 51,472,272 (×2.21) | 20 |

- Observed: time rises about ×3.5–3.7 for 2× N.
- Expected: about ×2.
- These runs used `--debug` builds; whether a normal build shows the same ratio is not yet
  measured (Phase 1).

## Root Cause

**Hypothesis 1, confirmed.** `lower_list_set_in_place`
(`src/codegen/collection/list/list_mutate.rs`) writes a replacement of a different length
where the old payload lies (plan-121-F): it shifts every data byte after the old span
(`emit_block_copy_backward`, `set_inplace_widen`) and then walks the whole entry table moving
each offset past it (`emit_offset_expansion_fixup`, `set_inplace_widenfix`). Both are O(N) per
write, so widening each element of an N-element list front to back is O(N²).

Measured at `af7d9b778` with normal (not `--debug`) release builds (`/tmp/b626/run627.sh`),
wall seconds:

| Loop body | N = 25,000 | 50,000 | 100,000 |
|---|---:|---:|---:|
| `set` of a 39–43-byte string over `"x"` (widening) | 0.58 | 1.81 | 6.69 |
| `set` of `"y"` over `"x"` (same size) | — | 0.16 | 0.18 |
| the same string built, no `set` | — | 0.23 | 0.17 |

The same-size and no-`set` loops are flat, which rules out hypotheses 3 (the concatenation)
and 4 (`--debug`). Hypothesis 2 is ruled out by allocation growing only ×2 with N: the
copying path would allocate a whole list per write.

What was checked first:

- `try_inplace_set_assign` (`src/codegen/collection/assign/builder_inplace_assign.rs`)
  requires a non-`by_ref` local, a three-argument `set` whose first argument is that local,
  no live `FOR EACH` over it, and a known collection layout. None of these excludes a
  `List OF String`, so the loop should take the in-place path.

Hypotheses, and how to confirm each:

1. The in-place path is taken, but replacing a string element does work proportional to the
   list (for example a string list stored in a packed layout, rebuilt or shifted on each
   write). Confirm: codegen inspection of the loop body for the in-place helper call, then
   time a `set` loop with every string pre-built, so only the `set` is timed.
2. The in-place path is not taken for this operand shape. Confirm: the same codegen inspection.
3. The time is outside the `set` (the `&` concatenation or `toString`). Confirm: time the loop
   without the `set`.
4. The `--debug` counters or the plan-133-C series sampling contribute. Confirm: the same
   probes built without `--debug`.

## Goal

- The reproduction scales about linearly: at most ×2.3 time for 2× N at N ≥ 100,000.

### Non-goals (must NOT change)

- `set` semantics, and the existing in-place gates' exclusions.

## Blast Radius

To be audited in Phase 1, once the cause is known: other element types (`List OF Record`,
`List OF List OF T`), and the other in-place gates in the same file.

## Fix Design

Depends on the root cause.

## Phases

### Phase 1 — failing test + root cause (no behavior change)

- [x] Run the four hypotheses' checks, and cite the cause here.
- [x] A test timing (or counting work for) the `set` loop at N and 2N that fails today.
      (Deviation: N and 8N, best of three after an untimed warm-up run; at N and 2N process
      start-up noise overlapped the linear and quadratic ratios, and the first version
      passed once on the broken compiler.)

Acceptance: Root Cause names the mechanism with evidence; the test fails for that reason.
Commit: daab32aeb (`tests/runtime/rt_list_set_widening_linear.rs`, RED ×48.7)

### Phase 2 — the fix

- [x] Fix at the cause.

Acceptance: the Phase 1 test passes; the reproduction scales linearly.
Commit: 0864d5247

### Phase 3 — expected outputs + full validation

- [x] Regenerate shifted goldens; full suite; `scripts/test-accept.sh`.

Acceptance: full suite green.
Commit: 429eccf66 (goldens)

## STATUS: FIXED (0864d5247)

Landed with bug-626 on one integration branch (`worktree-B-626-627`).

- A length-changing `set` no longer touches other elements: shorter → overwrite in place;
  longer and last in the data region → grow in place; longer elsewhere → aligned data tail,
  old span left as a hole; a tail write that overflows → `emit_repack_list_data` (replaces
  `emit_grow_list_data_capacity`), sized `step(live + need + count)`. `emit_offset_expansion_fixup`
  lost its only caller and is deleted.
- **Design deviation from the old invariant:** the variable-width data region may now hold
  holes. An audit of every reader (copy, compare, slice, join, sort, `FOR EACH`, thread
  transfer, graph copy) found all locate payloads by their own entry; the memory spec
  (`dataLength`, Payload Order, `set`, Compaction) and `.ai/collections.md` now say so.
- Regression test RED ×48.7 → GREEN ×7.0 (N=12,500: 10.8 ms; 100,000: 75.7 ms). The
  reproduction: 0.19 / 0.20 / 0.24 s for N = 25k / 50k / 100k (was 0.58 / 1.81 / 6.69).
- `tests/codegen/codegen_string_set_shift.rs`: the plan-121-F assertions that the tail shift and
  offset fixups are emitted were disproved by this bug's measurement and replaced; its
  no-rebuild, geometric-overflow and fixed-width assertions are unchanged.
- Correctness: `p121f-string-set-readback-rt` and `collection-set-string-grow-rt` pass; an edge
  probe (six forward/backward rounds with snapshots, set mixed with removeAt/insert/prepend,
  records with inline strings, nested lists, FOR EACH, copy-before-set) matched hand and Python
  expectations; rewriting one index 200,000 times kept `mapped_bytes` at 4,096.
- A repack on a tiny list whose one element keeps alternating length runs about every other
  write (alloc_bytes linear in writes, memory flat): linear, but not allocation-free.
- Goldens: 49 `.ncode` sums shared with bug-626; this bug's share is the
  `set_inplace_shift` → `set_inplace_resize` label rename (same instruction count on
  fixed-width lists). Regenerated in 429eccf66; re-run 0 diffs over 2023 goldens.
- `scripts/test-accept.sh`: 1472 passed. `cargo test --no-fail-fast`: 185 test binaries, 5,675 passed, 0 failed, 11 ignored.

## Validation Plan

- Regression test: the Phase 1 case.
- Runtime proof: the four probes above.
- Doc sync: none expected.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- None.

## Summary

A timing-only regression with no cause found yet. Phase 1 exists to find the cause before any fix.
