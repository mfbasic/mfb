use super::*;

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
    if let Some(metadata) = signing_metadata {
        append_elf_signing_section(&mut bytes, metadata);
    }
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
    if let Some(metadata) = signing_metadata {
        append_elf_signing_section(&mut bytes, metadata);
    }
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
    // PHDR, INTERP, LOAD(text), LOAD(data-rw), DYNAMIC, GNU_STACK, NOTE (plan-43)
    // — plus a leading R LOAD for the constant partition when there is one
    // (bug-187).
    let ph_count = if rodata_size > 0 { 8_u16 } else { 7_u16 };
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
    let file_size = data_offset + data_file_size;
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
    // plus the dynamic payload (GOT/.rela/.dynamic), which must stay R+W.
    let rw_offset = data_offset + rodata_size;
    let rw_vmaddr = data_vmaddr + rodata_size as u64;
    let rw_size = data_file_size - rodata_size;
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
    // PT_GNU_STACK: mark the stack non-executable (R+W, no X) — LNK-02 / bug-186.
    // (PT_GNU_RELRO is deferred to compose with the bug-187 const/mutable data
    // partition, which page-isolates the GOT from the writable arena global.)
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
    if let Some(metadata) = image.signing_metadata.as_deref() {
        append_elf_signing_section(&mut bytes, metadata);
    }
    Ok(bytes)
}

