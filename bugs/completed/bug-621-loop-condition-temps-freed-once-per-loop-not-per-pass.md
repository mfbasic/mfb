# bug-621: a call result in a WHILE / DO WHILE condition is freed once per loop, not once per pass

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory)

Status: Fixed
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (`a_do_while_condition_temp_is_freed_on_every_pass`, `a_while_condition_*`, `a_loop_until_condition_*`, `a_return_from_a_while_with_a_condition_temp_*`)

> **STATUS: FIXED (dc3b3d80a)** — 2026-09-14. Landed with bug-620 (one watermark model), tests
> `ee4da1dba` and `21bd00f4c`, goldens `4f21f7685`. A per-pass condition (WHILE, DO WHILE,
> LOOP UNTIL, the numeric FOR test) frees its temps before the branch (`lower_loop_condition`).
> The repro's growth between N=1000 and 2000 went from 128,000 B to 0 (`alloc_calls` =
> `free_calls`, `double_free_skips 0`). Not the archived append-growth bug-621, which shares the
> number.
>
> Deviations:
> - One free before the branch, with the value spilled and reloaded, instead of a free on each
>   edge — every evaluation takes that one edge.
> - `FOR … TO` bound audit: not affected (evaluated once into a synthetic local).
> - Phase 3's browser stages were re-run as the `rt_debug_soak` cases (growth under 1 MB), not
>   as per-call numbers; `pt_layout` is covered by the paint case, still ignored for bug-625.

A heap value produced while evaluating a loop condition — `DO WHILE i < n AND
strings::mid(s, i, 1) <> "="`, `WHILE … END WHILE` the same — is allocated on every pass but
freed only once, after the loop ends. A loop whose condition allocates on P passes leaks P − 1
blocks. Scanning loops over a string (`strings::mid` per character) leak one block per
character.

**The single correct behavior a fix produces:** each pass frees the temps its own condition
evaluation allocated, so a condition temp never outlives the pass that made it and the
reproduction reports equal `live_bytes` at N=1000 and N=2000.

References:

- `src/docs/spec/memory/04_arenas.md`, the scope-drop contract.
- Found by plan-133-A Phase 2 (`planning/completed/plan-133-A-browser-memory-diagnosis-and-soak-test.md`).
- Sibling: bug-620 (the same drop, skipped by an early exit from an `IF` branch).
- bug-440 (`tests/codegen/codegen_owned_drop_free_and_null.rs`): a record call in a `WHILE`
  condition, pinned for zero-after-free only, not for per-pass freeing.

## Failing Reproduction

`/tmp/plan-133-a/stages/r10_while8.mfb`, `{N}` = 1000 and 2000, `target/release/mfb build
--debug`, macOS, main `14c9fc1ca`:

