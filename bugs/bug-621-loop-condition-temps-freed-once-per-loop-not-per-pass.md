# bug-621: a call result in a WHILE / DO WHILE condition is freed once per loop, not once per pass

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (to add, Phase 1)

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
- Found by plan-133-A Phase 2 (`planning/plan-133-A-browser-memory-diagnosis-and-soak-test.md`).
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

- [ ] `rt_scope_drop_leaks.rs`: the repro, `WHILE … END WHILE`, `LOOP UNTIL`, and a record
      temp (`collections::get`) in the condition; confirm each fails.
- [ ] Audit `FOR … TO` bound temps; record the verdict.

Acceptance: the cases fail for the documented reason; the audit has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Per-pass condition temp drop on both condition edges (`builder_control.rs`).

Acceptance: Phase 1 cases pass; `double_free_skips 0`; contrast cases unchanged.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate the goldens the per-pass frees shift; confirm each delta is a condition-edge
      free.
- [ ] Full suite and `scripts/test-accept.sh`.
- [ ] Re-run plan-133-A's `parse`, `resolve` and `pt_layout` stages.

Acceptance: full suite green; golden deltas only condition-edge frees.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 cases.
- Runtime proof: plan-133-A's stage programs before and after.
- Doc sync: none expected.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Land with bug-620 (one watermark model) — recommended — vs. separately.

## Summary

A small change in one lowering arm, with the usual risk of double-freeing the exit pass's temps.
