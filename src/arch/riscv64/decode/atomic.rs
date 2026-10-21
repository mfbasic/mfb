//! The `A` extension: `lr`, `sc` and the `amo*` read-modify-write forms, each
//! `.w` or `.d`, with the `.aq` / `.rl` / `.aqrl` ordering suffix `objdump`
//! prints.

use super::{funct3, instruction, rd, rs1, rs2, Flow, Instruction, X};

/// The `AMO`-opcode instruction `word`, or `None` for a form `A` does not
/// define.
pub(super) fn decode(word: u32) -> Option<Instruction> {
    let width = match funct3(word) {
        2 => "w",
        3 => "d",
        _ => return None,
    };
    let order = match (word >> 25) & 3 {
        0 => "",
        1 => ".rl",
        2 => ".aq",
        _ => ".aqrl",
    };
    let operation = match word >> 27 {
        0b00010 => "lr",
        0b00011 => "sc",
        0b00001 => "amoswap",
        0b00000 => "amoadd",
        0b00100 => "amoxor",
        0b01100 => "amoand",
        0b01000 => "amoor",
        0b10000 => "amomin",
        0b10100 => "amomax",
        0b11000 => "amominu",
        0b11100 => "amomaxu",
        _ => return None,
    };
    let mnemonic = format!("{operation}.{width}{order}");
    let (d, value, address) = (
        X[usize::from(rd(word))],
        X[usize::from(rs2(word))],
        X[usize::from(rs1(word))],
    );
    if operation == "lr" {
        if rs2(word) != 0 {
            return None;
        }
        let mut decoded = instruction(word, &mnemonic, format!("{d},({address})"));
        decoded.flow = Flow::AtomicStart;
        return Some(decoded);
    }
    Some(instruction(
        word,
        &mnemonic,
        format!("{d},{value},({address})"),
    ))
}
