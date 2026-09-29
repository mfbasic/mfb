# bug-779: an inline `TRAP` on a call-free expression with a nested raising operator compiles

Last updated: 2026-09-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (a program the spec rejects is accepted)

Status: Fixed
Regression Test: `tests/syntax/trap/inline-trap-call-free-operator-chain-invalid`

`mfb spec language error-model` rule 11 rejects an inline `TRAP` whose scrutinee has
no runtime call anywhere in it (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`): "A raising
operator widens what an inline `TRAP` *covers*, not what may *be* one: the scrutinee
must still contain a runtime call, so `LET d = 1 / z TRAP(e)` is rejected". A
single-operator scrutinee is rejected as specified, but a call-free scrutinee whose
root operator has a **nested** raising operator is accepted and compiles:
`LET v = i * 2 + 3 TRAP(err)`, `(i * 2) + 3`, `i / 2 + 1`. The handler is live code
guarding only operators, which is exactly the shape the rule forbids.

**The single correct behavior a fix produces:** every inline-`TRAP` scrutinee that
contains no runtime call — however many raising operators it nests — is rejected with
`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`; every scrutinee that contains a call keeps its
current lowering (bug-457 check chain, bug-471 operator coverage) unchanged.

## STATUS: FIXED (merged from `worktree-B-779`, fix `9f6965ebc`)

`lower_inline_trap` lifts nested fallible nodes only when the scrutinee contains a
runtime call (`scrutinee_has_runtime_call`: a `Call`/`CallResult` that is not a
package constant, lambda bodies excluded); a call-free scrutinee reaches the verifier
as the bare bind `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` reports on. As designed; no
deviation. The new fixture (LET ×3, assignment, bare statement) is RED on the pre-fix
compiler (all five forms build silently) and green now with five diagnostics. `expectTrap` is unaffected —
`ir/shape.rs` checks it on the HIR (`testing-assert-invalid`). Gates: artifact gate
0 diffs (no existing golden moved), acceptance 1582/1582, full suite 270 ok.

References:

- `src/docs/spec/language/08_error-model.md` rule 11 (quoted above).
- Found by the bug-778 investigation (`bugs/bug-778-every-inline-trap-evaluation-heap-allocates-a-result.md`, "Adjacent finding").

## Failing Reproduction

```basic
' /tmp/t779/src/main.mfb
IMPORT io

FUNC main() AS Integer
  LET i AS Integer = 5
  LET v AS Integer = i * 2 + 3 TRAP(err)
    RETURN 1
  END TRAP
  io::print(toString(v))
  RETURN 0
END FUNC
```

`target/release/mfb build /tmp/t779` (main at `7e53c82b9`):

- Observed: `Wrote executable to ./build/t779.out` — accepted.
- Expected: `error[2-203-0069 TYPE_INLINE_TRAP_REQUIRES_FALLIBLE]: inline TRAP requires a fallible call`.

Contrast (correct today): `LET v = i * 2 TRAP(err)` → `./src/main.mfb:5
error[2-203-0069 TYPE_INLINE_TRAP_REQUIRES_FALLIBLE]` (measured, same build);
`LET c = inner(x) + 1 TRAP(e)` compiles and traps both `inner` and the `+` (rule 11's
blessed shape, bug-471).

## Root Cause

`src/ir/lower.rs:lower_inline_trap` calls `hoist_trap_calls`, which lifts every
nested fallible **call and raising operator** (bug-471) ahead of the residual
expression. The requires-fallible rule is enforced in
`src/ir/verify/resources.rs:check_inline_trap_scrutinee` on the first
`$trap_resN` bind: a `CallResult`/`Call` is a call; a `Checked` is accepted as "a
raising operator the desugar lifted — fallible by construction"; anything else
reports `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`. The lowering's own comment states the
invariant it relies on: "with nothing lifted the root is left unchecked … that bare
bind is the evidence `ir::verify` reports `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` on".

For `i * 2` nothing nested is liftable, so the bare bind reaches the verifier and is
rejected. For `i * 2 + 3` the nested `i * 2` **is** lifted as a `Checked`, so the
first `$trap_res` bind is a `Checked` and the verifier accepts it — the "contains a
runtime call" requirement is never checked once an operator was lifted. (The
`Checked` arm was written for bug-471's `inner(x) + 1`, where a call is also lifted.)

## Goal

- `i * 2 + 3`, `(i * 2) + 3`, `i / 2 + 1`, `-(i * 2)` and a deeper call-free operator
  nest under an inline `TRAP` each report `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`.
- `inner(x) + 1`, `f(1 / z)`, `inner(x) * 2 + 3` and every existing inline-TRAP fixture
  lower and run exactly as before.

### Non-goals (must NOT change)

- The language specification and the memory model: this makes the compiler enforce
  rule 11 as already written; no rule, error code, message text or accepted program
  changes other than the call-free ones the rule already forbids.
- The bug-457/bug-471 lowering of scrutinees that contain a call (IR goldens for those
  must not move).
- Forbidden wrong fix: rejecting every lifted `Checked` (would break rule 11's blessed
  `inner(x) + 1`).

## Blast Radius

- `check_inline_trap_scrutinee`'s `Checked` arm — the site of the bug.
- `testing::desugar` `expectTrap`/`expectNTrap` share the shape but are skipped by the
  `$expect_` guard and have their own rule (`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE`)
  — to be checked with the same call-free operator nest in Phase 1.
- Assignment and bare-statement inline `TRAP` targets (`InlineTrapTarget`) go through
  the same `lower_inline_trap` — covered by the same fix; Phase 1 adds one fixture each.

## Fix Design

Decide "contains a runtime call" on the **scrutinee**, not on the first lifted bind:
in `lower_inline_trap`, when no lifted hoist is a call and the root is not a call,
lower the scrutinee as the plain unchecked bind (no operator hoisting), so the
verifier sees the non-call evidence exactly as it does for `i * 2`. Alternative:
carry a "scrutinee has a call" fact to the verifier on the `$trap_res` bind — rejected
as more plumbing for the same verdict. Expected golden shift: none for accepted
programs (their scrutinees contain a call); new diagnostic goldens only for the new
fixtures.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] `tests/syntax/**` invalid fixtures (sibling of the existing
      `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` fixtures — find them with
      `grep -rl REQUIRES_FALLIBLE tests/syntax`) for `LET`, assignment and bare
      statement forms of `i * 2 + 3`, `i / 2 + 1`, `-(i * 2)`; RED (they compile today).
- [x] Check `expectTrap(i * 2 + 3)` — unaffected (HIR-level rule in `ir/shape.rs`).

Acceptance: fixtures fail for the documented reason.
Commit: `9f6965ebc`

### Phase 2 — the fix

- [x] `lower_inline_trap`: no operator hoisting when the scrutinee has no call.

Acceptance: new fixtures report the diagnostic; bug-457/bug-471 fixtures unchanged.
Commit: `9f6965ebc`

### Phase 3 — full validation

- [x] `scripts/test-accept.sh`, `scripts/artifact-gate.sh all`, full `cargo test`.

Commit: `9f6965ebc` (no golden moved)

## Validation Plan

- Regression tests: the Phase 1 syntax fixtures.
- Runtime proof: the repro reports the error; `inner(x) + 1 TRAP` still catches both.
- Doc sync: none expected (the spec already states the rule).
- Full suite: per `.ai/compiler.md`.

## Summary

A small verifier-evidence gap: lifting a nested operator hides the fact that the
scrutinee has no call. The fix restores the invariant the lowering documents.
