//! Integer data processing: the immediate forms (`adr`, add/subtract, logical,
//! move wide, bitfield, extract) and the register forms (logical, add/subtract,
//! with carry, conditional compare and select, one, two and three sources).

use super::{
    bit, condition, field, hex, instruction, ra, rd, register, rm, rn, sign_extend, signed_hex,
    to_target, w, x, Flow, Instruction,
};

pub(super) fn immediate(word: u32, pc: u64) -> Option<Instruction> {
    let wide = bit(word, 31);
    match field(word, 23, 3) {
        0b000 | 0b001 => Some(pc_relative(word, pc)),
        0b010 => add_immediate(word, wide),
        0b011 if bit(word, 22) => min_max_immediate(word, wide),
        0b100 => logical_immediate(word, wide),
        0b101 => move_wide(word, wide),
        0b110 => bitfield(word, wide),
        0b111 => extract(word, wide),
        _ => None,
    }
}

/// `adr`, and `adrp` as a count of 4 KiB pages (as LLVM prints it).
fn pc_relative(word: u32, pc: u64) -> Instruction {
    let immediate = sign_extend((field(word, 5, 19) << 2) | field(word, 29, 2), 21);
    if bit(word, 31) {
        let page = (pc & !0xfff).wrapping_add((immediate << 12) as u64);
        let mut adrp = instruction("adrp", format!("{}, {immediate}", x(rd(word), false)));
        adrp.page = Some((rd(word), page));
        adrp
    } else {
        let target = pc.wrapping_add(immediate as u64);
        to_target(
            "adr",
            &format!("{}, ", x(rd(word), false)),
            target,
            Flow::Next,
        )
    }
}

fn add_immediate(word: u32, wide: bool) -> Option<Instruction> {
    let subtract = bit(word, 30);
    let set_flags = bit(word, 29);
    let shifted = bit(word, 22);
    let imm12 = u64::from(field(word, 10, 12));
    let immediate = if shifted {
        format!("{}, lsl #12", hex(imm12))
    } else {
        hex(imm12)
    };
    let (dst, src) = (rd(word), rn(word));
    let source = register(src, wide, true);
    Some(match (subtract, set_flags) {
        (false, false) if imm12 == 0 && !shifted && (dst == 31 || src == 31) => {
            instruction("mov", format!("{}, {source}", register(dst, wide, true)))
        }
        (_, true) if dst == 31 => instruction(
            if subtract { "cmp" } else { "cmn" },
            format!("{source}, {immediate}"),
        ),
        _ => {
            let mnemonic = match (subtract, set_flags) {
                (false, false) => "add",
                (false, true) => "adds",
                (true, false) => "sub",
                (true, true) => "subs",
            };
            let mut decoded = instruction(
                mnemonic,
                format!("{}, {source}, {immediate}", register(dst, wide, !set_flags)),
            );
            if mnemonic == "add" && wide && !shifted {
                decoded.page_offset = Some((src, imm12));
            }
            decoded
        }
    })
}

/// `smax`, `umax`, `smin`, `umin` of an 8-bit immediate.
fn min_max_immediate(word: u32, wide: bool) -> Option<Instruction> {
    let opc = field(word, 18, 4);
    if opc > 0b0011 {
        return None;
    }
    let mnemonic = ["smax", "umax", "smin", "umin"][opc as usize];
    let imm8 = field(word, 10, 8);
    let immediate = if opc & 1 == 0 {
        signed_hex(sign_extend(imm8, 8))
    } else {
        hex(u64::from(imm8))
    };
    Some(instruction(
        mnemonic,
        format!(
            "{}, {}, {immediate}",
            register(rd(word), wide, false),
            register(rn(word), wide, false)
        ),
    ))
}

