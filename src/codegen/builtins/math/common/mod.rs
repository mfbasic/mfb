//! Target-generic native lowering for the `math` package.
//!
//! (Slice B placeholder.) Each member's call-site lowering is currently the thin
//! shim in its `func_*.rs`, which delegates to the `CodeBuilder::lower_math_call`
//! dispatcher still resident in `src/target/shared/code/builder_math.rs`. Slice C of
//! the migration relocates the `lower_math_*` dispatchers, `builder_fixed_math`, the
//! SIMD kernels, `builder_simd_float_math`, and `builder_simd_fixed_math` into this
//! module (the STAYS-core float-observation / `emit_pow_scalar` / `emit_alloc_result_list`
//! helpers remain in `src/target/shared/code`).
