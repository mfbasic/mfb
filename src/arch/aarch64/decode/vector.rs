//! Advanced SIMD, vector and scalar, in Apple's syntax — the arrangement on the
//! mnemonic (`fadd.2d v1, v0, v2`) — and the AES and SHA-1/SHA-256
//! instructions.

use super::memory::{arrangement, register_list};
use super::{bit, field, hex, instruction, rd, register, rm, rn, Instruction};

pub(super) fn decode(word: u32) -> Option<Instruction> {
    let pattern = |mask: u32, bits: u32| word & mask == bits;
    if pattern(0xfe00_0000, 0xce00_0000) {
        return sha3(word);
    }
    if bit(word, 31) {
        return None;
    }
    if pattern(0xff3e_0c00, 0x4e28_0800) {
        return aes(word);
    }
    if pattern(0xff20_8c00, 0x5e00_0000) {
        return sha_three(word);
    }
    if pattern(0xff3e_0c00, 0x5e28_0800) {
        return sha_two(word);
    }
    if bit(word, 28) {
        return if bit(word, 30) { scalar(word) } else { None };
    }
    if pattern(0xbfe0_8c00, 0x0e00_0000) {
        return table(word);
    }
    if pattern(0xbf20_8c00, 0x0e00_0800) {
        return permute(word);
    }
    if pattern(0xbfe0_8400, 0x2e00_0000) {
        return extract(word);
    }
    if pattern(0x9fe0_8400, 0x0e00_0400) {
        return copy(word);
    }
    if pattern(0x9f20_0400, 0x0e20_0400) {
        return three_same(word);
    }
    if pattern(0x9f20_fc00, 0x0e00_9400) && field(word, 22, 2) == 2 {
        let mnemonic = if bit(word, 29) { "udot" } else { "sdot" };
        return Some(three(
            mnemonic,
            if bit(word, 30) { "4s" } else { "2s" },
            word,
        ));
    }
    if pattern(0x9f60_c400, 0x0e40_0400) {
        return half_three_same(word);
    }
    if pattern(0x9f7e_0c00, 0x0e78_0800) {
        return half_two_misc(word);
    }
    if pattern(0x9f20_0c00, 0x0e20_0000) {
        return three_different(word);
    }
    if pattern(0x9f3e_0c00, 0x0e20_0800) {
        return two_misc(word);
    }
    if pattern(0x9f3e_0c00, 0x0e30_0800) {
        return across(word);
    }
    if pattern(0x9ff8_0400, 0x0f00_0400) {
        return modified_immediate(word);
    }
    if pattern(0x9f80_0400, 0x0f00_0400) {
        return shift_immediate(word);
    }
    if pattern(0x9f00_0400, 0x0f00_0000) {
        return indexed(word);
    }
    None
}

/// The arrangement of a floating-point vector: `2s`/`4s` for single, `2d`
/// for double precision (which must be 128 bits).
fn float_arrangement(double: bool, full: bool) -> Option<&'static str> {
    match (double, full) {
        (false, false) => Some("2s"),
        (false, true) => Some("4s"),
        (true, true) => Some("2d"),
        (true, false) => None,
    }
}

/// A scalar register of `2^size` bytes: `b0`, `h0`, `s0`, `d0`.
fn scalar_register(size: u32, number: u32) -> String {
    format!("{}{number}", ["b", "h", "s", "d", "q"][size as usize])
}

/// The letter of a `2^size`-byte element: `b`, `h`, `s`, `d`.
fn element(size: u32) -> &'static str {
    ["b", "h", "s", "d"][size as usize]
}

/// `mnemonic.arrangement vd, vn, vm`.
fn three(mnemonic: &str, arranged: &str, word: u32) -> Instruction {
    instruction(
        &format!("{mnemonic}.{arranged}"),
        format!("v{}, v{}, v{}", rd(word), rn(word), rm(word)),
    )
}

/// `mnemonic.arrangement vd, vn` plus `rest`.
fn two(mnemonic: &str, arranged: &str, word: u32, rest: &str) -> Instruction {
    instruction(
        &format!("{mnemonic}.{arranged}"),
        format!("v{}, v{}{rest}", rd(word), rn(word)),
    )
}

/// `mnemonic`, with `2` added when it works on the upper half (`full`).
fn upper(mnemonic: &str, full: bool) -> String {
    if full {
        format!("{mnemonic}2")
    } else {
        mnemonic.to_string()
    }
}

