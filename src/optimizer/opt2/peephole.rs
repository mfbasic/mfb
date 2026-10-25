//! Block-local store-to-load forwarding (plan-01-float-codegen Phase 3a).
//!
//! The legacy bump-and-reset register model (since removed) spilled every value
//! to a stack slot between statements and defensively spilled an operand to the
//! stack before lowering the next one (see `emit_float_binary` /
//! `lower_comparison_binary`, which retain that shape). That leaves runs like
//!
//! ```text
//!   str x10, [sp, #0xa8]      ; spill a*b
//!   str x16, [sp, #0xb0]      ; spill c
//!   ldr x8,  [sp, #0xa8]      ; reload a*b  -- x10 still holds it
//!   ldr x9,  [sp, #0xb0]      ; reload c    -- x16 still holds it
//! ```
//!
//! This pass rewrites a load from a stack slot into a register move from the
//! register that last stored that slot, when that register provably still holds
//! the value. It never removes an instruction and never reorders, so it cannot
//! change behaviour; the win is turning a memory reload into a register move
//! (and exposing the source register to the hardware's own forwarding).
//!
//! Safety model. Forwarding is sound only while we know (a) which `sp` slot a
//! value was last stored to and (b) that the storing register has not been
//! overwritten since. We therefore track, per basic block, a map from `sp`
//! offset to the register that stored it, and:
//!
//!   * clear the whole map at any label, branch, call, return, `sp` adjustment,
//!     or any instruction whose register effects we do not explicitly model
//!     (the conservative default), and at any store to a non-`sp` base (which
//!     might alias a frame slot);
//!   * invalidate every slot whose source register is written by an instruction
//!     we do model.
//!
//! Because the safety of a *forward* depends only on the register-*write* model
//! (not on reads), mis-modelling a read can never produce a wrong forward; an
//! unmodelled op clears state instead.

// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::target::shared::regmodel::RegisterModel;

use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::types::CodeInstruction;

/// What an instruction does to the registers that forwarding cares about.
///
/// Owns its operand strings (`String`, not a borrow of the instruction): since
/// plan-78-B `CodeInstruction::get` renders an owned value rather than lending a
/// `&str`, there is nothing in the instruction to borrow. The forwarding loop
/// took owned copies anyway, so this is byte-identical.
enum Effect {
    /// Store `src` to `[sp + offset]`.
    StoreSp { src: String, offset: String },
    /// Load into `dst` from `[sp + offset]`.
    LoadSp { dst: String, offset: String },
    /// Store `src` to `[sp + offset]` and `src2` to `[sp + offset + 8]`: a
    /// non-writeback 64-bit `stp` (plan-171-A) is the two slot stores it pairs.
    StoreSpPair {
        src: String,
        src2: String,
        offset: i64,
    },
    /// Defines exactly the single register named by its `dst` field; no other
    /// register or memory effect that matters here.
    DefDst,
    /// Defines exactly `dst` and `dst2`: a non-writeback 64-bit `ldp`
    /// (plan-171-A), whose loaded slots this pass does not forward.
    DefDstPair { dst: String, dst2: String },
    /// Reads/flags only — no register definition, no clobber (compares).
    NoDef,
    /// Anything else: control flow, calls, non-`sp` or sub-word stores, vector
    /// ops, `sp` adjustments, or any op we do not model. Forwarding state is
    /// flushed.
    Barrier,
}

