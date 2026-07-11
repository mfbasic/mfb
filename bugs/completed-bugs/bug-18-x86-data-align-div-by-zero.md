# bug-18: x86 `align()` divides by `alignment` with no guard — a data object with align 0 panics codegen (divide-by-zero)

Last updated: 2026-07-08
Effort: small (<1h)

`src/arch/x86_64/encode/data.rs::align` (`:50-52`) computes
`value.div_ceil(alignment) * alignment`. If a `NativeCodePlan` data object reaches
`encode_data` / the symbol-layout loop with `alignment == 0`, `div_ceil(0)`
panics with a divide-by-zero, aborting codegen with a Rust panic rather than a
clean diagnostic.

The single correct behavior a fix produces: a malformed plan (align 0) yields a
clean codegen error (or align 0 is normalized to 1), never a panic.

Severity LOW / **latent / defense-in-depth**: every data object emitted today
supplies a power-of-two `align >= 1`, so no current path triggers it. Filed
because it converts a malformed/attacker-influenced plan into a panic instead of a
handled error — relevant given the `.mfp` decode trust boundary (PKG-02 already
notes decoded IR is not re-validated; a hostile plan could carry align 0).

References:

- `src/arch/x86_64/encode/data.rs:50-52` (`align`, unguarded division), called
  from `:11,18` (`encode_data`) and `src/arch/x86_64/encode/mod.rs:72` (symbol
  layout loop).
- Related trust-boundary context: audit-1 PKG-02 (decoded IR/plan not
  re-validated before codegen).
- Found during goal-01 review of `src/arch/x86_64/**`.

## Failing Reproduction

None end-to-end (no current path emits align 0). Directly: `align(1, 0)` panics
(`attempt to divide by zero` via `div_ceil(0)`).

- Observed: panic, codegen aborts.
- Expected: clean error, or align 0 treated as align 1.

Contrast: every currently-emitted data object supplies a power-of-two align `>= 1`
(the panic is unobserved).

## Root Cause

`align` (`data.rs:50`) assumes a non-zero alignment and has no guard; the callers
do not validate the field either.

## Goal

- `align` never panics for `alignment == 0`: either return `value` unchanged
  (treat 0 as 1) or surface a clean `Err` from the caller.

### Non-goals (must NOT change)

- Alignment behavior for `alignment >= 1` (correct today).

## Blast Radius

- `align` and its callers (`data.rs:11,18`, `mod.rs:72`). The rv64/aarch64 data
  encoders should be checked for the same unguarded division and hardened
  together if present.

## Fix Design

Guard `alignment == 0` in `align` (treat as 1, i.e. `if alignment <= 1 { return
value }`), or validate the data object's align at plan-decode / encode entry and
return a clean error. Given PKG-02's trust-boundary note, a validating error at
the boundary is preferable to a silent normalization, but normalizing to 1 is an
acceptable minimal fix.

## Phases

### Phase 1 — audit

- [x] Confirm no current path emits align 0 (latent).
- [ ] Check rv64/aarch64 `encode/data.rs` for the same unguarded division.

### Phase 2 — the fix

- [ ] Guard `alignment == 0` (and mirror on the other backends if they share it).

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — byte-identical (no current path uses align 0).

## Validation Plan

- Regression test(s): a unit test asserting `align(v, 0)` does not panic.
- Full suite: `scripts/test-accept.sh`.

## Summary

A one-line unguarded division turns a malformed plan into a panic. Trivial guard;
should be mirrored across the three backends' data encoders.
