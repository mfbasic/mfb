use crate::arch::aarch64::encode::{EncodedImage, EncodedSection};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const VM_BASE: u64 = 0x1_0000_0000;
/// Byte size of one imported-symbol stub (adrp + ldr + br).
const IMPORT_STUB_SIZE: usize = 12;
const PAGE_SIZE: usize = 0x4000;

pub(crate) fn write_executable(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    let libraries = import_libraries(image)?;
    let has_imports = !libraries.is_empty();
    let code_offset = code_offset(&libraries);
    let mut text = image.text.clone();
    let import_locations = if has_imports {
        append_import_stubs(
            &mut text,
            image,
            VM_BASE + code_offset as u64,
            code_offset,
            image.data.len(),
        )?
    } else {
        ImportLocations::default()
    };
    patch_relocations(
        &mut text,
        image,
        VM_BASE + code_offset as u64,
        &import_locations,
    )?;
    let entry_offset = image
        .symbols
        .iter()
        .find(|symbol| symbol.name == image.entry)
        .filter(|symbol| symbol.section == EncodedSection::Text)
        .map(|symbol| symbol.offset)
        .ok_or_else(|| format!("entry symbol '{}' does not resolve to text", image.entry))?;
    let bytes = encode_mach_o(
        project_name,
        code_offset,
        entry_offset,
        &text,
        &image.data,
        &libraries,
        image,
    );
    let path = project_dir.join(format!("{project_name}.out"));
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
    import_locations: &ImportLocations,
) -> Result<(), String> {
    let data_vmaddr = text_vmaddr + text.len() as u64;
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
                        "macOS linker cannot bind external symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let word = 0x9400_0000
                    | branch_imm26(text_vmaddr as usize + relocation.offset, target as usize);
                write_u32(text, relocation.offset, word);
            }
            "external" if relocation.kind == "page21" => {
                let Some(&target) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "macOS linker cannot bind external data symbol '{}' from {}",
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
                        "macOS linker cannot bind external data symbol '{}' from {}",
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
                    "macOS linker does not support relocation {} {}",
                    relocation.binding, relocation.kind
                ));
            }
        }
    }
    Ok(())
}

/// Map a logical library name to its Mach-O dylib install path (plan-linker.md
/// §7.3). Frameworks resolve to their framework binary, plain dylibs to
/// `/usr/lib`. The `tls` driver's concrete entry is `Network.framework`.
fn dylib_path(library: &str) -> Result<String, String> {
    Ok(match library {
        "libSystem" => "/usr/lib/libSystem.B.dylib".to_string(),
        "Network" => "/System/Library/Frameworks/Network.framework/Network".to_string(),
        "AppKit" => "/System/Library/Frameworks/AppKit.framework/AppKit".to_string(),
        "Foundation" => "/System/Library/Frameworks/Foundation.framework/Foundation".to_string(),
        "libobjc" => "/usr/lib/libobjc.A.dylib".to_string(),
        "libz" => "/usr/lib/libz.1.dylib".to_string(),
        other => {
            return Err(format!(
                "macOS linker has no dylib path for library '{other}'"
            ));
        }
    })
}

/// The distinct dynamic libraries the image imports from, in first-seen order,
/// each paired with its install path and an implicit 1-based dylib ordinal
/// (its position + 1). Empty when the image imports nothing (plan-linker.md §7.1).
fn import_libraries(image: &EncodedImage) -> Result<Vec<(String, String)>, String> {
    let mut libraries: Vec<(String, String)> = Vec::new();
    for import in &image.imports {
        if !libraries.iter().any(|(name, _)| name == &import.library) {
            libraries.push((import.library.clone(), dylib_path(&import.library)?));
        }
    }
    Ok(libraries)
}

/// The 1-based dylib ordinal for a symbol's library within `libraries`.
fn library_ordinal(libraries: &[(String, String)], library: &str) -> Result<u64, String> {
    libraries
        .iter()
        .position(|(name, _)| name == library)
        .map(|index| index as u64 + 1)
        .ok_or_else(|| format!("macOS linker has no dylib ordinal for library '{library}'"))
}

#[derive(Default)]
struct ImportLocations {
    stubs: HashMap<String, u64>,
    got_entries: HashMap<String, u64>,
}

