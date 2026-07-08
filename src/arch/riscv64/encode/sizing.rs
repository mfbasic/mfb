use super::operand::{field, immediate};
use super::*;

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

pub(super) fn li_step_count(value: i64) -> usize {
    li_steps(value).len()
}

// --- Base-ISA bit-manipulation expansions (no Zbb) ---------------------------
//
// RV64GC (RVA20) has no `clz`/`ctz`/`rev8`/`brev8`, so `Clz`/`Rbit`/`RevX`/`RevW`
// lower to base-ISA sequences of parallel masked swaps (and, for `clz`, a SWAR
// popcount of the down-smeared value). The `(shift, mask)` levels below are the
// single source of truth: the emitter iterates them to produce the words, and
// the sizer sums `li_step_count(mask) + 5` per level, so the two passes agree.

/// One masked swap level emits `li t2,mask` (variable) plus a fixed 5 words
/// (`srli t1; and t1; and run; slli run; or run`).
fn swap_level_words(mask: u64) -> usize {
    li_step_count(mask as i64) + 5
}

/// `rev_x` (64-bit byte reverse): swap adjacent bytes, then adjacent 16-bit
/// halves, then the two 32-bit halves.
pub(super) const REV_X_LEVELS: &[(u32, u64)] = &[
    (8, 0x00FF_00FF_00FF_00FF),
    (16, 0x0000_FFFF_0000_FFFF),
];

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

pub(super) fn rev_x_words() -> usize {
    // mv + levels + (srli; slli; or) for the 32-bit half swap.
    1 + REV_X_LEVELS.iter().map(|&(_, m)| swap_level_words(m)).sum::<usize>() + 3
}

pub(super) fn rbit_words() -> usize {
    1 + RBIT_LEVELS.iter().map(|&(_, m)| swap_level_words(m)).sum::<usize>() + 3
}

pub(super) fn rev_w_words() -> usize {
    // (slli; srli) zero-extend + one swap level + (slli; srli; or) 16-swap +
    // (slli; srli) final zero-extend.
    2 + swap_level_words(REV_W_MASK) + 3 + 2
}

pub(super) fn clz_words() -> usize {
    // mv + 6×(srli; or) smear + popcount + (li 64; sub).
    let popcount = (li_step_count(CLZ_POPCOUNT_MASKS[0] as i64) + 3)
        + (li_step_count(CLZ_POPCOUNT_MASKS[1] as i64) + 4)
        + (li_step_count(CLZ_POPCOUNT_MASKS[2] as i64) + 3)
        + (li_step_count(CLZ_POPCOUNT_MASKS[3] as i64) + 2);
    1 + 12 + popcount + (li_step_count(64) + 1)
}

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

pub(super) fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
    let bytes = match instruction.op {
        CodeOp::Label => 0,
        CodeOp::MovImm => li_step_count(immediate(field(instruction, "value")?)? as i64) * 4,
        CodeOp::AddImm => sized_add_imm(immediate(field(instruction, "imm")?)?),
        CodeOp::AddSp => sized_add_imm(immediate(field(instruction, "imm")?)?),
        CodeOp::SubImm => sized_sub_imm(immediate(field(instruction, "imm")?)?),
        CodeOp::SubSp => sized_sub_imm(immediate(field(instruction, "imm")?)?),
        CodeOp::LdrU64
        | CodeOp::LdrU32
        | CodeOp::LdrU16
        | CodeOp::LdrU8
        | CodeOp::StrU64
        | CodeOp::StrU32
        | CodeOp::StrU8
        | CodeOp::LdrD
        | CodeOp::StrD => sized_memory(immediate(field(instruction, "offset")?)?),
        // `rv.br` is always the 8-byte long form (inverted branch over a `jal`);
        // `bl` is the 8-byte `auipc; jalr` call pair; `msub` is `mul; sub`.
        CodeOp::RvBr | CodeOp::BranchLink | CodeOp::MSub => 8,
        // Explicit-carry add/sub expand to a fixed 7-instruction `sltu` sequence.
        CodeOp::AddCarry | CodeOp::SubBorrow => 28,
        // Base-ISA rotate (no Zbb): `rorv` is 4 shift/or words, `rorv_w` adds a
        // 2-word zero-extension.
        CodeOp::Rorv => 16,
        CodeOp::RorvW => 24,
        // Base-ISA bit-manipulation expansions (no Zbb): masked parallel swaps
        // (plus a SWAR popcount for `clz`). Sizes computed from the shared level
        // tables so they match the emitter's `li` sequences exactly.
        CodeOp::Clz => clz_words() * 4,
        CodeOp::Rbit => rbit_words() * 4,
        CodeOp::RevX => rev_x_words() * 4,
        CodeOp::RevW => rev_w_words() * 4,
        _ => 4,
    };
    Ok(bytes)
}

/// `add_imm`/`add_sp`: one `addi` when the immediate fits the 12-bit signed
/// field, else `li t0, imm` (the `li` sequence) plus one `add`.
fn sized_add_imm(value: u64) -> usize {
    if value <= 2047 {
        4
    } else {
        (li_step_count(value as i64) + 1) * 4
    }
}

/// `sub_imm`/`sub_sp`: one `addi rd, rs, -imm` when `-imm` fits, else `li` + `sub`.
fn sized_sub_imm(value: u64) -> usize {
    if value <= 2048 {
        4
    } else {
        (li_step_count(value as i64) + 1) * 4
    }
}

/// A load/store: one word when the offset fits the 12-bit signed field, else
/// `li t0, offset` + `add t0, base, t0` + the memory access.
fn sized_memory(offset: u64) -> usize {
    if offset <= 2047 {
        4
    } else {
        (li_step_count(offset as i64) + 2) * 4
    }
}
