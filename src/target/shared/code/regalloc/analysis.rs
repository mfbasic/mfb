//! Instruction-effect model, CFG, and liveness for the liveness-driven
//! allocation strategies (plan-03 Stage B). ISA-neutral: it reads register
//! operands by field role and classifies each value, but names no specific
//! physical register.
//!
//! Performance matters: generated functions (e.g. the regex engine) have
//! thousands of instructions and hundreds of virtual registers, so liveness uses
//! interned register ids and bitsets, and the result is compact — per-virtual-
//! register live *intervals* plus per-instruction physical-occupancy bitsets —
//! so the linear-scan coloring stays near-linear.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::target::shared::regmodel::{RegClass, RegisterModel};

use super::super::types::CodeInstruction;

/// Fields that name a register the instruction *writes*. AArch64 is
/// three-address with no tied operands, so a `dst` field is always a pure
/// definition. `carry_out`/`borrow_out` are the second result of the
/// explicit-carry `add_carry`/`sub_borrow` ops (plan-00-G §4).
const DEF_FIELDS: &[&str] = &["dst", "carry_out", "borrow_out"];

/// Fields that name a register the instruction *reads*. `carry_in`/`borrow_in`
/// are the explicit-carry input of `add_carry`/`sub_borrow`.
const USE_FIELDS: &[&str] = &[
    "src",
    "lhs",
    "rhs",
    "minuend",
    "base",
    "register",
    "addend",
    "carry_in",
    "borrow_in",
];

/// Per-register-class hooks the allocator core queries: which operand strings are
/// this class's virtual registers, and which are its physical registers. The Int
/// class matches `%vN` / `x0`–`x30`; the Fp class matches `%fN` / `d0`–`d31`. A
/// register of the *other* class is invisible to a pass (the two physical files
/// never interfere), so cross-class moves (`fmov x, d`) are handled correctly by
/// each pass seeing only its own operands.
#[derive(Clone, Copy)]
pub(super) struct ClassModel {
    pub(super) parse_vreg: fn(&str) -> Option<u32>,
    pub(super) physical_index: fn(&str) -> Option<u32>,
    /// Whether this is the FP class (selects the FP vs integer clobber sets).
    pub(super) is_fp: bool,
    /// This class's physical registers a PCS call destroys, as a bitmask over
    /// `physical_index`. Derived once per allocation from the target's own
    /// [`RegisterModel::caller_saved`] table by [`caller_saved_mask`] — never a
    /// hand-written per-ISA constant.
    ///
    /// bug-350: it *was* three hand-written constant pairs behind an `is_riscv`
    /// flag, with no x86 arm, so x86-64 silently inherited the AArch64 masks.
    /// The masks are indexed by physical-register *number* and the ISAs number
    /// their registers differently, so AArch64's `d8`–`d15` callee-saved hole
    /// read on x86 as "`xmm8`–`xmm14` survive a call" — which SysV flatly
    /// denies (it has no callee-saved xmm bank at all). Deriving the mask from
    /// the model states the ISA fact exactly once, so it cannot drift and the
    /// next backend cannot inherit the wrong one by omission.
    pub(super) caller_saved: PhysMask,
}

/// Every physical register — forbidding all of them forces a spill across an
/// internal helper call (`_mfb_arena_alloc` tramples callee-saved registers
/// too). Bits above a target's register-number space name no register, so no
/// candidate index can match them and the extra bits are inert on every ISA.
const ALL_PHYS: PhysMask = PhysMask::MAX;

/// Build the call-clobber mask for `class` from the target's own caller-saved
/// register table, mapping each name through this class's physical-index
/// function.
///
/// This is the single statement of "what a call destroys" the allocator uses.
/// `RegisterModel::caller_saved` is the authoritative, per-ISA-maintained list;
/// reading it here (rather than restating it as a constant) is what makes the
/// mask correct by construction on every target, including ones added later
/// (bug-350).
///
/// A name the index function does not recognize contributes no bit. That is
/// correct rather than lossy: such a register is outside the class's index
/// space, so it can never be a coloring candidate and excluding it would be a
/// no-op. (x86's FP table is `xmm0`–`xmm14`; `xmm15` is reserved as the SSE
/// encoder's fixed scratch and absent from the allocatable pool, so its bit
/// could not affect a decision either way.)
pub(super) fn caller_saved_mask(
    model: &dyn RegisterModel,
    class: RegClass,
    physical_index: fn(&str) -> Option<u32>,
) -> PhysMask {
    model
        .caller_saved(class)
        .iter()
        .filter_map(|name| physical_index(name))
        .fold(0, |mask, index| mask | (1u64 << index))
}

