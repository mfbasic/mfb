//! x86-64 (System V) register model — plan-00-H.
//!
//! The x86_64 sibling of `arch::aarch64::regmodel`: it answers the same
//! [`RegisterModel`] questions the ISA-neutral allocator core asks, but for the
//! SysV/x86-64 register file (16 GPRs + 16 xmm). The allocator runs unchanged
//! with this model when an `-target linux-x86_64` build is active (selected via
//! `mir::Backend::register_model`).
//!
//! `arena_base` is pinned to `r15` (reserved from allocation, like AArch64 pins
//! `x19`) by a recorded deliberate decision, not an unfinished step:
//! `planning/old-plans/plan-00-H-x86_64-backend.md`'s Phase 4 status states
//! "`arena_base` remains pinned to r15 (works; the optional TLS move is a perf
//! refinement, not a gap — r15 is a valid callee-saved home and no test needs it
//! freed)." Earlier revisions of this comment cited a "plan-00-H §7" that the
//! plan does not contain (it has sections 1–6 and phases 1–4).

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
/// `rsp` (stack), `rbp` (reserved frame register), and `r15` (pinned
/// `arena_base`).
///
/// bug-300 E5: this list used to also name `r14` as a pinned **zero register**,
/// contradicting the pool one line below, which allocates it. x86-64 has no zero
/// register at all: the neutral zero token is realized as an *immediate* zero
/// (`store xzr` → `mov r/m, 0`), which is precisely what freed `r14` for
/// allocation in plan-34-C.
///
/// Tight (4) versus AArch64's 14 and rv64's 12 — the linear-scan allocator
/// spills under pressure. Freeing `arena_base` from `r15` to a TLS slot would
/// add a fifth, but that is a perf refinement the plan deliberately declined,
/// not an unfinished step (see the module comment).
// `r13` is deliberately absent: it realizes the `%closure_env` role token
// ([`X86_64RegisterModel::closure_env`], plan-34-C §2.5), so the allocator must
// never color a body vreg onto it. `r14` (the former zero register) IS allocatable
// now: `store xzr` encodes an immediate zero on x86, so r14 no longer needs to be
// pinned at 0 (plan-34-C — the extra GPR the machine-floor scratch needs).
// `rbx` is deliberately absent: it realizes the `%thread` token
// ([`X86_64RegisterModel::current_thread`]), the program-wide worker
// current-thread register every function must preserve.
const INT_ALLOCATABLE: &[&str] = &["r10", "r11", "r12", "r14"];

/// The register the arena-state pointer is pinned in, program-wide and reserved
/// from allocation. Named (rather than spelled inline) so shared code can identify
/// the active ISA without naming a physical register itself -- the plan-34-D
/// invariant `shared_lowering_names_no_physical_register` enforces. Mirrors
/// `riscv64::regmodel::ARENA_BASE_REGISTER`.
pub(crate) const ARENA_BASE_REGISTER: &str = "r15";

/// The x86-64 stack-pointer spelling selection rewrites the neutral `sp` to.
/// Shared frame finalization recognizes it through this const so the shared
/// source never spells a physical register (plan-34-D).
pub(crate) const STACK_POINTER: &str = "rsp";

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
        ARENA_BASE_REGISTER
    }

    /// SysV x86-64 passes six integer arguments in registers
    /// (rdi, rsi, rdx, rcx, r8, r9); everything past that goes on the stack.
    /// `CALL_ARGS` extends the list with `rax`/`rbp` for arguments 7 and 8, which
    /// is an INTERNAL convention only -- an external C callee reads its 7th and
    /// 8th integer arguments from the stack and would see garbage (bug-296).
    fn external_int_argument_registers(&self) -> usize {
        6
    }

    fn closure_env(&self) -> &'static str {
        // `%closure_env` realizes to `r13` (map_scratch_register(28)); excluded
        // from `INT_ALLOCATABLE` so no body vreg collides with the closure call's
        // hardcoded env write (plan-34-C §2.5).
        "r13"
    }

    fn current_thread(&self) -> &'static str {
        // The `%thread` token realizes to `rbx` (map_scratch_register(20));
        // excluded from `INT_ALLOCATABLE` so every function preserves the worker
        // current-thread control-block pointer the trampoline pins.
        "rbx"
    }

    fn math_pool_base(&self) -> Option<&'static str> {
        // No spare physical to pin (all 16 GPRs are ABI-role, reserved, or in the
        // 4-register allocatable pool) and `x2` is an ABI register remap rewrites
        // per control-flow context — so the base is an allocator-placed vreg.
        None
    }
}

// --- Win64 register model (plan-47-B §4.4) ---------------------------------
//
// A sibling of `X86_64RegisterModel`, NOT an edit to it — SysV stays byte-fact,
// not claim. It diverges in exactly the four methods §4.4 names; every other
// method delegates to the SysV model, since the ISA (spill widths, mnemonics,
// pinned `arena_base`/`%thread`/`%closure_env`, `class_of`) is identical.

