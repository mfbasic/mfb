//! The `F` and `D` extensions: loads and stores, arithmetic, conversions,
//! moves, comparisons and the fused multiply-adds, with the rounding mode
//! `objdump` prints when it is not the dynamic one.

use super::integer::memory;
use super::{funct3, i_imm, instruction, rd, rs1, rs2, s_imm, Instruction, F, X};

/// The rounding modes by number; 5 and 6 are reserved.
const ROUNDING: [Option<&str>; 8] = [
    Some("rne"),
    Some("rtz"),
    Some("rdn"),
    Some("rup"),
    Some("rmm"),
    None,
    None,
    Some("dyn"),
];

/// How an instruction's rounding mode is printed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rounding {
    /// Not at all: the instruction has no rounding mode field.
    None,
    /// After the operands, unless it is the dynamic mode (7).
    UnlessDynamic,
    /// After the operands, unless it is 0: an exact conversion, whose
    /// rounding mode is ignored and written as 0 (`fcvt.d.w`, `fcvt.d.s`).
    UnlessZero,
}

/// `operands`, then `,<mode>` when `rounding` says to print `word`'s rounding
/// mode; `None` for a reserved mode.
fn with_rounding(word: u32, operands: String, rounding: Rounding) -> Option<String> {
    let mode = funct3(word);
    let shown = match rounding {
        Rounding::None => false,
        Rounding::UnlessDynamic => mode != 7,
        Rounding::UnlessZero => mode != 0,
    };
    if rounding != Rounding::None && ROUNDING[mode as usize].is_none() {
        return None;
    }
    Some(if shown {
        format!("{operands},{}", ROUNDING[mode as usize]?)
    } else {
        operands
    })
}

/// `OP-FP`: arithmetic, sign injection, minimum and maximum, conversions,
/// moves between register files, comparisons and `fclass`, for single (`.s`)
/// and double (`.d`) precision.
pub(super) fn op_fp(word: u32) -> Option<Instruction> {
    let format = match (word >> 25) & 3 {
        0 => "s",
        1 => "d",
        _ => return None,
    };
    let (d, s1, s2) = (rd(word), rs1(word), rs2(word));
    let (fd, fa, fb) = (F[usize::from(d)], F[usize::from(s1)], F[usize::from(s2)]);
    let (xd, xa) = (X[usize::from(d)], X[usize::from(s1)]);
    let (mnemonic, operands, rounding) = match word >> 27 {
        0b00000 => (
            format!("fadd.{format}"),
            format!("{fd},{fa},{fb}"),
            Rounding::UnlessDynamic,
        ),
        0b00001 => (
            format!("fsub.{format}"),
            format!("{fd},{fa},{fb}"),
            Rounding::UnlessDynamic,
        ),
        0b00010 => (
            format!("fmul.{format}"),
            format!("{fd},{fa},{fb}"),
            Rounding::UnlessDynamic,
        ),
        0b00011 => (
            format!("fdiv.{format}"),
            format!("{fd},{fa},{fb}"),
            Rounding::UnlessDynamic,
        ),
        0b01011 if s2 == 0 => (
            format!("fsqrt.{format}"),
            format!("{fd},{fa}"),
            Rounding::UnlessDynamic,
        ),
        0b00100 => {
            let (plain, alias) = match funct3(word) {
                0 => ("fsgnj", "fmv"),
                1 => ("fsgnjn", "fneg"),
                2 => ("fsgnjx", "fabs"),
                _ => return None,
            };
            if s1 == s2 {
                (
                    format!("{alias}.{format}"),
                    format!("{fd},{fa}"),
                    Rounding::None,
                )
            } else {
                (
                    format!("{plain}.{format}"),
                    format!("{fd},{fa},{fb}"),
                    Rounding::None,
                )
            }
        }
        0b00101 => {
            let operation = match funct3(word) {
                0 => "fmin",
                1 => "fmax",
                _ => return None,
            };
            (
                format!("{operation}.{format}"),
                format!("{fd},{fa},{fb}"),
                Rounding::None,
            )
        }
        // fcvt.s.d (rounds) and fcvt.d.s (exact).
        0b01000 => match (format, s2) {
            ("s", 1) => (
                "fcvt.s.d".to_string(),
                format!("{fd},{fa}"),
                Rounding::UnlessDynamic,
            ),
            ("d", 0) => (
                "fcvt.d.s".to_string(),
                format!("{fd},{fa}"),
                Rounding::UnlessZero,
            ),
            _ => return None,
        },
        0b10100 => {
            let operation = match funct3(word) {
                2 => "feq",
                1 => "flt",
                0 => "fle",
                _ => return None,
            };
            (
                format!("{operation}.{format}"),
                format!("{xd},{fa},{fb}"),
                Rounding::None,
            )
        }
        // To an integer register.
        0b11000 => (
            format!("fcvt.{}.{format}", integer_kind(s2)?),
            format!("{xd},{fa}"),
            Rounding::UnlessDynamic,
        ),
        // From an integer register: from a 32-bit integer to a double is exact.
        0b11010 => {
            let kind = integer_kind(s2)?;
            let rounding = if format == "d" && matches!(kind, "w" | "wu") {
                Rounding::UnlessZero
            } else {
                Rounding::UnlessDynamic
            };
            (
                format!("fcvt.{format}.{kind}"),
                format!("{fd},{xa}"),
                rounding,
            )
        }
        0b11100 if s2 == 0 => match funct3(word) {
            0 => (
                format!("fmv.x.{}", if format == "s" { "w" } else { "d" }),
                format!("{xd},{fa}"),
                Rounding::None,
            ),
            1 => (
                format!("fclass.{format}"),
                format!("{xd},{fa}"),
                Rounding::None,
            ),
            _ => return None,
        },
        0b11110 if s2 == 0 && funct3(word) == 0 => (
            format!("fmv.{}.x", if format == "s" { "w" } else { "d" }),
            format!("{fd},{xa}"),
            Rounding::None,
        ),
        _ => return None,
    };
    let operands = with_rounding(word, operands, rounding)?;
    Some(instruction(word, &mnemonic, operands))
}

