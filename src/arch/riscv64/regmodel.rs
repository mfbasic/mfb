//! RISC-V 64 (RVA20 / RV64GC, Linux lp64d) register model — plan-99.
//!
//! The rv64 sibling of `arch::aarch64::regmodel` / `arch::x86_64::regmodel`: it
//! answers the same [`RegisterModel`] questions the ISA-neutral allocator core
//! asks, for the RISC-V register file (32 GPRs `x0`–`x31`, 32 FP regs `f0`–`f31`,
//! named by their lp64d ABI roles). The allocator runs unchanged with this model
//! when a `-target linux-riscv64` build is active (selected via
//! `mir::Backend::register_model`).
//!
//! RISC-V's 32+32 registers are generous, so — following the plan — `arena_base`
//! is **pinned** to a callee-saved register (`s11`), and a few temporaries are
//! reserved for the instruction-lowering expansions `select_riscv64` performs
//! (compare-and-branch immediate materialization, overflow detection, float
//! compare staging). The remaining registers form a large allocatable pool, so
//! the linear-scan allocator rarely spills.
#![allow(dead_code)]

use crate::arch::aarch64::regmodel::{RegClass, RegisterModel};
use crate::target::shared::code::CodeInstruction;

/// Every integer register by its lp64d ABI name. `class_of` recognizes these as
/// the integer class so the allocator treats each as a busy physical when it
/// appears in the post-selection stream (ABI `a*`, fixed scratch `t0`–`t2`, the
/// pinned `s11`, `sp`, `ra`, `zero`, …).
const GPRS: &[&str] = &[
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// Allocatable integer registers, caller-saved scratch first (no save/restore
/// cost) then callee-saved. Excludes: `zero`/`ra`/`sp`/`gp`/`tp` (fixed roles),
/// `a0`–`a7` (ABI argument/return, placed physically by selection at boundaries),
/// `t0`–`t2` (reserved as `select_riscv64` lowering scratch), `s0` (frame
/// pointer, reserved), and `s11` (pinned `arena_base`). 14 registers — roomy
/// versus x86's 5, so the allocator spills far less.
const INT_ALLOCATABLE: &[&str] = &[
    "t3", "t4", "t5", "t6", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10",
];

/// Caller-saved (volatile) integer registers — clobbered across a `call`.
/// `t0`–`t2` are reserved lowering scratch but are still caller-saved facts.
const INT_CALLER_SAVED: &[&str] = &[
    "ra", "t0", "t1", "t2", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "t3", "t4", "t5", "t6",
];

/// Callee-saved integer registers — survive a `call` (`s0`–`s11`).
const INT_CALLEE_SAVED: &[&str] = &[
    "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11",
];

/// Every FP register by ABI name. RV64GC has no 128-bit vector file; `v128` ops
/// scalarize to `2× f64` (plan-99 §6), so an FP virtual register carries a
/// single `f64`.
const FP_REGS: &[&str] = &[
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
    "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
    "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
];

/// Allocatable FP registers, caller-saved scratch first then callee-saved.
/// Excludes `ft0`/`ft1` (reserved float-compare / non-commutative staging
/// scratch) and `fa0`–`fa7` (ABI argument/return, placed physically at FP call
/// boundaries by selection).
const FP_ALLOCATABLE: &[&str] = &[
    "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "ft8", "ft9", "ft10", "ft11", "fs0", "fs1", "fs2",
    "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9", "fs10", "fs11",
];

/// Caller-saved FP registers — clobbered across a `call` (`ft*` and `fa*`).
const FP_CALLER_SAVED: &[&str] = &[
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fa0", "fa1", "fa2", "fa3", "fa4",
    "fa5", "fa6", "fa7", "ft8", "ft9", "ft10", "ft11",
];

/// Callee-saved FP registers (`fs0`–`fs11`).
const FP_CALLEE_SAVED: &[&str] = &[
    "fs0", "fs1", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9", "fs10", "fs11",
];

/// The register `select_riscv64` realizes the neutral `arena_base` as: pinned
/// `s11`, callee-saved and reserved from allocation, initialized once by the
/// program entry (the rv64 counterpart of AArch64's `x19`).
pub(crate) const ARENA_BASE_REGISTER: &str = "s11";

pub(crate) struct Riscv64RegisterModel;

impl RegisterModel for Riscv64RegisterModel {
    fn allocatable(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            RegClass::Int => INT_ALLOCATABLE,
            RegClass::Fp => FP_ALLOCATABLE,
        }
    }

    fn class_of(&self, reg: &str) -> Option<RegClass> {
        if GPRS.contains(&reg) {
            return Some(RegClass::Int);
        }
        if FP_REGS.contains(&reg) {
            return Some(RegClass::Fp);
        }
        None
    }

    fn is_callee_saved(&self, reg: &str) -> bool {
        INT_CALLEE_SAVED.contains(&reg) || FP_CALLEE_SAVED.contains(&reg)
    }

    fn caller_saved(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            RegClass::Int => INT_CALLER_SAVED,
            RegClass::Fp => FP_CALLER_SAVED,
        }
    }

    fn spill_slot_bytes(&self) -> usize {
        // 16-byte stride keeps every spill offset 16-aligned (matching the shared
        // frame math and the other backends). An FP spill carries only a single
        // `f64` on rv64 (no 128-bit file), stored with `fsd` into the low 8.
        16
    }

    fn emit_spill(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        let mnemonic = match class {
            RegClass::Int => "str_u64",
            RegClass::Fp => "str_d",
        };
        CodeInstruction::new(mnemonic)
            .field("src", reg)
            .field("base", "sp")
            .field("offset", &offset.to_string())
    }

    fn emit_reload(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        let mnemonic = match class {
            RegClass::Int => "ldr_u64",
            RegClass::Fp => "ldr_d",
        };
        CodeInstruction::new(mnemonic)
            .field("dst", reg)
            .field("base", "sp")
            .field("offset", &offset.to_string())
    }

    fn emit_move(&self, dst: &str, src: &str) -> CodeInstruction {
        // Both `mv` (integer) and `fmv.d` (float) are selected from the neutral
        // `mov`/`fmov_d_from_d` — but the allocator only ever moves within a class,
        // and the class is decided by the register names, so `mov` works for int
        // and the FP path uses `fmov_d_from_d` via `emit_move` only for GPRs. The
        // allocator calls this for integer moves; FP moves use the class-specific
        // reload/spill. Use the neutral register move.
        CodeInstruction::new("mov")
            .field("dst", dst)
            .field("src", src)
    }

    fn arena_base(&self) -> &'static str {
        ARENA_BASE_REGISTER
    }

    fn math_pool_base(&self) -> Option<&'static str> {
        // No pinned physical for the SIMD-kernel constant-pool base: let the
        // allocator place it as a virtual register (like x86). The rv64 pool is
        // large, so this is cheap.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_and_banks() {
        let m = Riscv64RegisterModel;
        assert_eq!(m.allocatable(RegClass::Int), INT_ALLOCATABLE);
        assert_eq!(m.allocatable(RegClass::Fp), FP_ALLOCATABLE);
        assert_eq!(m.caller_saved(RegClass::Int), INT_CALLER_SAVED);
        assert_eq!(m.caller_saved(RegClass::Fp), FP_CALLER_SAVED);
        assert_eq!(m.class_of("a0"), Some(RegClass::Int));
        assert_eq!(m.class_of("s11"), Some(RegClass::Int));
        assert_eq!(m.class_of("ft0"), Some(RegClass::Fp));
        assert_eq!(m.class_of("fa0"), Some(RegClass::Fp));
        assert_eq!(m.class_of("nonsense"), None);
        // arena_base and the lowering scratch are reserved from allocation.
        assert!(!m.allocatable(RegClass::Int).contains(&"s11"));
        assert!(!m.allocatable(RegClass::Int).contains(&"t0"));
        assert!(!m.allocatable(RegClass::Fp).contains(&"ft0"));
        for reg in m.allocatable(RegClass::Int) {
            assert!(!["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "sp", "ra", "zero"]
                .contains(reg));
        }
    }

    #[test]
    fn callee_saved_and_spill() {
        let m = Riscv64RegisterModel;
        assert!(m.is_callee_saved("s1"));
        assert!(m.is_callee_saved("fs0"));
        assert!(!m.is_callee_saved("a0"));
        assert!(!m.is_callee_saved("ft0"));
        assert_eq!(m.spill_slot_bytes(), 16);
        assert_eq!(m.arena_base(), "s11");
        assert_eq!(m.math_pool_base(), None);
        let sp = m.emit_spill(RegClass::Int, "s1", 16);
        assert_eq!(sp.op.mnemonic(), "str_u64");
        assert_eq!(sp.get("base"), Some("sp"));
        assert_eq!(m.emit_spill(RegClass::Fp, "fs0", 16).op.mnemonic(), "str_d");
        assert_eq!(m.emit_reload(RegClass::Int, "s1", 16).op.mnemonic(), "ldr_u64");
        assert_eq!(m.emit_reload(RegClass::Fp, "fs0", 16).op.mnemonic(), "ldr_d");
        assert_eq!(m.emit_move("s1", "s2").op.mnemonic(), "mov");
    }
}