```
IMPORT io
IMPORT strings

FUNC upToDo(s AS String) AS Integer
  LET n AS Integer = len(s)
  MUT i AS Integer = 0
  DO WHILE i < n AND strings::mid(s, i, 1) <> "="
    i = i + 1
  LOOP
  RETURN i
END FUNC

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    total = total + upToDo("abcdefgh=")
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

- Observed: N=1000 `live_bytes 141520`, `alloc_calls 9004`, `free_calls 1002`; N=2000
  `live_bytes 269520`, `alloc_calls 18004`, `free_calls 2002`. Nine condition evaluations
  allocate per call, one is freed: 8 × 16 B = 128 B per call.
- Expected: `live_bytes` equal at both N.

Contrast and bounding cases (N=1000 → 2000):

| Shape | `live_bytes` | Per call |
|---|---|---|
| the repro (`DO WHILE`, 9 passes) | 141,520 → 269,520 | 8 blocks |
| `WHILE … END WHILE`, same condition (`r10_whileend`) | 141,520 → 269,520 | 8 blocks |
| same over `"ab=cd"` (3 passes, `r6_whilecond`) | 45,520 → 77,520 | 2 blocks |
| one pass evaluates the temp, the next short-circuits on `i < n` (`cm_match`) | 13,520 → 13,520 | 0 |
| the temp bound inside the body: `DO WHILE i < n` / `LET c = strings::mid(s, i, 1)` / `IF c = "=" THEN EXIT DO` (plan-133-A patched copy) | flat | 0 |

## Root Cause

The `NirOp::While` arm (`src/codegen/engine/control/builder_control.rs`) lowers the condition
once in the code, between the `while_loop` label and `branch_eq(end_label)`; the back edge
jumps to `while_loop`, so the same code runs every pass. Its temps are registered in the
`While` statement's pending list (`register_pending_temp`,
`src/codegen/engine/value/builder_values.rs`), and the only free is the statement-scope
`drop_pending_temps_to(temp_watermark)` emitted after `while_end`. Each pass stores its fresh
block into the same `pending_temp` slot, overwriting the previous pass's pointer, so the drop
after the loop frees only the last one. `DO … LOOP UNTIL` lowers its condition after the body
with the same end-of-statement drop.

## Goal

- The repro, `r10_whileend` and `r6_whilecond` report equal `live_bytes` at N and 2N;
  `DO … LOOP WHILE/UNTIL` variants likewise.
- The pass that exits the loop still frees its condition temps (no leak, no double free).

### Non-goals (must NOT change)

- Condition semantics and short-circuit order.
- **Tempting wrong fix:** rewriting callers to bind the character first — that is how the
  plan-133-A diagnosis proved ownership, not a fix.

## Blast Radius

- `WHILE … END WHILE`, `DO WHILE … LOOP`, `DO … LOOP WHILE/UNTIL` with a heap temp in the
  condition — fixed by this bug.
- `FOR i = a TO f(x)` bounds — audit in Phase 1 (evaluated once or per pass?).
- `FOR EACH x IN f()` — unaffected: the collection is evaluated once and owned by the loop
  (measured flat, `sa_foreach_split`, `r9_exitforcall`).
- An `EXIT` out of the loop body — bug-620.

Consumers observed leaking (plan-133-A, `examples/browser`): `dom/src/parse.mfb` `parseAttrs`
(four `DO WHILE i < n AND isSpace(strings::mid(s, i, 1))`-style loops), `dom/src/resolve.mfb`
`compoundMatches` (`DO WHILE i < n AND strings::mid(s, i, 1) <> "." AND …`),
`dom/src/layout.mfb` `glyphsSpans` (`DO WHILE j + 1 < n AND sameStyle(collections::get(gs,
j + 1), g)`).

## Fix Design

Free the condition's temps on both edges out of the condition test: at the top of the body
(pass continues) and at `end_label` (loop exits), using the in-place free
(`emit_pending_temp_frees_in_place`, slot zeroed after free), then forget them from the
statement's list so the drop after `while_end` does not free them again.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] `rt_scope_drop_leaks.rs`: the repro, `WHILE … END WHILE`, `LOOP UNTIL`, and a record
      temp (`collections::get`) in the condition; confirm each fails.
- [x] Audit `FOR … TO` bound temps; record the verdict.
      Verdict: not affected. `FOR j = 1 TO len(strings::lower(s))` measured flat (`live_bytes`
      0 → 0, `alloc_calls` = `free_calls`): the IR evaluates the bound once into a synthetic
      `$for_end` local, so the per-pass condition reads a local. The per-pass condition still
      goes through the new condition lowering, which emits nothing when it makes no temp.
      Pinned in `condition_temps_that_already_had_an_owner_stay_flat`.

Acceptance: the cases fail for the documented reason; the audit has a verdict.
Commit: ee4da1dba (RED on main af7d9b778: DO WHILE / WHILE 128,000 B, LOOP UNTIL 112,000 B,
record condition 96,000 B growth between 1000 and 2000 calls)

### Phase 2 — the fix

- [x] Per-pass condition temp drop on both condition edges (`builder_control.rs`).
      Deviation: one free before the branch instead of one per edge — the value is spilled
      across the frees and reloaded (`lower_loop_condition`), so no edge needs its own free.

Acceptance: Phase 1 cases pass; `double_free_skips 0`; contrast cases unchanged.
Commit: dc3b3d80a

### Phase 3 — expected outputs + full validation

- [x] Regenerate the goldens the per-pass frees shift; confirm each delta is a condition-edge
      free.
- [x] Full suite and `scripts/test-accept.sh`. Full suite (`cargo test --no-fail-fast -- --skip artifact_gate_all`): EXIT 0, 183 binaries,
      5689 passed, 0 failed, 9 ignored. `scripts/test-accept.sh`: 1472 test(s) ran, all passed.
      `artifact-gate.sh all`: main 0 diff(s); fix 34 diff(s) in 7 fixtures, regenerated; re-run 0
      diff(s) over 2023 goldens.
- [x] Re-run plan-133-A's `parse`, `resolve` and `pt_layout` stages. As `rt_debug_soak
      --include-ignored`: dom parse, resolve styles and paint pass (growth under 1 MB).

Acceptance: full suite green; golden deltas only condition-edge frees.
Commit: 4f21f7685 (goldens), 935f2e72f (soak markers)

## Validation Plan

- Regression tests: the Phase 1 cases.
- Runtime proof: plan-133-A's stage programs before and after.
- Doc sync: none expected.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Land with bug-620 (one watermark model) — recommended — vs. separately.

## Summary

A small change in one lowering arm, with the usual risk of double-freeing the exit pass's temps.
