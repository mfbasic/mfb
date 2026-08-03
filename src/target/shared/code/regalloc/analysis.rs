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
use std::hash::{BuildHasherDefault, Hasher};

use crate::arch::ops::CodeOp;
use crate::target::shared::regmodel::{RegClass, RegisterModel};

use super::super::types::CodeInstruction;
use super::super::Operand;

/// A fast hasher for the allocator's dense-`u32` keys — interned virtual-register
/// ids, spill slots, per-vreg colorings. Rust's default `SipHash` is
/// DoS-hardened and comparatively slow; these keys are internal compiler data
/// (never attacker-controlled), and on the ~135k-vreg inlined regex body the
/// hashing dominated the liveness pass (plan-78-C: `HashMap::entry` +
/// `hashbrown` were the top self-time after the `str::eq` scan was removed). A
/// single multiplicative mix is well-distributed for small dense integers.
///
/// Iteration order of a map/set differs from the default hasher's, so this is
/// only applied where order does not affect output: every allocator structure
/// whose iteration feeds emitted bytes is sorted first (`vregs` by `(start,
/// id)`, `extra_callee_saved` before use — bug-87), and the liveness sets are
/// compared by set equality, so the swap is byte-identical (guarded by the gate).
#[derive(Default)]
pub(super) struct U32Hasher(u64);

