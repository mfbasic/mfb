use super::*;
use crate::os::debug_symbols::{DebugSymbol, DebugSymbolKind};
use crate::os::link_encode::section_vmaddr;

mod dynamic;
mod sections;
pub(super) use dynamic::dynamic_prefix_size;
use dynamic::DynamicPayload;
use sections::*;

/// The `Elf64_Nhdr`-framed provenance note (plan-43 §5): `namesz`/`descsz`/`type`,
/// then the `MFBasic\0` name and the descriptor. Both are already 4-aligned, so
/// neither needs the note-format padding.
fn mfb_note_bytes() -> Vec<u8> {
    let descriptor = mfb_note_descriptor();
    let mut bytes = Vec::new();
    put_u32(&mut bytes, MFB_NOTE_OWNER.len() as u32);
    put_u32(&mut bytes, descriptor.len() as u32);
    put_u32(&mut bytes, MFB_NOTE_TYPE);
    bytes.extend_from_slice(MFB_NOTE_OWNER);
    bytes.extend_from_slice(&descriptor);
    bytes
}

/// The `PT_NOTE` program header covering `note` at `note_offset`. Read-only, and
/// mapped by the enclosing text `PT_LOAD` (the note lives below
/// `TEXT_FILE_OFFSET`, inside that segment's file range).
fn note_program_header(bytes: &mut Vec<u8>, image_base: u64, note_offset: usize, note_len: usize) {
    program_header(
        bytes,
        PT_NOTE,
        4, // p_flags = R
        note_offset as u64,
        image_base + note_offset as u64,
        image_base + note_offset as u64,
        note_len as u64,
        note_len as u64,
        4,
    );
}

/// Program headers a static image carries: the text `PT_LOAD`, the data
/// `PT_LOAD`, `PT_GNU_STACK` (bug-224), and the `PT_NOTE` provenance marker
/// (plan-43).
const STATIC_PH_COUNT: usize = 4;

/// File offset of the provenance note in a static image: straight after the
/// program-header table, in the padding the text `PT_LOAD` already maps below
/// `TEXT_FILE_OFFSET`.
fn static_note_offset() -> usize {
    align(64 + STATIC_PH_COUNT * 56, 8)
}

