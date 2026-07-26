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

use std::collections::HashMap;

use crate::arch::aarch64::encode::{EncodedImage, EncodedRelocation, EncodedSection};

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

/// Runtime VM address of a symbol for the AArch64 linkers. Text symbols sit in
/// the code segment. When a read-only window is given (`rodata`, the macOS
/// `__DATA_CONST` split — bug-187), a data symbol below `rodata_size` maps into
/// the constant region at `rodata_vmaddr`, and one at or above it into writable
/// data past that prefix. With no window (`rodata == None`, the ELF layout)
/// every data symbol maps into `data_vmaddr` directly.
pub(in crate::os) fn symbol_vmaddr(
    image: &EncodedImage,
    symbol_name: &str,
    text_vmaddr: u64,
    data_vmaddr: u64,
    rodata: Option<(u64, usize)>,
) -> Result<u64, String> {
    let symbol = image
        .symbols
        .iter()
        .find(|symbol| symbol.name == symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' does not resolve"))?;
    Ok(match symbol.section {
        EncodedSection::Text => text_vmaddr + symbol.offset as u64,
        EncodedSection::Data => match rodata {
            Some((rodata_vmaddr, rodata_size)) if symbol.offset < rodata_size => {
                rodata_vmaddr + symbol.offset as u64
            }
            Some((_, rodata_size)) => data_vmaddr + (symbol.offset - rodata_size) as u64,
            None => data_vmaddr + symbol.offset as u64,
        },
    })
}

/// Everything the shared AArch64 relocation arms need beyond the `text` buffer
/// and the relocation itself: the image and address windows for symbol
/// resolution, the import stub/GOT tables for external bindings, and the
/// platform label for diagnostics (`macOS linker` / `linux-aarch64 linker`).
pub(in crate::os) struct AArch64RelocCtx<'a> {
    pub(in crate::os) image: &'a EncodedImage,
    pub(in crate::os) text_vmaddr: u64,
    pub(in crate::os) data_vmaddr: u64,
    /// `(rodata_vmaddr, rodata_size)` for the macOS `__DATA_CONST` split; `None`
    /// for the ELF layout, where data is one region.
    pub(in crate::os) rodata: Option<(u64, usize)>,
    pub(in crate::os) stubs: &'a HashMap<String, u64>,
    pub(in crate::os) got_entries: &'a HashMap<String, u64>,
    pub(in crate::os) label: &'a str,
}

/// Apply one AArch64 relocation — `branch26`, `page21`, or `pageoff12` in an
/// `internal`, `data`, or `external` binding. Returns `Ok(true)` when it handled
/// the relocation, `Ok(false)` when the kind is not one of the six AArch64 arms
/// (so an ELF caller can fall through to its x86-64/RISC-V arms), and `Err` when
/// a bind fails or a displacement is out of reach.
pub(in crate::os) fn patch_aarch64_reloc(
    text: &mut [u8],
    relocation: &EncodedRelocation,
    ctx: &AArch64RelocCtx,
) -> Result<bool, String> {
    match relocation.binding.as_str() {
        "internal" if relocation.kind == "branch26" => {
            let target = symbol_vmaddr(
                ctx.image,
                &relocation.target,
                ctx.text_vmaddr,
                ctx.data_vmaddr,
                ctx.rodata,
            )?;
            let word = 0x9400_0000
                | branch_imm26(
                    ctx.text_vmaddr as usize + relocation.offset,
                    target as usize,
                )?;
            write_u32(text, relocation.offset, word)?;
        }
        "data" if relocation.kind == "page21" => {
            let target = symbol_vmaddr(
                ctx.image,
                &relocation.target,
                ctx.text_vmaddr,
                ctx.data_vmaddr,
                ctx.rodata,
            )?;
            let pc = ctx.text_vmaddr + relocation.offset as u64;
            let (immlo, immhi) = adrp_page21(pc, target)?;
            let rd = read_u32(text, relocation.offset)? & 0x1f;
            write_u32(
                text,
                relocation.offset,
                0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
            )?;
        }
        "data" if relocation.kind == "pageoff12" => {
            let target = symbol_vmaddr(
                ctx.image,
                &relocation.target,
                ctx.text_vmaddr,
                ctx.data_vmaddr,
                ctx.rodata,
            )?;
            let imm12 = (target & 0xfff) as u32;
            let word = read_u32(text, relocation.offset)?;
            let rd = word & 0x1f;
            let rn = (word >> 5) & 0x1f;
            write_u32(
                text,
                relocation.offset,
                0x9100_0000 | (imm12 << 10) | (rn << 5) | rd,
            )?;
        }
        "external" if relocation.kind == "branch26" => {
            let Some(&target) = ctx.stubs.get(&relocation.target) else {
                return Err(format!(
                    "{} cannot bind external symbol '{}' from {}",
                    ctx.label,
                    relocation.target,
                    relocation.library.as_deref().unwrap_or("<unknown library>")
                ));
            };
            let word = 0x9400_0000
                | branch_imm26(
                    ctx.text_vmaddr as usize + relocation.offset,
                    target as usize,
                )?;
            write_u32(text, relocation.offset, word)?;
        }
        "external" if relocation.kind == "page21" => {
            let Some(&target) = ctx.got_entries.get(&relocation.target) else {
                return Err(format!(
                    "{} cannot bind external data symbol '{}' from {}",
                    ctx.label,
                    relocation.target,
                    relocation.library.as_deref().unwrap_or("<unknown library>")
                ));
            };
            let pc = ctx.text_vmaddr + relocation.offset as u64;
            let (immlo, immhi) = adrp_page21(pc, target)?;
            let rd = read_u32(text, relocation.offset)? & 0x1f;
            write_u32(
                text,
                relocation.offset,
                0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
            )?;
        }
        "external" if relocation.kind == "pageoff12" => {
            let Some(&target) = ctx.got_entries.get(&relocation.target) else {
                return Err(format!(
                    "{} cannot bind external data symbol '{}' from {}",
                    ctx.label,
                    relocation.target,
                    relocation.library.as_deref().unwrap_or("<unknown library>")
                ));
            };
            let imm12 = (target & 0xfff) as u32;
            let word = read_u32(text, relocation.offset)?;
            let rd = word & 0x1f;
            let rn = (word >> 5) & 0x1f;
            write_u32(
                text,
                relocation.offset,
                0x9100_0000 | (imm12 << 10) | (rn << 5) | rd,
            )?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
