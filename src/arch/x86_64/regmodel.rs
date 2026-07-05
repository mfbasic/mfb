//! x86-64 (System V) register model — plan-00-H.
//!
//! The x86_64 sibling of `arch::aarch64::regmodel`: it answers the same
//! [`RegisterModel`] questions the ISA-neutral allocator core asks, but for the
//! SysV/x86-64 register file (16 GPRs + 16 xmm). The allocator runs unchanged
//! with this model when an `-target linux-x86_64` build is active (selected via
//! `mir::Backend::register_model`).
//!
//! `arena_base` is pinned to `r15` for correctness-first bring-up (reserved from
//! allocation, like AArch64 pins `x19`); plan-00-H §7 will move it to a TLS slot
//! load to recover the register under x86's tighter 16-GPR pressure.
#![allow(dead_code)]

use crate::arch::aarch64::regmodel::{RegClass, RegisterModel};
use crate::target::shared::code::CodeInstruction;

/// The 16 general-purpose registers (64-bit names). `class_of` recognizes these
/// as the integer class.
const GPRS: &[&str] = &[
    "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];

/// Allocatable integer registers, caller-saved scratch first then callee-saved.
/// Excludes: the SysV argument/return + implicit registers (`rax`/`rdx` —
/// mul/div and return; `rcx` — variable shift/rotate count; `rsi`/`rdi`/`r8`/`r9`
/// — argument registers placed physically by selection at ABI boundaries),
/// `rsp` (stack), `rbp` (reserved frame register), `r15` (pinned `arena_base`),
/// and `r14` (pinned **zero register** — AArch64 has `xzr`, x86 does not, so
/// `select_x86` realizes `xzr`/`x31` as `r14`, which the entry zeroes once and
/// every function preserves because it is callee-saved and never allocated).
/// Tight (5) versus AArch64's 19 — the linear-scan allocator spills under
/// pressure; correctness-first for bring-up (plan-00-H §7 frees a register by
/// moving arena_base to TLS).
const INT_ALLOCATABLE: &[&str] = &["r10", "r11", "rbx", "r12", "r13"];

/// The pinned zero register `xzr`/`x31` realizes as (see [`INT_ALLOCATABLE`]).
pub(crate) const ZERO_REGISTER: &str = "r14";

/// Caller-saved (volatile) integer registers — clobbered across a `call`.
const INT_CALLER_SAVED: &[&str] = &["rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11"];

/// Callee-saved integer registers — survive a `call`.
const INT_CALLEE_SAVED: &[&str] = &["rbx", "rbp", "r12", "r13", "r14", "r15"];

/// The xmm registers (the FP/SIMD class). SysV makes every xmm caller-saved, so
/// a float live across a `call` must spill (there is no callee-saved bank).
// xmm15 is reserved as a fixed FP scratch (the SSE encoder needs one to stage
// the non-commutative `dst == rhs` subsd/divsd case, which has no in-place form),
// so it is excluded from allocation — mirroring how r14/r15 are reserved for GPR.
const FP_REGS: &[&str] = &[
    "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7", "xmm8", "xmm9", "xmm10",
    "xmm11", "xmm12", "xmm13", "xmm14",
];

pub(crate) struct X86_64RegisterModel;

impl RegisterModel for X86_64RegisterModel {
    fn allocatable(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            RegClass::Int => INT_ALLOCATABLE,
            RegClass::Fp => FP_REGS,
        }
    }

    fn class_of(&self, reg: &str) -> Option<RegClass> {
        if GPRS.contains(&reg) {
            return Some(RegClass::Int);
        }
        if reg.starts_with("xmm") {
            return Some(RegClass::Fp);
        }
        None
    }

    fn is_callee_saved(&self, reg: &str) -> bool {
        // No callee-saved xmm under SysV, so the integer table is the whole set.
        INT_CALLEE_SAVED.contains(&reg)
    }

    fn caller_saved(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            RegClass::Int => INT_CALLER_SAVED,
            RegClass::Fp => FP_REGS,
        }
    }

    fn spill_slot_bytes(&self) -> usize {
        // FP spills carry 128-bit SIMD vectors (vregified v16-v31); 16-byte slots
        // + `movups` keep both lanes. Int spills use the low 8 of their slot.
        16
    }

    fn emit_spill(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        let mnemonic = match class {
            RegClass::Int => "str_u64",
            // 128-bit `movups` — 64-bit `movsd` would drop a spilled vector's high
            // lane, corrupting the vector::/math-array kernels.
            RegClass::Fp => "str_q",
        };
        CodeInstruction::new(mnemonic)
            .field("src", reg)
            .field("base", "rsp")
            .field("offset", &offset.to_string())
    }

    fn emit_reload(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        let mnemonic = match class {
            RegClass::Int => "ldr_u64",
            RegClass::Fp => "ldr_q",
        };
        CodeInstruction::new(mnemonic)
            .field("dst", reg)
            .field("base", "rsp")
            .field("offset", &offset.to_string())
    }

    fn emit_move(&self, dst: &str, src: &str) -> CodeInstruction {
        CodeInstruction::new("mov")
            .field("dst", dst)
            .field("src", src)
    }

    fn arena_base(&self) -> &'static str {
        "r15"
    }

    fn math_pool_base(&self) -> Option<&'static str> {
        // No spare physical to pin (all 16 GPRs are ABI-role, reserved, or in the
        // 5-register allocatable pool) and `x2` is an ABI register remap rewrites
        // per control-flow context — so the base is an allocator-placed vreg.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_and_saved_partition() {
        let m = X86_64RegisterModel;
        assert_eq!(m.class_of("rax"), Some(RegClass::Int));
        assert_eq!(m.class_of("r15"), Some(RegClass::Int));
        assert_eq!(m.class_of("xmm0"), Some(RegClass::Fp));
        assert_eq!(m.class_of("rip"), None);
        // Callee/caller-saved are disjoint and cover the ABI roles.
        assert!(m.is_callee_saved("rbx"));
        assert!(m.is_callee_saved("r15"));
        assert!(!m.is_callee_saved("rax"));
        assert!(!m.is_callee_saved("r10"));
        // arena_base is reserved from allocation.
        assert!(!m.allocatable(RegClass::Int).contains(&m.arena_base()));
        // No allocatable register is an argument/implicit register.
        for reg in m.allocatable(RegClass::Int) {
            assert!(!["rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "rsp", "rbp"].contains(reg));
        }
    }

    #[test]
    fn allocatable_and_caller_saved_banks() {
        let m = X86_64RegisterModel;
        assert_eq!(m.allocatable(RegClass::Int), INT_ALLOCATABLE);
        assert_eq!(m.allocatable(RegClass::Fp), FP_REGS);
        assert_eq!(m.caller_saved(RegClass::Int), INT_CALLER_SAVED);
        // SysV has no callee-saved xmm, so the FP caller-saved set is the whole file.
        assert_eq!(m.caller_saved(RegClass::Fp), FP_REGS);
        // No xmm is callee-saved.
        assert!(!m.is_callee_saved("xmm0"));
    }

    #[test]
    fn spill_reload_move_and_pool_bases() {
        let m = X86_64RegisterModel;
        assert_eq!(m.spill_slot_bytes(), 16);
        // Integer spill/reload use the 64-bit str/ldr; FP use the 128-bit movups.
        let int_spill = m.emit_spill(RegClass::Int, "rbx", 8);
        assert_eq!(int_spill.op.mnemonic(), "str_u64");
        assert_eq!(int_spill.get("src"), Some("rbx"));
        assert_eq!(int_spill.get("base"), Some("rsp"));
        assert_eq!(int_spill.get("offset"), Some("8"));
        assert_eq!(
            m.emit_spill(RegClass::Fp, "xmm3", 16).op.mnemonic(),
            "str_q"
        );
        let int_reload = m.emit_reload(RegClass::Int, "rbx", 8);
        assert_eq!(int_reload.op.mnemonic(), "ldr_u64");
        assert_eq!(int_reload.get("dst"), Some("rbx"));
        assert_eq!(
            m.emit_reload(RegClass::Fp, "xmm3", 16).op.mnemonic(),
            "ldr_q"
        );
        let mov = m.emit_move("rax", "rbx");
        assert_eq!(mov.op.mnemonic(), "mov");
        assert_eq!(mov.get("dst"), Some("rax"));
        assert_eq!(mov.get("src"), Some("rbx"));
        // arena_base is pinned to r15; the math pool base is an allocator vreg.
        assert_eq!(m.arena_base(), "r15");
        assert_eq!(m.math_pool_base(), None);
        // The zero register realizes xzr as r14.
        assert_eq!(ZERO_REGISTER, "r14");
    }
}