/// A static AArch64/RISC-V ELF executable: the shape a build takes when it imports
/// nothing, so no interpreter or PLT/GOT is needed.
///
/// Data is placed at the **page-aligned** offset `write_executable` already assumes
/// when it patches relocations (`data_vmaddr = IMAGE_BASE + align(TEXT_FILE_OFFSET
/// + text.len(), PAGE_SIZE)`). Appending it straight after text instead put every
/// string and constant `align(…) - (TEXT_FILE_OFFSET + text.len())` bytes below the
/// address each `page21`/`pageoff12` (AArch64) or `pcrel` (RISC-V) relocation
/// resolved to. As in `encode_static_elf_x86`, data lives in its own **writable**
/// PT_LOAD, because the entry writes `_mfb_rt_main_arena` into it.
pub(super) fn encode_static_elf(
    arch: &str,
    entry_offset: usize,
    text: &[u8],
    data: &[u8],
    signing_metadata: Option<&[u8]>,
    debug: Option<&[DebugSymbol<'_>]>,
) -> Vec<u8> {
    let text_offset = TEXT_FILE_OFFSET;
    let text_vmaddr = IMAGE_BASE + text_offset as u64;
    let data_offset = align(text_offset + text.len(), PAGE_SIZE);
    let data_vmaddr = IMAGE_BASE + data_offset as u64;
    let text_seg_filesz = (text_offset + text.len()) as u64; // ELF header + phdrs + text

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x7f, b'E', b'L', b'F']);
    bytes.extend_from_slice(&[2, 1, 1, 0]); // ELFCLASS64, LE, version, SysV ABI
    bytes.resize(16, 0);
    put_u16(&mut bytes, 2); // e_type = ET_EXEC
    put_u16(&mut bytes, e_machine(arch)); // EM_AARCH64 (183) or EM_RISCV (243)
    put_u32(&mut bytes, 1); // e_version
    put_u64(&mut bytes, text_vmaddr + entry_offset as u64); // e_entry
    put_u64(&mut bytes, 64); // e_phoff
    put_u64(&mut bytes, 0); // e_shoff
    put_u32(&mut bytes, 0); // e_flags
    put_u16(&mut bytes, 64); // e_ehsize
    put_u16(&mut bytes, 56); // e_phentsize
    put_u16(&mut bytes, STATIC_PH_COUNT as u16); // e_phnum (text + data + GNU_STACK + NOTE)
    put_u16(&mut bytes, 0); // e_shentsize
    put_u16(&mut bytes, 0); // e_shnum
    put_u16(&mut bytes, 0); // e_shstrndx

    // PT_LOAD text (R+X)
    put_u32(&mut bytes, 1); // p_type = PT_LOAD
    put_u32(&mut bytes, 5); // p_flags = R+X
    put_u64(&mut bytes, 0); // p_offset
    put_u64(&mut bytes, IMAGE_BASE); // p_vaddr
    put_u64(&mut bytes, IMAGE_BASE); // p_paddr
    put_u64(&mut bytes, text_seg_filesz); // p_filesz
    put_u64(&mut bytes, text_seg_filesz); // p_memsz
    put_u64(&mut bytes, 0x1000); // p_align

    // PT_LOAD data (R+W)
    put_u32(&mut bytes, 1); // p_type = PT_LOAD
    put_u32(&mut bytes, 6); // p_flags = R+W
    put_u64(&mut bytes, data_offset as u64); // p_offset
    put_u64(&mut bytes, data_vmaddr); // p_vaddr
    put_u64(&mut bytes, data_vmaddr); // p_paddr
    put_u64(&mut bytes, data.len() as u64); // p_filesz
    put_u64(&mut bytes, data.len() as u64); // p_memsz
    put_u64(&mut bytes, 0x1000); // p_align

    // PT_GNU_STACK: mark the stack non-executable (R+W, no X) so the loader does
    // not fall back to an executable (RWX) stack for this static executable
    // (bug-224). All sizes 0 — it is a marker, not a loaded segment.
    program_header(&mut bytes, PT_GNU_STACK, 6, 0, 0, 0, 0, 0, 0x10);

    // PT_NOTE: the `MFBasic\0` provenance marker (plan-43).
    let note = mfb_note_bytes();
    let note_offset = static_note_offset();
    note_program_header(&mut bytes, IMAGE_BASE, note_offset, note.len());

    bytes.resize(note_offset, 0);
    bytes.extend_from_slice(&note);
    bytes.resize(text_offset, 0);
    bytes.extend_from_slice(text);
    bytes.resize(data_offset, 0);
    bytes.extend_from_slice(data);
    append_elf_sections(
        &mut bytes,
        &LoadedSections {
            text_vmaddr,
            text_len: text.len(),
            data_offset,
            data_vmaddr,
            data_len: data.len(),
            rodata_size: 0,
            dynamic: None,
        },
        debug,
        signing_metadata,
    );
    bytes
}