fn aes(word: u32) -> Option<Instruction> {
    let mnemonic = match field(word, 12, 5) {
        0b00100 => "aese",
        0b00101 => "aesd",
        0b00110 => "aesmc",
        0b00111 => "aesimc",
        _ => return None,
    };
    Some(two(mnemonic, "16b", word, ""))
}

fn sha_three(word: u32) -> Option<Instruction> {
    let (d, n, m) = (rd(word), rn(word), rm(word));
    Some(match field(word, 12, 3) {
        opcode @ 0b000..=0b010 => instruction(
            &format!("{}.4s", ["sha1c", "sha1p", "sha1m"][opcode as usize]),
            format!("q{d}, s{n}, v{m}"),
        ),
        0b011 => three("sha1su0", "4s", word),
        0b100 => instruction("sha256h.4s", format!("q{d}, q{n}, v{m}")),
        0b101 => instruction("sha256h2.4s", format!("q{d}, q{n}, v{m}")),
        0b110 => three("sha256su1", "4s", word),
        _ => return None,
    })
}

fn sha_two(word: u32) -> Option<Instruction> {
    Some(match field(word, 12, 5) {
        0b00000 => instruction("sha1h", format!("s{}, s{}", rd(word), rn(word))),
        0b00001 => two("sha1su1", "4s", word, ""),
        0b00010 => two("sha256su0", "4s", word, ""),
        _ => return None,
    })
}

/// SHA-512 and SHA-3: `sha512h.2d q0, q1, v2`, `eor3.16b`, `bcax.16b`,
/// `rax1.2d`, `xar.2d`.
fn sha3(word: u32) -> Option<Instruction> {
    let pattern = |mask: u32, bits: u32| word & mask == bits;
    let (d, n, m) = (rd(word), rn(word), rm(word));
    if pattern(0xffe0_b000, 0xce60_8000) {
        return Some(match field(word, 10, 2) {
            0b00 => instruction("sha512h.2d", format!("q{d}, q{n}, v{m}")),
            0b01 => instruction("sha512h2.2d", format!("q{d}, q{n}, v{m}")),
            0b10 => three("sha512su1", "2d", word),
            _ => three("rax1", "2d", word),
        });
    }
    if pattern(0xffff_fc00, 0xcec0_8000) {
        return Some(two("sha512su0", "2d", word, ""));
    }
    if pattern(0xff80_8000, 0xce00_0000) {
        let mnemonic = match field(word, 21, 2) {
            0b00 => "eor3",
            0b01 => "bcax",
            _ => return None,
        };
        return Some(instruction(
            &format!("{mnemonic}.16b"),
            format!("v{d}, v{n}, v{m}, v{}", super::ra(word)),
        ));
    }
    if pattern(0xffe0_0000, 0xce80_0000) {
        return Some(instruction(
            "xar.2d",
            format!("v{d}, v{n}, v{m}, {}", hex(u64::from(field(word, 10, 6)))),
        ));
    }
    None
}

/// `tbl`/`tbx` over one to four table registers.
fn table(word: u32) -> Option<Instruction> {
    let full = bit(word, 30);
    let mnemonic = if bit(word, 12) { "tbx" } else { "tbl" };
    let count = field(word, 13, 2) + 1;
    Some(instruction(
        &format!("{mnemonic}.{}", if full { "16b" } else { "8b" }),
        format!(
            "v{}, {}, v{}",
            rd(word),
            register_list(rn(word), count),
            rm(word)
        ),
    ))
}

fn permute(word: u32) -> Option<Instruction> {
    let (size, full) = (field(word, 22, 2), bit(word, 30));
    if size == 3 && !full {
        return None;
    }
    let mnemonic = match field(word, 12, 3) {
        0b001 => "uzp1",
        0b010 => "trn1",
        0b011 => "zip1",
        0b101 => "uzp2",
        0b110 => "trn2",
        0b111 => "zip2",
        _ => return None,
    };
    Some(three(mnemonic, arrangement(size, full), word))
}

fn extract(word: u32) -> Option<Instruction> {
    let full = bit(word, 30);
    let index = field(word, 11, 4);
    if !full && index > 7 {
        return None;
    }
    Some(instruction(
        &format!("ext.{}", if full { "16b" } else { "8b" }),
        format!(
            "v{}, v{}, v{}, {}",
            rd(word),
            rn(word),
            rm(word),
            hex(u64::from(index))
        ),
    ))
}

