//! The ISA-neutral register model the allocator core queries (plan-34-B Phase 2,
//! hoisted out of `arch::aarch64::regmodel`).
//!
//! Formalizes the register facts the ISA-neutral allocator core
//! (`crate::target::shared::code::regalloc`) queries: which physical registers
//! exist, which class each belongs to, the caller/callee-saved partition per
//! class, and the spill/reload/move emitters. Each backend supplies its own
//! implementation (`Aarch64RegisterModel`, `X86RegisterModel`,
//! `Riscv64RegisterModel`) without touching the core.
#![allow(dead_code)]

use crate::target::shared::code::CodeInstruction;

/// The two register classes the allocator distinguishes. On AArch64 the
/// floating-point/SIMD class is one physical file (`d_n` ⊂ `v_n`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum RegClass {
    Int,
    Fp,
}

/// The register questions the allocator core asks an ISA. Implemented per backend
/// (`Aarch64RegisterModel`, and the x86_64 / riscv64 siblings).
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

    /// Bytes reserved per stack spill slot — the widest spill this ISA performs.
    /// Every shipping backend (AArch64 and x86-64) overrides this to 16: their FP
    /// spills carry 128-bit SIMD vectors that a 64-bit `str d`/`movsd` would
    /// truncate, so a `str q`/`movups` into a 16-byte slot keeps both lanes. The
    /// `8` default is the scalar-only fallback. Every spill slot (int and fp)
    /// uses this stride uniformly.
    fn spill_slot_bytes(&self) -> usize {
        8
    }

    /// The location this ISA realizes the abstract `arena_base` MIR source as
    /// (`mir.md §7`, plan-00-D §1). The neutral MIR references `arena_base`
    /// wherever it reaches the arena; the backend decides whether that is a
    /// pinned register or a TLS/memory load. AArch64 pins `x19` program-wide
    /// (reserved from allocation — it is absent from [`Self::allocatable`] — and
    /// initialized by the entry sequence); x86_64, with only 16 GPRs, will
    /// realize it as a TLS slot load instead (plan-00-H).
    fn arena_base(&self) -> &'static str;

    /// The register this ISA realizes the `%closure_env` role token as — the
    /// closure environment pointer a closure call site writes just before the
    /// indirect `blr`/`call` (`spec: memory/09_closures.md`). Like
    /// [`Self::arena_base`], it is **absent from [`Self::allocatable`]**: shared
    /// code names it only through the token, and if the allocator could color a
    /// body vreg onto it, coloring a closure's *code* pointer there would let the
    /// hardcoded `move %closure_env, <env>` overwrite the code pointer with the
    /// environment pointer between its definition and the indirect call through it
    /// (plan-34-C §2.5). AArch64 `x28`, x86-64 `r13`, riscv64 `s10`.
    fn closure_env(&self) -> &'static str;

    /// The register this ISA realizes the `%thread` token as — the worker
    /// current-thread control-block pointer the thread trampoline pins across the
    /// worker call so the worker's own `thread::` ops (`is_cancelled` reads it
    /// directly) can find it. Like [`Self::arena_base`], it is a program-wide
    /// pinned register **absent from [`Self::allocatable`]**: shared code names it
    /// only through the `%thread` token, and every function (including the worker
    /// body) must preserve it, so the allocator must never color a body vreg onto
    /// it. AArch64 `x20`, x86-64 `rbx`, riscv64 `s2`.
    fn current_thread(&self) -> &'static str;

    /// The register the SIMD float-math kernels (`builder_simd_float_math`) use
    /// as the constant-pool base: `adrp`/`add` to `_mfb_math_const_pool` once,
    /// then every coefficient `ldr q [base, #offset]`. `Some(reg)` pins a
    /// physical register for the kernel's lifetime; `None` means the base must be
    /// an allocator-placed virtual register.
    ///
    /// The default is `None` — the base is an allocator-placed virtual
    /// register. A backend with a spare physical below its allocatable file
    /// overrides this with a *token* (never a physical spelling — plan-34-D);
    /// AArch64 pins [`crate::target::shared::abi::MATH_POOL`], realized `x2` at
    /// the Phase-3b seam. x86-64 stays `None`: all 16 GPRs are either SysV
    /// ABI-role, reserved (`rsp`/`rbp`/`r15`), or in the four-register
    /// allocatable pool — there is no spare physical to pin, and the realized
    /// `x2` is an ABI register `remap_x86_abi` would rewrite per control-flow
    /// context (rdx as a call-arg, rcx as a result), splitting the base across
    /// the quadrant branch. A vreg lets the allocator place it consistently
    /// (its busy-physical check keeps it off the residual
    /// `map_scratch_register` homes the kernels also use).
    fn math_pool_base(&self) -> Option<&'static str> {
        None
    }

    /// How many integer arguments this target's **external C ABI** passes in
    /// registers.
    ///
    /// bug-296: this is deliberately distinct from the neutral 8-register model
    /// shared code uses for the compiler's own calls. aarch64 (AAPCS64) and
    /// riscv64 both pass 8 and so agree with it, but SysV x86-64 passes only 6 --
    /// its backend extends the internal list with `rax`/`rbp` for arguments 7 and
    /// 8, which is sound for internal calls and wrong for an external callee, which
    /// takes those from the stack. A LINK thunk calls a real C function, so it must
    /// consult this rather than the internal count.
    fn external_int_argument_registers(&self) -> usize {
        crate::target::shared::abi::REGISTER_ARGUMENT_COUNT
    }
}
