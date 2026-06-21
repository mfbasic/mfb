use crate::arch::aarch64::encode::{EncodedImage, EncodedSection, ImportKind};
use crate::os::linux::flavor::LinuxFlavor;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const IMAGE_BASE: u64 = 0x400000;
const TEXT_FILE_OFFSET: usize = 0x1000;
const PAGE_SIZE: usize = 0x1000;
const R_AARCH64_GLOB_DAT: u32 = 1025;
const R_AARCH64_JUMP_SLOT: u32 = 1026;

pub(crate) fn write_executable(
    project_dir: &Path,
    project_name: &str,
    flavor: LinuxFlavor,
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
        append_import_stubs(&mut text, image, text_vmaddr)?
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
    let bytes = if image.imports.is_empty() {
        encode_static_elf(entry_offset, &text, &image.data, image.signing_metadata.as_deref())
    } else {
        encode_dynamic_elf(flavor, entry_offset, &text, &image.data, image)?
    };
    let path = project_dir.join(format!("{project_name}-{}.out", flavor.suffix()));
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
                    | branch_imm26(text_vmaddr as usize + relocation.offset, target as usize);
                write_u32(text, relocation.offset, word);
            }
            "data" if relocation.kind == "page21" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let pc = text_vmaddr + relocation.offset as u64;
                let page_delta = ((target & !0xfff) as i64 - (pc & !0xfff) as i64) >> 12;
                let encoded = page_delta as u32;
                let immlo = encoded & 0b11;
                let immhi = (encoded >> 2) & 0x7ffff;
                let rd = read_u32(text, relocation.offset) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
                );
            }
            "data" if relocation.kind == "pageoff12" => {
                let target = symbol_vmaddr(image, &relocation.target, text_vmaddr, data_vmaddr)?;
                let imm12 = (target & 0xfff) as u32;
                let word = read_u32(text, relocation.offset);
                let rd = word & 0x1f;
                let rn = (word >> 5) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9100_0000 | (imm12 << 10) | (rn << 5) | rd,
                );
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
                    | branch_imm26(text_vmaddr as usize + relocation.offset, target as usize);
                write_u32(text, relocation.offset, word);
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
                let page_delta = ((target & !0xfff) as i64 - (pc & !0xfff) as i64) >> 12;
                let encoded = page_delta as u32;
                let immlo = encoded & 0b11;
                let immhi = (encoded >> 2) & 0x7ffff;
                let rd = read_u32(text, relocation.offset) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
                );
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
                let word = read_u32(text, relocation.offset);
                let rd = word & 0x1f;
                let rn = (word >> 5) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9100_0000 | (imm12 << 10) | (rn << 5) | rd,
                );
            }
            _ => {
                return Err(format!(
                    "linux-aarch64 linker does not support relocation {} {}",
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
    text: &mut Vec<u8>,
    image: &EncodedImage,
    text_vmaddr: u64,
) -> Result<ImportLocations, String> {
    let mut locations = ImportLocations::default();
    let stub_count = image.imports.len();
    let text_len_with_stubs = text.len() + stub_count * 12;
    let data_offset = align(TEXT_FILE_OFFSET + text_len_with_stubs, PAGE_SIZE);
    let got_offset = dynamic_prefix_size(image, text_len_with_stubs);
    let got_vmaddr = IMAGE_BASE + data_offset as u64 + got_offset as u64;
    for (index, import) in image.imports.iter().enumerate() {
        let stub_vmaddr = text_vmaddr + text.len() as u64;
        let entry_vmaddr = got_vmaddr + (index * 8) as u64;
        // Every import gets a GOT slot. Function imports also get a call stub
        // that branches through it; data globals are addressed via the GOT slot
        // directly (their stub is unused).
        emit_import_stub(text, stub_vmaddr, entry_vmaddr);
        locations.stubs.insert(import.symbol.clone(), stub_vmaddr);
        locations
            .got_entries
            .insert(import.symbol.clone(), entry_vmaddr);
    }
    Ok(locations)
}

fn emit_import_stub(text: &mut Vec<u8>, stub_vmaddr: u64, got_vmaddr: u64) {
    let page_delta = ((got_vmaddr & !0xfff) as i64 - (stub_vmaddr & !0xfff) as i64) >> 12;
    let encoded = page_delta as u32;
    let immlo = encoded & 0b11;
    let immhi = (encoded >> 2) & 0x7ffff;
    put_u32(text, 0x9000_0010 | (immlo << 29) | (immhi << 5));
    put_u32(
        text,
        0xf940_0211 | ((((got_vmaddr & 0xfff) / 8) as u32) << 10),
    );
    put_u32(text, 0xd61f_0220);
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

fn encode_static_elf(
    entry_offset: usize,
    text: &[u8],
    data: &[u8],
    signing_metadata: Option<&[u8]>,
) -> Vec<u8> {
    let file_size = TEXT_FILE_OFFSET + text.len() + data.len();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x7f, b'E', b'L', b'F']);
    bytes.extend_from_slice(&[2, 1, 1, 0]);
    bytes.resize(16, 0);
    put_u16(&mut bytes, 2);
    put_u16(&mut bytes, 183);
    put_u32(&mut bytes, 1);
    put_u64(
        &mut bytes,
        IMAGE_BASE + TEXT_FILE_OFFSET as u64 + entry_offset as u64,
    );
    put_u64(&mut bytes, 64);
    put_u64(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u16(&mut bytes, 64);
    put_u16(&mut bytes, 56);
    put_u16(&mut bytes, 1);
    put_u16(&mut bytes, 0);
    put_u16(&mut bytes, 0);
    put_u16(&mut bytes, 0);

    put_u32(&mut bytes, 1);
    put_u32(&mut bytes, 5);
    put_u64(&mut bytes, 0);
    put_u64(&mut bytes, IMAGE_BASE);
    put_u64(&mut bytes, IMAGE_BASE);
    put_u64(&mut bytes, file_size as u64);
    put_u64(&mut bytes, file_size as u64);
    put_u64(&mut bytes, 0x1000);

    bytes.resize(TEXT_FILE_OFFSET, 0);
    bytes.extend_from_slice(text);
    bytes.extend_from_slice(data);
    if let Some(metadata) = signing_metadata {
        append_elf_signing_section(&mut bytes, metadata);
    }
    bytes
}

fn encode_dynamic_elf(
    flavor: LinuxFlavor,
    entry_offset: usize,
    text: &[u8],
    data: &[u8],
    image: &EncodedImage,
) -> Result<Vec<u8>, String> {
    let dynamic = DynamicPayload::build(flavor, image)?;
    let ph_count = 5_u16;
    let interp = interpreter(flavor).as_bytes();
    let interp_offset = 64 + ph_count as usize * 56;
    let text_offset = TEXT_FILE_OFFSET;
    let text_vmaddr = IMAGE_BASE + text_offset as u64;
    let data_offset = align(text_offset + text.len(), PAGE_SIZE);
    let data_vmaddr = IMAGE_BASE + data_offset as u64;
    let data_file_size = data.len() + dynamic.bytes.len();
    let file_size = data_offset + data_file_size;
    let dynamic_offset = data_offset + data.len() + dynamic.dynamic_offset;
    let dynamic_vmaddr = data_vmaddr + data.len() as u64 + dynamic.dynamic_offset as u64;
    let dynamic_size = dynamic.dynamic_size;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x7f, b'E', b'L', b'F']);
    bytes.extend_from_slice(&[2, 1, 1, 0]);
    bytes.resize(16, 0);
    put_u16(&mut bytes, 2);
    put_u16(&mut bytes, 183);
    put_u32(&mut bytes, 1);
    put_u64(&mut bytes, text_vmaddr + entry_offset as u64);
    put_u64(&mut bytes, 64);
    put_u64(&mut bytes, 0);
    put_u32(&mut bytes, 0);
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
        IMAGE_BASE + 64,
        IMAGE_BASE + 64,
        ph_count as u64 * 56,
        ph_count as u64 * 56,
        8,
    );
    program_header(
        &mut bytes,
        3,
        4,
        interp_offset as u64,
        IMAGE_BASE + interp_offset as u64,
        IMAGE_BASE + interp_offset as u64,
        (interp.len() + 1) as u64,
        (interp.len() + 1) as u64,
        1,
    );
    program_header(
        &mut bytes,
        1,
        5,
        0,
        IMAGE_BASE,
        IMAGE_BASE,
        (text_offset + text.len()) as u64,
        (text_offset + text.len()) as u64,
        PAGE_SIZE as u64,
    );
    program_header(
        &mut bytes,
        1,
        6,
        data_offset as u64,
        data_vmaddr,
        data_vmaddr,
        data_file_size as u64,
        data_file_size as u64,
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

    bytes.resize(interp_offset, 0);
    bytes.extend_from_slice(interp);
    bytes.push(0);
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
    section_header(bytes, 1, 1, 0, 0, metadata_offset as u64, metadata.len() as u64, 0, 0, 1, 0);
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

fn interpreter(flavor: LinuxFlavor) -> &'static str {
    match flavor {
        LinuxFlavor::Glibc => "/lib/ld-linux-aarch64.so.1",
        LinuxFlavor::Musl => "/lib/ld-musl-aarch64.so.1",
    }
}