/// `dup`, `ins` (printed `mov`), `smov` and `umov` (printed `mov` for a full
/// register).
fn copy(word: u32) -> Option<Instruction> {
    let full = bit(word, 30);
    let imm5 = field(word, 16, 5);
    let imm4 = field(word, 11, 4);
    let size = imm5.trailing_zeros();
    if size > 3 {
        return None;
    }
    let index = imm5 >> (size + 1);
    let (d, n) = (rd(word), rn(word));
    let letter = element(size);
    if bit(word, 29) {
        if !full {
            return None;
        }
        let from = imm4 >> size;
        return Some(instruction(
            &format!("mov.{letter}"),
            format!("v{d}[{index}], v{n}[{from}]"),
        ));
    }
    Some(match imm4 {
        0b0000 => {
            if size == 3 && !full {
                return None;
            }
            instruction(
                &format!("dup.{}", arrangement(size, full)),
                format!("v{d}, v{n}[{index}]"),
            )
        }
        0b0001 => {
            if size == 3 && !full {
                return None;
            }
            instruction(
                &format!("dup.{}", arrangement(size, full)),
                format!("v{d}, {}", register(n, size == 3, false)),
            )
        }
        0b0011 if full => instruction(
            &format!("mov.{letter}"),
            format!("v{d}[{index}], {}", register(n, size == 3, false)),
        ),
        0b0101 if size < 2 || size == 2 && full => instruction(
            &format!("smov.{letter}"),
            format!("{}, v{n}[{index}]", register(d, full, false)),
        ),
        0b0111 if (size == 3) == full => {
            let mnemonic = if size >= 2 { "mov" } else { "umov" };
            instruction(
                &format!("{mnemonic}.{letter}"),
                format!("{}, v{n}[{index}]", register(d, full, false)),
            )
        }
        _ => return None,
    })
}

/// Whether an integer three-same operation accepts `.2d`.
fn takes_doubles(unsigned: bool, opcode: u32) -> bool {
    matches!(
        opcode,
        0b00001 | 0b00101 | 0b00110 | 0b00111 | 0b01000..=0b01011 | 0b10000 | 0b10001
    ) || (!unsigned && opcode == 0b10111)
}

fn three_same(word: u32) -> Option<Instruction> {
    let (full, unsigned) = (bit(word, 30), bit(word, 29));
    let size = field(word, 22, 2);
    let opcode = field(word, 11, 5);
    if opcode == 0b00011 {
        let bytes = if full { "16b" } else { "8b" };
        let mnemonic = match (unsigned, size) {
            (false, 0) => "and",
            (false, 1) => "bic",
            (false, 2) if rn(word) == rm(word) => {
                return Some(two("mov", bytes, word, ""));
            }
            (false, 2) => "orr",
            (false, _) => "orn",
            (true, 0) => "eor",
            (true, 1) => "bsl",
            (true, 2) => "bit",
            (true, _) => "bif",
        };
        return Some(three(mnemonic, bytes, word));
    }
    if opcode >= 0b11000 {
        let arranged = float_arrangement(size & 1 == 1, full)?;
        let mnemonic = float_three_same(unsigned, size >> 1 == 1, opcode & 0b111)?;
        return Some(three(mnemonic, arranged, word));
    }
    if size == 3 && (!full || !takes_doubles(unsigned, opcode)) {
        return None;
    }
    let names: [&str; 24] = if unsigned {
        [
            "uhadd", "uqadd", "urhadd", "", "uhsub", "uqsub", "cmhi", "cmhs", "ushl", "uqshl",
            "urshl", "uqrshl", "umax", "umin", "uabd", "uaba", "sub", "cmeq", "mls", "pmul",
            "umaxp", "uminp", "sqrdmulh", "",
        ]
    } else {
        [
            "shadd", "sqadd", "srhadd", "", "shsub", "sqsub", "cmgt", "cmge", "sshl", "sqshl",
            "srshl", "sqrshl", "smax", "smin", "sabd", "saba", "add", "cmtst", "mla", "mul",
            "smaxp", "sminp", "sqdmulh", "addp",
        ]
    };
    let mnemonic = names[opcode as usize];
    let valid = match mnemonic {
        "" => false,
        "pmul" => size == 0,
        "sqdmulh" | "sqrdmulh" => size == 1 || size == 2,
        _ => true,
    };
    valid.then(|| three(mnemonic, arrangement(size, full), word))
}