/// The set of physical registers (of `is_fp`'s class) a call instruction
/// destroys, so a value live across it must avoid them (plan-03 §4.3). Every case
/// rests on the PCS contract that callee-saved registers (`x19`–`x28`, `d8`–`d15`)
/// survive any call; only the caller-saved set, plus any extra a given callee is
/// known to trample, is clobbered. Modeled per target:
/// - `_mfb_fn_*` / `_mfb_ifn_*` (user/built-in functions, compiled here with a PCS
///   frame that saves the callee-saved registers it uses) and libc clobber only
///   caller-saved registers.
/// - other `_mfb_*` runtime helpers clobber every integer register: their integer
///   clobber sets are unknown to the allocator (the helpers are hand-written and
///   varied), so the conservative `all_int` mask keeps a value out of every
///   caller-saved *and* callee-saved integer register across such a call.
///   (`_mfb_arena_alloc` is itself PCS-framed and preserves `x19`–`x28`; there is
///   no survivor set — see `.ai/compiler.md`.) Their FP clobber still follows the
///   PCS (caller-saved only) — `_mfb_arena_alloc` touches no FP on its fast path
///   and reaches `mmap` (PCS) when it grows.
/// - `blr` is an indirect call to a PCS function; `svc` is a syscall (no FP).
pub(super) fn call_clobber_mask(instruction: &CodeInstruction, model: &ClassModel) -> PhysMask {
    // The PCS clobber set for this class, derived from the target's own
    // caller-saved table (bug-350). `model.is_fp` already selected which class's
    // table was read, so this one value serves both classes.
    let caller_saved = model.caller_saved;
    match instruction.op {
        CodeOp::Svc => {
            // A syscall preserves callee-saved integer registers and touches no FP.
            if model.is_fp {
                0
            } else {
                caller_saved
            }
        }
        CodeOp::BranchLinkRegister => caller_saved,
        CodeOp::BranchLink => {
            let target = instruction.get("target").unwrap_or("");
            let is_runtime_helper = target.starts_with("_mfb_")
                && !target.starts_with("_mfb_fn_")
                && !target.starts_with("_mfb_ifn_");
            if is_runtime_helper && !model.is_fp {
                // A runtime helper: every integer register is treated as
                // destroyed (`_mfb_arena_alloc` tramples callee-saved `x20`–`x28`),
                // because these helpers are hand-written and their integer clobber
                // sets are unknown to the allocator.
                ALL_PHYS
            } else {
                // A compiled user/built-in function (`_mfb_fn_*`/`_mfb_ifn_*`) or
                // libc: PCS, preserves callee-saved. A runtime helper's FP clobber
                // also follows the PCS — `_mfb_arena_alloc` touches no FP on its
                // fast path and reaches `mmap` (PCS) when it grows.
                caller_saved
            }
        }
        _ => 0,
    }
}

/// The integer physical-register index, or `None`. AArch64 `x0`–`x30` map to
/// `0..=30`; x86-64 GPRs (plan-00-H) map to their encoding numbers `0..=15`. A
/// function is single-ISA, so the two name spaces never collide. Excludes
/// `x31`/`xzr`, `sp`/`rsp`, and FP registers.
pub(super) fn int_physical_index(name: &str) -> Option<u32> {
    // AArch64 variant: the `%scratch`/`%sysnr` tokens realize INSIDE the AArch64
    // allocatable file at these indices, so their occupancy is modeled here.
    if let Some(idx) = aarch64_scratch_occupancy_index(name) {
        return Some(idx);
    }
    int_concrete_physical_index(name)
}

