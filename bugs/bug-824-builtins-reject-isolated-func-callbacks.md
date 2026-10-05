# bug-824: built-in functions reject an `ISOLATED FUNC` where a `FUNC` callback is expected

Last updated: 2026-10-04
Effort: small (guess)
Severity: MEDIUM
Class: Correctness (type rules)

Status: Open
Regression Test: proposed — valid fixtures passing an `ISOLATED FUNC` to built-in
callback parameters, plus an invalid fixture passing a plain `FUNC` where a built-in
requires `ISOLATED FUNC`.

Found while fixing bug-822 (bug-staging-12).

## Rule (owner, 2026-10-04)

"an ISOLATED FUNC is a FUNC. a FUNC is NOT an ISOLATED FUNC."

## Failing Reproduction

`collections::forEach(xs, isolatedFn)` (and every other built-in with a plain
`FUNC` callback parameter: `transform`, `filter`, `reduce`, `sortBy`, `groupBy`,
`json::parse`'s reviver, …) gives `TYPE_CALL_ARGUMENT_MISMATCH` when `isolatedFn`
is an `ISOLATED FUNC`.

## Root Cause

The registry's `unify_at` deliberately accepts only a plain concrete function
for a plain callback parameter ("a plain callback param only a plain concrete").
That is inconsistent with bug-786, which lets an `ISOLATED FUNC` stand in for a
`FUNC` of the same shape in user calls.

## Goal

- An `ISOLATED FUNC` of the right shape is accepted wherever a built-in expects a
  `FUNC`, and it runs as usual.
- A plain `FUNC` is still rejected where a built-in requires `ISOLATED FUNC`
  (e.g. `thread::start`'s worker).
- One rule shared by user calls and built-ins.
- Spec (language functions / type rules) updated to state the rule once.