/// A floating-point three-same operation by `U`, the size's high bit and the
/// low three bits of its opcode (shared by single, double and half precision).
fn float_three_same(unsigned: bool, high: bool, opcode: u32) -> Option<&'static str> {
    Some(match (unsigned, high, opcode) {
        (false, false, 0b000) => "fmaxnm",
        (false, false, 0b001) => "fmla",
        (false, false, 0b010) => "fadd",
        (false, false, 0b011) => "fmulx",
        (false, false, 0b100) => "fcmeq",
        (false, false, 0b110) => "fmax",
        (false, false, 0b111) => "frecps",
        (false, true, 0b000) => "fminnm",
        (false, true, 0b001) => "fmls",
        (false, true, 0b010) => "fsub",
        (false, true, 0b110) => "fmin",
        (false, true, 0b111) => "frsqrts",
        (true, false, 0b000) => "fmaxnmp",
        (true, false, 0b010) => "faddp",
        (true, false, 0b011) => "fmul",
        (true, false, 0b100) => "fcmge",
        (true, false, 0b101) => "facge",
        (true, false, 0b110) => "fmaxp",
        (true, false, 0b111) => "fdiv",
        (true, true, 0b000) => "fminnmp",
        (true, true, 0b010) => "fabd",
        (true, true, 0b100) => "fcmgt",
        (true, true, 0b101) => "facgt",
        (true, true, 0b110) => "fminp",
        _ => return None,
    })
}

/// Half-precision three-same: `fmla.4h v1, v21, v0`.
fn half_three_same(word: u32) -> Option<Instruction> {
    let full = bit(word, 30);
    let mnemonic = float_three_same(bit(word, 29), bit(word, 23), field(word, 11, 3))?;
    Some(three(mnemonic, if full { "8h" } else { "4h" }, word))
}

/// Half-precision two-register miscellaneous: `fneg.4h v1, v1`.
fn half_two_misc(word: u32) -> Option<Instruction> {
    let full = bit(word, 30);
    let (mnemonic, rest) = float_misc(bit(word, 29), bit(word, 23), field(word, 12, 5))?;
    Some(two(mnemonic, if full { "8h" } else { "4h" }, word, rest))
}

/// A floating-point two-register miscellaneous operation by `U`, the size's
/// high bit and its opcode (shared by every precision), with the operand it
/// adds (`, #0.0` for a comparison with zero).
fn float_misc(unsigned: bool, high: bool, opcode: u32) -> Option<(&'static str, &'static str)> {
    let zero = ", #0.0";
    Some(match (unsigned, high, opcode) {
        (false, false, 0b11000) => ("frintn", ""),
        (false, false, 0b11001) => ("frintm", ""),
        (false, false, 0b11010) => ("fcvtns", ""),
        (false, false, 0b11011) => ("fcvtms", ""),
        (false, false, 0b11100) => ("fcvtas", ""),
        (false, false, 0b11101) => ("scvtf", ""),
        (false, true, 0b01100) => ("fcmgt", zero),
        (false, true, 0b01101) => ("fcmeq", zero),
        (false, true, 0b01110) => ("fcmlt", zero),
        (false, true, 0b01111) => ("fabs", ""),
        (false, true, 0b11000) => ("frintp", ""),
        (false, true, 0b11001) => ("frintz", ""),
        (false, true, 0b11010) => ("fcvtps", ""),
        (false, true, 0b11011) => ("fcvtzs", ""),
        (false, true, 0b11101) => ("frecpe", ""),
        (true, false, 0b11000) => ("frinta", ""),
        (true, false, 0b11001) => ("frintx", ""),
        (true, false, 0b11010) => ("fcvtnu", ""),
        (true, false, 0b11011) => ("fcvtmu", ""),
        (true, false, 0b11100) => ("fcvtau", ""),
        (true, false, 0b11101) => ("ucvtf", ""),
        (true, true, 0b01100) => ("fcmge", zero),
        (true, true, 0b01101) => ("fcmle", zero),
        (true, true, 0b01111) => ("fneg", ""),
        (true, true, 0b11001) => ("frinti", ""),
        (true, true, 0b11010) => ("fcvtpu", ""),
        (true, true, 0b11011) => ("fcvtzu", ""),
        (true, true, 0b11101) => ("frsqrte", ""),
        (true, true, 0b11111) => ("fsqrt", ""),
        _ => return None,
    })
}