// Identical to SysV's four allocatable ints. Win64's internal arg 7/8 use rax/rbp
// (like SysV), so `r10` stays allocatable — a 3-register pool cannot allocate an
// instruction that needs 4 simultaneously-live registers (e.g. `add_carry`), so the
// plan's proposed 3-register Win64 pool was a hard failure, not a perf regression
// (§Corrections). rdi/rsi (Win64 internal args 4/5) remain excluded.
const WIN64_INT_ALLOCATABLE: &[&str] = &["r10", "r11", "r12", "r14"];
// Win64 callee-saved integers: SysV's bank plus `rdi`/`rsi` (caller-saved under
// SysV). The callee-saved xmm bank (xmm6–xmm15) is handled in `is_callee_saved`.
const WIN64_INT_CALLEE_SAVED: &[&str] =
    &["rbx", "rbp", "rdi", "rsi", "r12", "r13", "r14", "r15"];
// Win64 volatile (caller-saved) FP: xmm0–xmm5 only; xmm6–xmm15 are callee-saved.
// Narrowing the volatile set from SysV's "every xmm" is the safe direction — the
// allocator keeps FP values live across a call in the preserved xmm6–xmm15.
// (xmm15 remains the reserved FP scratch, as under SysV — it is simply not
// offered as caller-saved here.)
const WIN64_FP_CALLER_SAVED: &[&str] =
    &["xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5"];

pub(crate) struct Win64RegisterModel;

