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

pub(super) fn reg(name: String) -> Result<u8, String> {
    match name.as_str() {
        "sp" | "raw_sp" | "x31" | "xzr" => Ok(31),
        "x0" | "w0" => Ok(0),
        "x1" | "w1" => Ok(1),
        "x2" | "w2" => Ok(2),
        "x3" | "w3" => Ok(3),
        "x4" | "w4" => Ok(4),
        "x5" | "w5" => Ok(5),
        "x6" | "w6" => Ok(6),
        "x7" | "w7" => Ok(7),
        "x8" | "w8" => Ok(8),
        "x9" | "w9" => Ok(9),
        "x10" | "w10" => Ok(10),
        "x11" | "w11" => Ok(11),
        "x12" | "w12" => Ok(12),
        "x13" | "w13" => Ok(13),
        "x14" | "w14" => Ok(14),
        "x15" | "w15" => Ok(15),
        "x16" | "w16" => Ok(16),
        "x17" | "w17" => Ok(17),
        "x19" | "w19" => Ok(19),
        "x20" | "w20" => Ok(20),
        "x21" | "w21" => Ok(21),
        "x22" | "w22" => Ok(22),
        "x23" | "w23" => Ok(23),
        "x24" | "w24" => Ok(24),
        "x25" | "w25" => Ok(25),
        "x26" | "w26" => Ok(26),
        "x27" | "w27" => Ok(27),
        "x28" | "w28" => Ok(28),
        "x30" | "lr" => Ok(30),
        // Scalar FP/SIMD `d0`..`d31` share the 5-bit register field with the
        // vector registers; decode the number directly.
        _ if name.starts_with('d') => name[1..]
            .parse::<u8>()
            .ok()
            .filter(|n| *n < 32)
            .ok_or_else(|| format!("unknown AArch64 register '{name}'")),
        other => Err(format!("unknown AArch64 register '{other}'")),
    }
}

/// Parse a NEON vector register operand. Accepts `v0`..`v31` and the `q0`..`q31`
/// load/store spelling (the arrangement suffix, e.g. `.2d`, is implied by the op,
/// so only the register number is decoded here).
pub(super) fn vreg(name: String) -> Result<u8, String> {
    let digits = name
        .strip_prefix('v')
        .or_else(|| name.strip_prefix('q'))
        .ok_or_else(|| format!("unknown AArch64 vector register '{name}'"))?;
    let number = digits
        .parse::<u8>()
        .map_err(|_| format!("unknown AArch64 vector register '{name}'"))?;
    if number > 31 {
        return Err(format!("AArch64 vector register '{name}' out of range"));
    }
    Ok(number)
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

pub(super) fn scratch_excluding(a: u8, b: u8) -> u8 {
    [17, 16, 15]
        .into_iter()
        .find(|candidate| *candidate != a && *candidate != b)
        .expect("scratch register candidate list is non-empty")
}
