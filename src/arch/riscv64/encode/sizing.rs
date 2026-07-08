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

fn build_li(value: i64, steps: &mut Vec<LiStep>) {
    if (-2048..=2047).contains(&value) {
        steps.push(LiStep::Addi(value as i32));
        return;
    }
    let lo12 = ((value & 0xfff) as i32) << 20 >> 20; // sign-extend from bit 11
    let hi = (value - lo12 as i64) >> 12;
    if value == value as i32 as i64 {
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