/// The value a logical immediate's `N:immr:imms` encodes, replicated across
/// a 32- or 64-bit register; `None` for a reserved encoding.
fn bitmask(n: u32, imms: u32, immr: u32, wide: bool) -> Option<u64> {
    let combined = (n << 6) | (!imms & 0x3f);
    if combined == 0 || (!wide && n == 1) {
        return None;
    }
    let length = 31 - combined.leading_zeros();
    if length < 1 {
        return None;
    }
    let size = 1u32 << length;
    let levels = size - 1;
    let (ones, rotate) = (imms & levels, immr & levels);
    if ones == levels {
        return None;
    }
    let mask = if size == 64 {
        u64::MAX
    } else {
        (1u64 << size) - 1
    };
    let run = (1u64 << (ones + 1)) - 1;
    let element = if rotate == 0 {
        run
    } else {
        ((run >> rotate) | (run << (size - rotate))) & mask
    };
    let width = if wide { 64 } else { 32 };
    let mut value = 0u64;
    let mut at = 0;
    while at < width {
        value |= element << at;
        at += size;
    }
    Some(value)
}

/// Whether a `mov` of this bitmask is printed as `orr`, because a `movz` or
/// `movn` would set it (the ARM `MoveWidePreferred`).
fn move_wide_preferred(wide: bool, n: u32, imms: u32, immr: u32) -> bool {
    let width = if wide { 64 } else { 32 };
    if wide && n != 1 || !wide && (n != 0 || imms & 0x20 != 0) {
        return false;
    }
    if imms < 16 {
        return (16 - immr % 16) % 16 <= 15 - imms;
    }
    if imms >= width - 15 {
        return immr % 16 <= imms - (width - 15);
    }
    false
}

fn logical_immediate(word: u32, wide: bool) -> Option<Instruction> {
    let (n, immr, imms) = (field(word, 22, 1), field(word, 16, 6), field(word, 10, 6));
    let value = bitmask(n, imms, immr, wide)?;
    let (dst, src) = (rd(word), rn(word));
    let opc = field(word, 29, 2);
    let source = register(src, wide, false);
    Some(match opc {
        0b01 if src == 31 && !move_wide_preferred(wide, n, imms, immr) => {
            let value = if wide {
                value as i64
            } else {
                i64::from(value as u32 as i32)
            };
            let mut mov = instruction(
                "mov",
                format!("{}, {}", register(dst, wide, true), signed_hex(value)),
            );
            mov.value = Some(i128::from(value));
            mov
        }
        0b11 if dst == 31 => instruction("tst", format!("{source}, {}", hex(value))),
        _ => {
            let mnemonic = ["and", "orr", "eor", "ands"][opc as usize];
            instruction(
                mnemonic,
                format!(
                    "{}, {source}, {}",
                    register(dst, wide, opc != 0b11),
                    hex(value)
                ),
            )
        }
    })
}

fn move_wide(word: u32, wide: bool) -> Option<Instruction> {
    let hw = field(word, 21, 2);
    if !wide && hw > 1 {
        return None;
    }
    let imm16 = u64::from(field(word, 5, 16));
    let shift = hw * 16;
    let dst = register(rd(word), wide, false);
    let shifted = |mnemonic: &str| {
        let lsl = if hw == 0 {
            String::new()
        } else {
            format!(", lsl #{shift}")
        };
        instruction(mnemonic, format!("{dst}, {}{lsl}", hex(imm16)))
    };
    let alias = |text: String, value: i128| {
        let mut mov = instruction("mov", format!("{dst}, {text}"));
        mov.value = Some(value);
        mov
    };
    // LLVM prints the value a `mov` sets as a signed number of the
    // register's width: `mov w9, #-0x80000000`.
    let signed = |value: u64| {
        let value = if wide {
            value as i64
        } else {
            i64::from(value as u32 as i32)
        };
        alias(signed_hex(value), i128::from(value))
    };
    Some(match field(word, 29, 2) {
        0b00 if imm16 == 0 && hw != 0 || !wide && imm16 == 0xffff => shifted("movn"),
        0b00 => signed(!(imm16 << shift)),
        0b10 if imm16 == 0 && hw != 0 => shifted("movz"),
        0b10 => signed(imm16 << shift),
        0b11 => shifted("movk"),
        _ => return None,
    })
}