fn append_elf_signing_section(bytes: &mut Vec<u8>, metadata: &[u8]) {
    const SHDR_SIZE: usize = 64;
    let metadata_offset = align(bytes.len(), 8);
    bytes.resize(metadata_offset, 0);
    bytes.extend_from_slice(metadata);
    let shstrtab_offset = align(bytes.len(), 1);
    let shstrtab = b"\0.mfb_sign\0.shstrtab\0";
    bytes.extend_from_slice(shstrtab);
    let shoff = align(bytes.len(), 8);
    bytes.resize(shoff, 0);

    bytes.resize(bytes.len() + SHDR_SIZE, 0);
    section_header(
        bytes,
        1,
        1,
        0,
        0,
        metadata_offset as u64,
        metadata.len() as u64,
        0,
        0,
        1,
        0,
    );
    section_header(
        bytes,
        11,
        3,
        0,
        0,
        shstrtab_offset as u64,
        shstrtab.len() as u64,
        0,
        0,
        1,
        0,
    );

    bytes[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
    bytes[58..60].copy_from_slice(&(SHDR_SIZE as u16).to_le_bytes());
    bytes[60..62].copy_from_slice(&3_u16.to_le_bytes());
    bytes[62..64].copy_from_slice(&2_u16.to_le_bytes());
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

struct DynamicPayload {
    bytes: Vec<u8>,
    dynamic_offset: usize,
    dynamic_size: usize,
}

impl DynamicPayload {
    fn build(arch: &str, flavor: LinuxFlavor, image: &EncodedImage) -> Result<Self, String> {
        let payload_start = image.data.len();
        let data_base_offset = align(image.data.len(), 8);
        let mut libraries = Vec::<String>::new();
        for import in &image.imports {
            if !libraries.contains(&import.library) {
                libraries.push(import.library.clone());
            }
        }
        let mut dynstr = vec![0];
        let mut library_offsets = Vec::new();
        for library in &libraries {
            library_offsets.push(dynstr.len());
            dynstr.extend_from_slice(library.as_bytes());
            dynstr.push(0);
        }
        let mut symbol_name_offsets = Vec::new();
        for import in &image.imports {
            symbol_name_offsets.push(dynstr.len());
            dynstr.extend_from_slice(import.symbol.as_bytes());
            dynstr.push(0);
        }

        // Symbol versioning (plan-linker.md §6.2). Each distinct (library, version)
        // pair becomes one Vernaux with a unique version index (>= 2); each
        // imported symbol's `.gnu.version` entry names its index (1 = unversioned
        // global). Driven by OpenSSL 3's `@@OPENSSL_3.0.0` exports, validated here
        // against glibc's `GLIBC_2.17` aarch64 baseline.
        let versioned = image.imports.iter().any(|import| import.version.is_some());
        let mut version_needs: Vec<(usize, String)> = Vec::new();
        let mut import_versym: Vec<u16> = Vec::with_capacity(image.imports.len());
        for import in &image.imports {
            match &import.version {
                Some(version) => {
                    let library_index = libraries
                        .iter()
                        .position(|library| library == &import.library)
                        .expect("import library is in the library list");
                    let index = version_needs
                        .iter()
                        .position(|(lib, ver)| *lib == library_index && ver == version)
                        .unwrap_or_else(|| {
                            version_needs.push((library_index, version.clone()));
                            version_needs.len() - 1
                        });
                    import_versym.push((index + 2) as u16);
                }
                None => import_versym.push(1),
            }
        }
        let mut version_string_offsets: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (_, version) in &version_needs {
            if !version_string_offsets.contains_key(version) {
                version_string_offsets.insert(version.clone(), dynstr.len());
                dynstr.extend_from_slice(version.as_bytes());
                dynstr.push(0);
            }
        }

        let dynstr_offset = data_base_offset;
        let dynsym_offset = align(dynstr_offset + dynstr.len(), 8);
        let dynsym_size = (image.imports.len() + 1) * 24;
        let hash_offset = align(dynsym_offset + dynsym_size, 8);
        let hash_size = (2 + 1 + image.imports.len() + 1) * 4;
        let rela_offset = align(hash_offset + hash_size, 8);
        // .rela.dyn holds the import GLOB_DAT/JUMP_SLOT bindings first, then one
        // R_*_RELATIVE per DT_INIT_ARRAY entry that biases its absolute function
        // pointer into the PIE (bug-186). DT_RELA/DT_RELASZ cover the whole table;
        // DT_JMPREL/DT_PLTRELSZ cover only the leading import portion.
        let import_rela_size = image.imports.len() * 24;
        let rela_size = import_rela_size + image.initializers.len() * 24;
        let got_offset = align(rela_offset + rela_size, 8);
        let got_size = image.imports.len() * 8;
        // `.gnu.version` (one Elf64_Half per dynsym) and `.gnu.version_r` follow
        // the GOT, before the dynamic table, only when versioning is active.
        let versym_offset = align(got_offset + got_size, 8);
        let versym_size = if versioned {
            (image.imports.len() + 1) * 2
        } else {
            0
        };
        // Group needs by library to size the Verneed/Vernaux records.
        let mut needs_by_library: Vec<(usize, Vec<(String, usize)>)> = Vec::new();
        for (global, (library_index, version)) in version_needs.iter().enumerate() {
            let entry = needs_by_library
                .iter_mut()
                .find(|(lib, _)| lib == library_index);
            match entry {
                Some((_, versions)) => versions.push((version.clone(), global + 2)),
                None => {
                    needs_by_library.push((*library_index, vec![(version.clone(), global + 2)]))
                }
            }
        }
        let verneed_offset = align(versym_offset + versym_size, 8);
        let verneed_size = if versioned {
            needs_by_library.len() * 16 + version_needs.len() * 16
        } else {
            0
        };
        // Load-time initializers (plan-linker.md §5.3/§6.4): an array of absolute
        // text addresses the loader runs after relocation and before the entry.
        let init_array_offset = align(verneed_offset + verneed_size, 8);
        let init_array_size = image.initializers.len() * 8;
        let dynamic_offset = align(init_array_offset + init_array_size, 8);
        let dynamic_count = libraries.len()
            + 14
            + if versioned { 3 } else { 0 }
            + if image.initializers.is_empty() { 0 } else { 2 };
        let dynamic_size = dynamic_count * 16;
        let data_offset = align(
            TEXT_FILE_OFFSET + image.text.len() + image.imports.len() * 12,
            PAGE_SIZE,
        );
        // PIE: file-relative (base 0); the loader biases GOT/.rela/DT_* itself.
        let data_vmaddr = DYN_IMAGE_BASE + data_offset as u64;

        let mut bytes = Vec::new();
        bytes.resize(dynstr_offset - payload_start, 0);
        bytes.extend_from_slice(&dynstr);
        bytes.resize(dynsym_offset - payload_start, 0);
        bytes.resize(bytes.len() + 24, 0);
        for (index, import) in image.imports.iter().enumerate() {
            put_u32(&mut bytes, symbol_name_offsets[index] as u32);
            // st_info: GLOBAL OBJECT (0x11) for a data global, GLOBAL FUNC (0x12)
            // for a function.
            bytes.push(match import.kind {
                ImportKind::Data => 0x11,
                ImportKind::Function => 0x12,
            });
            bytes.push(0);
            put_u16(&mut bytes, 0);
            put_u64(&mut bytes, 0);
            put_u64(&mut bytes, 0);
        }

        // SysV `DT_HASH`: nbucket, nchain, one bucket, then `nchain` chain words.
        // Symbol 0 is the null symbol, so `chain[0]` is unused and must be 0; the
        // bucket starts the list at symbol 1 and each `chain[i]` links to the next
        // symbol, with 0 (STN_UNDEF) terminating it. Writing the first link into
        // `chain[0]` shifted every entry down one slot, so a by-name lookup of the
        // second and later symbols walked past its own entry.
        bytes.resize(hash_offset - payload_start, 0);
        put_u32(&mut bytes, 1);
        put_u32(&mut bytes, (image.imports.len() + 1) as u32);
        put_u32(&mut bytes, if image.imports.is_empty() { 0 } else { 1 });
        put_u32(&mut bytes, 0); // chain[0] — the null symbol
        for index in 1..=image.imports.len() {
            let next = if index == image.imports.len() {
                0
            } else {
                index + 1
            };
            put_u32(&mut bytes, next as u32);
        }

        bytes.resize(rela_offset - payload_start, 0);
        for (index, import) in image.imports.iter().enumerate() {
            let symbol_index = index + 1;
            // GLOB_DAT binds a data global's GOT slot to the symbol's address;
            // JUMP_SLOT binds a function's GOT slot for its call stub
            // (plan-linker.md §6.1).
            let reloc_type = match (arch, import.kind) {
                ("x86_64", ImportKind::Data) => R_X86_64_GLOB_DAT,
                ("x86_64", ImportKind::Function) => R_X86_64_JUMP_SLOT,
                ("riscv64", ImportKind::Data) => R_RISCV_64,
                ("riscv64", ImportKind::Function) => R_RISCV_JUMP_SLOT,
                (_, ImportKind::Data) => R_AARCH64_GLOB_DAT,
                (_, ImportKind::Function) => R_AARCH64_JUMP_SLOT,
            };
            put_u64(
                &mut bytes,
                data_vmaddr + got_offset as u64 + (index * 8) as u64,
            );
            put_u64(
                &mut bytes,
                ((symbol_index as u64) << 32) | reloc_type as u64,
            );
            put_u64(&mut bytes, 0);
        }
        // One R_*_RELATIVE per DT_INIT_ARRAY slot: *(base + r_offset) = base +
        // r_addend, biasing the absolute initializer pointer for the PIE slide
        // (bug-186). Symbol index 0 (RELATIVE has no symbol). Emitted after the
        // import bindings; DT_JMPREL's size stays the import-only prefix so the
        // PLT pass never sees these.
        let relative_type = match arch {
            "x86_64" => R_X86_64_RELATIVE,
            "riscv64" => R_RISCV_RELATIVE,
            _ => R_AARCH64_RELATIVE,
        };
        for (index, name) in image.initializers.iter().enumerate() {
            let symbol = image
                .symbols
                .iter()
                .find(|symbol| symbol.name == *name && symbol.section == EncodedSection::Text)
                .ok_or_else(|| format!("initializer '{name}' does not resolve to a text symbol"))?;
            put_u64(
                &mut bytes,
                data_vmaddr + init_array_offset as u64 + (index * 8) as u64,
            );
            put_u64(&mut bytes, relative_type as u64);
            put_u64(
                &mut bytes,
                DYN_IMAGE_BASE + TEXT_FILE_OFFSET as u64 + symbol.offset as u64,
            );
        }

        bytes.resize(got_offset - payload_start, 0);
        bytes.resize(bytes.len() + got_size, 0);

        if versioned {
            // .gnu.version: index 0 (null sym) then one per imported symbol.
            bytes.resize(versym_offset - payload_start, 0);
            put_u16(&mut bytes, 0);
            for value in &import_versym {
                put_u16(&mut bytes, *value);
            }
            // .gnu.version_r: one Verneed per library, one Vernaux per version.
            bytes.resize(verneed_offset - payload_start, 0);
            for (need_index, (library_index, versions)) in needs_by_library.iter().enumerate() {
                let last_need = need_index + 1 == needs_by_library.len();
                put_u16(&mut bytes, 1); // vn_version
                put_u16(&mut bytes, versions.len() as u16); // vn_cnt
                put_u32(&mut bytes, library_offsets[*library_index] as u32); // vn_file
                put_u32(&mut bytes, 16); // vn_aux: first Vernaux follows
                put_u32(
                    &mut bytes,
                    if last_need {
                        0
                    } else {
                        (16 + versions.len() * 16) as u32
                    },
                ); // vn_next
                for (aux_index, (version, version_index)) in versions.iter().enumerate() {
                    let last_aux = aux_index + 1 == versions.len();
                    put_u32(&mut bytes, elf_hash(version.as_bytes())); // vna_hash
                    put_u16(&mut bytes, 0); // vna_flags
                    put_u16(&mut bytes, *version_index as u16); // vna_other
                    put_u32(&mut bytes, version_string_offsets[version] as u32); // vna_name
                    put_u32(&mut bytes, if last_aux { 0 } else { 16 }); // vna_next
                }
            }
        }

        if !image.initializers.is_empty() {
            bytes.resize(init_array_offset - payload_start, 0);
            for name in &image.initializers {
                let symbol = image
                    .symbols
                    .iter()
                    .find(|symbol| symbol.name == *name && symbol.section == EncodedSection::Text)
                    .ok_or_else(|| {
                        format!("initializer '{name}' does not resolve to a text symbol")
                    })?;
                // PIE: store the file-relative addend; the R_*_RELATIVE reloc
                // emitted in the .rela table biases it to base + addend at load
                // time (bug-186). The value here is also the addend the loader
                // reads, so pre-filling it keeps REL/RELA implementations that
                // add-in-place correct too.
                put_u64(
                    &mut bytes,
                    DYN_IMAGE_BASE + TEXT_FILE_OFFSET as u64 + symbol.offset as u64,
                );
            }
        }

        bytes.resize(dynamic_offset - payload_start, 0);
        for offset in library_offsets {
            put_dynamic(&mut bytes, 1, offset as u64);
        }
        put_dynamic(&mut bytes, 4, data_vmaddr + hash_offset as u64);
        put_dynamic(&mut bytes, 5, data_vmaddr + dynstr_offset as u64);
        put_dynamic(&mut bytes, 6, data_vmaddr + dynsym_offset as u64);
        put_dynamic(&mut bytes, 10, dynstr.len() as u64);
        put_dynamic(&mut bytes, 11, 24);
        put_dynamic(&mut bytes, 3, data_vmaddr + got_offset as u64);
        put_dynamic(&mut bytes, 7, data_vmaddr + rela_offset as u64);
        put_dynamic(&mut bytes, 8, rela_size as u64);
        put_dynamic(&mut bytes, 9, 24);
        put_dynamic(&mut bytes, 20, 7);
        put_dynamic(&mut bytes, 23, data_vmaddr + rela_offset as u64);
        // DT_PLTRELSZ is the import (PLT/GOT) prefix only; the R_*_RELATIVE tail is
        // covered by DT_RELASZ above, not the JMPREL pass (bug-186).
        put_dynamic(&mut bytes, 2, import_rela_size as u64);
        put_dynamic(&mut bytes, 30, 8);
        if versioned {
            // DT_VERSYM, DT_VERNEED, DT_VERNEEDNUM.
            put_dynamic(&mut bytes, 0x6fff_fff0, data_vmaddr + versym_offset as u64);
            put_dynamic(&mut bytes, 0x6fff_fffe, data_vmaddr + verneed_offset as u64);
            put_dynamic(&mut bytes, 0x6fff_ffff, needs_by_library.len() as u64);
        }
        if !image.initializers.is_empty() {
            // DT_INIT_ARRAY, DT_INIT_ARRAYSZ.
            put_dynamic(&mut bytes, 25, data_vmaddr + init_array_offset as u64);
            put_dynamic(&mut bytes, 27, init_array_size as u64);
        }
        put_dynamic(&mut bytes, 0, 0);

        let expected_dynamic_size = bytes.len() - (dynamic_offset - payload_start);
        if expected_dynamic_size != dynamic_size {
            return Err(format!(
                "internal Linux {} dynamic table size mismatch: expected {dynamic_size}, got {expected_dynamic_size}",
                flavor.suffix()
            ));
        }

        Ok(Self {
            bytes,
            dynamic_offset: dynamic_offset - payload_start,
            dynamic_size,
        })
    }
}

pub(super) fn dynamic_prefix_size(image: &EncodedImage) -> usize {
    let mut libraries = Vec::<&str>::new();
    for import in &image.imports {
        if !libraries.contains(&import.library.as_str()) {
            libraries.push(import.library.as_str());
        }
    }
    // Distinct version strings also live in `.dynstr` (plan-linker.md §6.2); they
    // must be counted here so the GOT offset baked into each stub matches the
    // offset `DynamicPayload::build` computes after appending them.
    let mut version_strings = Vec::<&str>::new();
    for import in &image.imports {
        if let Some(version) = &import.version {
            if !version_strings.contains(&version.as_str()) {
                version_strings.push(version.as_str());
            }
        }
    }
    let dynstr_len = 1
        + libraries
            .iter()
            .map(|library| library.len() + 1)
            .sum::<usize>()
        + image
            .imports
            .iter()
            .map(|import| import.symbol.len() + 1)
            .sum::<usize>()
        + version_strings
            .iter()
            .map(|version| version.len() + 1)
            .sum::<usize>();
    let dynstr_offset = align(image.data.len(), 8);
    let dynsym_offset = align(dynstr_offset + dynstr_len, 8);
    let dynsym_size = (image.imports.len() + 1) * 24;
    let hash_offset = align(dynsym_offset + dynsym_size, 8);
    let hash_size = (2 + 1 + image.imports.len() + 1) * 4;
    let rela_offset = align(hash_offset + hash_size, 8);
    // Must match DynamicPayload::build: imports + one R_*_RELATIVE per initializer
    // (bug-186), so the GOT offset baked into each stub stays correct.
    let rela_size = (image.imports.len() + image.initializers.len()) * 24;
    align(rela_offset + rela_size, 8)
}
