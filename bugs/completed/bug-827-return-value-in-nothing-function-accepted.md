# bug-827: `RETURN <value>` in an `AS Nothing` function is accepted and dropped

Last updated: 2026-10-05
Effort: small (measured: one predicate)
Severity: MEDIUM
Class: Correctness (type checking)

Status: Fixed (this commit; see `git log -S 'bug-827' -- src/ir/verify/compat.rs`)
Regression Test: `tests/syntax/functions/func-nothing-returns-value-invalid`,
`ir::verify::tests::{rejects_value_return_from_nothing_function,
accepts_nothing_value_return_from_nothing_function}`, and
`rt_error_model_recovery::returning_an_error_from_a_non_error_function_is_a_type_mismatch`.

Found by plan-165-A Phase 1 (2026-10-05), on main `f23af579c`.

## Failing Reproduction

```basic
FUNC f() AS Nothing
  RETURN error(1, "x")      ' also: RETURN 5, RETURN "s"
END FUNC
FUNC main() AS Integer
  f()
  RETURN 0
END FUNC
```

`mfb build` wrote an executable; the returned value was discarded and `f()`
succeeded. Under the error model (`mfb spec language error-model` §8.5, "`RETURN v`
always means function success") a returned `Error` that silently becomes a
successful `NOTHING` is exactly the confusion plan-165-A pins against: the author
plainly meant `FAIL`.

## Root Cause

`ir::verify::compat::check_return_type` returned early when the declared type was
`Nothing`, a skip inherited from the original plan-20-E check (`8b0cd2916`), when a
value-returning `SUB` was policed separately by `TYPE_SUB_CANNOT_RETURN_VALUE`.
Plan-159-D removed `SUB` and that rule; `AS Nothing` functions took over its role
but nothing re-checked their `RETURN` operand.

## Resolution

`check_return_type` checks `Nothing` like any other success type. A `Nothing`
value (`RETURN NOTHING`, `RETURN log("x")`) is assignable; anything else is
`TYPE_RETURN_MISMATCH`. Spec `language/06_functions.md` (effect-only functions)
states the rule.
