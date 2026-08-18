use super::*;
use crate::arch::encode_plan::InstructionEncoder;

/// One step of a `li` (load-immediate) expansion. The first step is always an
/// absolute set (`Lui` or `Addi` from `zero`); the rest build on `rd`.
#[derive(Clone, Copy)]
pub(super) enum LiStep {
    Lui(u32),
    Addi(i32),     // addi rd, zero, imm
    Slli(u32),     // slli rd, rd, shift
    AddiFrom(i32), // addi rd, rd, imm
}

/// The `li` sequence for a 64-bit value (LLVM's `generateInstSeq` without the
/// trailing-zero optimization — always correct, at most ~8 steps). Shared by the
/// emitter (to produce the words) and by sizing (to count them), so the two-pass
/// sizes always match.
pub(super) fn li_steps(value: i64) -> Vec<LiStep> {
    let mut steps = Vec::new();
    build_li(value, &mut steps);
    steps
}

// --- Base-ISA bit-manipulation expansions (no Zbb) ---------------------------
//
// RV64GC (RVA20) has no `clz`/`ctz`/`rev8`/`brev8`, so `Clz`/`Rbit`/`RevX`/`RevW`
// lower to base-ISA sequences of parallel masked swaps (and, for `clz`, a SWAR
// popcount of the down-smeared value). The `(shift, mask)` levels below are the
// single source of truth the emitter iterates to produce the words; sizing is
// now derived from the emitter itself (bug-341-B3), so there is no separate
// per-level word count to keep in step.

/// `rev_x` (64-bit byte reverse): swap adjacent bytes, then adjacent 16-bit
/// halves, then the two 32-bit halves.
pub(super) const REV_X_LEVELS: &[(u32, u64)] =
    &[(8, 0x00FF_00FF_00FF_00FF), (16, 0x0000_FFFF_0000_FFFF)];

/// `rbit` (64-bit bit reverse): the six granularity levels (1,2,4,8,16 masked,
/// then the 32-bit half swap).
pub(super) const RBIT_LEVELS: &[(u32, u64)] = &[
    (1, 0x5555_5555_5555_5555),
    (2, 0x3333_3333_3333_3333),
    (4, 0x0F0F_0F0F_0F0F_0F0F),
    (8, 0x00FF_00FF_00FF_00FF),
    (16, 0x0000_FFFF_0000_FFFF),
];

/// `rev_w` (32-bit byte reverse, zero-extended) swaps adjacent bytes with this
/// mask, then swaps the two 16-bit halves.
pub(super) const REV_W_MASK: u64 = 0x00FF_00FF;

/// The four SWAR popcount masks `clz` uses on the down-smeared value.
pub(super) const CLZ_POPCOUNT_MASKS: [u64; 4] = [
    0x5555_5555_5555_5555,
    0x3333_3333_3333_3333,
    0x0F0F_0F0F_0F0F_0F0F,
    0x0101_0101_0101_0101,
];

fn build_li(value: i64, steps: &mut Vec<LiStep>) {
    if (-2048..=2047).contains(&value) {
        steps.push(LiStep::Addi(value as i32));
        return;
    }
    let lo12 = ((value & 0xfff) as i32) << 20 >> 20; // sign-extend from bit 11
                                                     // `wrapping_sub` is correct here: `li` materializes the exact 64-bit pattern,
                                                     // so wrap-around at the i64 extremes (e.g. MAX with lo12 = -1) reconstructs
                                                     // the same bits after the `slli 12; addi lo12` — and it avoids a debug panic
                                                     // on float bit patterns that sit near i64::MAX/MIN.
    let hi = value.wrapping_sub(lo12 as i64) >> 12;
    // Fast path `lui hi; addi lo` — valid only when `hi` fits the signed 20-bit
    // `lui` field. `lui` sign-extends bit 19, so a `hi` at/above 2^19 (e.g.
    // 0x7fffffff needs hi = 0x80000) would sign-extend negative and corrupt the
    // value; those fall through to the 64-bit recursion, which is always correct.
    if value == value as i32 as i64 && (-(1i64 << 19)..(1i64 << 19)).contains(&hi) {
        let hi20 = (hi as u32) & 0xfffff;
        steps.push(LiStep::Lui(hi20));
        if lo12 != 0 {
            steps.push(LiStep::AddiFrom(lo12));
        }
        return;
    }
    build_li(hi, steps);
    steps.push(LiStep::Slli(12));
    if lo12 != 0 {
        steps.push(LiStep::AddiFrom(lo12));
    }
}

/// The exact byte length [`super::emitter::Encoder::emit_instruction`] produces,
/// derived from the emitter itself so there is no second, drift-prone size table
/// (bug-341-B3). A throwaway encoder emits the instruction and we take the text
/// length; its relocation/label side effects are discarded, and each field value
/// is seeded as an import so a symbol-referencing op's binding resolution
/// succeeds (binding never changes the byte count).
pub(super) fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
    let mut probe = Encoder::new(Vec::new(), HashMap::new());
    for (_, value) in &instruction.fields {
        // Only a `Raw` operand can name a call/data symbol the encoder resolves
        // against `imports`; a `Phys`/`Imm`/`VReg` register or literal is never a
        // relocation target, so seeding it renders a `String` binding resolution
        // never reads. Binding never affects the byte count, so restricting the
        // seed to `Raw` operands is size-identical (mirrors the aarch64 sizing).
        if let crate::codegen::engine::operand::Operand::Raw(text) = value {
            probe.imports.insert(text.to_string(), String::new());
        }
    }
    probe.emit_instruction(instruction)?;
    Ok(probe.text.len())
}
