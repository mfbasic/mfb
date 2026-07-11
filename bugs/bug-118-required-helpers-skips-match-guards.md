# bug-118 — `required_helpers` never walks MATCH case guards → valid program rejected

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, runtime-specs slice).
**Severity:** MED — a valid program fails to compile with an internal error.
**Class:** correctness (collector asymmetry).

## Finding

`src/target/shared/runtime/usage.rs:180-185` (`push_op_helpers`, `IrOp::Match`
arm) walks only `value` and each `case.body`; `IrMatchCase.guard:
Option<IrValue>` (src/ir/value.rs:7) is skipped. So a runtime-helper call
appearing only in a `WHEN` guard is missing from `NirModule.runtime_helpers`.
`validate_nir`'s walker DOES validate guards into `used_helpers`
(validate.rs:1023-1031), so the strict parity check (validate.rs:96-101) fires.

Corroborating: `plan/symbols.rs`'s
`collect_runtime_symbols_from_ops_with_constants` Match arm also skips guards,
so even without the validate error the helper symbol would never be emitted.

## Trigger

```
MATCH n
  CASE 1 WHEN fs::exists("/tmp/x")
    ...
```
where that guard is the program's only `fs::` use → compile fails with the
internal error `NIR runtime call requires undeclared helper 'fs'`.

## Fix

Add the `case.guard` to the traversal in `push_op_helpers`'s Match arm (and the
matching arm in `plan/symbols.rs`), mirroring what `validate_nir` already
walks.

## Prior art

Same class as fixed bug-45 (`nir-validate-foreach-bind-gap`, collector
asymmetry) — a different, still-open site.