fn three_different(word: u32) -> Option<Instruction> {
    let (full, unsigned) = (bit(word, 30), bit(word, 29));
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 4);
    let mnemonic = match (unsigned, opcode) {
        (false, 0b0000) => "saddl",
        (false, 0b0001) => "saddw",
        (false, 0b0010) => "ssubl",
        (false, 0b0011) => "ssubw",
        (false, 0b0100) => "addhn",
        (false, 0b0101) => "sabal",
        (false, 0b0110) => "subhn",
        (false, 0b0111) => "sabdl",
        (false, 0b1000) => "smlal",
        (false, 0b1001) => "sqdmlal",
        (false, 0b1010) => "smlsl",
        (false, 0b1011) => "sqdmlsl",
        (false, 0b1100) => "smull",
        (false, 0b1101) => "sqdmull",
        (false, 0b1110) => "pmull",
        (true, 0b0000) => "uaddl",
        (true, 0b0001) => "uaddw",
        (true, 0b0010) => "usubl",
        (true, 0b0011) => "usubw",
        (true, 0b0100) => "raddhn",
        (true, 0b0101) => "uabal",
        (true, 0b0110) => "rsubhn",
        (true, 0b0111) => "uabdl",
        (true, 0b1000) => "umlal",
        (true, 0b1010) => "umlsl",
        (true, 0b1100) => "umull",
        _ => return None,
    };
    let arranged = match mnemonic {
        "pmull" => match size {
            0 => "8h",
            3 => "1q",
            _ => return None,
        },
        _ if size == 3 => return None,
        "sqdmlal" | "sqdmlsl" | "sqdmull" if size == 0 => return None,
        "addhn" | "subhn" | "raddhn" | "rsubhn" => arrangement(size, full),
        _ => arrangement(size + 1, true),
    };
    Some(three(&upper(mnemonic, full), arranged, word))
}

/// The operations of the two-register miscellaneous group and how each
/// prints.
enum Misc {
    /// An integer operation, with the largest element size it takes.
    Integer(u32),
    /// A comparison against zero.
    Zero,
    /// A byte-only operation (`cnt`, `mvn`, `rbit`).
    Bytes,
    /// Pairwise widening: printed with the wider arrangement.
    Widening,
    /// Narrowing (`xtn`): printed with the narrower arrangement.
    Narrowing,
    /// A floating-point operation.
    Float,
    /// A single-precision-only operation (`urecpe`, `ursqrte`).
    Single,
}

fn two_misc(word: u32) -> Option<Instruction> {
    let (full, unsigned) = (bit(word, 30), bit(word, 29));
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 5);
    let high = size >> 1;
    use Misc::*;
    let (mnemonic, kind) = match (unsigned, opcode) {
        (false, 0b00000) => ("rev64", Integer(2)),
        (false, 0b00001) => ("rev16", Integer(0)),
        (false, 0b00010) => ("saddlp", Widening),
        (false, 0b00011) => ("suqadd", Integer(3)),
        (false, 0b00100) => ("cls", Integer(2)),
        (false, 0b00101) if size == 0 => ("cnt", Bytes),
        (false, 0b00110) => ("sadalp", Widening),
        (false, 0b00111) => ("sqabs", Integer(3)),
        (false, 0b01000) => ("cmgt", Zero),
        (false, 0b01001) => ("cmeq", Zero),
        (false, 0b01010) => ("cmlt", Zero),
        (false, 0b01011) => ("abs", Integer(3)),
        (false, 0b10010) => ("xtn", Narrowing),
        (false, 0b10100) => ("sqxtn", Narrowing),
        (true, 0b00000) => ("rev32", Integer(1)),
        (true, 0b00010) => ("uaddlp", Widening),
        (true, 0b00011) => ("usqadd", Integer(3)),
        (true, 0b00100) => ("clz", Integer(2)),
        (true, 0b00101) if size == 0 => ("mvn", Bytes),
        (true, 0b00101) if size == 1 => ("rbit", Bytes),
        (true, 0b00110) => ("uadalp", Widening),
        (true, 0b00111) => ("sqneg", Integer(3)),
        (true, 0b01000) => ("cmge", Zero),
        (true, 0b01001) => ("cmle", Zero),
        (true, 0b01011) => ("neg", Integer(3)),
        (true, 0b10010) => ("sqxtun", Narrowing),
        (true, 0b10011) => return shift_left_long(word),
        (true, 0b10100) => ("uqxtn", Narrowing),
        (_, 0b10110 | 0b10111) if high == 0 => return float_resize(word),
        (false, 0b11110) if high == 0 => ("frint32z", Float),
        (false, 0b11111) if high == 0 => ("frint64z", Float),
        (true, 0b11110) if high == 0 => ("frint32x", Float),
        (true, 0b11111) if high == 0 => ("frint64x", Float),
        (false, 0b11100) if high == 1 => ("urecpe", Single),
        (true, 0b11100) if high == 1 => ("ursqrte", Single),
        (_, 0b01100..=0b01111 | 0b11000..=0b11111) => {
            let (mnemonic, rest) = float_misc(unsigned, high == 1, opcode)?;
            let arranged = float_arrangement(size & 1 == 1, full)?;
            return Some(two(mnemonic, arranged, word, rest));
        }
        _ => return None,
    };
    let integer =
        |largest: u32| ((full || size != 3) && size <= largest).then(|| arrangement(size, full));
    Some(match kind {
        Integer(largest) => two(mnemonic, integer(largest)?, word, ""),
        Zero => two(mnemonic, integer(3)?, word, ", #0"),
        Bytes => two(mnemonic, if full { "16b" } else { "8b" }, word, ""),
        Widening => {
            if size == 3 {
                return None;
            }
            two(mnemonic, arrangement(size + 1, full), word, "")
        }
        Narrowing => {
            if size == 3 {
                return None;
            }
            two(&upper(mnemonic, full), arrangement(size, full), word, "")
        }
        Float => two(mnemonic, float_arrangement(size & 1 == 1, full)?, word, ""),
        Single => {
            if size & 1 == 1 {
                return None;
            }
            two(mnemonic, if full { "4s" } else { "2s" }, word, "")
        }
    })
}

