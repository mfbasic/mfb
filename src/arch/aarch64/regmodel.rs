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
// `RegClass` + the `RegisterModel` trait were hoisted to the neutral
// `crate::target::shared::regmodel` (plan-34-B Phase 2); re-export them so the
// AArch64 impl below and existing `crate::arch::aarch64::regmodel::…` callers are
// unchanged.
pub(crate) use crate::target::shared::regmodel::{RegClass, RegisterModel};

/// The physical register AArch64 realizes the neutral `arena_base` token as —
/// pinned `x19` program-wide, reserved from allocation (absent from
/// `INT_ALLOCATABLE`). The mirror of RISC-V's `regmodel::ARENA_BASE_REGISTER`
/// (`s11`) and x86-64's `r15`. `select_aarch64` rewrites `arena_base` back to
/// this at selection, so the allocator sees the concrete register. (plan-34-A)
pub(crate) const ARENA_BASE_REGISTER: &str = "x19";

/// The integer registers the bump allocator hands out as temporaries, in the
/// exact order `abi::temporary_register` produced them: caller-saved
/// `x8`–`x17` first, then callee-saved `x20`–`x28`. Keeping this order makes the
/// linear-scan allocator prefer caller-saved scratch (no save/restore cost) and
/// fall through to callee-saved only under pressure, matching the legacy layout.
// `x28` is deliberately absent: it realizes the `%closure_env` role token
// ([`Aarch64RegisterModel::closure_env`], plan-34-C §2.5), so the allocator must
// never color a body vreg onto it — the mirror of `x19`'s (arena-base) exclusion.
// `x20` is deliberately absent: it realizes the `%thread` token
// ([`Aarch64RegisterModel::current_thread`]), the program-wide worker
// current-thread register every function must preserve.
const INT_ALLOCATABLE: &[&str] = &[
    "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x16", "x17", "x21", "x22", "x23", "x24",
    "x25", "x26", "x27",
];

/// Caller-saved integer registers (clobbered by any `bl`). `x16`/`x17` are the
/// platform scratch (IP0/IP1); `x18` is the reserved platform register and is
/// never allocated.
const INT_CALLER_SAVED: &[&str] = &[
    "x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14",
    "x15", "x16", "x17",
];

/// Allocatable scalar FP registers (plan-03 Stage C/D): caller-saved `d0`–`d7`
/// and `d16`–`d31` first (no save/restore cost), then callee-saved `d8`–`d15`
/// for values that must survive a call (their low 64 bits are callee-saved by
/// the PCS, §4.5). `d16`–`d31` joined the pool when the SIMD/transcendental
/// kernels stopped owning them physically — their register file is now FP
/// virtual registers minted at the emit site (`temporary_fp_vreg`), so the
/// allocator places the whole bank; a call-crossing value still lands in
/// `d8`–`d15` via the call-clobber interference, exactly as before.
const FP_ALLOCATABLE: &[&str] = &[
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d16", "d17", "d18", "d19", "d20", "d21",
    "d22", "d23", "d24", "d25", "d26", "d27", "d28", "d29", "d30", "d31", "d8", "d9", "d10", "d11",
    "d12", "d13", "d14", "d15",
];

