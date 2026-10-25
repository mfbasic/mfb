//! RV64C: each 2-byte instruction is expanded to the base instruction it
//! stands for and decoded as that, which is also how `objdump` prints it
//! (`c.addi16sp` is `addi sp,sp,-16`, `c.jr ra` is `ret`). `c.mv` expands to
//! `addi rd,rs,0`, which `objdump` prints as `mv` and means the same as the
//! `add rd,zero,rs` the specification gives.

use super::{base, instruction, Instruction};

/// The compressed instruction `parcel` at `pc`, or `None` for a reserved
/// encoding.
pub(super) fn decode(parcel: u16, pc: u64) -> Option<Instruction> {
    if parcel == 0 {
        // `c.unimp`: the all-zero parcel is defined to be illegal.
        let mut decoded = instruction(0, "unimp", String::new());
        decoded.length = 2;
        return Some(decoded);
    }
    base(expand(parcel)?, pc, 2)
}

/// The base instruction `parcel` stands for, or `None` for a reserved
/// encoding.
pub(super) fn expand(parcel: u16) -> Option<u32> {
    let p = u32::from(parcel);
    let funct3 = p >> 13;
    match (p & 3, funct3) {
        (0, 0) => {
            let imm = bits(p, 12, 11) << 4 | bits(p, 10, 7) << 6 | bit(p, 6) << 2 | bit(p, 5) << 3;
            (imm != 0).then(|| i_type(imm, 2, 0, low3(p, 2), 0x13))
        }
        (0, 1) => Some(i_type(ld_offset(p), low3(p, 7), 3, low3(p, 2), 0x07)),
        (0, 2) => Some(i_type(lw_offset(p), low3(p, 7), 2, low3(p, 2), 0x03)),
        (0, 3) => Some(i_type(ld_offset(p), low3(p, 7), 3, low3(p, 2), 0x03)),
        (0, 5) => Some(s_type(ld_offset(p), low3(p, 7), low3(p, 2), 3, 0x27)),
        (0, 6) => Some(s_type(lw_offset(p), low3(p, 7), low3(p, 2), 2, 0x23)),
        (0, 7) => Some(s_type(ld_offset(p), low3(p, 7), low3(p, 2), 3, 0x23)),
        (1, 0) => {
            let rd = bits(p, 11, 7);
            Some(i_type(imm6(p), rd, 0, rd, 0x13))
        }
        (1, 1) => {
            let rd = bits(p, 11, 7);
            (rd != 0).then(|| i_type(imm6(p), rd, 0, rd, 0x1b))
        }
        (1, 2) => Some(i_type(imm6(p), 0, 0, bits(p, 11, 7), 0x13)),
        (1, 3) => {
            let rd = bits(p, 11, 7);
            if rd == 2 {
                let imm = bit(p, 12) << 9
                    | bit(p, 6) << 4
                    | bit(p, 5) << 6
                    | bits(p, 4, 3) << 7
                    | bit(p, 2) << 5;
                let imm = sign(imm, 10);
                (imm != 0).then(|| i_type(imm, 2, 0, 2, 0x13))
            } else {
                let imm = imm6(p);
                (imm != 0 && rd != 0).then(|| (imm << 12) | rd << 7 | 0x37)
            }
        }
        (1, 4) => arithmetic(p),
        (1, 5) => {
            let imm = bit(p, 12) << 11
                | bit(p, 11) << 4
                | bits(p, 10, 9) << 8
                | bit(p, 8) << 10
                | bit(p, 7) << 6
                | bit(p, 6) << 7
                | bits(p, 5, 3) << 1
                | bit(p, 2) << 5;
            Some(j_type(sign(imm, 12)))
        }
        (1, 6) | (1, 7) => {
            let imm = bit(p, 12) << 8
                | bits(p, 11, 10) << 3
                | bits(p, 6, 5) << 6
                | bits(p, 4, 3) << 1
                | bit(p, 2) << 5;
            Some(b_type(sign(imm, 9), low3(p, 7), funct3 & 1))
        }
        (2, 0) => {
            let rd = bits(p, 11, 7);
            let shamt = bit(p, 12) << 5 | bits(p, 6, 2);
            Some(shamt << 20 | rd << 15 | 1 << 12 | rd << 7 | 0x13)
        }
        (2, 1) => Some(i_type(ldsp_offset(p), 2, 3, bits(p, 11, 7), 0x07)),
        (2, 2) => {
            let rd = bits(p, 11, 7);
            let imm = bit(p, 12) << 5 | bits(p, 6, 4) << 2 | bits(p, 3, 2) << 6;
            (rd != 0).then(|| i_type(imm, 2, 2, rd, 0x03))
        }
        (2, 3) => {
            let rd = bits(p, 11, 7);
            (rd != 0).then(|| i_type(ldsp_offset(p), 2, 3, rd, 0x03))
        }
        (2, 4) => {
            let (rs1, rs2) = (bits(p, 11, 7), bits(p, 6, 2));
            match (bit(p, 12), rs1, rs2) {
                (0, 0, 0) => None,
                // c.jr
                (0, _, 0) => Some(i_type(0, rs1, 0, 0, 0x67)),
                // c.mv, as `addi rd, rs2, 0`.
                (0, _, _) => Some(i_type(0, rs2, 0, rs1, 0x13)),
                // c.ebreak
                (1, 0, 0) => Some(0x0010_0073),
                // c.jalr
                (1, _, 0) => Some(i_type(0, rs1, 0, 1, 0x67)),
                // c.add
                _ => Some(rs2 << 20 | rs1 << 15 | rs1 << 7 | 0x33),
            }
        }
        (2, 5) => Some(s_type(sdsp_offset(p), 2, bits(p, 6, 2), 3, 0x27)),
        (2, 6) => {
            let imm = bits(p, 12, 9) << 2 | bits(p, 8, 7) << 6;
            Some(s_type(imm, 2, bits(p, 6, 2), 2, 0x23))
        }
        (2, 7) => Some(s_type(sdsp_offset(p), 2, bits(p, 6, 2), 3, 0x23)),
        _ => None,
    }
}