/// The integer of a conversion, by its `rs2` field.
fn integer_kind(selector: u8) -> Option<&'static str> {
    Some(match selector {
        0 => "w",
        1 => "wu",
        2 => "l",
        3 => "lu",
        _ => return None,
    })
}

/// The fused multiply-adds: `MADD`, `MSUB`, `NMSUB`, `NMADD`, with a third
/// source register in bits 27-31.
pub(super) fn fused(word: u32) -> Option<Instruction> {
    let format = match (word >> 25) & 3 {
        0 => "s",
        1 => "d",
        _ => return None,
    };
    let operation = match word & 0x7f {
        0x43 => "fmadd",
        0x47 => "fmsub",
        0x4b => "fnmsub",
        _ => "fnmadd",
    };
    let operands = format!(
        "{},{},{},{}",
        F[usize::from(rd(word))],
        F[usize::from(rs1(word))],
        F[usize::from(rs2(word))],
        F[(word >> 27) as usize]
    );
    let operands = with_rounding(word, operands, Rounding::UnlessDynamic)?;
    Some(instruction(
        word,
        &format!("{operation}.{format}"),
        operands,
    ))
}

/// `flw`/`fld` (`LOAD-FP`) and `fsw`/`fsd` (`STORE-FP`); `None` for the
/// vector forms those opcodes also hold.
pub(super) fn load_store(word: u32) -> Option<Instruction> {
    let store = word & 0x7f == 0x27;
    let mnemonic = match (store, funct3(word)) {
        (false, 2) => "flw",
        (false, 3) => "fld",
        (true, 2) => "fsw",
        (true, 3) => "fsd",
        _ => return None,
    };
    let (register, offset) = if store {
        (rs2(word), s_imm(word))
    } else {
        (rd(word), i_imm(word))
    };
    let mut decoded = instruction(
        word,
        mnemonic,
        format!("{},{}", F[usize::from(register)], memory(offset, rs1(word))),
    );
    decoded.page_offset = Some((rs1(word), offset));
    Some(decoded)
}
