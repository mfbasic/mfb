//! Operand decoding shared by every backend encoder (bug-341-B7).
//!
//! `field` (look up an instruction field by name), `immediate` (parse an integer
//! or boolean immediate), and `shift` (parse a 0–63 shift amount) were duplicated
//! verbatim in all three `arch::<isa>::encode::operand` modules — byte-identical
//! between AArch64 and x86-64, and differing on riscv64 only by an inconsistent
//! `"rv64 "` prefix on the diagnostics. They carry no ISA-specific knowledge (the
//! register-name decoders, which do, stay per-ISA), so they live here once with a
//! single, unprefixed diagnostic convention.

use crate::target::shared::code::CodeInstruction;

/// The value of `instruction`'s field named `name`, or an error naming the
/// missing field and the instruction's mnemonic.
pub(crate) fn field(instruction: &CodeInstruction, name: &str) -> Result<String, String> {
    instruction
        .fields
        .iter()
        .find(|(field, _)| *field == name)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| {
            format!(
                "instruction '{}' missing field '{name}'",
                instruction.op.mnemonic()
            )
        })
}

/// Parse an immediate operand: the booleans `true`/`false` decode to `1`/`0`,
/// everything else parses as an unsigned 64-bit integer.
pub(crate) fn immediate(value: String) -> Result<u64, String> {
    match value.as_str() {
        "true" => Ok(1),
        "false" => Ok(0),
        _ => value
            .parse::<u64>()
            .map_err(|_| format!("invalid immediate '{value}'")),
    }
}

/// Parse a shift amount, rejecting a value that does not fit a 64-bit shift
/// (`0..=63`).
pub(crate) fn shift(value: String) -> Result<u8, String> {
    let value = value
        .parse::<u8>()
        .map_err(|_| format!("invalid shift immediate '{value}'"))?;
    if value >= 64 {
        return Err(format!("shift immediate {value} is out of range"));
    }
    Ok(value)
}