/// A static x86-64 ELF executable (plan-00-H). Unlike the AArch64 console path
/// (which dynamically links libc), the x86 backend uses raw syscalls, so there
/// are no imports and a static, interpreter-less ELF suffices. Two PT_LOAD
/// segments — text R+X and a separate **writable** R+W data segment, page-
/// aligned so a data symbol's vmaddr matches `write_executable`'s
/// `data_vmaddr = IMAGE_BASE + align(TEXT_FILE_OFFSET + text.len(), PAGE_SIZE)`
/// (the entry writes `_mfb_rt_main_arena`, so data must be writable).
pub(super) fn encode_static_elf_x86(
    entry_offset: usize,
    text: &[u8],
    data: &[u8],
    signing_metadata: Option<&[u8]>,
    debug: Option<&[DebugSymbol<'_>]>,
) -> Vec<u8> {
    let text_offset = TEXT_FILE_OFFSET;
    let text_vmaddr = IMAGE_BASE + text_offset as u64;
    let data_offset = align(text_offset + text.len(), PAGE_SIZE);
    let data_vmaddr = IMAGE_BASE + data_offset as u64;
    let text_seg_filesz = (text_offset + text.len()) as u64; // ELF header + phdrs + text

    let mut bytes = Vec::new();
    // e_ident
    bytes.extend_from_slice(&[0x7f, b'E', b'L', b'F']);
    bytes.extend_from_slice(&[2, 1, 1, 0]); // ELFCLASS64, LE, version, SysV ABI
    bytes.resize(16, 0);
    put_u16(&mut bytes, 2); // e_type = ET_EXEC
    put_u16(&mut bytes, 62); // e_machine = EM_X86_64
    put_u32(&mut bytes, 1); // e_version
    put_u64(&mut bytes, text_vmaddr + entry_offset as u64); // e_entry
    put_u64(&mut bytes, 64); // e_phoff
    put_u64(&mut bytes, 0); // e_shoff
    put_u32(&mut bytes, 0); // e_flags
    put_u16(&mut bytes, 64); // e_ehsize
    put_u16(&mut bytes, 56); // e_phentsize
    put_u16(&mut bytes, STATIC_PH_COUNT as u16); // e_phnum (text + data + GNU_STACK + NOTE)
    put_u16(&mut bytes, 0); // e_shentsize
    put_u16(&mut bytes, 0); // e_shnum
    put_u16(&mut bytes, 0); // e_shstrndx

    // PT_LOAD text (R+X)
    put_u32(&mut bytes, 1); // p_type = PT_LOAD
    put_u32(&mut bytes, 5); // p_flags = R+X
    put_u64(&mut bytes, 0); // p_offset
    put_u64(&mut bytes, IMAGE_BASE); // p_vaddr
    put_u64(&mut bytes, IMAGE_BASE); // p_paddr
    put_u64(&mut bytes, text_seg_filesz); // p_filesz
    put_u64(&mut bytes, text_seg_filesz); // p_memsz
    put_u64(&mut bytes, 0x1000); // p_align

    // PT_LOAD data (R+W)
    put_u32(&mut bytes, 1); // p_type = PT_LOAD
    put_u32(&mut bytes, 6); // p_flags = R+W
    put_u64(&mut bytes, data_offset as u64); // p_offset
    put_u64(&mut bytes, data_vmaddr); // p_vaddr
    put_u64(&mut bytes, data_vmaddr); // p_paddr
    put_u64(&mut bytes, data.len() as u64); // p_filesz
    put_u64(&mut bytes, data.len() as u64); // p_memsz
    put_u64(&mut bytes, 0x1000); // p_align

    // PT_GNU_STACK: mark the stack non-executable (R+W, no X) so the loader does
    // not fall back to an executable (RWX) stack for this static executable
    // (bug-224). All sizes 0 — it is a marker, not a loaded segment.
    program_header(&mut bytes, PT_GNU_STACK, 6, 0, 0, 0, 0, 0, 0x10);

    // PT_NOTE: the `MFBasic\0` provenance marker (plan-43).
    let note = mfb_note_bytes();
    let note_offset = static_note_offset();
    note_program_header(&mut bytes, IMAGE_BASE, note_offset, note.len());

    bytes.resize(note_offset, 0);
    bytes.extend_from_slice(&note);
    bytes.resize(text_offset, 0);
    bytes.extend_from_slice(text);
    bytes.resize(data_offset, 0);
    bytes.extend_from_slice(data);
    append_elf_sections(
        &mut bytes,
        &LoadedSections {
            text_vmaddr,
            text_len: text.len(),
            data_offset,
            data_vmaddr,
            data_len: data.len(),
            rodata_size: 0,
            dynamic: None,
        },
        debug,
        signing_metadata,
    );
    bytes
}

pub(super) fn encode_dynamic_elf(
    arch: &str,
    flavor: LinuxFlavor,
    entry_offset: usize,
    text: &[u8],
    data: &[u8],
    image: &EncodedImage,
) -> Result<Vec<u8>, String> {
    let dynamic = DynamicPayload::build(arch, flavor, image)?;
    // Read-only constant prefix of `data` (bug-187), page-aligned by the encoder.
    // When present it is carved into its own R `PT_LOAD`, leaving the arena global
    // and the dynamic payload in the writable one.
    let rodata_size = image.rodata_size.min(data.len());
    // PHDR, INTERP, LOAD(text), LOAD(data-rw), DYNAMIC, GNU_RELRO, GNU_STACK, NOTE
    // (plan-43) — plus a leading R LOAD for the constant partition when there is
    // one (bug-187). The GNU_RELRO header is the bug-263 addition.
    let ph_count = if rodata_size > 0 { 9_u16 } else { 8_u16 };
    let interp = interpreter(arch, flavor).as_bytes();
    let interp_offset = 64 + ph_count as usize * 56;
    // The provenance note shares the header/text gap with the interpreter string;
    // both stay below `TEXT_FILE_OFFSET`, inside the text PT_LOAD's file range.
    let note = mfb_note_bytes();
    let note_offset = align(interp_offset + interp.len() + 1, 8);
    let text_offset = TEXT_FILE_OFFSET;
    // PIE (ET_DYN): file-relative virtual addresses, loader adds the slide.
    let text_vmaddr = DYN_IMAGE_BASE + text_offset as u64;
    let data_offset = align(text_offset + text.len(), PAGE_SIZE);
    let data_vmaddr = DYN_IMAGE_BASE + data_offset as u64;
    let data_file_size = data.len() + dynamic.bytes.len();
    // bug-263: the dynamic payload (GOT/.dynamic) is page-aligned inside `data`'s
    // trailing region (see `DynamicPayload::build`). `relro_start` is that page
    // boundary; pad the payload end up to `relro_end` so PT_GNU_RELRO covers whole
    // pages (glibc rounds the RELRO end down to a page, so an unpadded tail page of
    // `.dynamic` would be left writable). The arena global lives below
    // `relro_start` and stays read-write.
    let relro_start = align(data.len(), PAGE_SIZE);
    let relro_end = align(data_file_size, PAGE_SIZE);
    let file_size = data_offset + relro_end;
    let dynamic_offset = data_offset + data.len() + dynamic.dynamic_offset;
    let dynamic_vmaddr = data_vmaddr + data.len() as u64 + dynamic.dynamic_offset as u64;
    let dynamic_size = dynamic.dynamic_size;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x7f, b'E', b'L', b'F']);
    bytes.extend_from_slice(&[2, 1, 1, 0]);
    bytes.resize(16, 0);
    // ET_DYN: a position-independent executable the loader maps at a random base
    // (bug-186). macOS is already PIE; this brings Linux in line.
    put_u16(&mut bytes, 3);
    // e_machine: EM_AARCH64 (183), EM_X86_64 (62), or EM_RISCV (243).
    put_u16(&mut bytes, e_machine(arch));
    put_u32(&mut bytes, 1);
    put_u64(&mut bytes, text_vmaddr + entry_offset as u64);
    put_u64(&mut bytes, 64);
    put_u64(&mut bytes, 0);
    // e_flags: RISC-V encodes the float ABI here (EF_RISCV_FLOAT_ABI_DOUBLE =
    // 0x4 for lp64d). The musl/glibc rv64 dynamic loader refuses a soft-float
    // (0x0) executable, so this must be set. Zero for x86/aarch64.
    put_u32(&mut bytes, e_flags(arch));
    put_u16(&mut bytes, 64);
    put_u16(&mut bytes, 56);
    put_u16(&mut bytes, ph_count);
    put_u16(&mut bytes, 0);
    put_u16(&mut bytes, 0);
    put_u16(&mut bytes, 0);

    program_header(
        &mut bytes,
        6,
        4,
        64,
        DYN_IMAGE_BASE + 64,
        DYN_IMAGE_BASE + 64,
        ph_count as u64 * 56,
        ph_count as u64 * 56,
        8,
    );
    program_header(
        &mut bytes,
        3,
        4,
        interp_offset as u64,
        DYN_IMAGE_BASE + interp_offset as u64,
        DYN_IMAGE_BASE + interp_offset as u64,
        (interp.len() + 1) as u64,
        (interp.len() + 1) as u64,
        1,
    );
    program_header(
        &mut bytes,
        1,
        5,
        0,
        DYN_IMAGE_BASE,
        DYN_IMAGE_BASE,
        (text_offset + text.len()) as u64,
        (text_offset + text.len()) as u64,
        PAGE_SIZE as u64,
    );
    // Constant partition: a read-only (R) PT_LOAD over `data[..rodata_size]` — the
    // string literals / error messages an attacker with an arbitrary write must
    // not be able to corrupt (bug-187). Emitted before the writable load so the
    // two occupy disjoint pages (the encoder page-aligned the boundary).
    if rodata_size > 0 {
        program_header(
            &mut bytes,
            1,
            4,
            data_offset as u64,
            data_vmaddr,
            data_vmaddr,
            rodata_size as u64,
            rodata_size as u64,
            PAGE_SIZE as u64,
        );
    }
    // Writable data: the mutable suffix of `data` (arena global + runtime globals)
    // plus the dynamic payload (GOT/.rela/.dynamic). The whole segment is mapped
    // R+W; PT_GNU_RELRO below re-protects the GOT/.dynamic pages to R after
    // binding. `rw_size` runs to the page-aligned payload end (`relro_end`) so the
    // padded RELRO pages are backed by this load.
    let rw_offset = data_offset + rodata_size;
    let rw_vmaddr = data_vmaddr + rodata_size as u64;
    let rw_size = relro_end - rodata_size;
    program_header(
        &mut bytes,
        1,
        6,
        rw_offset as u64,
        rw_vmaddr,
        rw_vmaddr,
        rw_size as u64,
        rw_size as u64,
        PAGE_SIZE as u64,
    );
    program_header(
        &mut bytes,
        2,
        6,
        dynamic_offset as u64,
        dynamic_vmaddr,
        dynamic_vmaddr,
        dynamic_size as u64,
        dynamic_size as u64,
        8,
    );
    // PT_GNU_RELRO (bug-263 / LNK-03): the loader `mprotect`s this range back to
    // read-only after startup relocation, so the GOT and `.dynamic` — which
    // `DF_BIND_NOW` fully resolves at startup — are no longer a writable
    // control-flow-hijack target. The range is the page-aligned dynamic payload,
    // page-disjoint from the still-writable arena global below `relro_start`.
    program_header(
        &mut bytes,
        PT_GNU_RELRO,
        4,
        (data_offset + relro_start) as u64,
        data_vmaddr + relro_start as u64,
        data_vmaddr + relro_start as u64,
        (relro_end - relro_start) as u64,
        (relro_end - relro_start) as u64,
        1,
    );
    // PT_GNU_STACK: mark the stack non-executable (R+W, no X) — LNK-02 / bug-186.
    program_header(&mut bytes, PT_GNU_STACK, 6, 0, 0, 0, 0, 0, 0x10);
    // PT_NOTE: the `MFBasic\0` provenance marker (plan-43).
    note_program_header(&mut bytes, DYN_IMAGE_BASE, note_offset, note.len());

    bytes.resize(interp_offset, 0);
    bytes.extend_from_slice(interp);
    bytes.push(0);
    bytes.resize(note_offset, 0);
    bytes.extend_from_slice(&note);
    bytes.resize(text_offset, 0);
    bytes.extend_from_slice(text);
    bytes.resize(data_offset, 0);
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(&dynamic.bytes);
    bytes.resize(file_size, 0);
    let debug = if image.debug_symbols {
        Some(crate::os::debug_symbols::debug_symbols(image)?)
    } else {
        None
    };
    let payload_offset = data_offset + data.len();
    let payload_vmaddr = data_vmaddr + data.len() as u64;
    append_elf_sections(
        &mut bytes,
        &LoadedSections {
            text_vmaddr,
            text_len: text.len(),
            data_offset,
            data_vmaddr,
            data_len: data.len(),
            rodata_size,
            dynamic: Some(DynamicSections {
                dynstr_offset: payload_offset + dynamic.dynstr_offset,
                dynstr_vmaddr: payload_vmaddr + dynamic.dynstr_offset as u64,
                dynstr_size: dynamic.dynstr_size,
                dynamic_offset,
                dynamic_vmaddr,
                dynamic_size,
            }),
        },
        debug.as_deref(),
        image
            .signing_metadata
            .as_ref()
            .map(|signing| signing.metadata.as_slice()),
    );
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
fn section_header(
    bytes: &mut Vec<u8>,
    name: u32,
    type_: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    addralign: u64,
    entsize: u64,
) {
    put_u32(bytes, name);
    put_u32(bytes, type_);
    put_u64(bytes, flags);
    put_u64(bytes, addr);
    put_u64(bytes, offset);
    put_u64(bytes, size);
    put_u32(bytes, link);
    put_u32(bytes, info);
    put_u64(bytes, addralign);
    put_u64(bytes, entsize);
}

