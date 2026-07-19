//! Block-local store-to-load forwarding (plan-01-float-codegen Phase 3a).
//!
//! The bump-and-reset register model spills every value to a stack slot between
//! statements and defensively spills an operand to the stack before lowering the
//! next one (see `emit_float_binary` / `lower_comparison_binary`). That leaves
//! runs like
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

use crate::arch::ops::CodeOp;
use crate::target::shared::abi;

use super::regalloc;
use super::types::CodeInstruction;

/// What an instruction does to the registers that forwarding cares about.
enum Effect<'a> {
    /// Store `src` to `[sp + offset]`.
    StoreSp { src: &'a str, offset: &'a str },
    /// Load into `dst` from `[sp + offset]`.
    LoadSp { dst: &'a str, offset: &'a str },
    /// Defines exactly the single register named by its `dst` field; no other
    /// register or memory effect that matters here.
    DefDst,
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
fn classify(instruction: &CodeInstruction, is_x86: bool) -> Effect<'_> {
    match instruction.op {
        CodeOp::StrU64 => match (
            instruction.get("base"),
            instruction.get("offset"),
            instruction.get("src"),
        ) {
            (Some("sp"), Some(offset), Some(src)) => Effect::StoreSp { src, offset },
            _ => Effect::Barrier, // store to a non-sp base may alias a frame slot
        },
        CodeOp::LdrU64 => match (
            instruction.get("base"),
            instruction.get("offset"),
            instruction.get("dst"),
        ) {
            (Some("sp"), Some(offset), Some(dst)) => Effect::LoadSp { dst, offset },
            (Some(_), _, Some(_)) => Effect::DefDst, // non-sp load: just defines dst
            _ => Effect::Barrier,
        },
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
            } else if instruction.get("dst").is_some() {
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
        | CodeOp::FMaddD
        | CodeOp::FMsubD
        | CodeOp::FNmsubD
        | CodeOp::FNmaddD
        | CodeOp::SCvtfDFromX
        | CodeOp::FCvtzsXFromD
        | CodeOp::FCvtmsXFromD
        | CodeOp::FCvtpsXFromD
        | CodeOp::FCvtasXFromD => {
            if instruction.get("dst").is_some() {
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
pub(super) fn forward_stores_to_loads(instructions: &mut [CodeInstruction], is_x86: bool) {
    // slot offset -> register that last stored it (and still holds the value).
    let mut slots: Vec<(String, String)> = Vec::new();
    let invalidate_reg = |slots: &mut Vec<(String, String)>, reg: &str| {
        slots.retain(|(_, src)| src != reg);
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
    let set_slot = |slots: &mut Vec<(String, String)>, offset: &str, src: &str| {
        let this = offset.parse::<i64>().ok();
        slots.retain(|(off, _)| {
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
        slots.push((offset.to_string(), src.to_string()));
    };
    let slot_reg = |slots: &[(String, String)], offset: &str| -> Option<String> {
        slots
            .iter()
            .find(|(off, _)| off == offset)
            .map(|(_, src)| src.clone())
    };

    for index in 0..instructions.len() {
        match classify(&instructions[index], is_x86) {
            Effect::StoreSp { src, offset } => {
                let (src, offset) = (src.to_string(), offset.to_string());
                set_slot(&mut slots, &offset, &src);
            }
            Effect::LoadSp { dst, offset } => {
                let (dst, offset) = (dst.to_string(), offset.to_string());
                if let Some(reg) = slot_reg(&slots, &offset) {
                    if reg != dst {
                        instructions[index] = abi::move_register(&dst, &reg);
                    }
                }
                // The load (or the rewritten move) defines `dst`.
                invalidate_reg(&mut slots, &dst);
            }
            Effect::DefDst => {
                if let Some(dst) = instructions[index].get("dst") {
                    let dst = dst.to_string();
                    invalidate_reg(&mut slots, &dst);
                } else {
                    slots.clear();
                }
            }
            Effect::NoDef => {}
            Effect::Barrier => slots.clear(),
        }
    }
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
/// allocation), before `finalize_frame`. `is_riscv` selects the per-ISA
/// call-clobber masks in the underlying liveness (threaded from the active
/// backend's arch, not sniffed from operand strings).
pub(super) fn remove_fp_shuttles(instructions: &mut Vec<CodeInstruction>, is_riscv: bool) {
    let live_out = regalloc::integer_live_out(instructions, is_riscv);
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
                first.get("dst"),
                first.get("src"),
                second.get("src"),
                second.get("base"),
                second.get("offset"),
                abi::store_double,
            ),
            // Operand reload: ldr xN, [base,#off] ; fmov dM, xN  ->  ldr d dM, [base,#off].
            (CodeOp::LdrU64, CodeOp::FMovDFromX) => fold_pair(
                first.get("dst"),
                second.get("dst"),
                second.get("src"),
                first.get("base"),
                first.get("offset"),
                abi::load_double,
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

    if drop_def.iter().all(|drop| !drop) {
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
    fpr: Option<&str>,
    linked: Option<&str>,
    base: Option<&str>,
    offset: Option<&str>,
    build: fn(&str, &str, usize) -> CodeInstruction,
) -> Option<(u32, CodeInstruction)> {
    let (gpr, fpr, linked, base, offset) = (gpr?, fpr?, linked?, base?, offset?);
    if gpr != linked || gpr == base {
        return None;
    }
    let index = regalloc::int_physical_index(gpr)?;
    let offset: usize = offset.parse().ok()?;
    Some((index, build(fpr, base, offset)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(name: &str) -> CodeInstruction {
        CodeInstruction::new(name)
    }

    /// The result shuttle `fmov xN,dM ; str xN,[sp,#k]` collapses to `str d dM`
    /// when `xN` is dead afterwards, and the reload `ldr xN ; fmov dM,xN`
    /// collapses to `ldr d dM` likewise.
    #[test]
    fn folds_dead_result_and_operand_shuttles() {
        let mut instructions = vec![
            op("fmov_x_from_d").field("dst", "x8").field("src", "d11"),
            op("str_u64")
                .field("src", "x8")
                .field("base", "sp")
                .field("offset", "1120"),
            // x8 redefined here, so it is dead after the store above.
            op("ldr_u64")
                .field("dst", "x8")
                .field("base", "sp")
                .field("offset", "80"),
            op("fmov_d_from_x").field("dst", "d9").field("src", "x8"),
            op("ret"),
        ];
        remove_fp_shuttles(&mut instructions, false);
        assert_eq!(instructions.len(), 3);
        assert_eq!(instructions[0].op, CodeOp::StrD);
        assert_eq!(instructions[0].get("src"), Some("d11"));
        assert_eq!(instructions[0].get("offset"), Some("1120"));
        assert_eq!(instructions[1].op, CodeOp::LdrD);
        assert_eq!(instructions[1].get("dst"), Some("d9"));
        assert_eq!(instructions[1].get("offset"), Some("80"));
        assert_eq!(instructions[2].op, CodeOp::Ret);
    }

    /// When the shuttle GPR is still live after the store (read by a later
    /// instruction before being redefined), the fold is unsound and must not fire.
    #[test]
    fn keeps_shuttle_when_gpr_still_live() {
        let mut instructions = vec![
            op("fmov_x_from_d").field("dst", "x8").field("src", "d11"),
            op("str_u64")
                .field("src", "x8")
                .field("base", "sp")
                .field("offset", "1120"),
            // x8 is read here, so it must survive the store.
            op("add")
                .field("dst", "x9")
                .field("lhs", "x8")
                .field("rhs", "x8"),
            op("ret"),
        ];
        let before = instructions.len();
        remove_fp_shuttles(&mut instructions, false);
        assert_eq!(instructions.len(), before);
        assert_eq!(instructions[0].op, CodeOp::FMovXFromD);
        assert_eq!(instructions[1].op, CodeOp::StrU64);
    }

    /// Full-slot, non-overlapping sp stores still forward: the `ldr` from `#8`
    /// becomes a `mov` from the register that stored it. (This is the today path —
    /// proves the overlap guard did not disturb byte-identical behaviour.)
    #[test]
    fn forwards_disjoint_full_slot_store() {
        let mut instructions = vec![
            op("str_u64")
                .field("src", "x10")
                .field("base", "sp")
                .field("offset", "8"),
            op("str_u64")
                .field("src", "x11")
                .field("base", "sp")
                .field("offset", "16"),
            op("ldr_u64")
                .field("dst", "x8")
                .field("base", "sp")
                .field("offset", "8"),
        ];
        forward_stores_to_loads(&mut instructions, false);
        assert_eq!(instructions[2].op, CodeOp::Mov);
        assert_eq!(instructions[2].get("dst"), Some("x8"));
        assert_eq!(instructions[2].get("src"), Some("x10"));
    }

    /// bug-284 C8: on x86-64 `mul`/`umulh`/`smulh`/`sdiv`/`udiv`/`msub` expand to
    /// sequences that clobber rdx:rax in addition to their named `dst`, which the
    /// arch-neutral `DefDst` classification ("defines exactly dst") cannot express.
    /// A slot whose forwarding source lived in rax/rdx across one of these was not
    /// invalidated, so the reload was forwarded to a clobbered register.
    #[test]
    fn x86_implicit_clobber_ops_flush_forwarding_state() {
        let stream = || {
            vec![
                op("str_u64")
                    .field("src", "rax")
                    .field("base", "sp")
                    .field("offset", "8"),
                // dst is neither rax nor rdx, so `DefDst` invalidates nothing --
                // but the x86 expansion clobbers rax regardless.
                op("udiv")
                    .field("dst", "r10")
                    .field("lhs", "r11")
                    .field("rhs", "r12"),
                op("ldr_u64")
                    .field("dst", "r14")
                    .field("base", "sp")
                    .field("offset", "8"),
            ]
        };

        // On x86 the reload must survive as a real memory load.
        let mut instructions = stream();
        forward_stores_to_loads(&mut instructions, true);
        assert_eq!(
            instructions[2].op,
            CodeOp::LdrU64,
            "x86 must not forward across an op that clobbers rax"
        );

        // On aarch64/riscv64 these ops really do define only `dst`, so the
        // forwarding stays available -- the flush is x86-specific, not a blanket
        // pessimization.
        let mut instructions = stream();
        forward_stores_to_loads(&mut instructions, false);
        assert_eq!(instructions[2].op, CodeOp::Mov);
        assert_eq!(instructions[2].get("src"), Some("rax"));
    }

    /// A later store that *partially overwrites* an 8-byte slot (offset `#12`
    /// clobbers bytes 12..16 of the value stored at `#8`) must invalidate that slot,
    /// so the reload from `#8` is NOT forwarded to the stale register. Without the
    /// range-overlap invalidation this would wrongly rewrite the load to `mov x8,x10`.
    #[test]
    fn does_not_forward_partially_overwritten_slot() {
        let mut instructions = vec![
            op("str_u64")
                .field("src", "x10")
                .field("base", "sp")
                .field("offset", "8"),
            // Overlaps bytes 12..16 of the store above (|12 - 8| = 4 < 8).
            op("str_u64")
                .field("src", "x11")
                .field("base", "sp")
                .field("offset", "12"),
            op("ldr_u64")
                .field("dst", "x8")
                .field("base", "sp")
                .field("offset", "8"),
        ];
        forward_stores_to_loads(&mut instructions, false);
        // The load survives as a real memory reload.
        assert_eq!(instructions[2].op, CodeOp::LdrU64);
        assert_eq!(instructions[2].get("dst"), Some("x8"));
        assert_eq!(instructions[2].get("offset"), Some("8"));
    }
}