/// Non-AArch64 (x86-64 / rv64) integer physical-register index. The
/// `%scratch`/`%sysnr` tokens realize to *different* per-ISA registers on these
/// targets (via each backend's `map_scratch_register` / syscall-nr register) and
/// are lowered to concrete register names before `regalloc::allocate` sees them
/// (plan-34-D), so returning the AArch64-indexed scratch occupancy here would
/// mis-model a non-AArch64 stream. Skip the AArch64 scratch arms entirely; the
/// concrete-register lookup is ISA-neutral (bug-127).
pub(super) fn int_physical_index_non_aarch64(name: &str) -> Option<u32> {
    int_concrete_physical_index(name)
}

/// The AArch64 occupancy index of a `%scratch`/`%sysnr` token, or `None` for any
/// other name. `%scratch0`–`%scratch9` realize `x9`–`x18`, `%scratch10`–`%scratch18`
/// realize `x20`–`x28`, `%sysnr` realizes `x8`, `%sysnr_darwin` realizes `x16`
/// (plan-34-D). The role banks (`%arg`/`%ret`/`%sysarg`/`%sysret`, realizations
/// `x0`–`x7`) are deliberately unparsed — below every allocatable file, so moot.
fn aarch64_scratch_occupancy_index(name: &str) -> Option<u32> {
    if let Some(rest) = name.strip_prefix("%scratch") {
        if let Ok(n) = rest.parse::<u32>() {
            return match n {
                0..=9 => Some(9 + n),
                10..=18 => Some(10 + n),
                _ => None,
            };
        }
    }
    match name {
        "%sysnr" => Some(8),
        "%sysnr_darwin" => Some(16),
        _ => None,
    }
}

/// The concrete integer physical-register index (AArch64 `x0`–`x30`, x86-64 GPRs,
/// or rv64 lp64d ABI names), or `None`. ISA-neutral: a function is single-ISA and
/// the three name spaces never collide.
fn int_concrete_physical_index(name: &str) -> Option<u32> {
    if let Some(rest) = name.strip_prefix('x') {
        if let Ok(n) = rest.parse::<u32>() {
            return (n <= 30).then_some(n);
        }
    }
    // x86-64 GPRs, in encoding order (rax=0 … r15=15). `rsp` is the stack
    // pointer (excluded), like AArch64 `sp`.
    const X86_GPRS: &[&str] = &[
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12",
        "r13", "r14", "r15",
    ];
    if let Some(i) = X86_GPRS
        .iter()
        .position(|&reg| reg == name)
        .filter(|&i| i != 4)
    {
        return Some(i as u32);
    }
    // RISC-V lp64d GPRs, indexed by their register number (`zero`=0 … `t6`=31,
    // plan-99). ABI names are distinct from the AArch64 `x*`/x86 spellings, so
    // this is additive.
    riscv_int_index(name)
}