fn bitfield(word: u32, wide: bool) -> Option<Instruction> {
    let n = field(word, 22, 1);
    if n != u32::from(wide) {
        return None;
    }
    let width = if wide { 64 } else { 32 };
    let (immr, imms) = (field(word, 16, 6), field(word, 10, 6));
    if !wide && (immr > 31 || imms > 31) {
        return None;
    }
    let dst = register(rd(word), wide, false);
    let src = register(rn(word), wide, false);
    let narrow_src = w(rn(word));
    let shift =
        |mnemonic: &str, amount: u32| instruction(mnemonic, format!("{dst}, {src}, #{amount}"));
    let field_of = |mnemonic: &str, lsb: u32, length: u32| {
        instruction(mnemonic, format!("{dst}, {src}, #{lsb}, #{length}"))
    };
    let insert = imms < immr;
    Some(match field(word, 29, 2) {
        0b00 if imms == width - 1 => shift("asr", immr),
        0b00 if immr == 0 && imms == 7 => instruction("sxtb", format!("{dst}, {narrow_src}")),
        0b00 if immr == 0 && imms == 15 => instruction("sxth", format!("{dst}, {narrow_src}")),
        0b00 if immr == 0 && imms == 31 => instruction("sxtw", format!("{dst}, {narrow_src}")),
        0b00 if insert => field_of("sbfiz", width - immr, imms + 1),
        0b00 => field_of("sbfx", immr, imms - immr + 1),
        // `bfc` is `bfm` from the zero register: a lsb of 0 (`immr == 0`) is
        // spelled `bfc` too, not `bfxil` of the zero register.
        0b01 if rn(word) == 31 && (insert || immr == 0) => instruction(
            "bfc",
            format!("{dst}, #{}, #{}", (width - immr) % width, imms + 1),
        ),
        0b01 if insert => field_of("bfi", width - immr, imms + 1),
        0b01 => field_of("bfxil", immr, imms - immr + 1),
        0b10 if imms == width - 1 => shift("lsr", immr),
        0b10 if imms + 1 == immr => shift("lsl", width - 1 - imms),
        0b10 if !wide && immr == 0 && imms == 7 => instruction("uxtb", format!("{dst}, {src}")),
        0b10 if !wide && immr == 0 && imms == 15 => instruction("uxth", format!("{dst}, {src}")),
        0b10 if insert => field_of("ubfiz", width - immr, imms + 1),
        0b10 => field_of("ubfx", immr, imms - immr + 1),
        _ => return None,
    })
}

fn extract(word: u32, wide: bool) -> Option<Instruction> {
    let imms = field(word, 10, 6);
    if field(word, 29, 2) != 0
        || bit(word, 21)
        || field(word, 22, 1) != u32::from(wide)
        || !wide && imms > 31
    {
        return None;
    }
    let dst = register(rd(word), wide, false);
    let (first, second) = (rn(word), rm(word));
    let amount = hex(u64::from(imms));
    Some(if first == second {
        instruction(
            "ror",
            format!("{dst}, {}, {amount}", register(first, wide, false)),
        )
    } else {
        instruction(
            "extr",
            format!(
                "{dst}, {}, {}, {amount}",
                register(first, wide, false),
                register(second, wide, false)
            ),
        )
    })
}

pub(super) fn registers(word: u32) -> Option<Instruction> {
    let wide = bit(word, 31);
    if !bit(word, 28) {
        return if !bit(word, 24) {
            logical_shifted(word, wide)
        } else if !bit(word, 21) {
            add_shifted(word, wide)
        } else {
            add_extended(word, wide)
        };
    }
    match field(word, 21, 4) {
        0b0000 if field(word, 10, 6) == 0 => with_carry(word, wide),
        0b0010 => conditional_compare(word, wide),
        0b0100 => conditional_select(word, wide),
        0b0110 if bit(word, 30) => one_source(word, wide),
        0b0110 => two_source(word, wide),
        0b1000..=0b1111 => three_source(word, wide),
        _ => None,
    }
}

/// `, lsl #3` (and `lsr`, `asr`, `ror`); nothing for `lsl #0`.
fn shift_suffix(kind: u32, amount: u32) -> String {
    if kind == 0 && amount == 0 {
        String::new()
    } else {
        format!(
            ", {} #{amount}",
            ["lsl", "lsr", "asr", "ror"][kind as usize]
        )
    }
}