fn append_import_stubs(
    text: &mut Vec<u8>,
    image: &EncodedImage,
    text_vmaddr: u64,
    code_offset: usize,
    data_len: usize,
) -> Result<ImportLocations, String> {
    let mut locations = ImportLocations::default();
    // Each import appends a 3-instruction (12-byte) stub to the text section.
    // The GOT lives at `data_const_file_offset`, which is the page-aligned end of
    // the final code (stubs included) plus the constant data. Compute the layout
    // from that final code length, not the pre-stub length, so the GOT address
    // baked into every stub matches where the GOT is actually placed. Using the
    // pre-stub length makes the two diverge by a page whenever the stub bytes push
    // the total across a `PAGE_SIZE` boundary, which sends each stub's `br` to a
    // garbage address.
    let final_code_len = text.len() + image.imports.len() * IMPORT_STUB_SIZE;
    let layout = macho_layout(code_offset, final_code_len, data_len, true);
    for (index, import) in image.imports.iter().enumerate() {
        let stub_offset = text.len();
        let stub_vmaddr = text_vmaddr + stub_offset as u64;
        let got_vmaddr = VM_BASE + layout.data_const_file_offset as u64 + (index * 8) as u64;
        emit_import_stub(text, stub_vmaddr, got_vmaddr);
        locations.stubs.insert(import.symbol.clone(), stub_vmaddr);
        locations
            .got_entries
            .insert(import.symbol.clone(), got_vmaddr);
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
        0xf940_0210 | ((((got_vmaddr & 0xfff) / 8) as u32) << 10),
    );
    put_u32(text, 0xd61f_0200);
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

fn encode_mach_o(
    name: &str,
    code_offset: usize,
    entry_offset: usize,
    code: &[u8],
    data: &[u8],
    libraries: &[(String, String)],
    image: &EncodedImage,
) -> Vec<u8> {
    let unsigned = encode_unsigned_mach_o(
        code_offset,
        entry_offset,
        code,
        data,
        0,
        libraries,
        image,
    );
    let signature = code_signature(&unsigned, name);
    let unsigned = encode_unsigned_mach_o(
        code_offset,
        entry_offset,
        code,
        data,
        signature.len(),
        libraries,
        image,
    );
    let signature = code_signature(&unsigned, name);
    let mut bytes = unsigned;
    bytes.extend_from_slice(&signature);
    bytes
}

fn encode_unsigned_mach_o(
    code_offset: usize,
    entry_offset: usize,
    code: &[u8],
    data: &[u8],
    signature_size: usize,
    libraries: &[(String, String)],
    image: &EncodedImage,
) -> Vec<u8> {
    let has_imports = !libraries.is_empty();
    let layout = macho_layout(code_offset, code.len(), data.len(), has_imports);
    let linkedit = linkedit_layout(image, libraries, layout.linkedit_file_offset);
    let signature_offset = align(linkedit.data_in_code_offset, 16);
    let linkedit_file_size = signature_offset + signature_size - layout.linkedit_file_offset;
    let load_commands_size = load_commands_size(libraries);
    let mut bytes = Vec::new();

    put_u32(&mut bytes, 0xfeed_facf);
    put_u32(&mut bytes, 0x0100_000c);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 2);
    put_u32(&mut bytes, load_command_count(libraries));
    put_u32(&mut bytes, load_commands_size as u32);
    put_u32(&mut bytes, 0x0020_0085);
    put_u32(&mut bytes, 0);

    segment(&mut bytes, "__PAGEZERO", 0, VM_BASE, 0, 0, 0, 0, 0);
    text_segment(
        &mut bytes,
        code_offset,
        image.text.len(),
        code.len() - image.text.len(),
        layout.text_file_size,
    );
    if has_imports {
        data_const_segment(
            &mut bytes,
            layout.data_const_file_offset,
            image.imports.len(),
        );
    }
    segment(
        &mut bytes,
        "__LINKEDIT",
        VM_BASE + layout.linkedit_file_offset as u64,
        align(linkedit_file_size, PAGE_SIZE) as u64,
        layout.linkedit_file_offset as u64,
        linkedit_file_size as u64,
        1,
        1,
        0,
    );
    if has_imports {
        dyld_info(&mut bytes, &linkedit);
    } else {
        linkedit_data(
            &mut bytes,
            0x8000_0034,
            linkedit.fixups_offset,
            linkedit.fixups_size,
        );
        linkedit_data(&mut bytes, 0x8000_0033, linkedit.exports_offset, 0);
    }
    symtab(&mut bytes, &linkedit);
    dysymtab(&mut bytes, &linkedit);
    load_dylinker(&mut bytes);
    for (_, path) in libraries {
        load_dylib(&mut bytes, path);
    }
    uuid_command(&mut bytes);
    build_version(&mut bytes);
    source_version(&mut bytes);
    entry_point(&mut bytes, code_offset + entry_offset);
    linkedit_data(&mut bytes, 0x26, linkedit.function_starts_offset, 1);
    linkedit_data(&mut bytes, 0x29, linkedit.data_in_code_offset, 0);
    linkedit_data(&mut bytes, 0x1d, signature_offset, signature_size);

    bytes.resize(code_offset, 0);
    bytes.extend_from_slice(code);
    bytes.extend_from_slice(data);
    bytes.resize(layout.text_file_size, 0);
    if has_imports {
        bytes.resize(layout.data_const_file_offset, 0);
        bytes.resize(layout.data_const_file_offset + image.imports.len() * 8, 0);
        bytes.resize(layout.linkedit_file_offset, 0);
        bytes.extend_from_slice(&bind_info(image, libraries));
        bytes.resize(linkedit.symtab_offset, 0);
        bytes.extend_from_slice(&symbol_table(image));
        bytes.resize(linkedit.indirect_symbol_offset, 0);
        for index in 0..image.imports.len() {
            put_u32(&mut bytes, index as u32);
        }
        bytes.resize(linkedit.string_offset, 0);
        bytes.extend_from_slice(&string_table(image));
    } else {
        bytes.extend_from_slice(&empty_chained_fixups());
        bytes.push(0);
    }
    bytes.resize(signature_offset, 0);
    bytes
}