/// The RISC-V lp64d GPR index (0–31) for an ABI register name, or `None`.
pub(super) fn riscv_int_index(name: &str) -> Option<u32> {
    const RISCV_GPRS: &[&str] = &[
        "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    RISCV_GPRS
        .iter()
        .position(|&reg| reg == name)
        .map(|i| i as u32)
}

/// The FP/SIMD physical-register index, or `None`. AArch64 scalar `d0`–`d31` /
/// vector `v0`–`v31` (aliased) map to `0..=31`; x86-64 `xmm0`–`xmm15`
/// (plan-00-H) to `0..=15`.
pub(super) fn fp_physical_index(name: &str) -> Option<u32> {
    // An `abi::FP_SCRATCH`/`VEC_SCRATCH` token occupies the physical index its
    // realization (`d{i}`/`v{i}`) maps to (plan-34-D). Builder-lowered bodies
    // realize tokens in `Backend::select` before [`allocate`] runs, but the
    // hand-built helper bodies (`finalize_vreg_body_with_locals`: runtime
    // helpers, link thunks) reach the allocator token-bearing — and `d0`–`d7`
    // lead `FP_ALLOCATABLE`, so the token must be visible to `phys_busy_at` or
    // the allocator would color a live `%fN` onto a busy scratch realization.
    if let Some(rest) = name
        .strip_prefix("%fscratch")
        .or_else(|| name.strip_prefix("%vscratch"))
    {
        if let Ok(n) = rest.parse::<u32>() {
            return (n <= 7).then_some(n);
        }
    }
    if let Some(rest) = name.strip_prefix('d').or_else(|| name.strip_prefix('v')) {
        if let Ok(n) = rest.parse::<u32>() {
            return (n <= 31).then_some(n);
        }
    }
    if let Some(n) = name
        .strip_prefix("xmm")
        .and_then(|rest| rest.parse::<u32>().ok())
        .filter(|n| *n <= 15)
    {
        return Some(n);
    }
    // RISC-V FP registers, indexed by their register number (`ft0`=0 … `ft11`=31,
    // plan-99). ABI names start with `f` and are distinct from the AArch64
    // `d*`/`v*` and x86 `xmm*` spellings.
    riscv_fp_index(name)
}

/// The RISC-V FP register index (0–31) for an ABI register name, or `None`.
pub(super) fn riscv_fp_index(name: &str) -> Option<u32> {
    const RISCV_FPRS: &[&str] = &[
        "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
        "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
        "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
    ];
    RISCV_FPRS
        .iter()
        .position(|&reg| reg == name)
        .map(|i| i as u32)
}

impl ClassModel {
    /// Whether `name` is a register this class tracks (a virtual or physical one).
    pub(super) fn is_tracked(&self, name: &str) -> bool {
        (self.parse_vreg)(name).is_some() || (self.physical_index)(name).is_some()
    }
}

/// The registers (of one class) an instruction defines and uses, plus whether it
/// is a call/syscall (clobbers caller-saved registers).
pub(super) struct Effect {
    pub(super) defs: Vec<String>,
    pub(super) uses: Vec<String>,
    pub(super) is_call: bool,
}

pub(super) fn effect(instruction: &CodeInstruction, model: &ClassModel) -> Effect {
    let mut defs = Vec::new();
    let mut uses = Vec::new();
    for (name, value) in &instruction.fields {
        if DEF_FIELDS.contains(name) {
            if model.is_tracked(value) {
                defs.push(value.clone());
            }
        } else if USE_FIELDS.contains(name) && model.is_tracked(value) {
            uses.push(value.clone());
        }
    }
    // Read-modify-write ops accumulate into / select through `dst`, so `dst` is
    // also a SOURCE, not a pure definition. Without this, a spilled accumulator
    // is spilled *after* the op but never reloaded *before* it, so the
    // multiply-add lands on whatever stale value the scratch register held. This
    // only bites under the x86 file's FP pressure — AArch64's 32 vector registers
    // never spill these accumulators, so the same neutral MIR is correct there.
    // (Symptom: log/log10's `k*ln2` double-double lost its low word — cancelling
    // the high word to 0.0 — whenever a prior kernel like `exp` raised FP pressure
    // in the same function.)
    if matches!(
        instruction.op,
        CodeOp::FMlaV | CodeOp::FMlsV | CodeOp::BslV | CodeOp::BitV
    ) {
        if let Some((_, dst)) = instruction.fields.iter().find(|(name, _)| *name == "dst") {
            if model.is_tracked(dst) {
                uses.push(dst.clone());
            }
        }
    }
    let is_call = matches!(
        instruction.op,
        CodeOp::BranchLink | CodeOp::BranchLinkRegister | CodeOp::Svc
    );
    Effect {
        defs,
        uses,
        is_call,
    }
}

/// A basic block: a half-open instruction range `[start, end)` and its
/// successor block indices.
struct Block {
    start: usize,
    end: usize,
    succ: Vec<usize>,
}

fn is_block_terminator(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::Branch
            | CodeOp::BranchEq
            | CodeOp::BranchNe
            | CodeOp::BranchGe
            | CodeOp::BranchLt
            | CodeOp::BranchGt
            | CodeOp::BranchLe
            | CodeOp::BranchVc
            | CodeOp::BranchVs
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::BranchMi
            | CodeOp::BranchLs
            // x86-only conditional branches `select_x86` emits for IEEE float
            // compares (`ucomisd` → jp/jnp/jae/…). The allocator runs AFTER
            // selection, so a block ending in one MUST split here — otherwise its
            // jump-target CFG edge is missing, liveness across the branch is wrong,
            // and a value the branch keeps live gets its register reused → the
            // transcendental (cos/sin/tan/exp) miscompiles under spill pressure.
            | CodeOp::X86Ja
            | CodeOp::X86Jb
            | CodeOp::X86Jbe
            | CodeOp::X86Je
            | CodeOp::X86Jne
            | CodeOp::X86Jae
            | CodeOp::X86Jp
            | CodeOp::X86Jnp
            // rv64 native compare-and-branch `select_riscv64` emits for flagless
            // fused compares (plan-99). Same reasoning as the x86 branches: the
            // allocator runs after selection, so a block ending in one must split
            // here or its jump-target CFG edge and cross-branch liveness are wrong.
            | CodeOp::RvBr
            | CodeOp::Ret
            | CodeOp::BranchSelf
    )
}