fn logical_shifted(word: u32, wide: bool) -> Option<Instruction> {
    let (kind, amount) = (field(word, 22, 2), field(word, 10, 6));
    if !wide && amount > 31 {
        return None;
    }
    let opc = field(word, 29, 2);
    let invert = bit(word, 21);
    let (dst, first, second) = (rd(word), rn(word), rm(word));
    let reg = |number| register(number, wide, false);
    let shifted = format!("{}{}", reg(second), shift_suffix(kind, amount));
    Some(match (opc, invert) {
        (0b01, false) if first == 31 && kind == 0 && amount == 0 => {
            instruction("mov", format!("{}, {}", reg(dst), reg(second)))
        }
        (0b01, true) if first == 31 => instruction("mvn", format!("{}, {shifted}", reg(dst))),
        (0b11, false) if dst == 31 => instruction("tst", format!("{}, {shifted}", reg(first))),
        _ => {
            let mnemonic = match (opc, invert) {
                (0b00, false) => "and",
                (0b00, true) => "bic",
                (0b01, false) => "orr",
                (0b01, true) => "orn",
                (0b10, false) => "eor",
                (0b10, true) => "eon",
                (0b11, false) => "ands",
                _ => "bics",
            };
            instruction(mnemonic, format!("{}, {}, {shifted}", reg(dst), reg(first)))
        }
    })
}

fn add_shifted(word: u32, wide: bool) -> Option<Instruction> {
    let (kind, amount) = (field(word, 22, 2), field(word, 10, 6));
    if kind == 0b11 || !wide && amount > 31 {
        return None;
    }
    let subtract = bit(word, 30);
    let set_flags = bit(word, 29);
    let (dst, first, second) = (rd(word), rn(word), rm(word));
    let reg = |number| register(number, wide, false);
    let shifted = format!("{}{}", reg(second), shift_suffix(kind, amount));
    Some(if set_flags && dst == 31 {
        instruction(
            if subtract { "cmp" } else { "cmn" },
            format!("{}, {shifted}", reg(first)),
        )
    } else if subtract && first == 31 {
        instruction(
            if set_flags { "negs" } else { "neg" },
            format!("{}, {shifted}", reg(dst)),
        )
    } else {
        let mnemonic = match (subtract, set_flags) {
            (false, false) => "add",
            (false, true) => "adds",
            (true, false) => "sub",
            (true, true) => "subs",
        };
        instruction(mnemonic, format!("{}, {}, {shifted}", reg(dst), reg(first)))
    })
}

fn add_extended(word: u32, wide: bool) -> Option<Instruction> {
    let option = field(word, 13, 3);
    let amount = field(word, 10, 3);
    if field(word, 22, 2) != 0 || amount > 4 {
        return None;
    }
    let subtract = bit(word, 30);
    let set_flags = bit(word, 29);
    let (dst, first, second) = (rd(word), rn(word), rm(word));
    let wide_source = wide && option & 0b011 == 0b011;
    let uses_sp = first == 31 || (!set_flags && dst == 31);
    let extend = if uses_sp && option == if wide { 0b011 } else { 0b010 } {
        if amount == 0 {
            String::new()
        } else {
            format!(", lsl #{amount}")
        }
    } else {
        let name = [
            "uxtb", "uxth", "uxtw", "uxtx", "sxtb", "sxth", "sxtw", "sxtx",
        ][option as usize];
        if amount == 0 {
            format!(", {name}")
        } else {
            format!(", {name} #{amount}")
        }
    };
    let source = format!("{}{extend}", register(second, wide_source, false));
    let first = register(first, wide, true);
    Some(if set_flags && dst == 31 {
        instruction(
            if subtract { "cmp" } else { "cmn" },
            format!("{first}, {source}"),
        )
    } else {
        let mnemonic = match (subtract, set_flags) {
            (false, false) => "add",
            (false, true) => "adds",
            (true, false) => "sub",
            (true, true) => "subs",
        };
        instruction(
            mnemonic,
            format!("{}, {first}, {source}", register(dst, wide, !set_flags)),
        )
    })
}