#[derive(Clone, Copy)]
struct MachOLayout {
    text_file_size: usize,
    data_const_file_offset: usize,
    linkedit_file_offset: usize,
}

fn macho_layout(
    code_offset: usize,
    code_len: usize,
    data_len: usize,
    imports_libsystem: bool,
) -> MachOLayout {
    let text_file_size = align(code_offset + code_len + data_len, PAGE_SIZE);
    let data_const_file_offset = text_file_size;
    let linkedit_file_offset = if imports_libsystem {
        data_const_file_offset + PAGE_SIZE
    } else {
        text_file_size
    };
    MachOLayout {
        text_file_size,
        data_const_file_offset,
        linkedit_file_offset,
    }
}

fn code_offset(libraries: &[(String, String)]) -> usize {
    align(32 + load_commands_size(libraries), 4)
}

fn load_commands_size(libraries: &[(String, String)]) -> usize {
    let base = 72 + 232 + 72 + 24 + 80 + dylinker_command_size() + 24 + 32 + 16 + 24 + 16 + 16 + 16;
    if libraries.is_empty() {
        base + 16 + 16
    } else {
        // __DATA_CONST segment + LC_DYLD_INFO_ONLY + one LC_LOAD_DYLIB per library.
        let dylibs: usize = libraries
            .iter()
            .map(|(_, path)| dylib_command_size(path))
            .sum();
        base + 152 + 48 + dylibs
    }
}

fn load_command_count(libraries: &[(String, String)]) -> u32 {
    if libraries.is_empty() {
        15
    } else {
        // Base imported-program commands (15 with one dylib) plus one extra
        // LC_LOAD_DYLIB per additional library.
        15 + libraries.len() as u32
    }
}

fn text_segment(
    bytes: &mut Vec<u8>,
    code_offset: usize,
    code_len: usize,
    stub_len: usize,
    text_file_size: usize,
) {
    put_u32(bytes, 0x19);
    put_u32(bytes, 232);
    fixed_name(bytes, "__TEXT");
    put_u64(bytes, VM_BASE);
    put_u64(bytes, text_file_size as u64);
    put_u64(bytes, 0);
    put_u64(bytes, text_file_size as u64);
    put_u32(bytes, 5);
    put_u32(bytes, 5);
    put_u32(bytes, 2);
    put_u32(bytes, 0);
    section(
        bytes,
        "__text",
        VM_BASE + code_offset as u64,
        code_len as u64,
        code_offset,
        0x80000400,
        0,
        0,
    );
    section(
        bytes,
        if stub_len == 0 {
            "__unwind_info"
        } else {
            "__stubs"
        },
        VM_BASE + (code_offset + code_len) as u64,
        stub_len as u64,
        code_offset + code_len,
        if stub_len == 0 { 0 } else { 0x80000408 },
        0,
        12,
    );
}