fn is_unconditional_terminator(op: CodeOp) -> bool {
    matches!(op, CodeOp::Branch | CodeOp::Ret | CodeOp::BranchSelf)
}

fn build_cfg(instructions: &[CodeInstruction]) -> Vec<Block> {
    let n = instructions.len();
    if n == 0 {
        return Vec::new();
    }
    let mut is_leader = vec![false; n];
    is_leader[0] = true;
    for (i, instruction) in instructions.iter().enumerate() {
        if instruction.op == CodeOp::Label {
            is_leader[i] = true;
        }
        if is_block_terminator(instruction.op) && i + 1 < n {
            is_leader[i + 1] = true;
        }
    }
    let starts: Vec<usize> = (0..n).filter(|&i| is_leader[i]).collect();
    let mut block_of = vec![0usize; n];
    for (block_index, window) in starts.windows(2).enumerate() {
        block_of[window[0]..window[1]].fill(block_index);
    }
    if let Some(&last_start) = starts.last() {
        block_of[last_start..n].fill(starts.len() - 1);
    }
    let mut label_block = HashMap::new();
    for (i, instruction) in instructions.iter().enumerate() {
        if instruction.op == CodeOp::Label {
            if let Some(name) = instruction.get("name") {
                label_block.insert(name.to_string(), block_of[i]);
            }
        }
    }
    let mut blocks: Vec<Block> = Vec::with_capacity(starts.len());
    for (block_index, &start) in starts.iter().enumerate() {
        let end = starts.get(block_index + 1).copied().unwrap_or(n);
        let last = &instructions[end - 1];
        let mut succ = Vec::new();
        if is_block_terminator(last.op) {
            if let Some(target) = last.get("target") {
                if let Some(&tb) = label_block.get(target) {
                    succ.push(tb);
                }
            }
            if !is_unconditional_terminator(last.op) && block_index + 1 < starts.len() {
                succ.push(block_index + 1);
            }
        } else if block_index + 1 < starts.len() {
            succ.push(block_index + 1);
        }
        blocks.push(Block { start, end, succ });
    }
    blocks
}

/// The compact liveness result the coloring consumes.
pub(super) struct Liveness {
    /// Virtual register index -> `[min, max]` instruction indices over which it
    /// is busy. `allocate_register` temporaries are single-def, def-before-use,
    /// and statement-local, so the textual span from first to last occurrence is
    /// a sound, tight live interval (no dataflow needed for virtual registers).
    pub(super) vreg_interval: HashMap<u32, (usize, usize)>,
    /// Per-instruction occupancy of hardcoded physical registers: bit `p` set
    /// means physical `xP` is busy (live, used, or defined) at that instruction.
    /// Physical liveness *does* need dataflow (a value can be live across an
    /// instruction with no operand mentioning it), but over only 31 registers it
    /// fits one machine word, so it is cheap even on huge functions.
    pub(super) phys_busy_at: Vec<PhysMask>,
    /// Call/syscall instructions and the set of this class's physical registers
    /// each one clobbers (`call_clobber_mask`), sorted by instruction index. A
    /// value live across a call must avoid that call's clobbered registers.
    pub(super) call_clobber: Vec<(usize, PhysMask)>,
}