/// `shll`/`shll2`, a left shift by the element width while widening.
fn shift_left_long(word: u32) -> Option<Instruction> {
    let (full, size) = (bit(word, 30), field(word, 22, 2));
    if size == 3 {
        return None;
    }
    Some(two(
        &upper("shll", full),
        arrangement(size + 1, true),
        word,
        &format!(", #{}", 8 << size),
    ))
}

/// `fcvtn`, `fcvtxn` and `fcvtl`, which LLVM prints with an arrangement on each
/// register: `fcvtn v0.2s, v1.2d`.
fn float_resize(word: u32) -> Option<Instruction> {
    let (full, unsigned) = (bit(word, 30), bit(word, 29));
    let double = bit(word, 22);
    let lengthen = bit(word, 12);
    let wide = if double { "2d" } else { "4s" };
    let narrow = match (double, full) {
        (true, false) => "2s",
        (true, true) => "4s",
        (false, false) => "4h",
        (false, true) => "8h",
    };
    let (d, n) = (rd(word), rn(word));
    let mnemonic = match (unsigned, lengthen) {
        (false, false) => "fcvtn",
        (false, true) => "fcvtl",
        (true, false) if double => "fcvtxn",
        _ => return None,
    };
    let operands = if lengthen {
        format!("v{d}.{wide}, v{n}.{narrow}")
    } else {
        format!("v{d}.{narrow}, v{n}.{wide}")
    };
    Some(instruction(&upper(mnemonic, full), operands))
}

/// Reductions across a vector's lanes into a scalar: `addv.8b b0, v1`.
fn across(word: u32) -> Option<Instruction> {
    let (full, unsigned) = (bit(word, 30), bit(word, 29));
    let size = field(word, 22, 2);
    let (d, n) = (rd(word), rn(word));
    let opcode = field(word, 12, 5);
    if unsigned && matches!(opcode, 0b01100 | 0b01111) {
        if size & 1 == 1 || !full {
            return None;
        }
        let minimum = size >> 1 == 1;
        let mnemonic = match (opcode, minimum) {
            (0b01100, false) => "fmaxnmv",
            (0b01100, true) => "fminnmv",
            (_, false) => "fmaxv",
            (_, true) => "fminv",
        };
        return Some(instruction(
            &format!("{mnemonic}.4s"),
            format!("s{d}, v{n}"),
        ));
    }
    if size == 3 || size == 2 && !full {
        return None;
    }
    let (mnemonic, widening) = match (unsigned, opcode) {
        (false, 0b00011) => ("saddlv", true),
        (false, 0b01010) => ("smaxv", false),
        (false, 0b11010) => ("sminv", false),
        (false, 0b11011) => ("addv", false),
        (true, 0b00011) => ("uaddlv", true),
        (true, 0b01010) => ("umaxv", false),
        (true, 0b11010) => ("uminv", false),
        _ => return None,
    };
    let result = if widening { size + 1 } else { size };
    Some(instruction(
        &format!("{mnemonic}.{}", arrangement(size, full)),
        format!("{}, v{n}", scalar_register(result, d)),
    ))
}

/// The 64-bit value of a `movi` byte mask: each bit of `imm8` a byte of ones.
fn byte_mask(imm8: u32) -> u64 {
    (0..8).fold(0, |value, byte| {
        if imm8 & (1 << byte) != 0 {
            value | (0xff << (byte * 8))
        } else {
            value
        }
    })
}

/// A byte mask as LLVM prints it, `printf("%#016llx")`: sixteen zeros for 0,
/// else `0x` and the digits padded to sixteen characters in all.
fn byte_mask_text(value: u64) -> String {
    if value == 0 {
        "#0000000000000000".to_string()
    } else {
        format!("#0x{value:014x}")
    }
}

