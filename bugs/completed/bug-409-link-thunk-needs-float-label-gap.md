# bug-409: LINK thunk emits `nan_fail`/`inf_fail` branches for a CDouble struct-field / OUT-slot but never emits the labels → dangling-label build failure

Last updated: 2026-08-01
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (dangling branch label → hard build failure on a valid binding)

STATUS: FIXED (968068c24)

`emit_link_thunk` now derives a broader `needs_float_labels` (the existing
`needs_float` OR `struct_result_has_cdouble_field` OR `out_result_is_cdouble`)
and gates the `nan_fail`/`inf_fail` label emission on it, mirroring
`needs_encoding`'s `struct_has_cstring_field` term. `needs_float` itself stays
narrow — it still gates only the `d0`-return stash, which must not fire for the
stack-loaded struct/OUT cases. Byte-identical for every in-tree binding: the
sole in-tree `CDouble` use (`native-link-sqlite-rt` `columnDouble`) is a direct
ABI return, where `needs_float` was already true, so `needs_float_labels ==
needs_float` there and no golden shifts. Verified: `grep -rl CDouble` finds no
struct-out or OUT-slot CDouble fixture, so artifact-gate would exercise none of
the new paths — the new behaviour is proven by two RED→GREEN unit tests instead.

Regression Test: `src/target/shared/code/link_thunk.rs::tests` —
`struct_out_cdouble_field_emits_float_fail_labels` and
`out_slot_cdouble_emits_float_fail_labels` lower each shape and assert every
label-targeting branch resolves (the exact `CodeFunction::validate` invariant).
Both were RED (dangling `inf_fail`) before the fix, GREEN after.

Commit: 968068c24

In `emit_link_thunk` (`src/target/shared/code/link_thunk.rs`), the `nan_fail` /
`inf_fail` failure epilogues are emitted only when `needs_float` is true, defined
too narrowly (`link_thunk.rs:619`):

```rust
let needs_float = returns_value && function.abi_return_ctype == "CDouble";
```

i.e. only when the *ABI return itself* is a `CDouble` named by `RETURN`. But two
other marshaling paths branch to those same labels regardless:

1. `marshal_struct_out` emits `branch_eq(inf_fail); branch(nan_fail)` for any
   `CDouble` CSTRUCT field (`link_thunk.rs:2505-2506`). A LINK function returning a
   CSTRUCT that maps to a record with a `Float` field (C `struct { int; double }` →
   record `{Integer; Float}`, returned via `RETURN structOut`) has
   `abi_return_ctype = CInt32` and `returns_value = false`, so `needs_float = false`
   and the labels are never emitted.
2. The OUT-slot result arm for a `CDouble` OUT slot branches to the same labels
   (`link_thunk.rs:1270-1271`); `RETURN dblOut` with a status ABI return likewise
   leaves `needs_float = false`.

In both cases the thunk contains `b.eq …_inf_fail` / `b …_nan_fail` with **no
matching label**, which `CodeFunctionPlan::validate` rejects ("branch target label
does not resolve", `src/target/shared/code/validation.rs:161-166`) — a hard build
failure for a valid, intended binding shape.

The asymmetry is the tell: `needs_encoding` (`link_thunk.rs:615`) *does* carry a
`struct_has_cstring_field` term precisely so a struct `CString` field's
`encoding_fail` label is emitted; the parallel `struct_has_cdouble_field` /
OUT-CDouble term for `needs_float` is missing. bug-238's comment (link_thunk.rs
~1224) states a `CDouble` OUT is an intended supported case, so this defeats a
documented feature.

References:

- `src/target/shared/code/link_thunk.rs:619` (`needs_float`), `:615`
  (`needs_encoding` with the `struct_has_cstring_field` term it should mirror),
  `:1270-1271` / `:2505-2506` (arms branching to the labels), `:1135`/`:1631`
  (label emission gated on `needs_float`).
- `cstruct_field_mfb_type("CDouble") = Float` (`src/ir/link.rs:210`),
  `abi_ctype_valid_as_return` (`ir/link.rs:65`). Found during goal-07.

## Failing Reproduction

Static (no in-tree fixture of this shape exists — the only in-tree CDouble use is a
CDouble *return*, which sets `needs_float = true` and works, masking the gap). A
LINK binding returning a CSTRUCT with a `CDouble` field, or a `CDouble` OUT slot,
compiles the branches at `:2505`/`:1270` but not their labels.

- Observed: build fails with `branch target label does not resolve` (validation.rs).
- Expected: the thunk builds and runs the NaN/Inf failure handling for the CDouble
  field/OUT value.

## Root Cause

`needs_float` gates label emission on the ABI-return type only, but two other
marshaling paths (struct-out CDouble field, OUT-slot CDouble) also branch to those
labels.

## Goal

- `needs_float` (or the label-emission condition) includes the
  struct-has-CDouble-field and CDouble-OUT-slot cases, so every path that branches
  to `nan_fail`/`inf_fail` also emits them — mirroring `needs_encoding`'s
  `struct_has_cstring_field` term.

### Non-goals (must NOT change)

- The NaN/Inf failure semantics themselves.

## Blast Radius

- `src/target/shared/code/link_thunk.rs:619` — fix (add the struct-CDouble /
  OUT-CDouble term). The two branch sites (`:1270-1271`, `:2505-2506`) already exist.