fn with_carry(word: u32, wide: bool) -> Option<Instruction> {
    let subtract = bit(word, 30);
    let set_flags = bit(word, 29);
    let reg = |number| register(number, wide, false);
    let (dst, first, second) = (rd(word), rn(word), rm(word));
    Some(if subtract && first == 31 {
        instruction(
            if set_flags { "ngcs" } else { "ngc" },
            format!("{}, {}", reg(dst), reg(second)),
        )
    } else {
        let mnemonic = match (subtract, set_flags) {
            (false, false) => "adc",
            (false, true) => "adcs",
            (true, false) => "sbc",
            (true, true) => "sbcs",
        };
        instruction(
            mnemonic,
            format!("{}, {}, {}", reg(dst), reg(first), reg(second)),
        )
    })
}

/// `ccmp`/`ccmn` of a register or a 5-bit immediate.
fn conditional_compare(word: u32, wide: bool) -> Option<Instruction> {
    if !bit(word, 29) || bit(word, 10) || bit(word, 4) {
        return None;
    }
    let mnemonic = if bit(word, 30) { "ccmp" } else { "ccmn" };
    let second = if bit(word, 11) {
        hex(u64::from(rm(word)))
    } else {
        register(rm(word), wide, false)
    };
    Some(instruction(
        mnemonic,
        format!(
            "{}, {second}, {}, {}",
            register(rn(word), wide, false),
            hex(u64::from(word & 0xf)),
            condition(field(word, 12, 4))
        ),
    ))
}

/// `csel`, `csinc`, `csinv`, `csneg` and their aliases (`cset`, `cinc`, …).
fn conditional_select(word: u32, wide: bool) -> Option<Instruction> {
    let op2 = field(word, 10, 2);
    if bit(word, 29) || op2 > 1 {
        return None;
    }
    let reg = |number| register(number, wide, false);
    let (dst, first, second) = (rd(word), rn(word), rm(word));
    let code = field(word, 12, 4);
    let invertible = code >> 1 != 0b111;
    let inverted = condition(code ^ 1);
    let negate = bit(word, 30);
    let aliased = |unary: &str, nullary: &str| {
        if first == 31 && second == 31 {
            Some(instruction(nullary, format!("{}, {inverted}", reg(dst))))
        } else if first == second {
            Some(instruction(
                unary,
                format!("{}, {}, {inverted}", reg(dst), reg(first)),
            ))
        } else {
            None
        }
    };
    let alias = match (negate, op2) {
        (false, 1) if invertible => aliased("cinc", "cset"),
        (true, 0) if invertible => aliased("cinv", "csetm"),
        (true, 1) if invertible && first == second => Some(instruction(
            "cneg",
            format!("{}, {}, {inverted}", reg(dst), reg(first)),
        )),
        _ => None,
    };
    Some(alias.unwrap_or_else(|| {
        let mnemonic =
            ["csel", "csinc", "csinv", "csneg"][((u32::from(negate)) << 1 | op2) as usize];
        instruction(
            mnemonic,
            format!(
                "{}, {}, {}, {}",
                reg(dst),
                reg(first),
                reg(second),
                condition(code)
            ),
        )
    }))
}

fn two_source(word: u32, wide: bool) -> Option<Instruction> {
    if bit(word, 29) {
        return None;
    }
    let reg = |number| register(number, wide, false);
    let (dst, first, second) = (rd(word), rn(word), rm(word));
    let opcode = field(word, 10, 6);
    let three = |mnemonic: &str| {
        Some(instruction(
            mnemonic,
            format!("{}, {}, {}", reg(dst), reg(first), reg(second)),
        ))
    };
    match opcode {
        0b000010 => three("udiv"),
        0b000011 => three("sdiv"),
        0b001000 => three("lsl"),
        0b001001 => three("lsr"),
        0b001010 => three("asr"),
        0b001011 => three("ror"),
        0b011000 => three("smax"),
        0b011001 => three("umax"),
        0b011010 => three("smin"),
        0b011011 => three("umin"),
        0b001100 if wide => Some(instruction(
            "pacga",
            format!(
                "{}, {}, {}",
                x(dst, false),
                x(first, false),
                x(second, true)
            ),
        )),
        0b010000..=0b010111 => {
            let size = opcode & 0b11;
            if (size == 0b11) != wide {
                return None;
            }
            let base = if opcode & 0b100 != 0 {
                "crc32c"
            } else {
                "crc32"
            };
            Some(instruction(
                &format!("{base}{}", ["b", "h", "w", "x"][size as usize]),
                format!(
                    "{}, {}, {}",
                    w(dst),
                    w(first),
                    register(second, size == 0b11, false)
                ),
            ))
        }
        _ => None,
    }
}