/// Quadrant 1's `funct3` 100: the shifts and `andi` on `rd'`, and the
/// register-register forms.
fn arithmetic(p: u32) -> Option<u32> {
    let rd = low3(p, 7);
    let rs2 = low3(p, 2);
    let shamt = bit(p, 12) << 5 | bits(p, 6, 2);
    match bits(p, 11, 10) {
        0 => Some(shamt << 20 | rd << 15 | 5 << 12 | rd << 7 | 0x13),
        1 => Some(0b01_0000 << 26 | shamt << 20 | rd << 15 | 5 << 12 | rd << 7 | 0x13),
        2 => Some(i_type(imm6(p), rd, 7, rd, 0x13)),
        _ => {
            let (funct7, funct3, opcode) = match (bit(p, 12), bits(p, 6, 5)) {
                (0, 0) => (0b010_0000, 0, 0x33),
                (0, 1) => (0, 4, 0x33),
                (0, 2) => (0, 6, 0x33),
                (0, 3) => (0, 7, 0x33),
                (1, 0) => (0b010_0000, 0, 0x3b),
                (1, 1) => (0, 0, 0x3b),
                _ => return None,
            };
            Some(funct7 << 25 | rs2 << 20 | rd << 15 | funct3 << 12 | rd << 7 | opcode)
        }
    }
}

/// Bit `n` of `p`.
fn bit(p: u32, n: u32) -> u32 {
    (p >> n) & 1
}

/// Bits `high` down to `low` of `p`, shifted down.
fn bits(p: u32, high: u32, low: u32) -> u32 {
    (p >> low) & ((1 << (high - low + 1)) - 1)
}

/// A 3-bit register field at bit `at`: one of `x8`–`x15` (`s0`, `s1`,
/// `a0`–`a5`), or of `f8`–`f15`.
fn low3(p: u32, at: u32) -> u32 {
    8 + bits(p, at + 2, at)
}

/// The 6-bit immediate of `c.addi`, `c.li`, `c.andi` and `c.lui`: bit 12 and
/// bits 2-6, sign-extended, as the low bits of a 32-bit word.
fn imm6(p: u32) -> u32 {
    sign(bit(p, 12) << 5 | bits(p, 6, 2), 6)
}

/// `value`'s low `width` bits, sign-extended to 32.
fn sign(value: u32, width: u32) -> u32 {
    let shift = 32 - width;
    (((value << shift) as i32) >> shift) as u32
}

/// The offset of `c.ld`, `c.fld`, `c.sd`, `c.fsd`: a multiple of 8 below 256.
fn ld_offset(p: u32) -> u32 {
    bits(p, 12, 10) << 3 | bits(p, 6, 5) << 6
}

/// The offset of `c.lw`, `c.sw`: a multiple of 4 below 128.
fn lw_offset(p: u32) -> u32 {
    bits(p, 12, 10) << 3 | bit(p, 6) << 2 | bit(p, 5) << 6
}

/// The offset of `c.ldsp`, `c.fldsp`.
fn ldsp_offset(p: u32) -> u32 {
    bit(p, 12) << 5 | bits(p, 6, 5) << 3 | bits(p, 4, 2) << 6
}

/// The offset of `c.sdsp`, `c.fsdsp`.
fn sdsp_offset(p: u32) -> u32 {
    bits(p, 12, 10) << 3 | bits(p, 9, 7) << 6
}

/// An I-type word: `imm` is the 12-bit immediate (its low 12 bits are used).
fn i_type(imm: u32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (imm & 0xfff) << 20 | rs1 << 15 | funct3 << 12 | rd << 7 | opcode
}

/// An S-type word: a store of `rs2` to `imm(rs1)`.
fn s_type(imm: u32, rs1: u32, rs2: u32, funct3: u32, opcode: u32) -> u32 {
    (imm >> 5 & 0x7f) << 25 | rs2 << 20 | rs1 << 15 | funct3 << 12 | (imm & 0x1f) << 7 | opcode
}

/// `beq` (`funct3` 0) or `bne` (1) of `rs1` against `zero`, by `imm`.
fn b_type(imm: u32, rs1: u32, funct3: u32) -> u32 {
    (imm >> 12 & 1) << 31
        | (imm >> 5 & 0x3f) << 25
        | rs1 << 15
        | funct3 << 12
        | (imm >> 1 & 0xf) << 8
        | (imm >> 11 & 1) << 7
        | 0x63
}

/// `jal zero` by `imm`.
fn j_type(imm: u32) -> u32 {
    (imm >> 20 & 1) << 31
        | (imm >> 1 & 0x3ff) << 21
        | (imm >> 11 & 1) << 20
        | (imm >> 12 & 0xff) << 12
        | 0x6f
}