/// `movi`, `mvni`, `orr`, `bic` and `fmov` of an immediate.
fn modified_immediate(word: u32) -> Option<Instruction> {
    let (full, op) = (bit(word, 30), bit(word, 29));
    let cmode = field(word, 12, 4);
    let imm8 = (field(word, 16, 3) << 5) | field(word, 5, 5);
    let d = rd(word);
    let immediate = hex(u64::from(imm8));
    let words = if full { "4s" } else { "2s" };
    let halves = if full { "8h" } else { "4h" };
    let shifted = |mnemonic: &str, arranged: &str, shift: u32| {
        let lsl = if shift == 0 {
            String::new()
        } else {
            format!(", lsl #{shift}")
        };
        instruction(
            &format!("{mnemonic}.{arranged}"),
            format!("v{d}, {immediate}{lsl}"),
        )
    };
    if bit(word, 11) {
        return None;
    }
    Some(match (cmode, op) {
        (0b0000 | 0b0010 | 0b0100 | 0b0110, _) => {
            shifted(if op { "mvni" } else { "movi" }, words, (cmode >> 1) * 8)
        }
        (0b0001 | 0b0011 | 0b0101 | 0b0111, _) => {
            shifted(if op { "bic" } else { "orr" }, words, (cmode >> 1) * 8)
        }
        (0b1000 | 0b1010, _) => shifted(
            if op { "mvni" } else { "movi" },
            halves,
            ((cmode >> 1) & 1) * 8,
        ),
        (0b1001 | 0b1011, _) => shifted(
            if op { "bic" } else { "orr" },
            halves,
            ((cmode >> 1) & 1) * 8,
        ),
        (0b1100 | 0b1101, _) => instruction(
            &format!("{}.{words}", if op { "mvni" } else { "movi" }),
            format!(
                "v{d}, {immediate}, msl #{}",
                if cmode & 1 == 1 { 16 } else { 8 }
            ),
        ),
        (0b1110, false) => instruction(
            &format!("movi.{}", if full { "16b" } else { "8b" }),
            format!("v{d}, {immediate}"),
        ),
        (0b1110, true) => {
            let mask = byte_mask_text(byte_mask(imm8));
            if full {
                instruction("movi.2d", format!("v{d}, {mask}"))
            } else {
                instruction("movi", format!("d{d}, {mask}"))
            }
        }
        (0b1111, false) => instruction(
            &format!("fmov.{words}"),
            format!("v{d}, #{:.8}", super::float::expand(imm8)),
        ),
        (0b1111, true) if full => instruction(
            "fmov.2d",
            format!("v{d}, #{:.8}", super::float::expand(imm8)),
        ),
        _ => return None,
    })
}

/// The element size (log2 bytes) a shift-by-immediate's `immh` selects.
fn shift_size(immh: u32) -> u32 {
    31 - immh.leading_zeros()
}

/// Shifts of a vector by an immediate, with the narrowing, widening and
/// fixed-point conversion forms.
fn shift_immediate(word: u32) -> Option<Instruction> {
    let (full, unsigned) = (bit(word, 30), bit(word, 29));
    let immh = field(word, 19, 4);
    let size = shift_size(immh);
    let esize = 8 << size;
    let combined = field(word, 16, 7);
    let right = 2 * esize - combined;
    let left = combined - esize;
    let opcode = field(word, 11, 5);
    if size == 3 && !full {
        return None;
    }
    let plain = |mnemonic: &str, amount: u32| {
        two(
            mnemonic,
            arrangement(size, full),
            word,
            &format!(", {}", hex(u64::from(amount))),
        )
    };
    let narrow = |mnemonic: &str| {
        if size == 3 {
            return None;
        }
        let amount = esize - (combined - esize);
        Some(two(
            &upper(mnemonic, full),
            arrangement(size, full),
            word,
            &format!(", {}", hex(u64::from(amount))),
        ))
    };
    Some(match (unsigned, opcode) {
        (false, 0b00000) => plain("sshr", right),
        (false, 0b00010) => plain("ssra", right),
        (false, 0b00100) => plain("srshr", right),
        (false, 0b00110) => plain("srsra", right),
        (false, 0b01010) => plain("shl", left),
        (false, 0b01110) => plain("sqshl", left),
        (true, 0b00000) => plain("ushr", right),
        (true, 0b00010) => plain("usra", right),
        (true, 0b00100) => plain("urshr", right),
        (true, 0b00110) => plain("ursra", right),
        (true, 0b01000) => plain("sri", right),
        (true, 0b01010) => plain("sli", left),
        (true, 0b01100) => plain("sqshlu", left),
        (true, 0b01110) => plain("uqshl", left),
        (false, 0b10000) => narrow("shrn")?,
        (false, 0b10001) => narrow("rshrn")?,
        (false, 0b10010) => narrow("sqshrn")?,
        (false, 0b10011) => narrow("sqrshrn")?,
        (true, 0b10000) => narrow("sqshrun")?,
        (true, 0b10001) => narrow("sqrshrun")?,
        (true, 0b10010) => narrow("uqshrn")?,
        (true, 0b10011) => narrow("uqrshrn")?,
        (_, 0b10100) => {
            if size == 3 {
                return None;
            }
            two(
                &upper(if unsigned { "ushll" } else { "sshll" }, full),
                arrangement(size + 1, true),
                word,
                &format!(", {}", hex(u64::from(left))),
            )
        }
        (_, 0b11100 | 0b11111) if size >= 2 => {
            let mnemonic = match (unsigned, opcode) {
                (false, 0b11100) => "scvtf",
                (true, 0b11100) => "ucvtf",
                (false, _) => "fcvtzs",
                (true, _) => "fcvtzu",
            };
            plain(mnemonic, right)
        }
        _ => return None,
    })
}