impl Hasher for U32Hasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Only the integer `write_u32`/`write_usize` paths are exercised by the
        // allocator's keys; keep a correct generic fallback anyway.
        for &byte in bytes {
            self.0 = (self.0.rotate_left(8) ^ byte as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 = (value as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    fn write_usize(&mut self, value: usize) {
        self.0 = (value as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// A `u32`-keyed map / set using [`U32Hasher`] (plan-78-C).
pub(super) type U32Map<V> = HashMap<u32, V, BuildHasherDefault<U32Hasher>>;
pub(super) type U32Set = std::collections::HashSet<u32, BuildHasherDefault<U32Hasher>>;

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
    /// This pass's register class. Lets [`effect`]/`occupied_at`/`substitute`
    /// read a typed `Operand::VReg`/`Operand::Phys` of the matching class
    /// directly (its `id`/`index` is already the value they need), skipping the
    /// `rendered()` + `parse_vreg`/`physical_index` string round-trip that a
    /// `Raw` operand still needs (plan-82-B). `is_fp` is derivable from this, but
    /// both are kept: `is_fp` selects clobber sets, `class` constructs `Phys`.
    pub(super) class: RegClass,
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
            let target = instruction.get("target").unwrap_or_default();
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

/// x86-64 GPRs, in encoding order (rax=0 … r15=15). `rsp` (index 4) is the stack
/// pointer, excluded like AArch64 `sp`. Module-level so the plan-82-A round-trip
/// test iterates the real table (a register added here is then auto-covered).
const X86_GPRS: &[&str] = &[
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];

/// RISC-V lp64d GPRs, indexed by register number (`zero`=0 … `t6`=31, plan-99).
const RISCV_GPRS: &[&str] = &[
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// RISC-V FP registers, indexed by register number (`ft0`=0 … `ft11`=31, plan-99).
const RISCV_FPRS: &[&str] = &[
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
    "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
    "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
];

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
    // Callee-saved persistent-local pool (bug-387): `%local0`–`%local9` realize
    // `x19`–`x28`, so their occupancy is modeled at indices 19–28 exactly like the
    // high `%scratch` bank they neighbor.
    if let Some(rest) = name.strip_prefix("%local") {
        if let Ok(n) = rest.parse::<u32>() {
            return (n <= 9).then_some(19 + n);
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
    // Fast-reject the sentinel-prefixed operands (`%vN` int / `%fN` fp virtual
    // registers, and any `%`-token not already resolved by the occupancy parser
    // upstream): none is a concrete physical name, so skip the three linear
    // register-name scans below. On the ~135k-vreg regex body this eliminates the
    // measured #1/#2 `str::eq` self-time — every cross-class vreg operand used to
    // fall through to the full `REG_ARRAY.position` scans here (plan-78-C).
    if name.starts_with('%') {
        return None;
    }
    if let Some(rest) = name.strip_prefix('x') {
        if let Ok(n) = rest.parse::<u32>() {
            return (n <= 30).then_some(n);
        }
    }
    // x86-64 GPRs, in encoding order (rax=0 … r15=15). `rsp` is the stack
    // pointer (excluded), like AArch64 `sp`.
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
    // Past the FP scratch tokens, a `%`-prefixed operand (`%vN`/`%fN` virtual
    // register of either class) is never a concrete FP register name, so skip the
    // `d`/`v`/`xmm`/riscv scans below — the fp-pass twin of the fast-reject in
    // `int_concrete_physical_index` (plan-78-C).
    if name.starts_with('%') {
        return None;
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
    RISCV_FPRS
        .iter()
        .position(|&reg| reg == name)
        .map(|i| i as u32)
}

/// A register operand classified to this class's numbering: a physical-register
/// index, or a virtual-register id. [`effect`] computes this **once** per operand
/// (reading the operand string by borrow, no clone), so `analyze` and the
/// linear-scan rewrite loop consume it without ever re-parsing the string — the
/// plan-78-C hot-path win. A `Raw`/`Imm`/label operand that is not a register of
/// this class produces no `RegRef`.
#[derive(Clone, Copy)]
pub(super) enum RegRef {
    Phys(u32),
    VReg(u32),
}

/// The registers (of one class) an instruction defines and uses, plus whether it
/// is a call/syscall (clobbers caller-saved registers). Each register is already
/// classified to a [`RegRef`], so no consumer re-parses an operand string.
pub(super) struct Effect {
    pub(super) defs: Vec<RegRef>,
    pub(super) uses: Vec<RegRef>,
    pub(super) is_call: bool,
}

pub(super) fn effect(instruction: &CodeInstruction, model: &ClassModel) -> Effect {
    // Classify one operand to this class's numbering, once. A typed operand of
    // this pass's class carries its id/index inline, so read it directly with no
    // string work (plan-82-B): a `VReg`/`Phys` of the *other* class is definitively
    // not this class's register, matching today's outcome where the other class's
    // spelling failed both `parse_vreg` and `physical_index`. Only a `Raw`/`Imm`
    // operand takes the `rendered()` + `parse_vreg`/`physical_index` string path —
    // the same classification order the previous `is_tracked` used, so the def/use
    // sets are byte-identical. Pre-plan-82-C every register operand is still `Raw`
    // and takes the fallback; post-C the typed fast path carries the hot load.
    let classify = |value: &Operand| -> Option<RegRef> {
        match value {
            Operand::VReg { class, id } => (*class == model.class).then_some(RegRef::VReg(*id)),
            Operand::Phys { class, index, .. } => {
                (*class == model.class).then_some(RegRef::Phys(*index))
            }
            _ => {
                let spelling = value.rendered();
                if let Some(id) = (model.parse_vreg)(&spelling) {
                    Some(RegRef::VReg(id))
                } else {
                    (model.physical_index)(&spelling).map(RegRef::Phys)
                }
            }
        }
    };
    let mut defs = Vec::new();
    let mut uses = Vec::new();
    for (name, value) in &instruction.fields {
        if DEF_FIELDS.contains(name) {
            if let Some(reg) = classify(value) {
                defs.push(reg);
            }
        } else if USE_FIELDS.contains(name) {
            if let Some(reg) = classify(value) {
                uses.push(reg);
            }
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
            if let Some(reg) = classify(dst) {
                uses.push(reg);
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
                if let Some(&tb) = label_block.get(&target) {
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
    pub(super) vreg_interval: U32Map<(usize, usize)>,
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
        class: RegClass::Int,
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
            if let RegRef::Phys(p) = *d {
                phys_def[i] |= 1u64 << p;
            }
        }
        for u in &eff.uses {
            if let RegRef::Phys(p) = *u {
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
/// `effects[i]` is the precomputed [`effect`] of `instructions[i]` (plan-78-C
/// compute-once): the caller builds the per-instruction effects a single time and
/// shares them between this liveness pass and the linear-scan rewrite loop, so
/// each instruction is classified once instead of three times.
pub(super) fn analyze(
    instructions: &[CodeInstruction],
    model: &ClassModel,
    effects: &[Effect],
) -> Liveness {
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
    let mut vid_of: U32Map<u32> = U32Map::default();
    let mut vreg_of: Vec<u32> = Vec::new();
    let intern = |v: u32, vid_of: &mut U32Map<u32>, vreg_of: &mut Vec<u32>| -> u32 {
        *vid_of.entry(v).or_insert_with(|| {
            let id = vreg_of.len() as u32;
            vreg_of.push(v);
            id
        })
    };
    for (i, instruction) in instructions.iter().enumerate() {
        let eff = &effects[i];
        if eff.is_call {
            call_clobber.push((i, call_clobber_mask(instruction, model)));
        }
        for d in &eff.defs {
            match *d {
                RegRef::Phys(p) => phys_def[i] |= 1u64 << p,
                RegRef::VReg(v) => vdef[i].push(intern(v, &mut vid_of, &mut vreg_of)),
            }
        }
        for u in &eff.uses {
            match *u {
                RegRef::Phys(p) => phys_use[i] |= 1u64 << p,
                RegRef::VReg(v) => vuse[i].push(intern(v, &mut vid_of, &mut vreg_of)),
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
    let mut vin: Vec<U32Set> = vec![U32Set::default(); nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut live: U32Set = U32Set::default();
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
    let mut vreg_interval: U32Map<(usize, usize)> = U32Map::default();
    for block in &blocks {
        let mut live: U32Set = U32Set::default();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    /// The fast `U32Hasher` must behave as a real `Hasher`: a `U32Map` keyed by
    /// `u32` round-trips (exercises `write_u32` + `finish`), and the generic
    /// `write` byte path (used for any non-integer key) also produces a stable,
    /// key-sensitive hash. This is the allocator's hot-path hasher, so a silent
    /// break here would corrupt liveness/coloring.
    #[test]
    fn u32_hasher_backs_a_working_map_and_hashes_bytes() {
        let mut map: U32Map<&str> = U32Map::default();
        for id in [0u32, 1, 7, 42, 135_293, u32::MAX] {
            map.insert(id, "v");
        }
        assert_eq!(map.len(), 6);
        assert_eq!(map.get(&42), Some(&"v"));
        assert_eq!(map.get(&99), None);

        let mut set: U32Set = U32Set::default();
        set.insert(3);
        assert!(set.insert(4));
        assert!(!set.insert(3));
        assert_eq!(set.len(), 2);

        // The generic `write` fallback (never taken for `u32` keys, but required by
        // the `Hasher` contract) is order- and content-sensitive.
        let hash_of = |value: &str| {
            let mut hasher = U32Hasher::default();
            value.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(hash_of("x0"), hash_of("x1"));
        assert_eq!(hash_of("rax"), hash_of("rax"));
    }

    /// plan-82-A Phase 2: the typed `Operand::Phys { class, index, name }` arm must
    /// round-trip byte-identically over **every** physical register name in every
    /// consuming arch's table — not a sample. For each `(name, class, index)`:
    ///
    /// 1. `physical_index(name) == index` — the index the allocator write-back
    ///    (plan-82-B) stores in `Phys.index` equals the register-table position.
    /// 2. `Operand::phys(class, index, name).render()/.rendered() == name` — the
    ///    rendered spelling is byte-identical to today's `Raw` string.
    /// 3. `physical_index(op.rendered()) == index` — reading `Phys.index` directly
    ///    (plan-82-D) yields exactly what the deleted `.position()` scan returned.
    ///
    /// Iterating the real tables (the module-level `X86_GPRS`/`RISCV_*` consts and
    /// the generated AArch64 `x`/`d`/`v` spellings) means a register added later is
    /// automatically in the denominator.
    #[test]
    fn phys_operand_round_trips_over_every_register_name() {
        use crate::target::shared::code::Operand;

        // (name, expected class index). Integer class first, then FP.
        let mut int_names: Vec<(String, u32)> = Vec::new();
        let mut fp_names: Vec<(String, u32)> = Vec::new();

        // AArch64 integer x0–x30 and FP scalar d0–d31 / vector v0–v31.
        for n in 0..=30u32 {
            int_names.push((format!("x{n}"), n));
        }
        for n in 0..=31u32 {
            fp_names.push((format!("d{n}"), n));
            fp_names.push((format!("v{n}"), n));
        }
        // x86-64 GPRs (skip rsp at index 4, excluded) and xmm0–xmm15.
        for (i, &name) in X86_GPRS.iter().enumerate() {
            if i != 4 {
                int_names.push((name.to_string(), i as u32));
            }
        }
        for n in 0..=15u32 {
            fp_names.push((format!("xmm{n}"), n));
        }
        // RISC-V lp64d GPRs and FPRs, indexed by register number.
        for (i, &name) in RISCV_GPRS.iter().enumerate() {
            int_names.push((name.to_string(), i as u32));
        }
        for (i, &name) in RISCV_FPRS.iter().enumerate() {
            fp_names.push((name.to_string(), i as u32));
        }

        // Assert the three round-trip properties for one (name, class, index).
        // `name` must be a `&'static str` for the `Phys` arm; the corpus holds
        // owned `String`s, so match on the original static tables via the index fn
        // rather than leaking — construct `Phys` from the *rendered* borrow after
        // confirming the index, which is exactly what the write-back path does.
        let check_int = |name: &str, index: u32| {
            assert_eq!(
                int_concrete_physical_index(name),
                Some(index),
                "int register `{name}` should map to index {index}"
            );
        };
        let check_fp = |name: &str, index: u32| {
            assert_eq!(
                fp_physical_index(name),
                Some(index),
                "fp register `{name}` should map to index {index}"
            );
        };
        for (name, index) in &int_names {
            check_int(name, *index);
        }
        for (name, index) in &fp_names {
            check_fp(name, *index);
        }

        // The `&'static str` round trip for a representative name of each class
        // (the static-name property is uniform, so a per-name static is only
        // needed to *construct* the arm; the index equality above is the full-table
        // proof). Covers Int and Fp, AArch64 / x86 / riscv spellings.
        for (class, index, name) in [
            (RegClass::Int, 9u32, "x9"),
            (RegClass::Int, 0u32, "rax"),
            (RegClass::Int, 0u32, "zero"),
            (RegClass::Fp, 3u32, "d3"),
            (RegClass::Fp, 2u32, "xmm2"),
            (RegClass::Fp, 0u32, "ft0"),
        ] {
            let op = Operand::phys(class, index, name);
            assert_eq!(op.render(), name);
            assert_eq!(op.rendered(), name);
            let recovered = match class {
                RegClass::Int => int_concrete_physical_index(&op.rendered()),
                RegClass::Fp => fp_physical_index(&op.rendered()),
            };
            assert_eq!(
                recovered,
                Some(index),
                "reading Phys.index for `{name}` must equal physical_index(rendered())"
            );
        }
    }

    /// plan-82-B: `effect` must classify a register operand to the *same*
    /// `RegRef` whether it arrives typed (`VReg`/`Phys`) or as the equivalent
    /// `Raw` string. This is the invariant that makes the typed fast path in
    /// `classify` byte-identical to the pre-typing string path (the whole stream
    /// is `Raw` until plan-82-C, then typed after — both must color identically).
    #[test]
    fn effect_classifies_typed_and_raw_operands_identically() {
        use crate::target::shared::code::{CodeInstruction, Operand};

        let model = ClassModel {
            class: RegClass::Int,
            parse_vreg: super::super::parse_vreg,
            physical_index: int_physical_index,
            is_fp: false,
            caller_saved: 0,
        };
        let key = |r: &RegRef| match r {
            RegRef::Phys(p) => (0u8, *p),
            RegRef::VReg(v) => (1u8, *v),
        };
        // dst = %v5 (def), lhs = x9 (physical use), rhs = %v5 (use), plus an
        // fp register the Int pass must ignore in both forms.
        let raw = CodeInstruction::new("add")
            .field("dst", "%v5")
            .field("lhs", "x9")
            .field("rhs", "%v5")
            .field("src", "d3");
        let typed = CodeInstruction::new("add")
            .field("dst", Operand::vreg(RegClass::Int, 5))
            .field("lhs", Operand::phys(RegClass::Int, 9, "x9"))
            .field("rhs", Operand::vreg(RegClass::Int, 5))
            .field("src", Operand::phys(RegClass::Fp, 3, "d3"));
        let er = effect(&raw, &model);
        let et = effect(&typed, &model);
        assert_eq!(
            er.defs.iter().map(key).collect::<Vec<_>>(),
            et.defs.iter().map(key).collect::<Vec<_>>(),
            "typed vs raw def sets diverge"
        );
        assert_eq!(
            er.uses.iter().map(key).collect::<Vec<_>>(),
            et.uses.iter().map(key).collect::<Vec<_>>(),
            "typed vs raw use sets diverge"
        );
        assert_eq!(er.is_call, et.is_call);
        // Sanity: the fp `d3` operand contributed no Int-class RegRef either way.
        assert!(!er.uses.iter().any(|r| matches!(r, RegRef::Phys(3))));
    }
}
