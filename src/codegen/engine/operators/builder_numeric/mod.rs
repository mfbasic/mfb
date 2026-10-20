//! Lowering for the numeric operators and their deterministic `Integer`/`Fixed`
//! helpers: integer and Fixed arithmetic, the checked-multiply and power
//! skeletons, and the numeric conversions. The two large standalone libm ports
//! that used to live here now have their own files — musl `fmod` in
//! `gen_fmod.rs` and fdlibm `pow` in `gen_pow.rs` — and the `String`
//! comparison/ordering lowering moved to `builder_strings/mod.rs` (bug-332 F1).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::operators::BinaryOp;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod binary_emit;
mod comparison;
mod numeric_kernels;
mod operator_lowering;
