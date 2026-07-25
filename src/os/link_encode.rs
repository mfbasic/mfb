//! Byte emit/patch helpers and AArch64 relocation encoders shared by the Mach-O
//! (`macos/link`) and ELF (`linux/link`) linkers (bug-335 A4/A5). The
//! instruction-encoding constants and bounds checks are ISA facts, identical on
//! both platforms; only the per-target relocation *dispatch* (`patch_relocations`)
//! and the format-specific writers stay per platform.
//!
//! `read_u32`/`write_u32` are bounds-checked so an out-of-range relocation
//! offset — an internal codegen defect, since offsets are in-bounds by
//! construction and `EncodedImage` is never deserialized — surfaces as a
//! diagnostic rather than a panic (bug-225/bug-351).

/// Encode a `BL`/`B` `imm26` branch displacement, reach-checked (bug-168). The
/// immediate is a signed 26-bit word offset: ±2^25 words = ±128 MiB. Masking
/// without a reach check silently wraps an over-range branch into a wrong
/// instruction, so an out-of-reach or misaligned delta errors instead.
pub(in crate::os) fn branch_imm26(source: usize, target: usize) -> Result<u32, String> {
    let delta = target as isize - source as isize;
    if delta % 4 != 0 || !(-(1 << 27)..(1 << 27)).contains(&delta) {
        return Err(format!(
            "linker: branch displacement {delta} exceeds the ±128 MiB reach of BL/B"
        ));
    }
    Ok(((delta / 4) as i32 as u32) & 0x03ff_ffff)
}

/// Encode an `ADRP` page displacement, reach-checked (bug-168). The immediate is
/// a signed 21-bit count of 4 KiB pages (±2^20 pages = ±4 GiB); an over-range
/// delta must error rather than truncate to a wrong page. Returns `(immlo,
/// immhi)` ready to splice into the instruction word.
pub(in crate::os) fn adrp_page21(pc: u64, target: u64) -> Result<(u32, u32), String> {
    let page_delta = ((target & !0xfff) as i64 - (pc & !0xfff) as i64) >> 12;
    if !(-(1 << 20)..(1 << 20)).contains(&page_delta) {
        return Err(format!(
            "linker: ADRP page displacement {page_delta} exceeds the ±4 GiB reach of ADRP"
        ));
    }
    let encoded = page_delta as u32;
    Ok((encoded & 0b11, (encoded >> 2) & 0x7ffff))
}

/// Read a little-endian `u32` at `offset`, bounds-checked (see module docs).
pub(in crate::os) fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(|| {
        format!(
            "linker: relocation offset {offset} + 4 exceeds text length {}",
            bytes.len()
        )
    })?;
    Ok(u32::from_le_bytes(slice.try_into().expect("slice length")))
}

/// Write a little-endian `u32` at `offset`, bounds-checked (see module docs).
pub(in crate::os) fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    let len = bytes.len();
    let slice = bytes.get_mut(offset..offset + 4).ok_or_else(|| {
        format!("linker: relocation offset {offset} + 4 exceeds text length {len}")
    })?;
    slice.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Append a little-endian `u16` to `bytes`.
pub(in crate::os) fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Append a little-endian `u32` to `bytes`.
pub(in crate::os) fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Append a little-endian `u64` to `bytes`.
pub(in crate::os) fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
