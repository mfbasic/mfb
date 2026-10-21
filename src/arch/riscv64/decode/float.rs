//! The `F` and `D` extensions: loads and stores, arithmetic, conversions,
//! moves, comparisons and the fused multiply-adds, with the rounding mode
//! `objdump` prints when it is not the dynamic one.

use super::integer::memory;
use super::{funct3, i_imm, instruction, rd, rs1, rs2, s_imm, Instruction, F};

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
