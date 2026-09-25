# bug-674: a `RES` parameter accepts a `STATE` type no handle may carry

Last updated: 2026-09-21
Effort: small (<1h)
Severity: LOW
Class: Correctness (a missing diagnostic)

Status: Fixed
Regression Test: `tests/syntax/resources/resource-param-state-invalid`

**STATUS: FIXED.** Recorded by plan-144-B (findings §3.5, "A `RES` parameter can
declare an invalid `STATE` type"); plan-145-A Open Decision 5 filed it.

## Failing Reproduction

```basic
IMPORT fs
IMPORT json

TYPE P
  j AS json::Json
END TYPE

SUB g(RES h AS fs::File STATE P)
END SUB

FUNC main AS Integer
  RETURN 0
END FUNC
```

- Observed (at `4e0c50a8b`): `Wrote executable`.
- Expected: `error[2-203-0085 TYPE_STATE_INVALID]` on the parameter, as for the same
  `STATE P` on a binding, a record field or a return.

## Root Cause

`STATE T` requires `T` to be a copyable, defaultable data type. The IR verifier
checks it on a binding (`ops.rs`), a record field (`types.rs`, plan-114-E) and a
declared return (`check_return_state_declaration`, `calls.rs`), but its parameter
loop (`verify_function`, `src/ir/verify/mod.rs`) never did. The callee is
unreachable — no declaration can create such a handle to pass — so the harm is a
program the language should reject compiling.

## Fix

The parameter loop applies the same `is_defaultable` test and emits
`TYPE_STATE_INVALID` ("Parameter `h` STATE type `P` must be a copyable, defaultable
data type.").

## Validation

- RED: the reproduction built at `4e0c50a8b`; GREEN: `TYPE_STATE_INVALID` at the
  parameter (the fixture's golden `build.log`).
- Unit: `cargo test --bin mfb rejects_param_state_type_not_defaultable` → ok.
- `scripts/test-accept.sh target/debug/mfb target/accept-actual 'syntax/resources/*'
  'rt-behavior/resources/*'` → "acceptance tests passed (113 test(s) ran)": no
  valid parameter declaration is newly rejected.
- The full suites run in plan-145's final gate (plan-145-I Phase 3).
