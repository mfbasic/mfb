//! Scalar floating point: arithmetic, compares, conditional forms, immediates,
//! and the conversions and moves between floating-point and general registers.

use super::{bit, condition, field, hex, instruction, ra, rd, register, rm, rn, Instruction};

pub(super) fn decode(word: u32) -> Option<Instruction> {
    if word & 0xff00_0000 == 0x1f00_0000 {
        return three_source(word);
    }
    if word & 0x5f00_0000 != 0x1e00_0000 {
        return None;
    }
    let size = precision(field(word, 22, 2));
    if !bit(word, 21) {
        return fixed_point(word);
    }
    if field(word, 10, 6) == 0 {
        return integer_conversion(word);
    }
    if bit(word, 31) {
        return None;
    }
    let size = size?;
    let reg = |number: u32| format!("{size}{number}");
    let (dst, first, second) = (rd(word), rn(word), rm(word));
    if field(word, 10, 5) == 0b10000 {
        return one_source(word, size);
    }
    if field(word, 10, 4) == 0b1000 {
        let mnemonic = if bit(word, 4) { "fcmpe" } else { "fcmp" };
        return match (field(word, 14, 2), field(word, 0, 4) & 0b0111) {
            (0, 0) if bit(word, 3) => Some(instruction(mnemonic, format!("{}, #0.0", reg(first)))),
            (0, 0) => Some(instruction(
                mnemonic,
                format!("{}, {}", reg(first), reg(second)),
            )),
            _ => None,
        };
    }
    if field(word, 10, 3) == 0b100 {
        if field(word, 5, 5) != 0 {
            return None;
        }
        return Some(instruction(
            "fmov",
            format!("{}, #{:.8}", reg(dst), expand(field(word, 13, 8))),
        ));
    }
    match field(word, 10, 2) {
        0b01 => Some(instruction(
            if bit(word, 4) { "fccmpe" } else { "fccmp" },
            format!(
                "{}, {}, {}, {}",
                reg(first),
                reg(second),
                hex(u64::from(word & 0xf)),
                condition(field(word, 12, 4))
            ),
        )),
        0b10 => {
            let mnemonic = match field(word, 12, 4) {
                0b0000 => "fmul",
                0b0001 => "fdiv",
                0b0010 => "fadd",
                0b0011 => "fsub",
                0b0100 => "fmax",
                0b0101 => "fmin",
                0b0110 => "fmaxnm",
                0b0111 => "fminnm",
                0b1000 => "fnmul",
                _ => return None,
            };
            Some(instruction(
                mnemonic,
                format!("{}, {}, {}", reg(dst), reg(first), reg(second)),
            ))
        }
        _ => Some(instruction(
            "fcsel",
            format!(
                "{}, {}, {}, {}",
                reg(dst),
                reg(first),
                reg(second),
                condition(field(word, 12, 4))
            ),
        )),
    }
}

/// The register prefix of a scalar of this `ptype`: `s`, `d`, `h`.
fn precision(ptype: u32) -> Option<&'static str> {
    match ptype {
        0b00 => Some("s"),
        0b01 => Some("d"),
        0b11 => Some("h"),
        _ => None,
    }
}

/// The value of an 8-bit floating-point immediate (`VFPExpandImm`).
pub(super) fn expand(imm8: u32) -> f64 {
    let negative = imm8 & 0x80 != 0;
    let b = (imm8 >> 6) & 1;
    let cd = ((imm8 >> 4) & 0b11) as i32;
    let exponent = if b == 1 { cd - 3 } else { cd + 1 };
    let value = f64::from(16 + (imm8 & 0xf)) / 16.0 * 2f64.powi(exponent);
    if negative {
        -value
    } else {
        value
    }
}

fn one_source(word: u32, size: &str) -> Option<Instruction> {
    let (dst, src) = (rd(word), rn(word));
    let opcode = field(word, 15, 6);
    let mnemonic = match opcode {
        0b000000 => "fmov",
        0b000001 => "fabs",
        0b000010 => "fneg",
        0b000011 => "fsqrt",
        0b000100 | 0b000101 | 0b000111 => {
            let target = precision(opcode & 0b11)?;
            if target == size {
                return None;
            }
            return Some(instruction("fcvt", format!("{target}{dst}, {size}{src}")));
        }
        0b001000 => "frintn",
        0b001001 => "frintp",
        0b001010 => "frintm",
        0b001011 => "frintz",
        0b001100 => "frinta",
        0b001110 => "frintx",
        0b001111 => "frinti",
        _ => return None,
    };
    Some(instruction(mnemonic, format!("{size}{dst}, {size}{src}")))
}

