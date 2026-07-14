# bug-121 — riscv64 v128 scalarizer missing FMinV/FMaxV/AbsV → reachable compiler panic (ICE) for math array builtins

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — compile-time panic (no binary) for common `math::`
array builtins on the riscv target.
**Class:** correctness (reachable ICE).

## Finding

`src/arch/riscv64/v128.rs:535` — `scalarize_v128`'s fallback is
`other => panic!("rv64 v128: op {} not yet scalarized")`. The gate `is_v128`
(v128.rs:58-72) classifies `FMinV`, `FMaxV`, `AbsV` (and `SshlV`/`UshlV`) as
scalarizable, but `scalarize_v128` has **no match arms** for them, so they fall
into the panic.

These ops are emitted unconditionally by shared lowering:
- `builder_simd_math.rs:283` (`vector_abs`, `SimdUnaryKernel::AbsInteger` —
  `math::abs(List OF Integer/Fixed)` per builder_math.rs:100-115)
- `builder_simd_math.rs:586-587` (`vector_fmin/fmax` — `math::min/max(List OF
  Float, List OF Float)` per builder_math.rs:571-573)
- `builder_simd_math.rs:774-775` (`vector_fmin/fmax` in `math::clamp(List OF
  Float, …)`)

## Trigger

`mfb build -target linux-riscv64` of any program containing `math::abs(xs)` on a
`List OF Integer`/`Fixed`, or `math::min/max/clamp` on `List OF Float` →
compile-time panic (ICE), no binary. (SshlV/UshlV are in the panic set too but
no builder emits them today.)

## Fix

Add `FMinV`/`FMaxV`/`AbsV` (and SshlV/UshlV) arms to `scalarize_v128`,
lowering each lane to the scalar equivalent (fmin.d/fmax.d, integer abs via
neg+max or shift-based).

## Prior art

Partially known but unfiled — bug-87's matrix notes "(fmin_v build error)" for
riscv and plan-34-D mentions "signzero/riscv fmin_v"; no bug doc covers it, and
AbsV/FMaxV/clamp are not mentioned anywhere.

## Resolution

FIXED in commit e0fa88b8. scalarize_v128 gained FMinV/FMaxV/AbsV arms; Float clamp bounds routed through float_value_as_gpr; validated on the riscv box (port 2229).

Regression test: `tests/rt-behavior/math/bug121_simd_abs_min_max_clamp` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
