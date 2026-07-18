use crate::arch::aarch64::encode::{EncodedImage, EncodedRelocation, EncodedSection, ImportKind};
use crate::os::linux::flavor::LinuxFlavor;
use crate::os::note::{mfb_note_descriptor, MFB_NOTE_OWNER, MFB_NOTE_TYPE};
use crate::os::BUILD_DIR;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const IMAGE_BASE: u64 = 0x400000;
/// Load base for the position-independent (`ET_DYN`) dynamic output (bug-186).
/// A PIE's virtual addresses are file-relative (base 0); the dynamic loader adds
/// a random slide at load time. All emitted-code references are PC-relative
/// (`adrp`/`add`, `auipc`, `lea`/`pc32`), so only the loader-relocated metadata
/// (GOT slots, `.rela` r_offsets, `DT_*` pointers, and the `DT_INIT_ARRAY`
/// entries via `R_*_RELATIVE`) needs the base applied — the loader does that.
/// The static (import-less) path keeps `IMAGE_BASE` / `ET_EXEC` for now.
const DYN_IMAGE_BASE: u64 = 0;
/// `PT_GNU_STACK`: its `p_flags` (R+W, no execute) set the non-executable-stack
/// policy the kernel/loader would otherwise leave to a default (LNK-02 / bug-186).
const PT_GNU_STACK: u32 = 0x6474_e551;
/// `PT_NOTE`: the segment carrying the unconditional `MFBasic\0` provenance note
/// (plan-43). It rides in the header/text gap, inside the text `PT_LOAD`'s file
/// range, so `readelf -n` and the mapped image both see it.
const PT_NOTE: u32 = 4;
/// `PT_GNU_RELRO`: marks the sub-range of the writable segment (GOT + `.dynamic`)
/// the loader must `mprotect` back to read-only once startup relocation finishes,
/// removing the GOT-overwrite target `DF_BIND_NOW` alone would leave writable
/// (bug-263 / LNK-03). Now wired: the bug-187 const/mutable partition plus the
/// page-aligned dynamic payload page-isolate the GOT from the writable arena
/// global.
const PT_GNU_RELRO: u32 = 0x6474_e552;
/// `R_*_RELATIVE` dynamic relocation types: `*(base + r_offset) = base + addend`.
/// Used to bias the absolute `DT_INIT_ARRAY` function pointers into a PIE.
const R_AARCH64_RELATIVE: u32 = 1027;
const R_X86_64_RELATIVE: u32 = 8;
const R_RISCV_RELATIVE: u32 = 3;
const TEXT_FILE_OFFSET: usize = 0x1000;
const PAGE_SIZE: usize = 0x1000;
const R_AARCH64_GLOB_DAT: u32 = 1025;
const R_AARCH64_JUMP_SLOT: u32 = 1026;
const R_X86_64_GLOB_DAT: u32 = 6;
const R_X86_64_JUMP_SLOT: u32 = 7;
// RISC-V dynamic relocation types. RISC-V has no dedicated GLOB_DAT — a data
// global's GOT slot is bound with an absolute R_RISCV_64.
const R_RISCV_64: u32 = 2;
const R_RISCV_JUMP_SLOT: u32 = 5;

mod elf;
#[cfg(test)]
mod tests;

use elf::*;