/// Occupancy bitset over physical registers `x0`–`x30` (31 < 64 bits).
pub(super) type PhysMask = u64;

pub(super) fn physical_busy(bits: PhysMask, index: u32) -> bool {
    bits & (1u64 << index) != 0
}

/// Per-instruction **live-out** of the integer physical registers, computed over
/// a fully-colored stream (no virtual registers remain). `live_out[i]` is the set
/// of `x0`–`x30` whose value at the point *after* instruction `i` may still be
/// read before being overwritten. Used by the FP-shuttle peephole to prove a GPR
/// that only carried a float's bit pattern is dead and the shuttle can be dropped.
///
/// A call destroys its caller-saved registers, so they are modeled as definitions
/// (killed) at the call — a value left in one is not live across it.
///
/// `model` is the active backend's register model, threaded in explicitly from
/// the codegen entry point rather than sniffed out of operand strings, so a
/// label or symbol literally spelled like a register on another ISA cannot
/// select the wrong caller-saved set (bug-350; previously an `is_riscv` flag
/// that selected between two hand-written constant pairs and had no x86 arm).
pub(super) fn integer_live_out(
    instructions: &[CodeInstruction],
    model: &dyn RegisterModel,
) -> Vec<PhysMask> {
    let model = ClassModel {
        parse_vreg: |_| None,
        physical_index: int_physical_index,
        is_fp: false,
        caller_saved: caller_saved_mask(model, RegClass::Int, int_physical_index),
    };
    let n = instructions.len();
    let blocks = build_cfg(instructions);
    let nb = blocks.len();

    let mut phys_def: Vec<PhysMask> = vec![0; n];
    let mut phys_use: Vec<PhysMask> = vec![0; n];
    for (i, instruction) in instructions.iter().enumerate() {
        let eff = effect(instruction, &model);
        if eff.is_call {
            phys_def[i] |= call_clobber_mask(instruction, &model);
        }
        for d in &eff.defs {
            if let Some(p) = (model.physical_index)(d) {
                phys_def[i] |= 1u64 << p;
            }
        }
        for u in &eff.uses {
            if let Some(p) = (model.physical_index)(u) {
                phys_use[i] |= 1u64 << p;
            }
        }
    }

    let mut phys_in: Vec<PhysMask> = vec![0; nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut live = 0u64;
            for &s in &blocks[b].succ {
                live |= phys_in[s];
            }
            for i in (blocks[b].start..blocks[b].end).rev() {
                live = (live & !phys_def[i]) | phys_use[i];
            }
            if live != phys_in[b] {
                phys_in[b] = live;
                changed = true;
            }
        }
    }

    let mut live_out: Vec<PhysMask> = vec![0; n];
    for block in &blocks {
        let mut live = 0u64;
        for &s in &block.succ {
            live |= phys_in[s];
        }
        for i in (block.start..block.end).rev() {
            live_out[i] = live;
            live = (live & !phys_def[i]) | phys_use[i];
        }
    }
    live_out
}

