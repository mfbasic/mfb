//! x86-64 operand decoding: instruction fields, register names, immediates.

use super::*;

pub(super) fn field(instruction: &CodeInstruction, name: &str) -> Result<String, String> {
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

/// x86-64 general-purpose register number (0..=15) for the canonical 64-bit
/// register names. The numbering is the architectural encoding:
/// `rax=0, rcx=1, rdx=2, rbx=3, rsp=4, rbp=5, rsi=6, rdi=7, r8..r15=8..15`.
/// `r8`..`r15` need the REX.B/R/X extension bit, handled by the emitter.
pub(super) fn reg(name: String) -> Result<u8, String> {
    Ok(match name.as_str() {
        "rax" => 0,
        "rcx" => 1,
        "rdx" => 2,
        "rbx" => 3,
        "rsp" | "sp" | "raw_sp" => 4,
        "rbp" => 5,
        "rsi" => 6,
        "rdi" => 7,
        "r8" => 8,
        "r9" => 9,
        "r10" => 10,
        "r11" => 11,
        "r12" => 12,
        "r13" => 13,
        "r14" => 14,
        "r15" => 15,
        // A canonical zero token (`xzr`/`rzero`) names "no register" — used by
        // the explicit-carry ops to express "no carry-in". Reported as 16 so the
        // emitter can branch on it without colliding with a real register.
        "xzr" | "rzero" | "zero" => 16,
        other => return Err(format!("unknown x86-64 register '{other}'")),
    })
}

/// True when a parsed register number names the synthetic zero token rather than
/// a hardware register.
pub(super) fn is_zero_token(r: u8) -> bool {
    r == 16
}

pub(super) fn immediate(value: String) -> Result<u64, String> {
    match value.as_str() {
        "true" => Ok(1),
        "false" => Ok(0),
        _ => value
            .parse::<u64>()
            .map_err(|_| format!("invalid immediate '{value}'")),
    }
}

pub(super) fn shift(value: String) -> Result<u8, String> {
    let value = value
        .parse::<u8>()
        .map_err(|_| format!("invalid shift immediate '{value}'"))?;
    if value >= 64 {
        return Err(format!("shift immediate {value} is out of range"));
    }
    Ok(value)
}
