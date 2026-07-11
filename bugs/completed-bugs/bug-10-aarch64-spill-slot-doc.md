# bug-10: `spill_slot_bytes` trait doc says AArch64 spills are 8 bytes, but the impl returns 16

Last updated: 2026-07-08
Effort: small (<1h)

The `RegisterModel::spill_slot_bytes` trait default-method doc comment
(`src/arch/aarch64/regmodel.rs:55-58`) states: *"Bytes reserved per stack spill
slot. AArch64 spills are 64-bit, so 8. x86 returns 16: its FP spills carry
128-bit SIMD vectors …"*. This is **factually wrong about AArch64**: the
`Aarch64RegisterModel::spill_slot_bytes` override (`regmodel.rs:177-183`) returns
**16**, not 8, because AArch64 FP virtual registers also carry 128-bit SIMD
vectors and spill via `str q`/`ldr q` into 16-byte slots (`emit_spill`,
`regmodel.rs:185-195`). So *both* backends reserve 16-byte slots; the trait doc's
claim that AArch64 uses 8 and only x86 uses 16 describes a world that no longer
exists (it predates FP vregs carrying 128-bit vectors).

The single correct behavior a fix produces: the trait-level doc describes the
generic per-ISA contract accurately and does not assert a specific AArch64 slot
size that the AArch64 override contradicts.

This is a **documentation defect only** — runtime behavior is correct because
every caller invokes `spill_slot_bytes()` (which returns the correct 16) rather
than trusting the comment. Severity LOW / latent footgun: it would only cause a
real bug if new frame-layout code hardcoded `8 * n` on the strength of the
comment instead of calling the method, silently truncating the high lane of any
spilled 128-bit vector — precisely the corruption the 16-byte override exists to
prevent.

References:

- `src/arch/aarch64/regmodel.rs:55-58` (stale trait default-method doc),
  `:177-183` (`Aarch64RegisterModel::spill_slot_bytes` → 16 with correct comment),
  `:185-195` (`emit_spill` uses `str q` / `vector_store`, confirming 16-byte
  stride), `:253` (unit test asserts `spill_slot_bytes() == 16`).
- `src/arch/x86_64/regmodel.rs` — x86 override also returns 16 (`movups`).
- Found during goal-01 review of `src/arch/aarch64/**`.

## Failing Reproduction

No runtime reproduction — the generated code is correct. The defect is the
contradiction between the comment and the code:

```
# src/arch/aarch64/regmodel.rs
:55  /// ... AArch64 spills are 64-bit, so 8. x86 returns 16 ...   <- claims 8
:182     16                                                        <- returns 16
:253     assert_eq!(model.spill_slot_bytes(), 16);                 <- test proves 16
```

- Observed (doc): "AArch64 spills are 64-bit, so 8."
- Expected (doc): AArch64 reserves 16-byte spill slots (both int and FP use the
  16-byte stride so 128-bit vector spills keep both lanes).

Contrast: the AArch64 override's own local comment (`:178-181`) and `emit_spill`
comment (`:188-192`) are both accurate; only the trait default doc is stale.

## Root Cause

The trait default-method doc (`regmodel.rs:55-58`) was written when AArch64 FP
spills were 64-bit scalars (`str d`, 8-byte slots) and only x86 needed 16-byte
`movups` slots. When AArch64 FP vregs began carrying 128-bit SIMD vectors
(vector::/math kernels), the AArch64 override was widened to 16 and given its own
correct comment, but the trait-level doc's "AArch64 spills are 64-bit, so 8"
clause was never updated. The default `fn spill_slot_bytes(&self) -> usize { 8 }`
is now only a fallback for a hypothetical scalar-only ISA; no shipping backend
uses it.

## Goal

- The trait `spill_slot_bytes` doc describes the generic contract (widest spill
  the ISA performs) without asserting an AArch64-specific value that the override
  contradicts, and does not claim AArch64 uses 8-byte slots.

### Non-goals (must NOT change)

- Do not change the returned value (16) in either override — it is correct.
- Do not change the default `8` unless the team wants a different fallback; the
  fix is the comment.

## Blast Radius

- Documentation-only. Every consumer calls the method; an exhaustive search finds
  no site that hardcodes a spill-slot stride from the comment.

## Fix Design

Reword the trait doc on `regmodel.rs:55-58` to something like: *"Bytes reserved
per stack spill slot — the widest spill this ISA performs. Backends that spill
128-bit SIMD vectors (both AArch64 and x86_64 today) override this to 16 so a
`str q`/`movups` keeps both lanes; the `8` default is the scalar-only
fallback."* Drop the false "AArch64 spills are 64-bit, so 8" clause.

## Phases

### Phase 1 — the fix (doc only)

- [ ] Reword the trait default-method doc; remove the AArch64=8 claim.

Acceptance: comment matches the override (16 for AArch64 and x86); no code change.
Commit: —

### Phase 2 — validation

- [ ] `scripts/test-accept.sh` — must be byte-identical (comment-only change).

Acceptance: zero golden movement.
Commit: —

## Validation Plan

- Regression test(s): none needed (existing `regmodel.rs:253` already asserts 16).
- Runtime proof: none — behavior unchanged.
- Doc sync: this comment is the doc.
- Full suite: `scripts/test-accept.sh`.

## Summary

A one-sentence stale trait doc claims AArch64 reserves 8-byte spill slots; the
AArch64 override in fact returns 16. Pure comment fix, no golden movement, no
runtime effect — filed because it is a doc-vs-behavior contradiction that could
mislead future frame-layout code into truncating vector spills.
