# bug-158 — riscv64 `v128.fms` (FMlsV) scalarized as fmul+fsub (two roundings) instead of a fused single-rounding op

Last updated: 2026-07-12
Severity: MEDIUM — riscv-only accuracy divergence in the SIMD transcendental kernels.
Class: Correctness (float fusion contract).
Status: Open

## Finding

`src/arch/riscv64/v128.rs:340-350` (the `CodeOp::FMlsV` arm). It computes
`dst -= lhs*rhs` per lane as `fmul_d ft1, a, b` then `fsub_d ft0, d, ft1` — **two**
IEEE roundings. The op's documented contract (`src/target/shared/code/mir.rs:956`,
"fused dst -= lhs*rhs, single rounding") is a *single*-rounded fused
multiply-subtract, and both peer backends honor it: aarch64 emits fused
`FMLS.2D` (`src/arch/aarch64/encode/emitter.rs:443`) and x86 emits fused
`vfnmadd231pd` (`src/arch/x86_64/encode/emitter.rs:1158-1165`). The sibling
`FMlaV` arm (`v128.rs:326-338`) correctly uses fused `fmadd_d`, making this an
asymmetric oversight.

## Trigger

Any MFBASIC transcendental through the SIMD math kernels — `math::sin/cos/tan`
on a large argument, or the `1 - x*x` step — lowers to `v128.fms` via
`abi::vector_fmls` in the sin/cos/tan argument-reduction paths
(`src/target/shared/code/builder_simd_float_math.rs:572,703,880,942,943`). Where
the product `lhs*rhs` is not exactly representable (`1 - x*x`, `sl - q*cl`), the
extra rounding perturbs the reduced argument, so riscv results diverge from
aarch64/x86 and can exceed the module's stated ≤1-ULP kernel contract. Latent
because riscv HW/ULP validation is limited.

## Fix

Replace the fmul+fsub pair with a single fused `fnmsub_d` (dst=ft0, addend=d-lane,
lhs=a-lane, rhs=b-lane), which the emitter already supports
(`src/arch/riscv64/encode/emitter.rs:308-312`, `NMSUB` = addend − lhs*rhs).
Validate with a ULP no-regression run against the aarch64 reference on riscv HW.
