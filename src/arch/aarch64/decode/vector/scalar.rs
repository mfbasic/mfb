//! plan-176-O: the AdvSIMD decoder's scalar forms (`scalar` and its families). The vector forms
//! and the shared arrangement/element helpers live in `decode/vector/mod.rs`.

use super::*;

/// The scalar forms of Advanced SIMD (`add d0, d1, d2`, `ushr d0, d1, #0x3`).
pub(super) fn scalar(word: u32) -> Option<Instruction> {
    let pattern = |mask: u32, bits: u32| word & mask == bits;
    if pattern(0xdfe0_8400, 0x5e00_0400) {
        return scalar_copy(word);
    }
    if pattern(0xdf20_0400, 0x5e20_0400) {
        return scalar_three_same(word);
    }
    if pattern(0xdf60_c400, 0x5e40_0400) {
        return scalar_half_three_same(word);
    }
    if pattern(0xdf20_0c00, 0x5e20_0000) {
        return scalar_three_different(word);
    }
    if pattern(0xdf3e_0c00, 0x5e20_0800) {
        return scalar_two_misc(word);
    }
    if pattern(0xdf3e_0c00, 0x5e30_0800) {
        return scalar_pairwise(word);
    }
    if pattern(0xdf80_0400, 0x5f00_0400) && field(word, 19, 4) != 0 {
        return scalar_shift(word);
    }
    if pattern(0xdf00_0400, 0x5f00_0000) {
        return scalar_indexed(word);
    }
    None
}

/// `mov b0, v1[3]` (`dup` of one element into a scalar).
fn scalar_copy(word: u32) -> Option<Instruction> {
    if bit(word, 29) || field(word, 11, 4) != 0 {
        return None;
    }
    let imm5 = field(word, 16, 5);
    let size = imm5.trailing_zeros();
    if size > 3 {
        return None;
    }
    Some(instruction(
        "mov",
        format!(
            "{}, v{}[{}]",
            scalar_register(size, rd(word)),
            rn(word),
            imm5 >> (size + 1)
        ),
    ))
}

fn scalar_three(mnemonic: &str, size: u32, word: u32) -> Instruction {
    instruction(
        mnemonic,
        format!(
            "{}, {}, {}",
            scalar_register(size, rd(word)),
            scalar_register(size, rn(word)),
            scalar_register(size, rm(word))
        ),
    )
}

fn scalar_three_same(word: u32) -> Option<Instruction> {
    let unsigned = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 11, 5);
    if opcode >= 0b11000 {
        let mnemonic = scalar_float_three_same(unsigned, size >> 1 == 1, opcode & 0b111)?;
        return Some(scalar_three(mnemonic, 2 + (size & 1), word));
    }
    let (mnemonic, any_size) = match (unsigned, opcode) {
        (false, 0b00001) => ("sqadd", true),
        (false, 0b00101) => ("sqsub", true),
        (false, 0b00110) => ("cmgt", false),
        (false, 0b00111) => ("cmge", false),
        (false, 0b01000) => ("sshl", false),
        (false, 0b01001) => ("sqshl", true),
        (false, 0b01010) => ("srshl", false),
        (false, 0b01011) => ("sqrshl", true),
        (false, 0b10000) => ("add", false),
        (false, 0b10001) => ("cmtst", false),
        (false, 0b10110) if size == 1 || size == 2 => ("sqdmulh", true),
        (true, 0b00001) => ("uqadd", true),
        (true, 0b00101) => ("uqsub", true),
        (true, 0b00110) => ("cmhi", false),
        (true, 0b00111) => ("cmhs", false),
        (true, 0b01000) => ("ushl", false),
        (true, 0b01001) => ("uqshl", true),
        (true, 0b01010) => ("urshl", false),
        (true, 0b01011) => ("uqrshl", true),
        (true, 0b10000) => ("sub", false),
        (true, 0b10001) => ("cmeq", false),
        (true, 0b10110) if size == 1 || size == 2 => ("sqrdmulh", true),
        _ => return None,
    };
    if !any_size && size != 3 {
        return None;
    }
    Some(scalar_three(mnemonic, size, word))
}

/// The floating-point operations of the scalar three-same group.
fn scalar_float_three_same(unsigned: bool, high: bool, opcode: u32) -> Option<&'static str> {
    Some(match (unsigned, high, opcode) {
        (false, false, 0b011) => "fmulx",
        (false, false, 0b100) => "fcmeq",
        (false, false, 0b111) => "frecps",
        (false, true, 0b111) => "frsqrts",
        (true, false, 0b100) => "fcmge",
        (true, false, 0b101) => "facge",
        (true, true, 0b010) => "fabd",
        (true, true, 0b100) => "fcmgt",
        (true, true, 0b101) => "facgt",
        _ => return None,
    })
}