fn data_const_segment(bytes: &mut Vec<u8>, file_offset: usize, import_count: usize) {
    put_u32(bytes, 0x19);
    put_u32(bytes, 152);
    fixed_name(bytes, "__DATA_CONST");
    put_u64(bytes, VM_BASE + file_offset as u64);
    put_u64(bytes, PAGE_SIZE as u64);
    put_u64(bytes, file_offset as u64);
    put_u64(bytes, PAGE_SIZE as u64);
    put_u32(bytes, 3);
    put_u32(bytes, 3);
    put_u32(bytes, 1);
    put_u32(bytes, 0x10);
    section_with_segment(
        bytes,
        "__got",
        "__DATA_CONST",
        VM_BASE + file_offset as u64,
        (import_count * 8) as u64,
        file_offset,
        0x6,
        0,
        0,
        3,
    );
}

fn segment(
    bytes: &mut Vec<u8>,
    name: &str,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    maxprot: u32,
    initprot: u32,
    nsects: u32,
) {
    put_u32(bytes, 0x19);
    put_u32(bytes, 72);
    fixed_name(bytes, name);
    put_u64(bytes, vmaddr);
    put_u64(bytes, vmsize);
    put_u64(bytes, fileoff);
    put_u64(bytes, filesize);
    put_u32(bytes, maxprot);
    put_u32(bytes, initprot);
    put_u32(bytes, nsects);
    put_u32(bytes, 0);
}

fn section(
    bytes: &mut Vec<u8>,
    name: &str,
    addr: u64,
    size: u64,
    offset: usize,
    flags: u32,
    reserved1: u32,
    reserved2: u32,
) {
    section_with_segment(
        bytes, name, "__TEXT", addr, size, offset, flags, reserved1, reserved2, 2,
    );
}

fn section_with_segment(
    bytes: &mut Vec<u8>,
    name: &str,
    segment_name: &str,
    addr: u64,
    size: u64,
    offset: usize,
    flags: u32,
    reserved1: u32,
    reserved2: u32,
    align_power: u32,
) {
    fixed_name(bytes, name);
    fixed_name(bytes, segment_name);
    put_u64(bytes, addr);
    put_u64(bytes, size);
    put_u32(bytes, offset as u32);
    put_u32(bytes, align_power);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, flags);
    put_u32(bytes, reserved1);
    put_u32(bytes, reserved2);
    put_u32(bytes, 0);
}

fn linkedit_data(bytes: &mut Vec<u8>, command: u32, offset: usize, size: usize) {
    put_u32(bytes, command);
    put_u32(bytes, 16);
    put_u32(bytes, offset as u32);
    put_u32(bytes, size as u32);
}

fn symtab(bytes: &mut Vec<u8>, linkedit: &LinkeditLayout) {
    put_u32(bytes, 0x2);
    put_u32(bytes, 24);
    put_u32(bytes, linkedit.symtab_offset as u32);
    put_u32(bytes, linkedit.symbol_count as u32);
    put_u32(bytes, linkedit.string_offset as u32);
    put_u32(bytes, linkedit.string_size as u32);
}

fn dysymtab(bytes: &mut Vec<u8>, linkedit: &LinkeditLayout) {
    put_u32(bytes, 0xb);
    put_u32(bytes, 80);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, linkedit.symbol_count as u32);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, linkedit.indirect_symbol_offset as u32);
    put_u32(bytes, linkedit.indirect_symbol_count as u32);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
}

fn load_dylinker(bytes: &mut Vec<u8>) {
    let size = dylinker_command_size();
    put_u32(bytes, 0xe);
    put_u32(bytes, size as u32);
    put_u32(bytes, 12);
    bytes.extend_from_slice(b"/usr/lib/dyld\0");
    bytes.resize(align(bytes.len(), 8), 0);
}

fn dylinker_command_size() -> usize {
    align(12 + b"/usr/lib/dyld\0".len(), 8)
}