/// `is_x86` selects the implicit-clobber model: on x86-64 `mul`/`umulh`/`smulh`/
/// `sdiv`/`udiv`/`msub` expand to sequences that clobber rdx:rax beyond their
/// named `dst`, which the arch-neutral `DefDst` classification cannot express
/// (bug-284 C8). aarch64 and riscv64 have no such implicit clobbers.
fn classify(instruction: &CodeInstruction, is_x86: bool) -> Effect {
    match instruction.op {
        CodeOp::StrU64 => {
            // Peek `base` through a borrow so a non-sp store never renders its
            // `offset`/`src` (it flushes anyway); only an sp store owns them (they
            // key the slot map).
            if instruction
                .operand("base")
                .is_some_and(|b| b.rendered() == "sp")
            {
                match (instruction.get("offset"), instruction.get("src")) {
                    (Some(offset), Some(src)) => Effect::StoreSp { src, offset },
                    _ => Effect::Barrier,
                }
            } else {
                Effect::Barrier // store to a non-sp base may alias a frame slot
            }
        }
        CodeOp::LdrU64 => {
            if instruction
                .operand("base")
                .is_some_and(|b| b.rendered() == "sp")
            {
                match (instruction.get("offset"), instruction.get("dst")) {
                    (Some(offset), Some(dst)) => Effect::LoadSp { dst, offset },
                    // base==sp but a field is missing: a bare dst still just
                    // defines dst (matches the old `(Some(_),_,Some(_))` arm).
                    (_, Some(_)) => Effect::DefDst,
                    _ => Effect::Barrier,
                }
            } else if instruction.operand("base").is_some() && instruction.operand("dst").is_some()
            {
                Effect::DefDst // non-sp load: just defines dst
            } else {
                Effect::Barrier
            }
        }
        // plan-171-A: a non-writeback 64-bit pair is its two halves. A write-back
        // pair moves its base (for `sp`, a frame adjustment) and a 128-bit pair
        // carries vector values, so both stay barriers like their single forms.
        CodeOp::Stp | CodeOp::Ldp => {
            let plain = instruction.get("addressing").as_deref() == Some("offset")
                && instruction.get("width").as_deref() == Some("8");
            if !plain {
                return Effect::Barrier;
            }
            if instruction.op == CodeOp::Ldp {
                return match (instruction.get("dst"), instruction.get("dst2")) {
                    (Some(dst), Some(dst2)) => Effect::DefDstPair { dst, dst2 },
                    _ => Effect::Barrier,
                };
            }
            let on_sp = instruction
                .operand("base")
                .is_some_and(|b| b.rendered() == "sp");
            match (
                on_sp,
                instruction.get("src"),
                instruction.get("src2"),
                instruction
                    .get("offset")
                    .and_then(|offset| offset.parse::<i64>().ok()),
            ) {
                (true, Some(src), Some(src2), Some(offset)) => {
                    Effect::StoreSpPair { src, src2, offset }
                }
                // A pair through another base may alias a frame slot.
                _ => Effect::Barrier,
            }
        }
        // Compares write only the flags.
        CodeOp::Cmp | CodeOp::CmpImm | CodeOp::FCmpD | CodeOp::FCmpZeroD => Effect::NoDef,
        // bug-284 C8: on x86-64 these expand to sequences that clobber rdx:rax in
        // addition to their named `dst` (see `div_seq`, `umulh`, `msub`), so
        // `DefDst` -- "defines exactly dst" -- understates their effect. A slot
        // whose forwarding source lived in rax/rdx across one of these would not be
        // invalidated and the reload would be forwarded to a clobbered register.
        // The x86 register model does not currently colour a value onto rax/rdx
        // (INT_ALLOCATABLE is r10/r11/r12/r14), so this was a soundness reliance
        // rather than a live bug; flushing removes the reliance instead of
        // documenting it.
        CodeOp::Mul
        | CodeOp::SMulH
        | CodeOp::UMulH
        | CodeOp::SDiv
        | CodeOp::UDiv
        | CodeOp::MSub => {
            if is_x86 {
                Effect::Barrier
            } else if instruction.operand("dst").is_some() {
                Effect::DefDst
            } else {
                Effect::Barrier
            }
        }
        // Scalar ops that define exactly their single `dst` operand.
        CodeOp::Mov
        | CodeOp::MovImm
        | CodeOp::Add
        | CodeOp::Adds
        | CodeOp::Sub
        | CodeOp::Subs
        | CodeOp::And
        | CodeOp::Orr
        | CodeOp::Eor
        | CodeOp::Mvn
        | CodeOp::Rorv
        | CodeOp::RorvW
        | CodeOp::Lslv
        | CodeOp::Lsrv
        | CodeOp::Asrv
        | CodeOp::Clz
        | CodeOp::Rbit
        | CodeOp::RevW
        | CodeOp::RevX
        | CodeOp::LslImm
        | CodeOp::LsrImm
        | CodeOp::AsrImm
        | CodeOp::AddImm
        | CodeOp::SubImm
        | CodeOp::Adrp
        | CodeOp::AddPageOff
        | CodeOp::LdrU32
        | CodeOp::LdrU16
        | CodeOp::LdrU8
        | CodeOp::FMovXFromD
        | CodeOp::FMovDFromX
        | CodeOp::FMovDFromD
        | CodeOp::FAddD
        | CodeOp::FSubD
        | CodeOp::FMulD
        | CodeOp::FDivD
        | CodeOp::FMinnmD
        | CodeOp::FMaxnmD
        | CodeOp::FNegD
        | CodeOp::FAbsD
        | CodeOp::FSqrtD
        | CodeOp::FCvtSFromD
        | CodeOp::FCvtDFromS
        | CodeOp::FMaddD
        | CodeOp::FMsubD
        | CodeOp::FNmsubD
        | CodeOp::FNmaddD
        | CodeOp::SCvtfDFromX
        | CodeOp::FCvtzsXFromD
        | CodeOp::FCvtmsXFromD
        | CodeOp::FCvtpsXFromD
        | CodeOp::FCvtasXFromD => {
            if instruction.operand("dst").is_some() {
                Effect::DefDst
            } else {
                Effect::Barrier
            }
        }
        // Everything else (control flow, calls, sub-word/vector stores, vector
        // ops, sp adjustments, …) conservatively flushes state.
        _ => Effect::Barrier,
    }
}