/// Half-precision scalar three-same: `fabd h0, h1, h2`.
fn scalar_half_three_same(word: u32) -> Option<Instruction> {
    let mnemonic = scalar_float_three_same(bit(word, 29), bit(word, 23), field(word, 11, 3))?;
    Some(scalar_three(mnemonic, 1, word))
}

/// `sqdmull d0, s1, s2` and its accumulating forms.
fn scalar_three_different(word: u32) -> Option<Instruction> {
    let size = field(word, 22, 2);
    if bit(word, 29) || !(size == 1 || size == 2) {
        return None;
    }
    let mnemonic = match field(word, 12, 4) {
        0b1001 => "sqdmlal",
        0b1011 => "sqdmlsl",
        0b1101 => "sqdmull",
        _ => return None,
    };
    Some(instruction(
        mnemonic,
        format!(
            "{}, {}, {}",
            scalar_register(size + 1, rd(word)),
            scalar_register(size, rn(word)),
            scalar_register(size, rm(word))
        ),
    ))
}

fn scalar_two_misc(word: u32) -> Option<Instruction> {
    let unsigned = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 5);
    let high = size >> 1;
    let (d, n) = (rd(word), rn(word));
    let same = |mnemonic: &str, size: u32, rest: &str| {
        instruction(
            mnemonic,
            format!(
                "{}, {}{rest}",
                scalar_register(size, d),
                scalar_register(size, n)
            ),
        )
    };
    let float = 2 + (size & 1);
    Some(match (unsigned, opcode) {
        (false, 0b00011) => same("suqadd", size, ""),
        (false, 0b00111) => same("sqabs", size, ""),
        (true, 0b00011) => same("usqadd", size, ""),
        (true, 0b00111) => same("sqneg", size, ""),
        (false, 0b01000) if size == 3 => same("cmgt", 3, ", #0"),
        (false, 0b01001) if size == 3 => same("cmeq", 3, ", #0"),
        (false, 0b01010) if size == 3 => same("cmlt", 3, ", #0"),
        (false, 0b01011) if size == 3 => same("abs", 3, ""),
        (true, 0b01000) if size == 3 => same("cmge", 3, ", #0"),
        (true, 0b01001) if size == 3 => same("cmle", 3, ", #0"),
        (true, 0b01011) if size == 3 => same("neg", 3, ""),
        (false, 0b10100) | (true, 0b10010) | (true, 0b10100) if size < 3 => {
            let mnemonic = match (unsigned, opcode) {
                (false, _) => "sqxtn",
                (true, 0b10010) => "sqxtun",
                _ => "uqxtn",
            };
            instruction(
                mnemonic,
                format!(
                    "{}, {}",
                    scalar_register(size, d),
                    scalar_register(size + 1, n)
                ),
            )
        }
        (true, 0b10110) if size == 1 => instruction("fcvtxn", format!("s{d}, d{n}")),
        (false, 0b01100) if high == 1 => same("fcmgt", float, ", #0.0"),
        (false, 0b01101) if high == 1 => same("fcmeq", float, ", #0.0"),
        (false, 0b01110) if high == 1 => same("fcmlt", float, ", #0.0"),
        (true, 0b01100) if high == 1 => same("fcmge", float, ", #0.0"),
        (true, 0b01101) if high == 1 => same("fcmle", float, ", #0.0"),
        (_, 0b11010..=0b11101 | 0b11111) => {
            let mnemonic = match (unsigned, high, opcode) {
                (false, 0, 0b11010) => "fcvtns",
                (false, 0, 0b11011) => "fcvtms",
                (false, 0, 0b11100) => "fcvtas",
                (false, 0, 0b11101) => "scvtf",
                (false, 1, 0b11010) => "fcvtps",
                (false, 1, 0b11011) => "fcvtzs",
                (false, 1, 0b11101) => "frecpe",
                (false, 1, 0b11111) => "frecpx",
                (true, 0, 0b11010) => "fcvtnu",
                (true, 0, 0b11011) => "fcvtmu",
                (true, 0, 0b11100) => "fcvtau",
                (true, 0, 0b11101) => "ucvtf",
                (true, 1, 0b11010) => "fcvtpu",
                (true, 1, 0b11011) => "fcvtzu",
                (true, 1, 0b11101) => "frsqrte",
                _ => return None,
            };
            same(mnemonic, float, "")
        }
        _ => return None,
    })
}