fn load_dylib(bytes: &mut Vec<u8>, name: &str) {
    let size = dylib_command_size(name);
    put_u32(bytes, 0xc);
    put_u32(bytes, size as u32);
    put_u32(bytes, 24);
    put_u32(bytes, 2);
    put_u32(bytes, 1356 << 16);
    put_u32(bytes, 1 << 16);
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);
    bytes.resize(align(bytes.len(), 8), 0);
}

fn dylib_command_size(name: &str) -> usize {
    align(24 + name.len() + 1, 8)
}

fn dyld_info(bytes: &mut Vec<u8>, linkedit: &LinkeditLayout) {
    put_u32(bytes, 0x8000_0022);
    put_u32(bytes, 48);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, linkedit.fixups_offset as u32);
    put_u32(bytes, linkedit.fixups_size as u32);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, linkedit.exports_offset as u32);
    put_u32(bytes, 0);
}

fn uuid_command(bytes: &mut Vec<u8>) {
    put_u32(bytes, 0x1b);
    put_u32(bytes, 24);
    bytes.extend_from_slice(&[0x4d, 0x46, 0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
}

fn build_version(bytes: &mut Vec<u8>) {
    put_u32(bytes, 0x32);
    put_u32(bytes, 32);
    put_u32(bytes, 1);
    put_u32(bytes, 11 << 16);
    put_u32(bytes, 0);
    put_u32(bytes, 1);
    put_u32(bytes, 3);
    put_u32(bytes, 0);
}

fn source_version(bytes: &mut Vec<u8>) {
    put_u32(bytes, 0x2a);
    put_u32(bytes, 16);
    put_u64(bytes, 0);
}

fn entry_point(bytes: &mut Vec<u8>, code_offset: usize) {
    put_u32(bytes, 0x8000_0028);
    put_u32(bytes, 24);
    put_u64(bytes, code_offset as u64);
    put_u64(bytes, 0);
}

struct LinkeditLayout {
    fixups_offset: usize,
    fixups_size: usize,
    exports_offset: usize,
    function_starts_offset: usize,
    data_in_code_offset: usize,
    symtab_offset: usize,
    indirect_symbol_offset: usize,
    string_offset: usize,
    string_size: usize,
    symbol_count: usize,
    indirect_symbol_count: usize,
}

fn linkedit_layout(
    image: &EncodedImage,
    libraries: &[(String, String)],
    linkedit_file_offset: usize,
) -> LinkeditLayout {
    let has_imports = !libraries.is_empty();
    let fixups_offset = linkedit_file_offset;
    let fixups_size = if has_imports {
        bind_info(image, libraries).len()
    } else {
        empty_chained_fixups().len()
    };
    let exports_offset = fixups_offset + fixups_size;
    let symtab_offset = exports_offset;
    let symbol_count = if has_imports {
        image.imports.len()
    } else {
        0
    };
    let indirect_symbol_offset = symtab_offset + symbol_count * 16;
    let indirect_symbol_count = if has_imports {
        image.imports.len()
    } else {
        0
    };
    let string_offset = indirect_symbol_offset + indirect_symbol_count * 4;
    let string_size = if has_imports {
        string_table(image).len()
    } else {
        0
    };
    let function_starts_offset = string_offset + string_size;
    let data_in_code_offset = function_starts_offset + 1;
    LinkeditLayout {
        fixups_offset,
        fixups_size,
        exports_offset,
        function_starts_offset,
        data_in_code_offset,
        symtab_offset,
        indirect_symbol_offset,
        string_offset,
        string_size,
        symbol_count,
        indirect_symbol_count,
    }
}

fn bind_info(image: &EncodedImage, libraries: &[(String, String)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, import) in image.imports.iter().enumerate() {
        let ordinal = library_ordinal(libraries, &import.library).unwrap_or(1);
        if ordinal <= 15 {
            // BIND_OPCODE_SET_DYLIB_ORDINAL_IMM (0x10) | ordinal.
            bytes.push(0x10 | ordinal as u8);
        } else {
            // BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB (0x80) + uleb ordinal.
            bytes.push(0x80);
            put_uleb128(&mut bytes, ordinal);
        }
        bytes.push(0x40);
        bytes.extend_from_slice(import.symbol.as_bytes());
        bytes.push(0);
        bytes.push(0x51);
        bytes.push(0x72);
        put_uleb128(&mut bytes, (index * 8) as u64);
        bytes.push(0x90);
    }
    bytes.push(0);
    bytes
}

fn symbol_table(image: &EncodedImage) -> Vec<u8> {
    let strings = string_offsets(image);
    let mut bytes = Vec::new();
    for import in &image.imports {
        put_u32(&mut bytes, strings[&import.symbol] as u32);
        bytes.push(0x1);
        bytes.push(0);
        put_u16(&mut bytes, 0);
        put_u64(&mut bytes, 0);
    }
    bytes
}

fn string_table(image: &EncodedImage) -> Vec<u8> {
    let mut bytes = vec![0];
    for import in &image.imports {
        bytes.extend_from_slice(import.symbol.as_bytes());
        bytes.push(0);
    }
    bytes
}

fn string_offsets(image: &EncodedImage) -> HashMap<String, usize> {
    let mut offsets = HashMap::new();
    let mut offset = 1;
    for import in &image.imports {
        offsets.insert(import.symbol.clone(), offset);
        offset += import.symbol.len() + 1;
    }
    offsets
}

fn empty_chained_fixups() -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 32);
    put_u32(&mut bytes, 48);
    put_u32(&mut bytes, 48);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 1);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 3);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    bytes
}

