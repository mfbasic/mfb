use super::operand::{field, immediate, reg};
use super::*;

pub(super) fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
    match instruction.op {
        CodeOp::Label => return Ok(0),
        CodeOp::MovImm => {
            return Ok(wide_imm_word_count(immediate(field(instruction, "value")?)?) * 4);
        }
        CodeOp::AddImm | CodeOp::SubImm => {
            return Ok(sized_add_sub_imm(immediate(field(instruction, "imm")?)?));
        }
        CodeOp::AddSp | CodeOp::SubSp => {
            return Ok(sized_add_sub_imm(immediate(field(instruction, "imm")?)?));
        }
        // `cmp_imm` is not chunked like add/sub: out of imm12 range `emit_cmp_imm`
        // materializes the immediate with `mov_imm` (1–4 words) and emits a
        // register `cmp`, so its length follows the mov_imm word count.
        CodeOp::CmpImm => {
            let rhs = immediate(field(instruction, "rhs")?)?;
            return Ok(if checked_imm12(rhs).is_ok() {
                4
            } else {
                wide_imm_word_count(rhs) * 4 + 4
            });
        }
        CodeOp::LdrU64 | CodeOp::StrU64 | CodeOp::LdrD | CodeOp::StrD => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                8,
            ));
        }
        CodeOp::LdrU32 | CodeOp::StrU32 => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                4,
            ));
        }
        CodeOp::LdrU16 | CodeOp::StrU16 => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                2,
            ));
        }
        CodeOp::LdrU8 | CodeOp::StrU8 => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                1,
            ));
        }
        // 128-bit q load/store: one scaled word when the offset is 16-aligned
        // and in range, else the GPR-scratch address fallback (a huge frame puts
        // FP spill slots past the 65520-byte scaled ceiling).
        CodeOp::LdrQ | CodeOp::StrQ => {
            return Ok(sized_memory_imm(
                immediate(field(instruction, "offset")?)?,
                16,
            ));
        }
        // Explicit-carry add (plan-00-G §4): `adds; cset` (no carry-in) or
        // `cmp; adcs; cset` (carry-in register) — the no-carry-in form avoids
        // `cmp xzr,#1` (x31 = SP in the immediate form). Explicit-borrow sub is
        // always `subs; sbcs; cset` (register form, no SP hazard).
        // Key the size on the *resolved* register number, exactly as
        // `emit_add_carry` does — `"xzr"`, `"sp"`, `"raw_sp"` and `"x31"` all
        // resolve to 31, so a spelling test would disagree with the emitter.
        CodeOp::AddCarry => {
            return Ok(if reg(field(instruction, "carry_in")?)? == 31 {
                8
            } else {
                12
            });
        }
        CodeOp::SubBorrow => return Ok(12),
        _ => {}
    }
    Ok(4)
}

fn wide_imm_word_count(value: u64) -> usize {
    1 + [16, 32, 48]
        .into_iter()
        .filter(|shift| ((value >> shift) & 0xffff) != 0)
        .count()
}

pub(super) fn checked_imm12(value: u64) -> Result<u32, String> {
    if value > 4095 {
        return Err(format!("AArch64 immediate {value} exceeds 12-bit encoding"));
    }
    Ok(value as u32)
}

pub(super) fn encode_add_sub_imm(value: u64) -> Option<(u32, bool)> {
    if value <= 4095 {
        Some((value as u32, false))
    } else if value.is_multiple_of(4096) && (value >> 12) <= 4095 {
        Some(((value >> 12) as u32, true))
    } else {
        None
    }
}

fn sized_add_sub_imm(value: u64) -> usize {
    if value == 0 {
        return 4;
    }
    let mut remaining = value;
    let mut words = 0;
    while remaining > 0 {
        let (chunk, shift12) = next_add_sub_chunk(remaining);
        remaining -= if shift12 {
            u64::from(chunk) << 12
        } else {
            u64::from(chunk)
        };
        words += 1;
    }
    words * 4
}

fn sized_memory_imm(offset: u64, scale: u64) -> usize {
    if offset.is_multiple_of(scale) && (offset / scale) <= 4095 {
        4
    } else {
        sized_add_sub_imm(offset) + 4
    }
}

pub(super) fn next_add_sub_chunk(remaining: u64) -> (u32, bool) {
    if remaining >= 4096 {
        (((remaining / 4096).min(4095)) as u32, true)
    } else {
        (remaining as u32, false)
    }
}

pub(super) fn branch_imm26(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x03ff_ffff
}

pub(super) fn branch_imm19(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x0007_ffff
}