fn one_source(word: u32, wide: bool) -> Option<Instruction> {
    if bit(word, 29) {
        return None;
    }
    let reg = |number| register(number, wide, false);
    let (dst, src) = (rd(word), rn(word));
    let two = |mnemonic: &str| Some(instruction(mnemonic, format!("{}, {}", reg(dst), reg(src))));
    match (field(word, 16, 5), field(word, 10, 6)) {
        (0, 0b000000) => two("rbit"),
        (0, 0b000001) => two("rev16"),
        (0, 0b000010) if wide => two("rev32"),
        (0, 0b000010) => two("rev"),
        (0, 0b000011) if wide => two("rev"),
        (0, 0b000100) => two("clz"),
        (0, 0b000101) => two("cls"),
        (0, 0b000110) => two("ctz"),
        (0, 0b000111) => two("cnt"),
        (0, 0b001000) => two("abs"),
        (1, opcode @ 0b000000..=0b000111) if wide => {
            let mnemonic = [
                "pacia", "pacib", "pacda", "pacdb", "autia", "autib", "autda", "autdb",
            ][opcode as usize];
            Some(instruction(
                mnemonic,
                format!("{}, {}", x(dst, false), x(src, true)),
            ))
        }
        (1, opcode @ 0b001000..=0b001111) if wide && src == 31 => {
            let mnemonic = [
                "paciza", "pacizb", "pacdza", "pacdzb", "autiza", "autizb", "autdza", "autdzb",
            ][(opcode & 0b111) as usize];
            Some(instruction(mnemonic, x(dst, false)))
        }
        (1, 0b010000) if wide && src == 31 => Some(instruction("xpaci", x(dst, false))),
        (1, 0b010001) if wide && src == 31 => Some(instruction("xpacd", x(dst, false))),
        _ => None,
    }
}

/// `madd`, `msub` and the widening and high multiplies, with their aliases
/// (`mul`, `mneg`, `smull`, …).
fn three_source(word: u32, wide: bool) -> Option<Instruction> {
    if field(word, 29, 2) != 0 {
        return None;
    }
    let (dst, first, second, addend) = (rd(word), rn(word), rm(word), ra(word));
    let subtract = bit(word, 15);
    let reg = |number| register(number, wide, false);
    let (mnemonic, alias, long) = match (field(word, 21, 3), subtract) {
        (0b000, false) => ("madd", "mul", false),
        (0b000, true) => ("msub", "mneg", false),
        (0b001, false) if wide => ("smaddl", "smull", true),
        (0b001, true) if wide => ("smsubl", "smnegl", true),
        (0b101, false) if wide => ("umaddl", "umull", true),
        (0b101, true) if wide => ("umsubl", "umnegl", true),
        (0b010, false) if wide => {
            return Some(instruction(
                "smulh",
                format!("{}, {}, {}", reg(dst), reg(first), reg(second)),
            ))
        }
        (0b110, false) if wide => {
            return Some(instruction(
                "umulh",
                format!("{}, {}, {}", reg(dst), reg(first), reg(second)),
            ))
        }
        _ => return None,
    };
    let source = |number| {
        if long {
            w(number)
        } else {
            reg(number)
        }
    };
    Some(if addend == 31 {
        instruction(
            alias,
            format!("{}, {}, {}", reg(dst), source(first), source(second)),
        )
    } else {
        instruction(
            mnemonic,
            format!(
                "{}, {}, {}, {}",
                reg(dst),
                source(first),
                source(second),
                reg(addend)
            ),
        )
    })
}