/// The element index and register of a by-element operand, by element size.
fn by_element(word: u32, size: u32) -> Option<(u32, u32)> {
    let (h, l, m) = (field(word, 11, 1), field(word, 21, 1), field(word, 20, 1));
    match size {
        1 => Some(((h << 2) | (l << 1) | m, field(word, 16, 4))),
        2 => Some(((h << 1) | l, rm(word))),
        3 if l == 0 => Some((h, rm(word))),
        _ => None,
    }
}

/// A floating-point operation by an element, by `U` and its opcode.
fn float_by_element(unsigned: bool, opcode: u32) -> Option<&'static str> {
    match (unsigned, opcode) {
        (false, 0b0001) => Some("fmla"),
        (false, 0b0101) => Some("fmls"),
        (false, 0b1001) => Some("fmul"),
        (true, 0b1001) => Some("fmulx"),
        _ => None,
    }
}

/// Vector operations by an element: `fmul.2d v0, v1, v2[1]`.
fn indexed(word: u32) -> Option<Instruction> {
    let (full, unsigned) = (bit(word, 30), bit(word, 29));
    let size = field(word, 22, 2);
    let opcode = field(word, 12, 4);
    let (d, n) = (rd(word), rn(word));
    if let Some(mnemonic) = float_by_element(unsigned, opcode) {
        let (index, m) = by_element(word, size.max(1))?;
        let arranged = match size {
            0 => {
                if full {
                    "8h"
                } else {
                    "4h"
                }
            }
            1 => return None,
            _ => float_arrangement(size == 3, full)?,
        };
        return Some(instruction(
            &format!("{mnemonic}.{arranged}"),
            format!("v{d}, v{n}, v{m}[{index}]"),
        ));
    }
    let (index, m) = by_element(word, size)?;
    let operands = format!("v{d}, v{n}, v{m}[{index}]");
    if size != 1 && size != 2 {
        return None;
    }
    let (mnemonic, long) = match (unsigned, opcode) {
        (false, 0b0010) => ("smlal", true),
        (false, 0b0011) => ("sqdmlal", true),
        (false, 0b0110) => ("smlsl", true),
        (false, 0b0111) => ("sqdmlsl", true),
        (false, 0b1000) => ("mul", false),
        (false, 0b1010) => ("smull", true),
        (false, 0b1011) => ("sqdmull", true),
        (false, 0b1100) => ("sqdmulh", false),
        (false, 0b1101) => ("sqrdmulh", false),
        (true, 0b0000) => ("mla", false),
        (true, 0b0010) => ("umlal", true),
        (true, 0b0100) => ("mls", false),
        (true, 0b0110) => ("umlsl", true),
        (true, 0b1010) => ("umull", true),
        (true, 0b1101) => ("sqrdmlah", false),
        (true, 0b1111) => ("sqrdmlsh", false),
        _ => return None,
    };
    Some(if long {
        instruction(
            &format!("{}.{}", upper(mnemonic, full), arrangement(size + 1, true)),
            operands,
        )
    } else {
        instruction(&format!("{mnemonic}.{}", arrangement(size, full)), operands)
    })
}

/// The scalar forms of Advanced SIMD (`add d0, d1, d2`, `ushr d0, d1, #0x3`).
fn scalar(word: u32) -> Option<Instruction> {
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
