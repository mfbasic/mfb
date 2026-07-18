# bug-286: the most-negative Integer literal `-9223372036854775808` compiles but always traps at runtime

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/ (new) — `LET c AS Integer = -9223372036854775808` builds and evaluates to i64::MIN

Syntaxcheck and `ir::verify` deliberately accept the negated literal `-N` where
`N == i64::MAX + 1` (i.e. `-9223372036854775808`), and spec 04_types.md states the
most-negative Integer literal "is accepted". But lowering only folds the
unary-minus-into-literal for `Fixed` and `Money` (the bug-07 / plan-29-B fix), not
`Integer`. So the Integer case leaves `Unary{-, Const{Integer,
"9223372036854775808"}}`; codegen materializes the u64 bit pattern (= i64::MIN) and
the runtime negate overflows, trapping. The program builds cleanly and then fails
at runtime — a silent-until-run correctness gap in a value the spec says is legal.

The single correct behavior a fix produces: `-9223372036854775808` as an Integer
literal compiles and evaluates to `i64::MIN` with no runtime trap, exactly as the
Fixed/Money minimums already do.

References:

- `src/docs/spec/language/04_types.md:445` (most-negative Integer literal accepted).
- `bugs/completed-bugs/bug-07-fixed-min-literal.md` (the Fixed fold this mirrors).
- Found during goal-06 review of `src/numeric.rs` (root cause in `src/ir/lower.rs`).

## Failing Reproduction

```
LET c AS Integer = -9223372036854775808
```

- Observed: builds cleanly, then at runtime `Error: 7-705-0010 Arithmetic
  overflow…`, exit 255.
- Expected: evaluates to `-9223372036854775808` (i64::MIN).

Contrast (correct today): the positive `9223372036854775808` errors at compile
time; `-9223372036854775807` works.

## Root Cause

`src/ir/lower.rs:3620-3646` (unary-minus literal fold): the fold that negates an
unrepresentable positive magnitude into the literal has arms for `Fixed` and
`Money` only. `integer_literal_in_range` (`src/syntaxcheck/helpers.rs:24`) and
`ir::verify::check_negated_const_literal` both accept the `-(i64::MAX+1)` form, so
the value reaches lowering unfolded and codegen emits a runtime negate that
overflows.

## Goal

- Add the Integer arm to the lower.rs fold: when `type_ == "Integer"`,
  `value.parse::<i64>().is_err()` and `format!("-{value}").parse::<i64>().is_ok()`,
  emit `Const{Integer, "-…"}` — mirroring the Fixed/Money guards.

### Non-goals (must NOT change)

- The compile-time rejection of the positive `9223372036854775808`.
- Fixed/Money folds.

## Blast Radius

- `lower.rs` unary-minus fold — fixed here.
- No other consumer materializes this literal shape (verified: syntaxcheck/verify
  already accept it; only lowering mishandles it).

## Fix Design

Copy the Fixed/Money arm structure for the Integer type. Rejected alternative:
rejecting the literal at syntaxcheck — contradicts the spec, which blesses it.

## Phases

### Phase 1 — failing test
- [ ] Runtime test asserting the literal evaluates to i64::MIN; confirm it traps
      today.
### Phase 2 — the fix
- [ ] Add the Integer fold arm.
### Phase 3 — validation
- [ ] Full suite green; no golden drift except the intended.

## Validation Plan

- Regression: the runtime test + a contrast test for the positive-literal reject.
- Doc sync: none (spec already says accepted).

## Summary

A three-arm fold that covers Fixed and Money but not Integer; adding the Integer
arm closes a spec-blessed literal that currently traps. Low risk, mirrors an
existing fix.