pub(crate) fn write_executable(
    project_dir: &Path,
    project_name: &str,
    arch: &str,
    flavor: LinuxFlavor,
    app_mode: bool,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    let mut text = image.text.clone();
    let text_vmaddr = IMAGE_BASE + TEXT_FILE_OFFSET as u64;
    let main_entry_offset = image
        .symbols
        .iter()
        .find(|symbol| symbol.name == image.entry)
        .filter(|symbol| symbol.section == EncodedSection::Text)
        .map(|symbol| symbol.offset)
        .ok_or_else(|| format!("entry symbol '{}' does not resolve to text", image.entry))?;
    let import_locations = if image.imports.is_empty() {
        ImportLocations::default()
    } else {
        append_import_stubs(arch, &mut text, image, text_vmaddr)?
    };
    let data_offset = align(TEXT_FILE_OFFSET + text.len(), PAGE_SIZE);
    let data_vmaddr = IMAGE_BASE + data_offset as u64;
    patch_relocations(
        &mut text,
        image,
        text_vmaddr,
        data_vmaddr,
        &import_locations,
    )?;
    let entry_offset = main_entry_offset;
    // The output shape is chosen by the target ISA: x86-64 (plan-00-H) uses raw
    // syscalls (no imports) → a static, writable-data ELF; AArch64 links libc
    // dynamically (a static ELF only when a build happens to import nothing).
    let bytes = if image.imports.is_empty() {
        // No libc imports (a build using only raw syscalls) → a static,
        // interpreter-less ELF; otherwise link libc dynamically (PLT/GOT +
        // interpreter). x86 uses raw syscalls for the primitives but pulls in
        // libc for what has no syscall (snprintf, signal, …).
        if arch == "x86_64" {
            encode_static_elf_x86(
                entry_offset,
                &text,
                &image.data,
                image.signing_metadata.as_deref(),
            )
        } else {
            encode_static_elf(
                arch,
                entry_offset,
                &text,
                &image.data,
                image.signing_metadata.as_deref(),
            )
        }
    } else {
        encode_dynamic_elf(arch, flavor, entry_offset, &text, &image.data, image)?
    };
    // Every build emits into the project's `build/` directory (plan-46-D §4.1) so
    // the executable and the `vendor/` its RPATH points at move as one unit, and
    // one `.gitignore` line covers every artifact. App mode
    // (plan-05-linux-app.md §5.2) emits a single glibc `<name>.out`; the console
    // build emits one flavored `<name>-{glibc,musl}.out` per libc world — both
    // flavors share the one directory.
    let out_dir = project_dir.join(BUILD_DIR);
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create '{}': {err}", out_dir.display()))?;
    let path = if app_mode {
        out_dir.join(format!("{project_name}.out"))
    } else {
        out_dir.join(format!("{project_name}-{}.out", flavor.suffix()))
    };
    fs::write(&path, bytes)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    let mut permissions = fs::metadata(&path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions)
        .map_err(|err| format!("failed to mark '{}' executable: {err}", path.display()))?;
    Ok(path)
}