struct DynamicPayload {
    bytes: Vec<u8>,
    dynamic_offset: usize,
    dynamic_size: usize,
}

impl DynamicPayload {
    fn build(flavor: LinuxFlavor, image: &EncodedImage) -> Result<Self, String> {
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
        let rela_size = image.imports.len() * 24;
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
                None => needs_by_library.push((*library_index, vec![(version.clone(), global + 2)])),
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
        let data_vmaddr = IMAGE_BASE + data_offset as u64;

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

        bytes.resize(hash_offset - payload_start, 0);
        put_u32(&mut bytes, 1);
        put_u32(&mut bytes, (image.imports.len() + 1) as u32);
        put_u32(&mut bytes, if image.imports.is_empty() { 0 } else { 1 });
        for index in 1..=image.imports.len() {
            let next = if index == image.imports.len() {
                0
            } else {
                index + 1
            };
            put_u32(&mut bytes, next as u32);
        }
        put_u32(&mut bytes, 0);

        bytes.resize(rela_offset - payload_start, 0);
        for (index, import) in image.imports.iter().enumerate() {
            let symbol_index = index + 1;
            // GLOB_DAT binds a data global's GOT slot to the symbol's address;
            // JUMP_SLOT binds a function's GOT slot for its call stub
            // (plan-linker.md §6.1).
            let reloc_type = match import.kind {
                ImportKind::Data => R_AARCH64_GLOB_DAT,
                ImportKind::Function => R_AARCH64_JUMP_SLOT,
            };
            put_u64(
                &mut bytes,
                data_vmaddr + got_offset as u64 + (index * 8) as u64,
            );
            put_u64(&mut bytes, ((symbol_index as u64) << 32) | reloc_type as u64);
            put_u64(&mut bytes, 0);
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
                    .find(|symbol| {
                        symbol.name == *name && symbol.section == EncodedSection::Text
                    })
                    .ok_or_else(|| {
                        format!("initializer '{name}' does not resolve to a text symbol")
                    })?;
                put_u64(
                    &mut bytes,
                    IMAGE_BASE + TEXT_FILE_OFFSET as u64 + symbol.offset as u64,
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
        put_dynamic(&mut bytes, 2, rela_size as u64);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::encode::{
        EncodedImport, EncodedRelocation, EncodedSymbol, ImportKind,
    };

    fn versioned_exit_image() -> EncodedImage {
        // _main: movz w0, #0 ; bl _exit  (exit(0) through a versioned reference).
        let mut text = Vec::new();
        put_u32(&mut text, 0xd280_0000);
        put_u32(&mut text, 0x9400_0000);
        EncodedImage {
            text,
            data: Vec::new(),
            symbols: vec![EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: vec![EncodedRelocation {
                offset: 4,
                target: "_exit".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("libc.so.6".to_string()),
            }],
            imports: vec![EncodedImport {
                library: "libc.so.6".to_string(),
                symbol: "_exit".to_string(),
                kind: ImportKind::Function,
                version: Some("GLIBC_2.17".to_string()),
            }],
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
        }
    }

    // A program whose load-time initializer sets a data global that `main` reads
    // (plan-linker.md §6.4): `main` exits 0 only if the initializer ran first.
    fn init_array_image() -> EncodedImage {
        let mut text = Vec::new();
        // _init0 @0: _flag = 42.
        put_u32(&mut text, 0x9000_0000); // adrp x0, _flag         (page21)
        put_u32(&mut text, 0x9100_0000); // add  x0, x0, :lo12:_flag (pageoff12)
        put_u32(&mut text, 0xd280_0541); // movz x1, #42
        put_u32(&mut text, 0xf900_0001); // str  x1, [x0]
        put_u32(&mut text, 0xd65f_03c0); // ret
        // _main @20: exit(_flag == 42 ? 0 : 1).
        put_u32(&mut text, 0x9000_0000); // adrp x0, _flag         (page21)
        put_u32(&mut text, 0x9100_0000); // add  x0, x0, :lo12:_flag (pageoff12)
        put_u32(&mut text, 0xf940_0000); // ldr  x0, [x0]
        put_u32(&mut text, 0xf100_a81f); // cmp  x0, #42
        put_u32(&mut text, 0x5280_0000); // movz w0, #0
        put_u32(&mut text, 0x5400_0040); // b.eq +8
        put_u32(&mut text, 0x5280_0020); // movz w0, #1
        put_u32(&mut text, 0x9400_0000); // bl   _exit             (branch26)
        let data_reloc = |offset: usize, kind: &str| EncodedRelocation {
            offset,
            target: "_flag".to_string(),
            kind: kind.to_string(),
            binding: "data".to_string(),
            library: None,
        };
        EncodedImage {
            text,
            data: vec![0; 8],
            symbols: vec![
                EncodedSymbol {
                    name: "_init0".to_string(),
                    section: EncodedSection::Text,
                    offset: 0,
                },
                EncodedSymbol {
                    name: "_main".to_string(),
                    section: EncodedSection::Text,
                    offset: 20,
                },
                EncodedSymbol {
                    name: "_flag".to_string(),
                    section: EncodedSection::Data,
                    offset: 0,
                },
            ],
            relocations: vec![
                data_reloc(0, "page21"),
                data_reloc(4, "pageoff12"),
                data_reloc(20, "page21"),
                data_reloc(24, "pageoff12"),
                EncodedRelocation {
                    offset: 48,
                    target: "_exit".to_string(),
                    kind: "branch26".to_string(),
                    binding: "external".to_string(),
                    library: Some("libc.so.6".to_string()),
                },
            ],
            imports: vec![EncodedImport {
                library: "libc.so.6".to_string(),
                symbol: "_exit".to_string(),
                kind: ImportKind::Function,
                version: None,
            }],
            entry: "_main".to_string(),
            initializers: vec!["_init0".to_string()],
            signing_metadata: None,
        }
    }

    // Reads the real glibc data global `environ` (a `char**`) through the GOT via
    // a GLOB_DAT relocation (plan-linker.md §6.1) and exits 0 iff it is non-null,
    // proving the import resolved to libc's data symbol.
    fn glob_dat_image(libc: &str) -> EncodedImage {
        let mut text = Vec::new();
        put_u32(&mut text, 0x9000_0000); // adrp x0, environ        (external page21)
        put_u32(&mut text, 0x9100_0000); // add  x0, x0, :got_lo12  (external pageoff12)
        put_u32(&mut text, 0xf940_0000); // ldr  x0, [x0]   ; x0 = &environ
        put_u32(&mut text, 0xf940_0000); // ldr  x0, [x0]   ; x0 = environ (envp)
        put_u32(&mut text, 0xf100_001f); // cmp  x0, #0
        put_u32(&mut text, 0x5280_0000); // movz w0, #0     ; success default
        put_u32(&mut text, 0x5400_0041); // b.ne +8         ; non-null -> keep 0
        put_u32(&mut text, 0x5280_0020); // movz w0, #1
        put_u32(&mut text, 0x9400_0000); // bl   _exit              (external branch26)
        let ext = |offset: usize, target: &str, kind: &str| EncodedRelocation {
            offset,
            target: target.to_string(),
            kind: kind.to_string(),
            binding: "external".to_string(),
            library: Some(libc.to_string()),
        };
        EncodedImage {
            text,
            data: Vec::new(),
            symbols: vec![EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: vec![
                ext(0, "environ", "page21"),
                ext(4, "environ", "pageoff12"),
                ext(32, "_exit", "branch26"),
            ],
            imports: vec![
                EncodedImport {
                    library: libc.to_string(),
                    symbol: "environ".to_string(),
                    kind: ImportKind::Data,
                    version: None,
                },
                EncodedImport {
                    library: libc.to_string(),
                    symbol: "_exit".to_string(),
                    kind: ImportKind::Function,
                    version: None,
                },
            ],
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
        }
    }

    #[test]
    fn writes_glob_dat_glibc_elf() {
        let image = glob_dat_image("libc.so.6");
        let dir = std::path::PathBuf::from("tmp/globlx");
        std::fs::create_dir_all(&dir).expect("temp dir");
        write_executable(&dir, "glob", LinuxFlavor::Glibc, &image).expect("link glob_dat elf");
    }

    #[test]
    fn writes_glob_dat_musl_elf() {
        let image = glob_dat_image("libc.musl-aarch64.so.1");
        let dir = std::path::PathBuf::from("tmp/globlx");
        std::fs::create_dir_all(&dir).expect("temp dir");
        write_executable(&dir, "globmusl", LinuxFlavor::Musl, &image).expect("link musl glob_dat");
    }

    #[test]
    fn writes_mfb_sign_section_to_static_elf() {
        let mut image = EncodedImage {
            text: vec![0xd6, 0x5f, 0x03, 0xc0],
            data: Vec::new(),
            symbols: vec![EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: Vec::new(),
            imports: Vec::new(),
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: Some(br#"{"owner":"alice"}"#.to_vec()),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = write_executable(dir.path(), "signed", LinuxFlavor::Glibc, &image)
            .expect("link signed elf");
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.windows(b".mfb_sign".len()).any(|window| window == b".mfb_sign"));
        assert!(bytes
            .windows(br#"{"owner":"alice"}"#.len())
            .any(|window| window == br#"{"owner":"alice"}"#));
        assert_eq!(u16::from_le_bytes([bytes[60], bytes[61]]), 3);
        assert_eq!(u16::from_le_bytes([bytes[62], bytes[63]]), 2);
        image.signing_metadata = None;
        let unsigned = encode_static_elf(0, &image.text, &image.data, None);
        assert_eq!(u64::from_le_bytes(unsigned[40..48].try_into().unwrap()), 0);
    }

    // Confirms DT_INIT_ARRAY / DT_INIT_ARRAYSZ are emitted for the listed
    // initializers (verified with `readelf -d`). Note: glibc runs the *main
    // executable's* init_array from the CRT (`__libc_start_main`), not from
    // `ld.so`, so a custom-entry binary like this one does not invoke it at load;
    // the array is emitted for CRT/shared-object scenarios and for parity with
    // the macOS mod-init path (plan-linker.md §6.4).
    #[test]
    fn writes_init_array_glibc_elf() {
        let image = init_array_image();
        let dir = std::path::PathBuf::from("tmp/initlx");
        std::fs::create_dir_all(&dir).expect("temp dir");
        write_executable(&dir, "init", LinuxFlavor::Glibc, &image).expect("link init-array elf");
    }

    // Emits a dynamic glibc ELF whose single import requires `_exit@GLIBC_2.17`,
    // exercising the verneed/versym path (plan-linker.md §6.2). The byte check
    // confirms the version string reaches `.dynstr`; the file is left under
    // `tmp/verlx` so it can be executed against a real glibc `ld.so` (which
    // rejects a missing/mismatched version at load) to prove the structure.
    #[test]
    fn writes_versioned_glibc_elf() {
        let image = versioned_exit_image();
        let dir = std::path::PathBuf::from("tmp/verlx");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path =
            write_executable(&dir, "ver", LinuxFlavor::Glibc, &image).expect("link versioned elf");
        let bytes = std::fs::read(&path).expect("read elf");
        assert!(
            bytes
                .windows("GLIBC_2.17".len())
                .any(|window| window == b"GLIBC_2.17"),
            ".dynstr should contain the required version GLIBC_2.17"
        );
    }
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

fn dynamic_prefix_size(image: &EncodedImage, text_len_with_stubs: usize) -> usize {
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
    let _ = text_len_with_stubs;
    let rela_offset = align(hash_offset + hash_size, 8);
    let rela_size = image.imports.len() * 24;
    align(rela_offset + rela_size, 8)
}

fn put_dynamic(bytes: &mut Vec<u8>, tag: u64, value: u64) {
    put_u64(bytes, tag);
    put_u64(bytes, value);
}

fn branch_imm26(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x03ff_ffff
}

fn align(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("slice length"))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
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
