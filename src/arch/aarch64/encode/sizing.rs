use super::*;
use crate::arch::encode_plan::InstructionEncoder;

/// The exact byte length [`super::emitter::Encoder::emit_instruction`] produces,
/// derived from the emitter itself so there is no second, drift-prone size table
/// to keep in sync — the same structural guarantee x86 has (bug-341-B3). A
/// throwaway encoder emits the instruction and we take the text length; its
/// relocation/label side effects are discarded. Every field value is seeded as
/// an import so a symbol-referencing op's binding resolution succeeds — binding
/// never affects the byte count (a `bl`/`adrp` is one word either way), so the
/// internal/external/data choice is immaterial to the size.
pub(super) fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
    let mut probe = Encoder::new(Vec::new(), HashMap::new());
    for (_, value) in &instruction.fields {
        probe.imports.insert(value.clone(), String::new());
    }
    probe.emit_instruction(instruction)?;
    Ok(probe.text.len())
}

pub(super) fn checked_imm12(value: u64) -> Result<u32, String> {
    if value > 4095 {
        return Err(format!("AArch64 immediate {value} exceeds 12-bit encoding"));
    }
    Ok(value as u32)
}

pub(super) fn encode_add_sub_imm(value: u64) -> Option<(u32, bool)> {
    if value <= 4095 {
        Some((value as u32, false))
    } else if value.is_multiple_of(4096) && (value >> 12) <= 4095 {
        Some(((value >> 12) as u32, true))
    } else {
        None
    }
}

/// Cap on how many add/sub immediate chunks a decomposition may take.
///
/// bug-284 C2: each iteration removes at most `4095 << 12` (~16.7M), so an
/// immediate near `u64::MAX` -- the shape a lowering arithmetic-wrap bug produces
/// -- needs on the order of 10^12 iterations. `instruction_size` runs before any
/// encoding, so the compiler would spin here forever rather than report anything.
/// Legitimate frame and field offsets take a single-digit number of chunks, and
/// `mov_imm` covers any u64 in at most four words, so nothing real is near this.
pub(super) const MAX_ADD_SUB_CHUNKS: usize = 8;

/// Number of chunks `value` decomposes into, saturating at `MAX_ADD_SUB_CHUNKS + 1`
/// so an absurd immediate is reported rather than counted out (bug-284 C2).
pub(super) fn add_sub_chunk_count(value: u64) -> usize {
    let mut remaining = value;
    let mut chunks = 0;
    while remaining > 0 {
        let (chunk, shift12) = next_add_sub_chunk(remaining);
        remaining -= if shift12 {
            u64::from(chunk) << 12
        } else {
            u64::from(chunk)
        };
        chunks += 1;
        if chunks > MAX_ADD_SUB_CHUNKS {
            break;
        }
    }
    chunks
}

pub(super) fn next_add_sub_chunk(remaining: u64) -> (u32, bool) {
    if remaining >= 4096 {
        (((remaining / 4096).min(4095)) as u32, true)
    } else {
        (remaining as u32, false)
    }
}

/// Encode a signed, word-scaled branch displacement into a `bits`-wide immediate
/// field, rejecting an out-of-reach or misaligned target instead of masking it to
/// a wrong address (bug-267 / LNK-11). Mirrors the reach-checking the linker
/// relocation copies do (LNK-06); the object-encoder twins previously truncated.
fn branch_imm(source: usize, target: usize, bits: u32, span: &str) -> Result<u32, String> {
    let delta = target as isize - source as isize;
    if delta % 4 != 0 {
        return Err(format!(
            "AArch64 branch displacement {delta} is not a multiple of 4 (unaligned target)"
        ));
    }
    let words = delta / 4;
    let limit = 1_isize << (bits - 1);
    if words < -limit || words >= limit {
        return Err(format!(
            "AArch64 branch displacement {delta} is out of range (exceeds {span})"
        ));
    }
    Ok((words as i32 as u32) & ((1u32 << bits) - 1))
}

pub(super) fn branch_imm26(source: usize, target: usize) -> Result<u32, String> {
    branch_imm(source, target, 26, "±128 MiB")
}

pub(super) fn branch_imm19(source: usize, target: usize) -> Result<u32, String> {
    branch_imm(source, target, 19, "±1 MiB")
}
