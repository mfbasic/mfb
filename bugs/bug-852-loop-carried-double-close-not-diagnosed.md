# bug-852: closing an outer resource inside a loop body is not diagnosed; the second iteration double-closes at run time (`ErrResourceClosed`)

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/compile-fail (or verifier unit) loop-body-close-outer-resource — name suggested

`fs::close(f)` of a resource declared outside a `FOR`/`WHILE` loop, inside the loop body,
compiles. The second iteration closes an already-closed handle and the program dies with
`7-703-0004 ErrResourceClosed`. A straight-line double close is diagnosed at compile time
(`TYPE_USE_AFTER_MOVE`). The move check runs a loop body exactly once against a clone of the
entry set, so a move made in iteration *n* is never seen by the reads of iteration *n+1*.

Correct behavior (per §14.8): `TYPE_USE_AFTER_MOVE` at the close (the loop back-edge makes the
name moved-on-some-path at the body's start), unless the body diverges after the close.

References:

- src/docs/spec/language/14_memory-semantics.md §14.8: "The compiler must diagnose: ... Any
  control-flow path that could drop the same resource or owned value more than once." §14.9
  (loop bodies are branches; "moved on *some* fall-through path" ... "a subsequent use is
  rejected exactly as a definite move would be").
- Note: §14.9 describes the single-pass clone/union mechanism faithfully; it is the §14.8
  guarantee that the mechanism does not meet for the loop back-edge. If the owner decides the
  runtime guard is the intended backstop, this becomes a SPEC fix to §14.8 instead.

## Failing Reproduction

```
IMPORT io
IMPORT fs
FUNC main() AS Integer
  RES f AS fs::File = fs::createTempFile()
  FOR i = 1 TO 2
    fs::close(f)
  NEXT
  RETURN 0
END FUNC
```

(`/tmp/hardening/B03/memory/src/mv_d2.mfb`; `mv_d.mfb` is the `WHILE` form.)

- Observed: builds; run → `Error: 7-703-0004` / `Resource handle is already closed.`, exit 255.
- Expected: compile error `2-203-0055 TYPE_USE_AFTER_MOVE` (Binding `f` was moved...).

Contrast (correct today): straight-line double close (`d_uam.mfb`) → `TYPE_USE_AFTER_MOVE`;
close in loop then read after the loop (`mv_e.mfb`) → `TYPE_USE_AFTER_MOVE`; close then
`EXIT DO` then read after loop (`mv_l.mfb`) → `TYPE_USE_AFTER_MOVE`; close then `RETURN` inside
an IF (`mv_b.mfb`) → accepted.

## Root Cause

`src/ir/verify/resources.rs:check_resource_moves` → `run_branch` processes a loop body once from
`entry` and only merges the branch's moves into the outer set afterward; there is no second pass
(or fixed point) seeding the body with its own fall-through moves for the back-edge.

## Goal

- A loop body whose fall-through path moves an outer name, and which reads that name, is
  rejected; a body that diverges after the move (RETURN/EXIT FOR/EXIT PROGRAM/FAIL) is not.

### Non-goals (must NOT change)

- Loop-local resources (declared inside the body) closing each iteration — must stay legal.
- Runtime closed/moved guards stay.

## Blast Radius

- `check_resource_moves` loop handling (FOR, FOR EACH, WHILE, DO) — fixed by this bug.
- Transfer in a loop body (`thread::transfer(t, f)` each iteration) — same hazard, runtime
  `ErrResourceMoved`; not separately run.

## Fix Design

Run loop bodies twice: second pass seeded with the first pass's fall-through moves of outer
names (enough for a set-union lattice). Note `EXIT FOR`/`EXIT DO` exits must count as leaving
the loop (no back-edge) — do not treat them as fall-through into the next iteration.

## Phases

### Phase 1 — failing test
- [ ] Verifier tests: FOR/WHILE/DO/FOR EACH close-in-body; contrast loop-local close and
      close+EXIT FOR.
Commit: —
### Phase 2 — the fix
- [ ] Two-pass loop body in `check_resource_moves`.
Commit: —
### Phase 3 — full validation
- [ ] Full suite (loop-local resource fixtures must still build).
Commit: —

## Validation Plan

- Regression test(s) above; Runtime proof: repro rejected at compile time; Doc sync: §14.9 add
  a sentence on the back-edge; Full suite.

## Open Decisions

- Diagnose statically (recommended, matches §14.8) vs. narrow §14.8 to "statically provable
  straight-line paths" and rely on the runtime guard.

## Summary

Static gap only; the runtime guard prevents a real double close, so no memory hazard.
