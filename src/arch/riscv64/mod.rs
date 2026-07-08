//! The RISC-V 64 (RVA20 / RV64GC) code-generation backend — plan-99.
//!
//! The third ISA after AArch64 and x86-64, and the one that most validates the
//! MIR's neutrality: RISC-V has **no condition flags** (so the flagless MIR is
//! the only reason a compare-and-branch lowers at all) and **no native 128-bit
//! SIMD** on RV64GC (so `v128` ops scalarize to `2× f64`). Hardware FMA is in
//! base `D`, so the ≤1-ULP kernels hold natively.
//!
//! It is additive: it consumes the same neutral MIR (`mir::MirInstruction`) the
//! builders/helpers produce, supplies its own [`regmodel::Riscv64RegisterModel`]
//! for the shared allocator, selects MIR into RV64GC machine ops
//! ([`select::select_riscv64`]), and encodes them — with no edits to the other
//! backends or the shared lowering at the selection/allocation sites (those
//! dispatch through `mir::Backend`).

pub(crate) mod backend;
pub(crate) mod encode;
pub(crate) mod regmodel;
pub(crate) mod reloc;
pub(crate) mod select;
pub(crate) mod v128;