/// Caller-saved FP registers: `d0`–`d7` and `d16`–`d31` (the low 64 bits of
/// `v0`–`v7` / `v16`–`v31`, the kernel-clobbered set, §4.6).
const FP_CALLER_SAVED: &[&str] = &[
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d16", "d17", "d18", "d19", "d20", "d21",
    "d22", "d23", "d24", "d25", "d26", "d27", "d28", "d29", "d30", "d31",
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
        // The FP/SIMD class is one physical file viewed at three widths: the
        // scalar `dN` (f64), the NEON `vN` (lane view), and the `qN` (the
        // 128-bit `v128` view, plan-00-E). All three name the same register and
        // belong to `RegClass::Fp`, so a `v128` value and a scalar float compete
        // for the same homes.
        if let Some(rest) = reg
            .strip_prefix('d')
            .or_else(|| reg.strip_prefix('v'))
            .or_else(|| reg.strip_prefix('q'))
        {
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

    fn spill_slot_bytes(&self) -> usize {
        // FP virtual registers can carry 128-bit SIMD vectors (the kernels'
        // register file); a 64-bit `str d` spill would drop the high lane, so FP
        // spills are `str q`/`ldr q` into 16-byte slots. Every slot (int and fp)
        // uses this stride uniformly, mirroring x86 (`movups`).
        16
    }

    fn emit_spill(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        match class {
            RegClass::Int => abi::store_u64(reg, abi::stack_pointer(), offset),
            // 128-bit store — a 64-bit `str d` would drop a spilled vector's high
            // lane, corrupting the vector::/math-array kernels. `str q` needs a
            // 16-aligned offset: the slot stride is 16, the spill base is
            // 16-aligned by the callers, and `finalize_frame` shifts by a
            // 16-aligned save area.
            RegClass::Fp => abi::vector_store(reg, abi::stack_pointer(), offset),
        }
    }

    fn emit_reload(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        match class {
            RegClass::Int => abi::load_u64(reg, abi::stack_pointer(), offset),
            RegClass::Fp => abi::vector_load(reg, abi::stack_pointer(), offset),
        }
    }

    fn emit_move(&self, dst: &str, src: &str) -> CodeInstruction {
        abi::move_register(dst, src)
    }

    fn arena_base(&self) -> &'static str {
        // AArch64 pins the arena-state pointer in `x19` program-wide, reserved
        // from allocation. This is the physical realization of the neutral
        // `arena_base` token shared code emits (plan-34-A).
        ARENA_BASE_REGISTER
    }

    fn closure_env(&self) -> &'static str {
        // The `%closure_env` role token realizes to `x28`; excluded from
        // `INT_ALLOCATABLE` so no body vreg collides with a closure call's
        // hardcoded env write (plan-34-C §2.5).
        "x28"
    }

    fn current_thread(&self) -> &'static str {
        // The `%thread` token realizes to `x20`; excluded from
        // `INT_ALLOCATABLE` so every function preserves the worker
        // current-thread control-block pointer the trampoline pins.
        "x20"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_and_banks() {
        let m = Aarch64RegisterModel;
        assert_eq!(m.allocatable(RegClass::Int), INT_ALLOCATABLE);
        assert_eq!(m.allocatable(RegClass::Fp), FP_ALLOCATABLE);
        assert_eq!(m.caller_saved(RegClass::Int), INT_CALLER_SAVED);
        assert_eq!(m.caller_saved(RegClass::Fp), FP_CALLER_SAVED);
        // class_of recognizes the integer, double, vector, and quad spellings.
        assert_eq!(m.class_of("x5"), Some(RegClass::Int));
        assert_eq!(m.class_of("d5"), Some(RegClass::Fp));
        assert_eq!(m.class_of("v5"), Some(RegClass::Fp));
        assert_eq!(m.class_of("q5"), Some(RegClass::Fp));
        // Names the model does not manage return None.
        assert_eq!(m.class_of("sp"), None);
        assert_eq!(m.class_of("xzr"), None);
        assert_eq!(m.class_of("d"), None); // no digits
    }

    #[test]
    fn callee_saved_and_pool_bases() {
        let m = Aarch64RegisterModel;
        assert!(m.is_callee_saved("x20"));
        assert!(m.is_callee_saved("d8"));
        assert!(!m.is_callee_saved("x0"));
        assert!(!m.is_callee_saved("d0"));
        assert_eq!(m.spill_slot_bytes(), 16);
        // AArch64 pins arena_base in x19 and the math-pool base in x2.
        assert_eq!(m.arena_base(), ARENA_BASE_REGISTER);
        assert_eq!(m.math_pool_base(), Some("x2"));
        // The standalone FP callee-saved predicate.
        assert!(is_fp_callee_saved("d15"));
        assert!(!is_fp_callee_saved("d16"));
    }

    #[test]
    fn spill_reload_move_emitters() {
        let m = Aarch64RegisterModel;
        assert_eq!(
            m.emit_spill(RegClass::Int, "x9", 8).op.mnemonic(),
            "str_u64"
        );
        assert_eq!(m.emit_spill(RegClass::Fp, "d9", 16).op.mnemonic(), "str_q");
        assert_eq!(
            m.emit_reload(RegClass::Int, "x9", 8).op.mnemonic(),
            "ldr_u64"
        );
        assert_eq!(m.emit_reload(RegClass::Fp, "d9", 16).op.mnemonic(), "ldr_q");
        let mv = m.emit_move("x0", "x1");
        assert_eq!(mv.op.mnemonic(), "mov");
    }
}