/// Pairwise reductions of two lanes into a scalar: `addp.2d d0, v1`.
fn scalar_pairwise(word: u32) -> Option<Instruction> {
    let size = field(word, 22, 2);
    let (d, n) = (rd(word), rn(word));
    let half = !bit(word, 29);
    if half && size == 3 && field(word, 12, 5) == 0b11011 {
        return Some(instruction("addp.2d", format!("d{d}, v{n}")));
    }
    if half && size & 1 == 1 {
        return None;
    }
    let minimum = size >> 1 == 1;
    let mnemonic = match (field(word, 12, 5), minimum) {
        (0b01100, false) => "fmaxnmp",
        (0b01100, true) => "fminnmp",
        (0b01101, false) => "faddp",
        (0b01111, false) => "fmaxp",
        (0b01111, true) => "fminp",
        _ => return None,
    };
    let (arranged, result) = if half {
        ("2h", format!("h{d}"))
    } else if size & 1 == 1 {
        ("2d", format!("d{d}"))
    } else {
        ("2s", format!("s{d}"))
    };
    Some(instruction(
        &format!("{mnemonic}.{arranged}"),
        format!("{result}, v{n}"),
    ))
}

fn scalar_shift(word: u32) -> Option<Instruction> {
    let unsigned = bit(word, 29);
    let size = shift_size(field(word, 19, 4));
    let esize = 8 << size;
    let combined = field(word, 16, 7);
    let right = 2 * esize - combined;
    let left = combined - esize;
    let (d, n) = (rd(word), rn(word));
    let same = |mnemonic: &str, amount: u32| {
        instruction(
            mnemonic,
            format!(
                "{}, {}, {}",
                scalar_register(size, d),
                scalar_register(size, n),
                hex(u64::from(amount))
            ),
        )
    };
    let doubles = size == 3;
    Some(match (unsigned, field(word, 11, 5)) {
        (false, 0b00000) if doubles => same("sshr", right),
        (false, 0b00010) if doubles => same("ssra", right),
        (false, 0b00100) if doubles => same("srshr", right),
        (false, 0b00110) if doubles => same("srsra", right),
        (false, 0b01010) if doubles => same("shl", left),
        (false, 0b01110) => same("sqshl", left),
        (true, 0b00000) if doubles => same("ushr", right),
        (true, 0b00010) if doubles => same("usra", right),
        (true, 0b00100) if doubles => same("urshr", right),
        (true, 0b00110) if doubles => same("ursra", right),
        (true, 0b01000) if doubles => same("sri", right),
        (true, 0b01010) if doubles => same("sli", left),
        (true, 0b01100) => same("sqshlu", left),
        (true, 0b01110) => same("uqshl", left),
        (_, opcode @ 0b10000..=0b10011) if size < 3 => {
            let mnemonic = match (unsigned, opcode) {
                (false, 0b10010) => "sqshrn",
                (false, 0b10011) => "sqrshrn",
                (true, 0b10000) => "sqshrun",
                (true, 0b10001) => "sqrshrun",
                (true, 0b10010) => "uqshrn",
                (true, 0b10011) => "uqrshrn",
                _ => return None,
            };
            instruction(
                mnemonic,
                format!(
                    "{}, {}, {}",
                    scalar_register(size, d),
                    scalar_register(size + 1, n),
                    hex(u64::from(esize - left))
                ),
            )
        }
        (_, opcode @ (0b11100 | 0b11111)) if size >= 2 => {
            let mnemonic = match (unsigned, opcode) {
                (false, 0b11100) => "scvtf",
                (true, 0b11100) => "ucvtf",
                (false, _) => "fcvtzs",
                (true, _) => "fcvtzu",
            };
            same(mnemonic, right)
        }
        _ => return None,
    })
}

/// Scalar operations by an element: `fmla.s s0, s1, v2[1]`.
fn scalar_indexed(word: u32) -> Option<Instruction> {
    let unsigned = bit(word, 29);
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 4);
    let (d, n) = (rd(word), rn(word));
    if let Some(mnemonic) = float_by_element(unsigned, opcode) {
        let size = match size {
            0 => 1,
            1 => return None,
            _ => size,
        };
        let (index, m) = by_element(word, size)?;
        return Some(instruction(
            &format!("{mnemonic}.{}", element(size)),
            format!(
                "{}, {}, v{m}[{index}]",
                scalar_register(size, d),
                scalar_register(size, n)
            ),
        ));
    }
    if size != 1 && size != 2 || unsigned {
        return None;
    }
    let (index, m) = by_element(word, size)?;
    let (mnemonic, long) = match opcode {
        0b0011 => ("sqdmlal", true),
        0b0111 => ("sqdmlsl", true),
        0b1011 => ("sqdmull", true),
        0b1100 => ("sqdmulh", false),
        0b1101 => ("sqrdmulh", false),
        _ => return None,
    };
    Some(instruction(
        &format!("{mnemonic}.{}", element(size)),
        format!(
            "{}, {}, v{m}[{index}]",
            scalar_register(size + u32::from(long), d),
            scalar_register(size, n)
        ),
    ))
}