impl RegisterModel for Win64RegisterModel {
    fn allocatable(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            RegClass::Int => WIN64_INT_ALLOCATABLE,
            // FP allocatable is unchanged; xmm6–xmm15 are simply saved on use
            // (they are callee-saved — see `is_callee_saved`).
            RegClass::Fp => X86_64RegisterModel.allocatable(RegClass::Fp),
        }
    }

    fn class_of(&self, reg: &str) -> Option<RegClass> {
        X86_64RegisterModel.class_of(reg)
    }

    fn is_callee_saved(&self, reg: &str) -> bool {
        if WIN64_INT_CALLEE_SAVED.contains(&reg) {
            return true;
        }
        // xmm6–xmm15 are callee-saved under Win64 (none are under SysV).
        if let Some(n) = reg.strip_prefix("xmm").and_then(|s| s.parse::<u32>().ok()) {
            return (6..=15).contains(&n);
        }
        false
    }

    fn caller_saved(&self, class: RegClass) -> &'static [&'static str] {
        match class {
            // The integer volatile set is left at SysV's — it names rsi/rdi, which
            // Win64 actually preserves, so it is *conservatively* correct for calls
            // out to Windows code (over-saves, never under-saves; §4.4).
            RegClass::Int => X86_64RegisterModel.caller_saved(RegClass::Int),
            RegClass::Fp => WIN64_FP_CALLER_SAVED,
        }
    }

    fn emit_spill(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        X86_64RegisterModel.emit_spill(class, reg, offset)
    }

    fn emit_reload(&self, class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        X86_64RegisterModel.emit_reload(class, reg, offset)
    }

    fn emit_move(&self, dst: &str, src: &str) -> CodeInstruction {
        X86_64RegisterModel.emit_move(dst, src)
    }

    fn spill_slot_bytes(&self) -> usize {
        X86_64RegisterModel.spill_slot_bytes()
    }

    fn arena_base(&self) -> &'static str {
        X86_64RegisterModel.arena_base()
    }

    fn closure_env(&self) -> &'static str {
        X86_64RegisterModel.closure_env()
    }

    fn current_thread(&self) -> &'static str {
        X86_64RegisterModel.current_thread()
    }

    fn math_pool_base(&self) -> Option<&'static str> {
        X86_64RegisterModel.math_pool_base()
    }

    /// A Win64 external C callee reads its first four integer arguments from
    /// rcx/rdx/r8/r9 and everything past that from the stack tail above the shadow
    /// space (§4.2). Mirrors `X86_64RegisterModel`'s 6, for the bug-296 reason.
    fn external_int_argument_registers(&self) -> usize {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_clobber_registers_are_never_allocatable() {
        // bug-284 C6: several x86-64 expansions use rax, rcx and rdx as fixed
        // scratch beyond their named operands -- `div_seq` (quotient/remainder),
        // `var_shift` (CL is the architectural shift count), `msub` (stages the
        // product in rax) and `rbit` (mask in rax, accumulator in rdx). Each is
        // correct only because the allocator never colours a value onto those
        // registers, an invariant those sites previously only asserted in prose
        // (and which bug-125 had already refuted once). Pin it here, so widening
        // the allocatable set fails this test instead of silently miscompiling.
        for reserved in ["rax", "rcx", "rdx"] {
            assert!(
                !INT_ALLOCATABLE.contains(&reserved),
                "{reserved} must stay out of the allocatable set: fixed-register \
                 expansions (div_seq, var_shift, msub, rbit) clobber it"
            );
        }
    }

    #[test]
    fn external_c_abi_passes_six_integer_arguments_not_eight() {
        // bug-296: CALL_ARGS extends the SysV six with rax/rbp for arguments 7 and
        // 8. That extension is INTERNAL -- sound for the compiler's own calls,
        // wrong for an external C callee, which reads those two from the stack. A
        // LINK thunk calls a real C function, so it must see 6 here even though
        // the neutral model shared code uses says 8.
        assert_eq!(X86_64RegisterModel.external_int_argument_registers(), 6);
        assert_eq!(
            crate::target::shared::abi::REGISTER_ARGUMENT_COUNT,
            8,
            "the internal model stays at 8; only the external count differs"
        );
    }

    #[test]
    fn every_model_method() {
        let m = X86_64RegisterModel;
        // allocatable: both classes.
        assert_eq!(m.allocatable(RegClass::Int), INT_ALLOCATABLE);
        assert_eq!(m.allocatable(RegClass::Fp), FP_REGS);
        // caller_saved: both classes.
        assert_eq!(m.caller_saved(RegClass::Int), INT_CALLER_SAVED);
        assert_eq!(m.caller_saved(RegClass::Fp), FP_REGS);
        // class_of covers int, fp, and the None fall-through.
        assert_eq!(m.class_of("r10"), Some(RegClass::Int));
        assert_eq!(m.class_of("xmm15"), Some(RegClass::Fp));
        assert_eq!(m.class_of("nonsense"), None);
        // spill/reload widths and mnemonics per class.
        let sp = m.emit_spill(RegClass::Int, "rbx", 8);
        assert_eq!(sp.op.mnemonic(), "str_u64");
        assert_eq!(sp.get("src"), Some("rbx"));
        assert_eq!(sp.get("base"), Some("rsp"));
        assert_eq!(sp.get("offset"), Some("8"));
        assert_eq!(
            m.emit_spill(RegClass::Fp, "xmm0", 16).op.mnemonic(),
            "str_q"
        );
        let rl = m.emit_reload(RegClass::Int, "rbx", 8);
        assert_eq!(rl.op.mnemonic(), "ldr_u64");
        assert_eq!(rl.get("dst"), Some("rbx"));
        assert_eq!(
            m.emit_reload(RegClass::Fp, "xmm0", 16).op.mnemonic(),
            "ldr_q"
        );
        // spill slot size, move, arena base, math pool base.
        assert_eq!(m.spill_slot_bytes(), 16);
        let mv = m.emit_move("rbx", "r10");
        assert_eq!(mv.op.mnemonic(), "mov");
        assert_eq!(mv.get("dst"), Some("rbx"));
        assert_eq!(mv.get("src"), Some("r10"));
        assert_eq!(m.arena_base(), "r15");
        assert_eq!(m.math_pool_base(), None);
    }

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
    }

    /// The Win64 model diverges from SysV in exactly the four methods §4.4 names
    /// and delegates the rest (plan-47-B).
    #[test]
    fn win64_model_diverges_in_four_methods_and_delegates_the_rest() {
        let win = Win64RegisterModel;
        let sysv = X86_64RegisterModel;

        // (1) 4-register external cap.
        assert_eq!(win.external_int_argument_registers(), 4);
        // (2) four allocatable ints (same as SysV: rax/rbp carry internal args
        // 7/8, so r10 stays allocatable); FP unchanged.
        assert_eq!(win.allocatable(RegClass::Int), &["r10", "r11", "r12", "r14"]);
        assert_eq!(win.allocatable(RegClass::Fp), sysv.allocatable(RegClass::Fp));
        // (3) Win64 callee-saved bank: rsi/rdi and xmm6–xmm15 join.
        assert!(win.is_callee_saved("rsi") && win.is_callee_saved("rdi"));
        assert!(win.is_callee_saved("xmm6") && win.is_callee_saved("xmm15"));
        assert!(!win.is_callee_saved("xmm5")); // volatile
        assert!(win.is_callee_saved("rbx")); // SysV bank still callee-saved
        assert!(!sysv.is_callee_saved("rsi")); // and it is NOT under SysV
        // (4) FP volatile set narrows to xmm0–xmm5; int volatile is left at SysV's
        //     (conservative — it names rsi/rdi, which Win64 actually preserves).
        assert_eq!(
            win.caller_saved(RegClass::Fp),
            &["xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5"]
        );
        assert_eq!(win.caller_saved(RegClass::Int), sysv.caller_saved(RegClass::Int));

        // Delegated: identical ISA (pins, spill widths, mnemonics, class_of).
        assert_eq!(win.arena_base(), "r15");
        assert_eq!(win.current_thread(), "rbx");
        assert_eq!(win.closure_env(), "r13");
        assert_eq!(win.spill_slot_bytes(), 16);
        assert_eq!(win.class_of("r11"), Some(RegClass::Int));
        assert_eq!(win.class_of("xmm3"), Some(RegClass::Fp));
        assert_eq!(
            win.emit_spill(RegClass::Int, "r11", 8).op.mnemonic(),
            "str_u64"
        );
    }
}
