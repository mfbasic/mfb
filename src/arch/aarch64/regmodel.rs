//! AArch64 register model (plan-03-register-allocator §5).
//!
//! Formalizes the register facts the ISA-neutral allocator core
//! (`crate::target::shared::code::regalloc`) queries: which physical registers
//! exist, which class each belongs to, the caller/callee-saved partition per
//! class, and the spill/reload/move emitters. Today these facts were scattered
//! across the `abi::*` primitives and the bump allocator's fixed numbering; the
//! allocator now asks this model instead of hardcoding names, so a future
//! `src/arch/x86_64/` sibling supplies its own description without touching the
//! core.
//!
//! Much of this model (the allocatable/caller-saved banks, the FP class, the
//! spill emitters) is consumed by the liveness-driven strategies in Stage B/C of
//! the plan; it is defined in full now as the ISA description the core queries.
#![allow(dead_code)]

use super::abi;
use crate::target::shared::code::CodeInstruction;

/// The two register classes the allocator distinguishes. On AArch64 the
/// floating-point/SIMD class is one physical file (`d_n` ⊂ `v_n`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum RegClass {
    Int,
    Fp,
}

/// The register questions the allocator core asks an ISA. Implemented by
/// [`Aarch64RegisterModel`] now; an x86_64 sibling implements the same trait.
pub(crate) trait RegisterModel {
    /// Allocatable physical registers for `class`, in allocation-preference
    /// order (caller-saved scratch first, then callee-saved).
    fn allocatable(&self, class: RegClass) -> &'static [&'static str];

    /// The class a physical register name belongs to, or `None` for a name the
    /// allocator does not manage (`sp`, `xzr`, immediates, …).
    fn class_of(&self, reg: &str) -> Option<RegClass>;

    /// Whether `reg` is callee-saved (survives a `bl`).
    fn is_callee_saved(&self, reg: &str) -> bool;

    /// Caller-saved (clobbered-across-call) registers for `class`. A value live
    /// across a `bl` must not be colored into one of these.
    fn caller_saved(&self, class: RegClass) -> &'static [&'static str];

    /// Emit a spill of `reg` (of `class`) to the stack slot at `[sp, #offset]`.
    fn emit_spill(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction;

    /// Emit a reload of `reg` (of `class`) from the stack slot at `[sp, #offset]`.
    fn emit_reload(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction;

    /// Emit a register-to-register move within a class.
    fn emit_move(&self, dst: &str, src: &str) -> CodeInstruction;
}

/// The integer registers the bump allocator hands out as temporaries, in the
/// exact order `abi::temporary_register` produced them: caller-saved
/// `x8`–`x17` first, then callee-saved `x20`–`x28`. Keeping this order makes the
/// linear-scan allocator prefer caller-saved scratch (no save/restore cost) and
/// fall through to callee-saved only under pressure, matching the legacy layout.
const INT_ALLOCATABLE: &[&str] = &[
    "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x16", "x17", "x20", "x21", "x22", "x23",
    "x24", "x25", "x26", "x27", "x28",
];

/// Caller-saved integer registers (clobbered by any `bl`). `x16`/`x17` are the
/// platform scratch (IP0/IP1); `x18` is the reserved platform register and is
/// never allocated.
const INT_CALLER_SAVED: &[&str] = &[
    "x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14",
    "x15", "x16", "x17",
];

/// Allocatable scalar FP registers (plan-03 Stage C/D): caller-saved `d0`–`d7`
/// scratch first, then callee-saved `d8`–`d15` for values that must survive a
/// call (their low 64 bits are callee-saved by the PCS, and the inlined `math::`
/// NEON kernels avoid `v8`–`v15`, §4.6). `d16`–`d31` are caller-saved and
/// kernel-clobbered, so they are never handed out as long-lived homes.
const FP_ALLOCATABLE: &[&str] = &[
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9", "d10", "d11", "d12", "d13", "d14",
    "d15",
];

/// Caller-saved FP registers: `d0`–`d7` and `d16`–`d31` (the low 64 bits of
/// `v0`–`v7` / `v16`–`v31`, the kernel-clobbered set, §4.6).
const FP_CALLER_SAVED: &[&str] = &[
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d16", "d17", "d18", "d19", "d20", "d21", "d22",
    "d23", "d24", "d25", "d26", "d27", "d28", "d29", "d30", "d31",
];

pub(crate) struct Aarch64RegisterModel;

impl RegisterModel for Aarch64RegisterModel {
    fn allocatable(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            RegClass::Int => INT_ALLOCATABLE,
            RegClass::Fp => FP_ALLOCATABLE,
        }
    }

    fn class_of(&self, reg: &str) -> Option<RegClass> {
        if let Some(rest) = reg.strip_prefix('x') {
            if rest.parse::<u8>().is_ok() {
                return Some(RegClass::Int);
            }
        }
        if let Some(rest) = reg.strip_prefix('d').or_else(|| reg.strip_prefix('v')) {
            if rest.parse::<u8>().is_ok() {
                return Some(RegClass::Fp);
            }
        }
        None
    }

    fn is_callee_saved(&self, reg: &str) -> bool {
        abi::is_callee_saved(reg) || is_fp_callee_saved(reg)
    }

    fn caller_saved(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            RegClass::Int => INT_CALLER_SAVED,
            RegClass::Fp => FP_CALLER_SAVED,
        }
    }

    fn emit_spill(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        match class {
            RegClass::Int => abi::store_u64(reg, abi::stack_pointer(), offset),
            RegClass::Fp => abi::store_double(reg, abi::stack_pointer(), offset),
        }
    }

    fn emit_reload(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        match class {
            RegClass::Int => abi::load_u64(reg, abi::stack_pointer(), offset),
            RegClass::Fp => abi::load_double(reg, abi::stack_pointer(), offset),
        }
    }

    fn emit_move(&self, dst: &str, src: &str) -> CodeInstruction {
        abi::move_register(dst, src)
    }
}

/// Whether `reg` is a callee-saved FP register (`d8`–`d15`; the low 64 bits are
/// callee-saved by the AArch64 PCS, §4.5/§4.6).
pub(crate) fn is_fp_callee_saved(reg: &str) -> bool {
    matches!(
        reg,
        "d8" | "d9" | "d10" | "d11" | "d12" | "d13" | "d14" | "d15"
    )
}
