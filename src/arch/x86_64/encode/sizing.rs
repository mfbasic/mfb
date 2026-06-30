//! Instruction sizing. By construction this is exactly the byte count
//! [`super::emitter::emit_instruction`] produces, because both delegate to the
//! one [`super::emitter::encode_instruction`] function — sizing simply discards
//! the relocation/label side effect and returns the byte length. There is no
//! second, drift-prone size table to keep in sync.

use super::emitter::encode_instruction;
use super::*;

pub(super) fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
    Ok(encode_instruction(instruction)?.bytes_len())
}
