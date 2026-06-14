use crate::arch::aarch64::encode::{EncodedImage, EncodedSection};
use crate::os::linux::flavor::LinuxFlavor;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const IMAGE_BASE: u64 = 0x400000;
const TEXT_FILE_OFFSET: usize = 0x1000;
const PAGE_SIZE: usize = 0x1000;
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
        encode_static_elf(entry_offset, &text, &image.data)
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
        emit_import_stub(text, stub_vmaddr, entry_vmaddr);
        locations.stubs.insert(import.symbol.clone(), stub_vmaddr);
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

fn encode_static_elf(entry_offset: usize, text: &[u8], data: &[u8]) -> Vec<u8> {
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
    Ok(bytes)
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

        let dynstr_offset = data_base_offset;
        let dynsym_offset = align(dynstr_offset + dynstr.len(), 8);
        let dynsym_size = (image.imports.len() + 1) * 24;
        let hash_offset = align(dynsym_offset + dynsym_size, 8);
        let hash_size = (2 + 1 + image.imports.len() + 1) * 4;
        let rela_offset = align(hash_offset + hash_size, 8);
        let rela_size = image.imports.len() * 24;
        let got_offset = align(rela_offset + rela_size, 8);
        let got_size = image.imports.len() * 8;
        let dynamic_offset = align(got_offset + got_size, 8);
        let dynamic_count = libraries.len() + 14;
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
            bytes.push(0x12);
            bytes.push(0);
            put_u16(&mut bytes, 0);
            put_u64(&mut bytes, 0);
            put_u64(&mut bytes, 0);
            let _ = import;
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
        for index in 0..image.imports.len() {
            let symbol_index = index + 1;
            put_u64(
                &mut bytes,
                data_vmaddr + got_offset as u64 + (index * 8) as u64,
            );
            put_u64(
                &mut bytes,
                ((symbol_index as u64) << 32) | R_AARCH64_JUMP_SLOT as u64,
            );
            put_u64(&mut bytes, 0);
        }

        bytes.resize(got_offset - payload_start, 0);
        bytes.resize(bytes.len() + got_size, 0);

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

fn dynamic_prefix_size(image: &EncodedImage, text_len_with_stubs: usize) -> usize {
    let mut libraries = Vec::<&str>::new();
    for import in &image.imports {
        if !libraries.contains(&import.library.as_str()) {
            libraries.push(import.library.as_str());
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
