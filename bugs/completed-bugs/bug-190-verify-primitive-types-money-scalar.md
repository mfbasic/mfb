# bug-190: ir::verify PRIMITIVE_TYPES omits Money/Scalar → member-access type-confusion reaches codegen

Last updated: 2026-07-14
Effort: small (<1h)
Severity: HIGH
Class: memory-safety / security

Status: Fixed (2026-07-15) — `Money`/`Scalar` added to `PRIMITIVE_TYPES` and the
`provably_data_type` match arm in `src/ir/verify/mod.rs`.
Regression Test: `src/ir/verify/tests.rs::rejects_member_access_on_money_and_scalar`
(asserts `TYPE_FIELD_ACCESS_REQUIRES_RECORD` on a Money/Scalar-typed MemberAccess,
the same `check`/`collect_diagnostics` entry `merge_packages` uses).

`ir::verify` is the sole rejecter of malformed IR on the untrusted package-merge
path (`merge_packages`). `check_member_access` rejects a `MemberAccess` whose
target provably has a scalar (non-record, non-enum) type by consulting the
`PRIMITIVE_TYPES` list. That list omits `Money` (plan-29) and `Scalar`
(plan-41):

```
src/ir/verify/mod.rs:149
const PRIMITIVE_TYPES: &[&str] = &[
    "Integer", "Float", "String", "Boolean", "Byte", "Fixed", "Nothing",
];
```

Consequently a `MemberAccess { target: Local of declared type "Money" (or
"Scalar"), member: "x", annotated: "Integer" }` in a crafted `.mfp` is **not**
rejected: the target is not an enum; `infer_type` → `"Money"`/`"Scalar"`; it is
not `Thread`; `PRIMITIVE_TYPES.contains(...)` is false (skip); `record_fields`
returns None (skip); `check_member_access_type` also skips because
`field_type("Money", ...)` is None. No diagnostic is emitted, so codegen lowers
the member access as an offset load, treating the scalar register value as a
base pointer → out-of-bounds / arbitrary read in the victim binary. `Integer`
is caught; `Money`/`Scalar` are the gap. This is exactly the PKG-02 type-confusion
class this pass exists to stop.

Note `is_comparable_seen` and `is_defaultable` were correctly updated for both
new primitives (plan-29/41); only `PRIMITIVE_TYPES` (and the analogous
`provably_data_type` match) were missed.

## Failing Reproduction

Construct a `.mfp` whose merged IR contains a `MemberAccess` on a `Money`- or
`Scalar`-typed local with an `Integer` annotation, then build a project that
imports it. Observed: verification passes and codegen emits an offset load on
the scalar value → OOB/garbage read. Expected: `merge_packages` rejects it with
a `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE` diagnostic (as it does for
`Integer`).

## Root Cause

`src/ir/verify/mod.rs:149` `PRIMITIVE_TYPES` is missing `"Money"` and
`"Scalar"`. The same omission exists in the `provably_data_type` match around
`src/ir/verify/mod.rs:2804`, so `RES`-on-Money/Scalar is likewise not caught by
`TYPE_RES_REQUIRES_RESOURCE`.

## Non-goals

- Do not change how `Integer`/`Float`/etc. are handled — only extend the list.
- Do not weaken any existing rule.

## Blast Radius

- `check_member_access` (line ~1794 consumes `PRIMITIVE_TYPES`) — fixed here.
- `provably_data_type` match (~2804) — same-class, fix in the same change.
- Both compile path and merge_packages path use this list; both benefit.

## Fix Design

Add `"Money"` and `"Scalar"` to `PRIMITIVE_TYPES`, and to the
`provably_data_type` match arm. Add a regression test on the package-verify path
asserting a Money/Scalar member access is rejected.