#[allow(clippy::too_many_arguments)]
fn program_header(
    bytes: &mut Vec<u8>,
    type_: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    file_size: u64,
    mem_size: u64,
    align: u64,
) {
    put_u32(bytes, type_);
    put_u32(bytes, flags);
    put_u64(bytes, offset);
    put_u64(bytes, vaddr);
    put_u64(bytes, paddr);
    put_u64(bytes, file_size);
    put_u64(bytes, mem_size);
    put_u64(bytes, align);
}

fn interpreter(arch: &str, flavor: LinuxFlavor) -> &'static str {
    match (arch, flavor) {
        ("x86_64", LinuxFlavor::Glibc) => "/lib64/ld-linux-x86-64.so.2",
        ("x86_64", LinuxFlavor::Musl) => "/lib/ld-musl-x86_64.so.1",
        ("riscv64", LinuxFlavor::Glibc) => "/lib/ld-linux-riscv64-lp64d.so.1",
        ("riscv64", LinuxFlavor::Musl) => "/lib/ld-musl-riscv64.so.1",
        (_, LinuxFlavor::Glibc) => "/lib/ld-linux-aarch64.so.1",
        (_, LinuxFlavor::Musl) => "/lib/ld-musl-aarch64.so.1",
    }
}

/// The ELF `e_machine` for a target arch: EM_X86_64 (62), EM_RISCV (243), or
/// EM_AARCH64 (183, the default).
fn e_machine(arch: &str) -> u16 {
    match arch {
        "x86_64" => 62,
        "riscv64" => 243,
        _ => 183,
    }
}

/// The ELF `e_flags` for a target arch. Only RISC-V uses them: the lp64d ABI
/// sets EF_RISCV_FLOAT_ABI_DOUBLE (0x4). x86/aarch64 use 0.
fn e_flags(arch: &str) -> u32 {
    match arch {
        "riscv64" => 0x0000_0004, // EF_RISCV_FLOAT_ABI_DOUBLE
        _ => 0,
    }
}