/// Run CFG construction and liveness, returning compact per-virtual-register
/// intervals and per-instruction physical occupancy.
///
/// Physical-register liveness uses a single machine word (31 registers).
/// Virtual-register liveness uses sparse interned-id sets — a temporary held
/// across a loop back-edge is live for the whole loop, which a textual span
/// would miss, so real dataflow is required; but the live set at any point is
/// small (statement-local temporaries), so it stays fast even on the
/// multi-thousand-block generated functions.
pub(super) fn analyze(instructions: &[CodeInstruction], model: &ClassModel) -> Liveness {
    let n = instructions.len();
    let blocks = build_cfg(instructions);
    let nb = blocks.len();

    // Per-instruction physical def/use masks, virtual-register def/use id lists,
    // and call indices. Virtual registers are interned to dense ids.
    let mut phys_def: Vec<PhysMask> = vec![0; n];
    let mut phys_use: Vec<PhysMask> = vec![0; n];
    let mut vdef: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut vuse: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut call_clobber: Vec<(usize, PhysMask)> = Vec::new();
    // Virtual-register index -> dense id, and the reverse.
    let mut vid_of: HashMap<u32, u32> = HashMap::new();
    let mut vreg_of: Vec<u32> = Vec::new();
    let intern = |v: u32, vid_of: &mut HashMap<u32, u32>, vreg_of: &mut Vec<u32>| -> u32 {
        *vid_of.entry(v).or_insert_with(|| {
            let id = vreg_of.len() as u32;
            vreg_of.push(v);
            id
        })
    };
    for (i, instruction) in instructions.iter().enumerate() {
        let eff = effect(instruction, model);
        if eff.is_call {
            call_clobber.push((i, call_clobber_mask(instruction, model)));
        }
        for d in &eff.defs {
            if let Some(p) = (model.physical_index)(d) {
                phys_def[i] |= 1u64 << p;
            } else if let Some(v) = (model.parse_vreg)(d) {
                vdef[i].push(intern(v, &mut vid_of, &mut vreg_of));
            }
        }
        for u in &eff.uses {
            if let Some(p) = (model.physical_index)(u) {
                phys_use[i] |= 1u64 << p;
            } else if let Some(v) = (model.parse_vreg)(u) {
                vuse[i].push(intern(v, &mut vid_of, &mut vreg_of));
            }
        }
    }

    // Physical-register liveness (single-word backward dataflow).
    let mut phys_in: Vec<PhysMask> = vec![0; nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut live = 0u64;
            for &s in &blocks[b].succ {
                live |= phys_in[s];
            }
            for i in (blocks[b].start..blocks[b].end).rev() {
                live = (live & !phys_def[i]) | phys_use[i];
            }
            if live != phys_in[b] {
                phys_in[b] = live;
                changed = true;
            }
        }
    }
    let mut phys_busy_at: Vec<PhysMask> = vec![0; n];
    for block in &blocks {
        let mut live = 0u64;
        for &s in &block.succ {
            live |= phys_in[s];
        }
        for i in (block.start..block.end).rev() {
            let live_in_i = (live & !phys_def[i]) | phys_use[i];
            phys_busy_at[i] = live_in_i | phys_def[i];
            live = live_in_i;
        }
    }

    // Virtual-register liveness (sparse backward dataflow over interned ids).
    let mut vin: Vec<std::collections::HashSet<u32>> = vec![std::collections::HashSet::new(); nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut live: std::collections::HashSet<u32> = std::collections::HashSet::new();
            for &s in &blocks[b].succ {
                for &id in &vin[s] {
                    live.insert(id);
                }
            }
            for i in (blocks[b].start..blocks[b].end).rev() {
                for &d in &vdef[i] {
                    live.remove(&d);
                }
                for &u in &vuse[i] {
                    live.insert(u);
                }
            }
            if live != vin[b] {
                vin[b] = live;
                changed = true;
            }
        }
    }
    // Expand to virtual-register intervals: busy(i) = live-in(i) ∪ def(i).
    let mut vreg_interval: HashMap<u32, (usize, usize)> = HashMap::new();
    for block in &blocks {
        let mut live: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for &s in &block.succ {
            for &id in &vin[s] {
                live.insert(id);
            }
        }
        for i in (block.start..block.end).rev() {
            for &d in &vdef[i] {
                live.remove(&d);
            }
            for &u in &vuse[i] {
                live.insert(u);
            }
            let mut note = |id: u32| {
                let v = vreg_of[id as usize];
                let entry = vreg_interval.entry(v).or_insert((i, i));
                entry.0 = entry.0.min(i);
                entry.1 = entry.1.max(i);
            };
            for &id in &live {
                note(id);
            }
            for &d in &vdef[i] {
                note(d);
            }
        }
    }

    Liveness {
        vreg_interval,
        phys_busy_at,
        call_clobber,
    }
}
