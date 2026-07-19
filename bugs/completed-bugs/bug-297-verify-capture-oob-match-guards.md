# bug-297: closure capture-index bounds check is not applied to MATCH patterns/WHEN guards (and param defaults / global initializers) → crafted `.mfp` OOB env read

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Memory-safety / Security (untrusted-package trust boundary)

Status: Fixed
Regression Test: src/ir/verify/tests.rs::rejects_capture_out_of_range_in_match_pattern_and_guard + ::rejects_stray_capture_in_parameter_default_and_global_initializer

The capture-index bounds defense (bug-99 / bug-32) lives in a separate walker,
`check_value_captures` → `walk_captures`; `check_value`/`check_value_depth` never
checks capture indices. In `check_ops` every value position is checked with *both*
`check_value_captures` and `check_value` — except MATCH case patterns and WHEN
guards, which get only `check_value`. So a `Capture{index: 9999}` embedded in a
guard/pattern of a MATCH inside a closure-body `$lambda` function (with
`closure_slots = Some(n)`) passes verification, and codegen lowers it to
`load_u64(CLOSURE_ENV_REGISTER, index*8)` — an out-of-bounds env read in the victim
binary. The same omission applies to parameter defaults and global-binding
initializers. This is not front-end-reachable (source lambdas lower to a single
`RETURN`), so it is purely a package-decode trust-boundary gap: a hand-crafted
`.mfp` triggers it.

The single correct behavior a fix produces: every capture-index in a MATCH
pattern/guard, parameter default, and global initializer is bounds-checked against
the enclosing closure's slot count during `ir::verify`, so no crafted `.mfp` can
produce an out-of-range env read.

References:

- `bugs/completed-bugs/bug-99-*`, `bug-32-*` (capture-bounds cluster — hardened the
  walker but not these call sites), `bug-118` (the analogous collector-asymmetry
  class for `required_helpers`).
- Found during goal-06 review of `src/ir/verify/mod.rs`.

## Failing Reproduction

A hand-crafted `.mfp` whose function `$lambda0` is referenced by
`Closure{name:"$lambda0", captures:[a]}` (1 slot) and whose body is
`MATCH x CASE 1 WHEN <Capture{index:9999}> …`.

- Observed: `check_ops` reaches the guard, calls only `check_value`, the
  out-of-range capture is not rejected → OOB env read in the compiled victim binary.
- Expected: verification rejects the out-of-range capture.

## Root Cause

`src/ir/verify/mod.rs:1202-1224` (`check_ops`, `IrOp::Match` arm) checks patterns
(`:1203/:1207`) and guards (`:1222`) with only `check_value`; `check_value`'s
`Capture` arm is a no-op (`:4999`); the scrutinee is correctly capture-checked at
`:1194`. Parameter defaults (`:268`, before `closure_slots` even exists) and global
initializers (`:358`) reach codegen via the non-closure bug-99 path (garbage env
register) with the same omission.

## Goal

- The `IrOp::Match` arm calls `self.check_value_captures(v, closure_slots)` on each
  pattern value and `check_value_captures(guard, closure_slots)` on each guard.
- Parameter defaults and global initializers are covered (thread `closure_slots`, or
  run a standalone stray-capture pass over those trees).

### Non-goals (must NOT change)

- The front-end lowering (source lambdas never produce this shape).
- The existing scrutinee/other-position capture checks (already correct).

## Blast Radius

- `check_ops` Match arm (patterns + guards) — fixed here.
- Parameter-default (`:268`) and global-initializer (`:358`) sites — same class,
  fix together.
- Audit any other `check_value`-only value position in `check_ops` for the same gap.

## Fix Design

Mirror the scrutinee: add `check_value_captures` calls at the pattern/guard sites and
thread `closure_slots` to the default/global sites (or a standalone stray-capture
pass). Rejected alternative: making `check_value`'s `Capture` arm check bounds — it
lacks the `closure_slots` context there; keep the two-walker split but call both.

## Phases

### Phase 1 — failing test
- [ ] Craft the `.mfp`/IR unit test with an OOB capture in a MATCH guard; confirm it
      verifies today.
### Phase 2 — the fix
- [ ] Add `check_value_captures` at the Match pattern/guard sites + defaults/globals.
### Phase 3 — validation
- [ ] Full `cargo test` green; the crafted input is now rejected; valid closures
      still verify.

## Validation Plan

- Regression: OOB-capture-in-guard rejection test + a valid-capture contrast.
- Doc sync: none.

## Summary

A collector-asymmetry: the capture-bounds walker exists but isn't called at MATCH
guards/patterns/defaults/globals, leaving a crafted-`.mfp` OOB env read. Adding the
missing calls closes it; risk is finding every uncovered value position.

## Resolution

The report's blast radius asked to "audit any other `check_value`-only value
position for the same gap". That audit was done mechanically rather than by
inspection: every `check_value` / `check_value_captures` call site in the file was
listed in source order and paired up. Exactly five `check_value` calls had no
`check_value_captures` sibling — and they are precisely the five the report named,
with none beyond them:

| site | position |
| --- | --- |
| 272 | parameter default |
| 380 | global-binding initializer |
| 1225 | MATCH pattern `Value` |
| 1229 | MATCH pattern `OneOf` |
| 1244 | MATCH `WHEN` guard |

All five now call the walker. The two contexts need different arguments, which is
the substantive part:

- the MATCH sites pass `closure_slots`, so an index is bounds-checked against the
  enclosing closure's slot count exactly as the scrutinee already was;
- a parameter default is evaluated in the **caller's** frame and a global
  initializer runs before any closure exists, so neither has a captured environment
  at all. They pass `None`, which selects `check_value_captures`' existing
  stray-capture rejection — *any* `Capture` there is malformed IR, not merely an
  out-of-range one. That path already existed for bug-99; it simply was never
  reached from these two positions.

### Verified against the unfixed verifier

All four crafted shapes were confirmed to pass verification before the fix: the five
new calls were stripped back out and both tests failed, then restored and both
passed. So the tests exercise the gap rather than merely asserting current
behaviour.

The in-range contrast case matters as much as the rejections: a `Capture{index: 0}`
in a MATCH pattern of a one-slot closure still verifies, so the new calls reject the
crafted shape rather than closures generally.

Not front-end reachable — source lambdas lower to a single `RETURN` — so this was
only ever a crafted-`.mfp` trust-boundary gap, and no legitimate program changes
behaviour. Full `cargo test` green.