fn patch_relocations(
    text: &mut [u8],
    image: &EncodedImage,
    text_vmaddr: u64,
    data_vmaddr: u64,
    import_locations: &ImportLocations,
) -> Result<(), String> {
    for relocation in &image.relocations {
        match relocation.binding.as_str() {
            "internal" if relocation.kind == "branch26" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let word = 0x9400_0000
                    | branch_imm26(text_vmaddr as usize + relocation.offset, target as usize)?;
                write_u32(text, relocation.offset, word)?;
            }
            "data" if relocation.kind == "page21" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let pc = text_vmaddr + relocation.offset as u64;
                let (immlo, immhi) = adrp_page21(pc, target)?;
                let rd = read_u32(text, relocation.offset)? & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
                )?;
            }
            "data" if relocation.kind == "pageoff12" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
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
                let Some(&target) = import_locations.stubs.get(&relocation.target) else {
                    return Err(format!(
                        "linux-aarch64 linker cannot bind external symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let word = 0x9400_0000
                    | branch_imm26(text_vmaddr as usize + relocation.offset, target as usize)?;
                write_u32(text, relocation.offset, word)?;
            }
            // Imported data global addressed through its GOT slot (plan-linker.md
            // §6.1): the slot is filled by a GLOB_DAT dynamic relocation.
            "external" if relocation.kind == "page21" => {
                let Some(&target) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "linux-aarch64 linker cannot bind external data symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let pc = text_vmaddr + relocation.offset as u64;
                let (immlo, immhi) = adrp_page21(pc, target)?;
                let rd = read_u32(text, relocation.offset)? & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
                )?;
            }
            "external" if relocation.kind == "pageoff12" => {
                let Some(&target) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "linux-aarch64 linker cannot bind external data symbol '{}' from {}",
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
            // --- x86-64 (plan-00-H): RIP-relative rel32 patches ----------------
            // A `call rel32` (internal call) or `lea reg,[rip+disp32]` (internal
            // data address). In both the encoder records `offset` at the disp32
            // field, which is the last 4 bytes of the instruction, so rip (the
            // next instruction) is `offset + 4`. rel32 = target − (site + 4).
            "internal" if relocation.kind == "call_pc32" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let site = text_vmaddr + relocation.offset as u64;
                let rel = rel32(target, site)?;
                write_u32(text, relocation.offset, rel as u32)?;
            }
            // x86-64 `call sym@PLT` to an imported libc function: the rel32
            // targets that symbol's PLT stub, which jumps through its GOT slot.
            "external" if relocation.kind == "call_pc32" => {
                let Some(&target) = import_locations.stubs.get(&relocation.target) else {
                    return Err(format!(
                        "linux-x86_64 linker cannot bind external symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let site = text_vmaddr + relocation.offset as u64;
                let rel = rel32(target, site)?;
                write_u32(text, relocation.offset, rel as u32)?;
            }
            // x86-64 imported data global via GOTPCREL: the rel32 targets the
            // symbol's GOT slot (filled by a GLOB_DAT reloc); the instruction
            // loads the symbol address from there. The encoder emits this as
            // `got_pc32` (the canonical GOT kind); `data_pc32` is accepted as its
            // historical alias.
            "external" if relocation.kind == "data_pc32" || relocation.kind == "got_pc32" => {
                let Some(&target) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "linux-x86_64 linker cannot bind external data symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let site = text_vmaddr + relocation.offset as u64;
                let rel = rel32(target, site)?;
                write_u32(text, relocation.offset, rel as u32)?;
            }
            "data" if relocation.kind == "data_pc32" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let site = text_vmaddr + relocation.offset as u64;
                let rel = rel32(target, site)?;
                write_u32(text, relocation.offset, rel as u32)?;
            }
            // --- RISC-V (plan-99) ----------------------------------------------
            // An internal `call` (auipc ra, hi; jalr ra, lo(ra)): patch both
            // words from the auipc's PC.
            "internal" if relocation.kind == "riscv_call" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let site = text_vmaddr + relocation.offset as u64;
                let (hi20, lo12) = riscv_hi_lo(target as i64 - site as i64)?;
                patch_riscv_auipc(text, relocation.offset, hi20)?;
                patch_riscv_itype_imm(text, relocation.offset + 4, lo12)?;
            }
            // An external `call` targets the imported symbol's PLT-like stub.
            "external" if relocation.kind == "riscv_call" => {
                let Some(&target) = import_locations.stubs.get(&relocation.target) else {
                    return Err(format!(
                        "linux-riscv64 linker cannot bind external symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let site = text_vmaddr + relocation.offset as u64;
                let (hi20, lo12) = riscv_hi_lo(target as i64 - site as i64)?;
                patch_riscv_auipc(text, relocation.offset, hi20)?;
                patch_riscv_itype_imm(text, relocation.offset + 4, lo12)?;
            }
            // Internal data address: `auipc rd, %pcrel_hi; addi rd, rd, %pcrel_lo`.
            // The lo12 is computed from the paired auipc's PC, located by pairing
            // (`paired_auipc_offset`) — the two halves need not be adjacent, since
            // the allocator may spill `rd` between them under register pressure.
            "data" if relocation.kind == "riscv_pcrel_hi20" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let site = text_vmaddr + relocation.offset as u64;
                let (hi20, _) = riscv_hi_lo(target as i64 - site as i64)?;
                patch_riscv_auipc(text, relocation.offset, hi20)?;
            }
            "data" if relocation.kind == "riscv_pcrel_lo12" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let auipc_offset =
                    paired_auipc_offset(&image.relocations, relocation, "riscv_pcrel_hi20")?;
                let auipc_site = text_vmaddr + auipc_offset as u64;
                let (_, lo12) = riscv_hi_lo(target as i64 - auipc_site as i64)?;
                patch_riscv_itype_imm(text, relocation.offset, lo12)?;
            }
            // Imported data global addressed through its GOT slot: `auipc rd,
            // %got_pcrel_hi; ld rd, %pcrel_lo(rd)` — the slot holds the address
            // (bound by an R_RISCV_64 dynamic reloc).
            "external" if relocation.kind == "riscv_got_hi20" => {
                let Some(&slot) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "linux-riscv64 linker cannot bind external data symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let site = text_vmaddr + relocation.offset as u64;
                let (hi20, _) = riscv_hi_lo(slot as i64 - site as i64)?;
                patch_riscv_auipc(text, relocation.offset, hi20)?;
            }
            "external" if relocation.kind == "riscv_got_lo12" => {
                let Some(&slot) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "linux-riscv64 linker cannot bind external data symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let auipc_offset =
                    paired_auipc_offset(&image.relocations, relocation, "riscv_got_hi20")?;
                let auipc_site = text_vmaddr + auipc_offset as u64;
                let (_, lo12) = riscv_hi_lo(slot as i64 - auipc_site as i64)?;
                patch_riscv_itype_imm(text, relocation.offset, lo12)?;
            }
            _ => {
                return Err(format!(
                    "linux linker does not support relocation {} {}",
                    relocation.binding, relocation.kind
                ));
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct ImportLocations {
    stubs: std::collections::HashMap<String, u64>,
    /// GOT entry vmaddr per imported symbol, used to address imported data
    /// globals directly (plan-linker.md §6.1).
    got_entries: std::collections::HashMap<String, u64>,
}

fn append_import_stubs(
    arch: &str,
    text: &mut Vec<u8>,
    image: &EncodedImage,
    text_vmaddr: u64,
) -> Result<ImportLocations, String> {
    let mut locations = ImportLocations::default();
    let stub_count = image.imports.len();
    let text_len_with_stubs = text.len() + stub_count * 12;
    let data_offset = align(TEXT_FILE_OFFSET + text_len_with_stubs, PAGE_SIZE);
    let got_offset = dynamic_prefix_size(image);
    let got_vmaddr = IMAGE_BASE + data_offset as u64 + got_offset as u64;
    for (index, import) in image.imports.iter().enumerate() {
        let stub_vmaddr = text_vmaddr + text.len() as u64;
        let entry_vmaddr = got_vmaddr + (index * 8) as u64;
        // Every import gets a GOT slot. Function imports also get a call stub
        // that branches through it; data globals are addressed via the GOT slot
        // directly (their stub is unused).
        emit_import_stub(arch, text, stub_vmaddr, entry_vmaddr)?;
        locations.stubs.insert(import.symbol.clone(), stub_vmaddr);
        locations
            .got_entries
            .insert(import.symbol.clone(), entry_vmaddr);
    }
    Ok(locations)
}

/// Locate the `auipc` a RISC-V lo12 relocation pairs with. A pcrel pair
/// (`auipc rd,%hi` + `addi rd,rd,%lo`, or `auipc rd,%got_hi` + `ld rd,%lo(rd)`)
/// shares one PC base — the `auipc`'s address — because the `auipc` alone is
/// PC-relative and the lo12 merely completes the low 12 bits of that same
/// displacement. The two halves need **not** be adjacent: the register allocator
/// may spill `rd` immediately after the `auipc` and reload it right before the
/// lo12 under pressure (e.g. two inlined SIMD math kernels in one function),
/// inserting stack traffic in the gap. So the lo12's base is the nearest
/// *preceding* `hi` relocation to the same target, not a hard-coded `offset - 4`.
fn paired_auipc_offset(
    relocations: &[EncodedRelocation],
    lo: &EncodedRelocation,
    hi_kind: &str,
) -> Result<usize, String> {
    relocations
        .iter()
        .filter(|r| r.kind == hi_kind && r.target == lo.target && r.offset < lo.offset)
        .map(|r| r.offset)
        .max()
        .ok_or_else(|| {
            format!(
                "linux-riscv64 linker: {} at {:#x} for '{}' has no paired {}",
                lo.kind, lo.offset, lo.target, hi_kind
            )
        })
}

/// The RISC-V high/low split of a PC-relative displacement: `auipc` materializes
/// the upper 20 bits (rounded so the sign-extended low 12 corrects it) and the
/// paired `addi`/`ld`/`jalr` adds the low 12. Returns `(auipc_imm20_field,
/// lo12)` where `lo12 ∈ [-2048, 2047]`.
///
/// The pair reaches roughly ±2 GiB. Masking `hi` to 20 bits without checking it
/// would silently drop the high bits of a longer displacement and patch a jump or
/// data load to the wrong address, so an out-of-reach displacement is an error
/// (the same discipline the other backends' branch/rel encoders follow).
fn riscv_hi_lo(delta: i64) -> Result<(u32, i32), String> {
    let out_of_reach =
        || format!("linux-riscv64 linker: displacement {delta} exceeds the ±2 GiB reach of auipc");
    let hi = delta.checked_add(0x800).ok_or_else(out_of_reach)? >> 12;
    if !(-(1 << 19)..(1 << 19)).contains(&hi) {
        return Err(out_of_reach());
    }
    let lo = (delta - (hi << 12)) as i32;
    Ok(((hi as u32) & 0xfffff, lo))
}

/// Patch a RISC-V `auipc rd, hi20` word in place, preserving `rd`.
fn patch_riscv_auipc(text: &mut [u8], offset: usize, hi20: u32) -> Result<(), String> {
    let existing = read_u32(text, offset)?;
    let rd = (existing >> 7) & 0x1f;
    write_u32(text, offset, (hi20 << 12) | (rd << 7) | 0x17)
}

/// Patch the 12-bit immediate of a RISC-V I-type word (`addi`/`ld`/`jalr`) in
/// place, preserving `rd`/`rs1`/`funct3`/opcode.
fn patch_riscv_itype_imm(text: &mut [u8], offset: usize, lo12: i32) -> Result<(), String> {
    let existing = read_u32(text, offset)? & 0x000f_ffff; // clear imm[31:20]
    write_u32(text, offset, existing | (((lo12 as u32) & 0xfff) << 20))
}

fn emit_import_stub(
    arch: &str,
    text: &mut Vec<u8>,
    stub_vmaddr: u64,
    got_vmaddr: u64,
) -> Result<(), String> {
    if arch == "riscv64" {
        // Load the resolved address from the GOT slot and jump: `auipc t3, hi;
        // ld t3, lo(t3); jr t3` (t3 = x28). 12 bytes, matching the fixed stub
        // slot. The loader fills the GOT slot via the JUMP_SLOT reloc.
        let (hi20, lo12) = riscv_hi_lo(got_vmaddr as i64 - stub_vmaddr as i64)?;
        put_u32(text, (hi20 << 12) | (28 << 7) | 0x17); // auipc t3, hi20
        put_u32(
            text,
            (((lo12 as u32) & 0xfff) << 20) | (28 << 15) | (0b011 << 12) | (28 << 7) | 0x03,
        ); // ld t3, lo12(t3)
        put_u32(text, (28 << 15) | 0x67); // jalr x0, 0(t3)
        return Ok(());
    }
    if arch == "x86_64" {
        // PLT stub `jmp *disp32(%rip)` (FF 25 disp32): jump through the GOT slot,
        // which the loader fills via the JUMP_SLOT reloc (non-lazy — the same rela
        // is also DT_RELA, resolved at load). disp32 is relative to the next
        // instruction (stub + 6). Padded to the fixed 12-byte per-stub slot with
        // int3 so the surrounding layout math (stub_count*12) is arch-independent.
        text.push(0xff);
        text.push(0x25);
        let rip = stub_vmaddr + 6;
        let delta = got_vmaddr as i64 - rip as i64;
        let disp = i32::try_from(delta).map_err(|_| {
            format!(
                "linux-x86_64 linker: PLT stub displacement {delta} exceeds the ±2 GiB reach of rel32"
            )
        })?;
        text.extend_from_slice(&disp.to_le_bytes());
        text.extend_from_slice(&[0xcc; 6]);
        return Ok(());
    }
    // aarch64: adrp x16, GOT_page; ldr x16, [x16, GOT_off]; br x16.
    let (immlo, immhi) = adrp_page21(stub_vmaddr, got_vmaddr)?;
    put_u32(text, 0x9000_0010 | (immlo << 29) | (immhi << 5));
    put_u32(
        text,
        0xf940_0211 | ((((got_vmaddr & 0xfff) / 8) as u32) << 10),
    );
    put_u32(text, 0xd61f_0220);
    Ok(())
}

fn symbol_vmaddr(
    image: &EncodedImage,
    symbol_name: &str,
    text_vmaddr: u64,
    data_vmaddr: u64,
) -> Result<u64, String> {
    let symbol = image
        .symbols
        .iter()
        .find(|symbol| symbol.name == symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' does not resolve"))?;
    Ok(match symbol.section {
        EncodedSection::Text => text_vmaddr + symbol.offset as u64,
        EncodedSection::Data => data_vmaddr + symbol.offset as u64,
    })
}

/// The classic SysV/ELF symbol hash, used for `Vernaux.vna_hash`
/// (plan-linker.md §6.2).
fn elf_hash(name: &[u8]) -> u32 {
    let mut hash: u32 = 0;
    for &byte in name {
        hash = (hash << 4).wrapping_add(byte as u32);
        let high = hash & 0xf000_0000;
        if high != 0 {
            hash ^= high >> 24;
        }
        hash &= !high;
    }
    hash
}

fn put_dynamic(bytes: &mut Vec<u8>, tag: u64, value: u64) {
    put_u64(bytes, tag);
    put_u64(bytes, value);
}

fn branch_imm26(source: usize, target: usize) -> Result<u32, String> {
    let delta = target as isize - source as isize;
    // A `BL`/`B` imm26 is a signed 26-bit word offset: ±2^25 words = ±128 MiB.
    // Masking without a reach check silently wraps an over-range branch into a
    // wrong instruction (bug-168); mirror `riscv_hi_lo` and return an error.
    if delta % 4 != 0 || !(-(1 << 27)..(1 << 27)).contains(&delta) {
        return Err(format!(
            "linux-aarch64 linker: branch displacement {delta} exceeds the ±128 MiB reach of BL/B"
        ));
    }
    Ok(((delta / 4) as i32 as u32) & 0x03ff_ffff)
}

/// Encode an `ADRP` page displacement, reach-checked (bug-168). The immediate is
/// a signed 21-bit count of 4 KiB pages (±2^20 pages = ±4 GiB); an over-range
/// delta must error rather than truncate to a wrong page. Returns `(immlo,
/// immhi)` ready to splice into the instruction word.
fn adrp_page21(pc: u64, target: u64) -> Result<(u32, u32), String> {
    let page_delta = ((target & !0xfff) as i64 - (pc & !0xfff) as i64) >> 12;
    if !(-(1 << 20)..(1 << 20)).contains(&page_delta) {
        return Err(format!(
            "linux-aarch64 linker: ADRP page displacement {page_delta} exceeds the ±4 GiB reach of ADRP"
        ));
    }
    let encoded = page_delta as u32;
    Ok((encoded & 0b11, (encoded >> 2) & 0x7ffff))
}

/// Compute a RIP-relative `rel32` displacement, erroring when it exceeds the ±2
/// GiB reach of a 32-bit displacement (bug-168) instead of silently wrapping.
/// `site` is the address of the 4-byte disp32 field; rip is `site + 4`.
fn rel32(target: u64, site: u64) -> Result<i32, String> {
    let delta = target as i64 - (site as i64 + 4);
    i32::try_from(delta).map_err(|_| {
        format!(
            "linux-x86_64 linker: RIP-relative displacement {delta} exceeds the ±2 GiB reach of rel32"
        )
    })
}

fn align(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(|| {
        format!(
            "linux linker: relocation offset {offset} + 4 exceeds text length {}",
            bytes.len()
        )
    })?;
    Ok(u32::from_le_bytes(slice.try_into().expect("slice length")))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    let len = bytes.len();
    let slice = bytes.get_mut(offset..offset + 4).ok_or_else(|| {
        format!("linux linker: relocation offset {offset} + 4 exceeds text length {len}")
    })?;
    slice.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