fn three_source(word: u32) -> Option<Instruction> {
    let size = precision(field(word, 22, 2))?;
    let mnemonic = match (bit(word, 21), bit(word, 15)) {
        (false, false) => "fmadd",
        (false, true) => "fmsub",
        (true, false) => "fnmadd",
        (true, true) => "fnmsub",
    };
    Some(instruction(
        mnemonic,
        format!(
            "{size}{}, {size}{}, {size}{}, {size}{}",
            rd(word),
            rn(word),
            rm(word),
            ra(word)
        ),
    ))
}

/// Conversions between floating point and integers, and `fmov` between the
/// register files.
fn integer_conversion(word: u32) -> Option<Instruction> {
    let wide = bit(word, 31);
    let ptype = field(word, 22, 2);
    let (dst, src) = (rd(word), rn(word));
    let gpr = |number| register(number, wide, false);
    let (mnemonic, to_integer) = match (field(word, 19, 2), field(word, 16, 3)) {
        (0b00, 0b000) => ("fcvtns", true),
        (0b00, 0b001) => ("fcvtnu", true),
        (0b00, 0b010) => ("scvtf", false),
        (0b00, 0b011) => ("ucvtf", false),
        (0b00, 0b100) => ("fcvtas", true),
        (0b00, 0b101) => ("fcvtau", true),
        (0b00, 0b110) => ("fmov", true),
        (0b00, 0b111) => ("fmov", false),
        (0b01, 0b000) => ("fcvtps", true),
        (0b01, 0b001) => ("fcvtpu", true),
        (0b01, 0b110) if wide && ptype == 0b10 => {
            return Some(instruction("fmov.d", format!("{}, v{src}[1]", gpr(dst))))
        }
        (0b01, 0b111) if wide && ptype == 0b10 => {
            return Some(instruction("fmov.d", format!("v{dst}[1], {}", gpr(src))))
        }
        (0b10, 0b000) => ("fcvtms", true),
        (0b10, 0b001) => ("fcvtmu", true),
        (0b11, 0b000) => ("fcvtzs", true),
        (0b11, 0b001) => ("fcvtzu", true),
        (0b11, 0b110) if !wide && ptype == 0b01 => {
            return Some(instruction("fjcvtzs", format!("{}, d{src}", gpr(dst))))
        }
        _ => return None,
    };
    let size = precision(ptype)?;
    if mnemonic == "fmov" && size != "h" && (size == "d") != wide {
        return None;
    }
    Some(if to_integer {
        instruction(mnemonic, format!("{}, {size}{src}", gpr(dst)))
    } else {
        instruction(mnemonic, format!("{size}{dst}, {}", gpr(src)))
    })
}

/// `scvtf`/`ucvtf`/`fcvtzs`/`fcvtzu` with a count of fraction bits.
fn fixed_point(word: u32) -> Option<Instruction> {
    let wide = bit(word, 31);
    let size = precision(field(word, 22, 2))?;
    let scale = field(word, 10, 6);
    if !wide && scale < 32 {
        return None;
    }
    let fraction = hex(u64::from(64 - scale));
    let (dst, src) = (rd(word), rn(word));
    let gpr = |number| register(number, wide, false);
    Some(match (field(word, 19, 2), field(word, 16, 3)) {
        (0b00, 0b010) => instruction("scvtf", format!("{size}{dst}, {}, {fraction}", gpr(src))),
        (0b00, 0b011) => instruction("ucvtf", format!("{size}{dst}, {}, {fraction}", gpr(src))),
        (0b11, 0b000) => instruction("fcvtzs", format!("{}, {size}{src}, {fraction}", gpr(dst))),
        (0b11, 0b001) => instruction("fcvtzu", format!("{}, {size}{src}, {fraction}", gpr(dst))),
        _ => return None,
    })
}