/// Run store-to-load forwarding over one function's instruction stream, in
/// place. Must run before `finalize_frame` (offsets are still pre-prologue and
/// the callee-save area / `sp` adjustments are not yet present).
///
/// A **Level-1** dial row -- `optimizations.md` "Peephole optimization", the
/// block-local machine embryo of the L3 alias-analysis "Store-to-load
/// forwarding" row -- so it runs at `-O1` (the default) and up, and is skipped
/// at `-O0`. It stays post-regalloc (it needs physical registers) rather than
/// moving into the reserved between-select seam (plan-100 §3).
pub(crate) fn forward_stores_to_loads(instructions: &mut [CodeInstruction], is_x86: bool) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    // slot offset -> register that last stored it (and still holds the value):
    // its spelling (for the invalidation compare) and its operand (to forward).
    let mut slots: Vec<(String, String, Operand)> = Vec::new();
    let invalidate_reg = |slots: &mut Vec<(String, String, Operand)>, reg: &str| {
        slots.retain(|(_, src, _)| src != reg);
    };
    // Record an 8-byte store of `src` to `[sp + offset]`, first invalidating every
    // slot whose 8-byte range `[o, o+8)` overlaps this store's `[offset, offset+8)`.
    // A forward is sound only when the stored value still fully occupies the loaded
    // slot; a partial overwrite of a neighbouring slot must therefore drop it.
    //
    // Today the frame allocator hands out 8-byte-granular, 8-aligned sp objects
    // (`allocate_stack_object(_, 8)`) and every sp store is a full 8-byte `StrU64`,
    // so no two *distinct* recorded offsets are within 8 bytes and this loop only
    // ever removes the exact-offset match (identical to the previous behaviour —
    // byte-identical output). It converts a future packed/unaligned sp store from a
    // silent stale-forward into a correct invalidation. Non-numeric offsets (which
    // the frame model never produces for sp slots) fall back to exact-string
    // matching so their behaviour is likewise unchanged.
    let set_slot =
        |slots: &mut Vec<(String, String, Operand)>, offset: &str, src: &str, operand: Operand| {
            let this = offset.parse::<i64>().ok();
            slots.retain(|(off, _, _)| {
                if off == offset {
                    return false; // exact slot: re-inserted below with the new source
                }
                match (this, off.parse::<i64>().ok()) {
                    // Both numeric: keep only when the 8-byte ranges are disjoint.
                    (Some(a), Some(b)) => (a - b).abs() >= 8,
                    // A non-numeric offset on either side is not range-comparable; keep
                    // it (exact-string keying, as before).
                    _ => true,
                }
            });
            slots.push((offset.to_string(), src.to_string(), operand));
        };
    let slot_reg =
        |slots: &[(String, String, Operand)], offset: &str| -> Option<(String, Operand)> {
            slots
                .iter()
                .find(|(off, _, _)| off == offset)
                .map(|(_, src, operand)| (src.clone(), operand.clone()))
        };

    // Indexed on purpose: the body rewrites `instructions[index]` in place while
    // `classify` is still holding a reference to that same element, so an `iter_mut` reference
    // cannot span both halves.
    let mut forwarded = 0;
    #[allow(clippy::needless_range_loop)]
    for index in 0..instructions.len() {
        match classify(&instructions[index], is_x86) {
            Effect::StoreSp { src, offset } => {
                let operand = instructions[index]
                    .operand("src")
                    .cloned()
                    .expect("a classified sp store has a src");
                set_slot(&mut slots, &offset, &src, operand);
            }
            Effect::LoadSp { dst, offset } => {
                if let Some((reg, source)) = slot_reg(&slots, &offset) {
                    if reg != dst {
                        let target = instructions[index]
                            .operand("dst")
                            .cloned()
                            .expect("a classified sp load has a dst");
                        instructions[index] =
                            super::rebuilt(CodeOp::Mov, vec![("dst", target), ("src", source)]);
                        forwarded += 1;
                    }
                }
                // The load (or the rewritten move) defines `dst`.
                invalidate_reg(&mut slots, &dst);
            }
            Effect::DefDst => {
                if let Some(dst) = instructions[index].operand("dst") {
                    let dst = dst.rendered();
                    invalidate_reg(&mut slots, &dst);
                } else {
                    slots.clear();
                }
            }
            Effect::StoreSpPair { src, src2, offset } => {
                for (role, register, at) in [("src", src, offset), ("src2", src2, offset + 8)] {
                    let operand = instructions[index]
                        .operand(role)
                        .cloned()
                        .expect("a classified sp pair store has both sources");
                    set_slot(&mut slots, &at.to_string(), &register, operand);
                }
            }
            Effect::DefDstPair { dst, dst2 } => {
                invalidate_reg(&mut slots, &dst);
                invalidate_reg(&mut slots, &dst2);
            }
            Effect::NoDef => {}
            Effect::Barrier => slots.clear(),
        }
    }
    crate::optimizer::stats::count_peephole_forwards(forwarded);
}

