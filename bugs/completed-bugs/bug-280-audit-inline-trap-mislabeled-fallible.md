# bug-280: `mfb audit` reports an inline-`TRAP … RECOVER`-handled call as fallible with propagation `return`

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (security-tooling over-reporting / wrong labeling)

Status: Fixed 2026-07-18
Regression Test: tests/ audit fixture (new) — a fully inline-recovered call does not mark its function (or transitive callers) fallible

A call whose error is fully handled by an inline `TRAP … RECOVER` is still reported
by `mfb audit` as fallible, and its call-site propagation is labeled `return`
(auto-propagates to the caller) — the opposite of what actually happens, since the
inline handler recovers and nothing escapes. This both over-reports (the enclosing
function and its transitive callers are wrongly marked fallible) and mislabels the
propagation edge, contradicting the auditability spec's promise to trace each
auto-propagation edge to the enclosing `TRAP` or function return.

The single correct behavior a fix produces: a call whose enclosing inline
`TRAP … RECOVER` fully handles the error does not cause its function to be reported
fallible, and its call-site propagation is labeled to reflect the inline handler
(e.g. `trap`), not `return`.

References:

- `src/docs/spec/tooling/08_auditability.md` (auto-propagation edges).
- `bugs/completed-bugs/bug-211-*` (fixed inline-TRAP *under*-reporting for
  resources/LINK gates — did not touch fallibility labeling).
- Found during goal-06 review of `src/audit/collect/source.rs`.

## Failing Reproduction

```
FUNC handled() AS String
  LET r = fs::readText("p") TRAP(e) RECOVER "x" END TRAP
  RETURN r
END FUNC
' main calls handled()
```

- Observed: audit reports `handled … (fallible)`, `fallible call fs.readText …
  -> return`, and transitively marks `main` fallible.
- Expected: `handled` is not fallible (no error can escape it); the call site is
  labeled as inline-trapped, not `return`.

## Root Cause

`src/audit/collect/source.rs:20` (`propagation = if has_trap`), `:426`
(`block_escapes`), `:332` (`walk_expression` `Trapped` arm): `block_escapes`
counts a `Trapped` expression as escaping even when the inline handler recovers,
and `CallSite.propagation` is chosen solely from whether the *function* has a
function-level trap — so an inline-trapped call is labeled `return`.

## Goal

- `block_escapes` treats a `Trapped` expression as escaping only if its handler
  itself propagates/fails or contains a fallible call.
- The call site's propagation is labeled `trap` (or a distinct inline-trap label)
  when the call sits under an inline handler.

### Non-goals (must NOT change)

- Reporting of calls whose inline handler *does* re-propagate (those remain
  fallible).
- The audit report format beyond the propagation label value.

## Blast Radius

- `block_escapes` + the propagation-label logic in `source.rs` — fixed here.
- Sibling inline-TRAP audit gaps: bug-283 (F8, resource acquisitions inside
  handler bodies not scanned) is a related but distinct recursion gap.

## Fix Design

In `block_escapes`, when encountering a `Trapped` expression, only treat it as an
escape if the handler statement list propagates (contains a fallible/failing path
that isn't itself recovered). Set the call site's propagation label from the
enclosing handler context. Rejected alternative: excluding `Trapped` calls from the
walk entirely — wrong, because a handler that re-propagates must still count.

## Phases

### Phase 1 — failing fixture
- [ ] Fixture with a fully-recovered inline trap; assert function is not fallible
      and the label is not `return`. Confirm it fails today.
### Phase 2 — the fix
- [ ] Handler-aware escape + label logic.
### Phase 3 — validation
- [ ] Regenerate audit goldens (only the intended fixtures change); full suite green.

## Validation Plan

- Regression: the new fixture + a contrast fixture where the handler re-propagates
  (must stay fallible).
- Doc sync: none (restores documented behavior).

## Summary

The audit conflates "call can fail" with "failure escapes the function"; making the
escape analysis handler-aware fixes both the fallibility over-report and the
propagation mislabel. Risk is only in correctly modeling a re-propagating handler.

## Resolution

The two AST walkers now thread an `in_trap` flag, so a call site knows whether an
inline `TRAP ... RECOVER` lexically contains it.

- `block_escapes` ignores a contained call, so a fully-recovered call no longer
  makes its function -- or, through the fixpoint, every transitive caller --
  fallible.
- The call-site `propagation` label is computed per call (`in_trap || has_trap`)
  instead of from the enclosing function's trap alone, so a contained call reads
  `trap` rather than `return`.

One subtlety the report did not mention: a handler that itself `FAIL`s or
`PROPAGATE`s contains nothing, so the guarded expression keeps its *enclosing*
context (`in_trap || !statements_fail_or_propagate(handler)`) -- which may still
be a trap one level out. Treating every `Trapped` as containing would have
under-reported that case, trading this bug for its mirror image. The handler body
itself is walked in the enclosing context, since that is where its own errors go.

Verified against the repro: `handled` was reported `(fallible)` with
`fs.readText ... -> return`; it is now unmarked with `-> trap`. A sibling
`unhandled` in the same file still reports `(fallible)` / `-> return`, confirming
no over-correction. Acceptance green across all 994 tests with no golden churn.
