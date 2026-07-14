# bug-197: assignment RHS lowered with no expected type → return-type-overloaded call can't disambiguate

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: MEDIUM
Class: correctness

Status: Open
Regression Test: tests/ (return-type-overloaded call on assignment RHS resolves)

`Statement::Assign` and `Statement::StateAssign` lower their RHS with
`expected_type = None`, so a return-type-overloaded call on the RHS cannot be
disambiguated even when the assignment target has a known declared type. The
identical `LET` form passes the expected type and resolves fine, so the two paths
diverge.

## Failing Reproduction

```
FUNC make() AS Integer ... END FUNC
FUNC make() AS String  ... END FUNC
...
MUT n AS Integer = 0
n = make()          ' reports TYPE_OVERLOAD_AMBIGUOUS
```
`LET n AS Integer = make()` resolves; `n = make()` does not. Expected: both
resolve using the target's declared type.

## Root Cause

`src/monomorph/lower.rs:767-780` — the `Assign`/`StateAssign` arms call
`lower_expression` with `expected_type = None`, never consulting
`context.locals.get(name)` (or the resource's state type for `StateAssign`).

## Non-goals

- Do not change `LET` lowering (already correct) or overload rules.

## Blast Radius

- `Assign` and `StateAssign` arms only.

## Fix Design

For `Assign`, look up `context.locals.get(name)` (and the resource state type for
`StateAssign`) and pass it as the expected type into `lower_expression` when the
value is a `Call`.