fn code_signature(unsigned: &[u8], name: &str) -> Vec<u8> {
    let page_size = 4096;
    let page_count = unsigned.len().div_ceil(page_size);
    let ident = format!("mfb.{name}");
    let ident = ident.as_bytes();
    let ident_offset = 88usize;
    let hash_offset = align(ident_offset + ident.len() + 1, 4);
    let code_directory_len = hash_offset + page_count * 32;
    let superblob_len = 20 + code_directory_len;
    let mut bytes = Vec::new();
    put_be_u32(&mut bytes, 0xfade_0cc0);
    put_be_u32(&mut bytes, superblob_len as u32);
    put_be_u32(&mut bytes, 1);
    put_be_u32(&mut bytes, 0);
    put_be_u32(&mut bytes, 20);
    put_be_u32(&mut bytes, 0xfade_0c02);
    put_be_u32(&mut bytes, code_directory_len as u32);
    put_be_u32(&mut bytes, 0x20400);
    put_be_u32(&mut bytes, 0x20002);
    put_be_u32(&mut bytes, hash_offset as u32);
    put_be_u32(&mut bytes, ident_offset as u32);
    put_be_u32(&mut bytes, 0);
    put_be_u32(&mut bytes, page_count as u32);
    put_be_u32(&mut bytes, unsigned.len() as u32);
    bytes.extend_from_slice(&[32, 2, 0, 12]);
    put_be_u32(&mut bytes, 0);
    put_be_u32(&mut bytes, 0);
    put_be_u32(&mut bytes, 0);
    put_be_u32(&mut bytes, 0);
    put_be_u64(&mut bytes, unsigned.len() as u64);
    put_be_u64(&mut bytes, 0);
    put_be_u64(&mut bytes, unsigned.len() as u64);
    put_be_u64(&mut bytes, 1);
    bytes.extend_from_slice(ident);
    bytes.push(0);
    bytes.resize(20 + hash_offset, 0);
    for page in unsigned.chunks(page_size) {
        bytes.extend_from_slice(&Sha256::digest(page));
    }
    bytes
}