/// Remove the GP shuttle that a float value round-trips through purely to satisfy
/// the GP-native value model (plan-16 Piece B). After the FP-domain finiteness
/// check (`emit_float_result_check_fp`), the GPR a float result is `fmov`'d into
/// is read only by its own spill store, and a float operand reloaded as a GPR is
/// read only by the `fmov` that puts it back in a `d`-register. Both can be done
/// directly in the FP domain:
///
/// ```text
///   fmov xN, dM ; str xN, [sp,#k]   (xN dead after)  ->  str d dM, [sp,#k]
///   ldr xN, [sp,#k] ; fmov dM, xN   (xN dead after)  ->  ldr d dM, [sp,#k]
/// ```
///
/// The 64 bits a `str d`/`ldr d` move are identical to the GPR store/load, so a
/// slot written by one and read by the other (e.g. an `ldr x` reload of a result
/// spilled with `str d`) stays correct. Soundness rests entirely on `xN` being
/// dead immediately after the second instruction — proven with integer
/// live-out over the colored stream — so dropping its definition removes nothing
/// another instruction needs.
///
/// Runs after `forward_stores_to_loads` and on physical registers (post register
/// allocation), before `finalize_frame`. `model` is the active backend's register
/// model; the underlying liveness derives its call-clobber mask from that model's
/// caller-saved table (threaded in, not sniffed from operand strings).
///
/// A **Level-1** dial row -- `optimizations.md` "Machine copy propagation /
/// redundant-move elimination" -- so it runs at `-O1` (the default) and up, and
/// is skipped at `-O0` (plan-100 §3).
pub(crate) fn remove_fp_shuttles(
    instructions: &mut Vec<CodeInstruction>,
    model: &dyn RegisterModel,
) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let live_out = regalloc::integer_live_out(instructions, model);
    // Index of the first (def) instruction of each matched pair -> the rewritten
    // second instruction. The def instruction is dropped; the second is replaced.
    let mut drop_def: Vec<bool> = vec![false; instructions.len()];
    let mut replacement: Vec<Option<CodeInstruction>> = std::iter::repeat_with(|| None)
        .take(instructions.len())
        .collect();

    let mut i = 0;
    while i + 1 < instructions.len() {
        let first = &instructions[i];
        let second = &instructions[i + 1];
        // The GPR that must be provably dead after `second` for the fold to be
        // sound, plus the rewritten `second`, if this is a foldable pair.
        let folded: Option<(u32, CodeInstruction)> = match (first.op, second.op) {
            // Result shuttle: fmov xN, dM ; str xN, [base,#off]  ->  str d dM, [base,#off].
            (CodeOp::FMovXFromD, CodeOp::StrU64) => fold_pair(
                first.get("dst").as_deref(),
                first.operand("src"),
                second.get("src").as_deref(),
                second.operand("base"),
                second.get("offset").as_deref(),
                |src, base, off| {
                    super::rebuilt(
                        CodeOp::StrD,
                        vec![
                            ("src", src),
                            ("base", base),
                            ("offset", Operand::from(&off.to_string())),
                        ],
                    )
                },
            ),
            // Operand reload: ldr xN, [base,#off] ; fmov dM, xN  ->  ldr d dM, [base,#off].
            (CodeOp::LdrU64, CodeOp::FMovDFromX) => fold_pair(
                first.get("dst").as_deref(),
                second.operand("dst"),
                second.get("src").as_deref(),
                first.operand("base"),
                first.get("offset").as_deref(),
                |dst, base, off| {
                    super::rebuilt(
                        CodeOp::LdrD,
                        vec![
                            ("dst", dst),
                            ("base", base),
                            ("offset", Operand::from(&off.to_string())),
                        ],
                    )
                },
            ),
            _ => None,
        };
        if let Some((reg_index, rewritten)) = folded {
            if !regalloc::physical_busy(live_out[i + 1], reg_index) {
                drop_def[i] = true;
                replacement[i + 1] = Some(rewritten);
                i += 2;
                continue;
            }
        }
        i += 1;
    }

    let folded = drop_def.iter().filter(|drop| **drop).count() as u64;
    crate::optimizer::stats::count_fp_shuttles_folded(folded);
    if folded == 0 {
        return;
    }
    let mut rewritten = Vec::with_capacity(instructions.len());
    for (index, instruction) in instructions.drain(..).enumerate() {
        if drop_def[index] {
            continue;
        }
        match replacement[index].take() {
            Some(replacement) => rewritten.push(replacement),
            None => rewritten.push(instruction),
        }
    }
    *instructions = rewritten;
}

/// Validate and build a folded FP load/store. `gpr` is the shuttle GPR (defined by
/// the first instruction, used by the second); `fpr` is the `d`-register; `linked`
/// is the GPR the second instruction names that must equal `gpr` for the pair to
/// be the expected shuttle. Returns the GPR's physical index (for the liveness
/// gate) and the rewritten FP memory op addressing `[base,#offset]`. Bails out
/// (returns `None`) if any operand is missing, the GPR is not a real `x`-register,
/// or `base` aliases the shuttle GPR (its definition is about to be dropped).
fn fold_pair(
    gpr: Option<&str>,
    fpr: Option<&Operand>,
    linked: Option<&str>,
    base: Option<&Operand>,
    offset: Option<&str>,
    build: impl Fn(Operand, Operand, usize) -> CodeInstruction,
) -> Option<(u32, CodeInstruction)> {
    let (gpr, fpr, linked, base, offset) = (gpr?, fpr?, linked?, base?, offset?);
    if gpr != linked || *base == *gpr {
        return None;
    }
    let index = regalloc::int_physical_index(gpr)?;
    let offset: usize = offset.parse().ok()?;
    Some((index, build(fpr.clone(), base.clone(), offset)))
}

#[cfg(test)]
mod tests;