fn fixed_name(bytes: &mut Vec<u8>, name: &str) {
    let mut buffer = [0u8; 16];
    let raw = name.as_bytes();
    buffer[..raw.len().min(16)].copy_from_slice(&raw[..raw.len().min(16)]);
    bytes.extend_from_slice(&buffer);
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn branch_imm26(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x03ff_ffff
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("slice length"))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_uleb128(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::encode::{EncodedImport, EncodedRelocation, EncodedSymbol};

    #[test]
    fn patches_external_data_relocations_to_got_entry() {
        let mut text = vec![
            0x00, 0x00, 0x00, 0x90, // adrp x0, symbol
            0x00, 0x00, 0x00, 0x91, // add x0, x0, pageoff(symbol)
        ];
        let image = EncodedImage {
            text: text.clone(),
            data: Vec::new(),
            symbols: vec![EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: vec![
                EncodedRelocation {
                    offset: 0,
                    target: "_mach_task_self_".to_string(),
                    kind: "page21".to_string(),
                    binding: "external".to_string(),
                    library: Some("libSystem".to_string()),
                },
                EncodedRelocation {
                    offset: 4,
                    target: "_mach_task_self_".to_string(),
                    kind: "pageoff12".to_string(),
                    binding: "external".to_string(),
                    library: Some("libSystem".to_string()),
                },
            ],
            imports: vec![EncodedImport {
                library: "libSystem".to_string(),
                symbol: "_mach_task_self_".to_string(),
            }],
            entry: "_main".to_string(),
        };
        let text_vmaddr = VM_BASE + 0x4000;
        let locations =
            append_import_stubs(&mut text, &image, text_vmaddr, 0x4000, 0).expect("import stubs");

        patch_relocations(&mut text, &image, text_vmaddr, &locations).expect("relocations");

        assert!(locations.got_entries.contains_key("_mach_task_self_"));
    }

    #[test]
    fn import_libraries_assigns_one_ordinal_per_distinct_library() {
        let image = EncodedImage {
            text: Vec::new(),
            data: Vec::new(),
            symbols: Vec::new(),
            relocations: Vec::new(),
            imports: vec![
                EncodedImport {
                    library: "libSystem".to_string(),
                    symbol: "_exit".to_string(),
                },
                EncodedImport {
                    library: "Network".to_string(),
                    symbol: "_nw_path_monitor_create".to_string(),
                },
                EncodedImport {
                    library: "libSystem".to_string(),
                    symbol: "_write".to_string(),
                },
            ],
            entry: "_main".to_string(),
        };
        let libraries = import_libraries(&image).expect("libraries");
        assert_eq!(libraries.len(), 2);
        assert_eq!(library_ordinal(&libraries, "libSystem").unwrap(), 1);
        assert_eq!(library_ordinal(&libraries, "Network").unwrap(), 2);
        // The bind stream tags _nw_path_monitor_create with dylib ordinal 2.
        let bind = bind_info(&image, &libraries);
        assert!(bind.contains(&0x12)); // SET_DYLIB_ORDINAL_IMM(2)
    }

    // Drives the multi-library Mach-O path (plan-linker.md §7) end to end against
    // the real `tls` driver library, Network.framework: a hand-built program that
    // imports a symbol from Network (ordinal 2) and `exit` from libSystem
    // (ordinal 1), then links and executes. A wrong dylib ordinal or a missing
    // LC_LOAD_DYLIB makes dyld fail to bind at launch, so a clean exit proves the
    // generalization.
    #[cfg(target_os = "macos")]
    #[test]
    fn links_and_runs_program_importing_from_two_dylibs() {
        // _main: x0 = nw_path_monitor_create(); exit(x0 != null ? 0 : 7).
        let words: [u32; 6] = [
            0x9400_0000, // bl _nw_path_monitor_create  (external branch26, patched)
            0xB400_0060, // cbz x0, fail (+12)
            0xD280_0000, // movz x0, #0
            0x1400_0002, // b done (+8)
            0xD280_00E0, // fail: movz x0, #7
            0x9400_0000, // done: bl _exit              (external branch26, patched)
        ];
        let mut text = Vec::new();
        for word in words {
            put_u32(&mut text, word);
        }
        let image = EncodedImage {
            text,
            data: Vec::new(),
            symbols: vec![EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: vec![
                EncodedRelocation {
                    offset: 0,
                    target: "_nw_path_monitor_create".to_string(),
                    kind: "branch26".to_string(),
                    binding: "external".to_string(),
                    library: Some("Network".to_string()),
                },
                EncodedRelocation {
                    offset: 20,
                    target: "_exit".to_string(),
                    kind: "branch26".to_string(),
                    binding: "external".to_string(),
                    library: Some("libSystem".to_string()),
                },
            ],
            imports: vec![
                EncodedImport {
                    library: "libSystem".to_string(),
                    symbol: "_exit".to_string(),
                },
                EncodedImport {
                    library: "Network".to_string(),
                    symbol: "_nw_path_monitor_create".to_string(),
                },
            ],
            entry: "_main".to_string(),
        };

        let dir = std::env::temp_dir().join(format!("mfb_nwlink_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = write_executable(&dir, "nwlink", &image).expect("link multi-dylib executable");
        let status = std::process::Command::new(&path)
            .status()
            .expect("run multi-dylib executable");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            status.code(),
            Some(0),
            "program importing from libSystem + Network.framework should exit 0"
        );
    }
}

fn put_be_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn put_be_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}
