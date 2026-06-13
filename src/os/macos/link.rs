use crate::arch::aarch64::encode::{EncodedImage, EncodedSection};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const VM_BASE: u64 = 0x1_0000_0000;
const TEXT_FILE_SIZE: usize = 0x4000;
const DATA_CONST_FILE_OFFSET: usize = 0x4000;
const LINKEDIT_FILE_OFFSET: usize = 0x8000;

pub(crate) fn write_executable(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    let imports_libsystem = imports_libsystem(image)?;
    let code_offset = code_offset(imports_libsystem);
    let mut text = image.text.clone();
    let import_stubs = if imports_libsystem {
        append_import_stubs(&mut text, image, VM_BASE + code_offset as u64)?
    } else {
        HashMap::new()
    };
    patch_relocations(
        &mut text,
        image,
        VM_BASE + code_offset as u64,
        &import_stubs,
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
        imports_libsystem,
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
    import_stubs: &HashMap<String, u64>,
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
            "external" => {
                let Some(&target) = import_stubs.get(&relocation.target) else {
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

fn imports_libsystem(image: &EncodedImage) -> Result<bool, String> {
    for import in &image.imports {
        if import.library != "libSystem" {
            return Err(format!(
                "macOS linker cannot load '{}' for imported symbol '{}'",
                import.library, import.symbol
            ));
        }
    }
    Ok(!image.imports.is_empty())
}

fn append_import_stubs(
    text: &mut Vec<u8>,
    image: &EncodedImage,
    text_vmaddr: u64,
) -> Result<HashMap<String, u64>, String> {
    let mut stubs = HashMap::new();
    for (index, import) in image.imports.iter().enumerate() {
        let stub_offset = text.len();
        let stub_vmaddr = text_vmaddr + stub_offset as u64;
        let got_vmaddr = VM_BASE + DATA_CONST_FILE_OFFSET as u64 + (index * 8) as u64;
        emit_import_stub(text, stub_vmaddr, got_vmaddr);
        stubs.insert(import.symbol.clone(), stub_vmaddr);
    }
    Ok(stubs)
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
    imports_libsystem: bool,
    image: &EncodedImage,
) -> Vec<u8> {
    let unsigned = encode_unsigned_mach_o(
        code_offset,
        entry_offset,
        code,
        data,
        0,
        imports_libsystem,
        image,
    );
    let signature = code_signature(&unsigned, name);
    let unsigned = encode_unsigned_mach_o(
        code_offset,
        entry_offset,
        code,
        data,
        signature.len(),
        imports_libsystem,
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
    imports_libsystem: bool,
    image: &EncodedImage,
) -> Vec<u8> {
    let linkedit = linkedit_layout(image, imports_libsystem);
    let signature_offset = align(linkedit.data_in_code_offset, 16);
    let linkedit_file_size = signature_offset + signature_size - LINKEDIT_FILE_OFFSET;
    let load_commands_size = load_commands_size(imports_libsystem);
    let mut bytes = Vec::new();

    put_u32(&mut bytes, 0xfeed_facf);
    put_u32(&mut bytes, 0x0100_000c);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 2);
    put_u32(&mut bytes, load_command_count(imports_libsystem));
    put_u32(&mut bytes, load_commands_size as u32);
    put_u32(&mut bytes, 0x0020_0085);
    put_u32(&mut bytes, 0);

    segment(&mut bytes, "__PAGEZERO", 0, VM_BASE, 0, 0, 0, 0, 0);
    text_segment(
        &mut bytes,
        code_offset,
        image.text.len(),
        code.len() - image.text.len(),
        code.len() + data.len(),
    );
    if imports_libsystem {
        data_const_segment(&mut bytes, image.imports.len());
    }
    segment(
        &mut bytes,
        "__LINKEDIT",
        VM_BASE + LINKEDIT_FILE_OFFSET as u64,
        0x4000,
        LINKEDIT_FILE_OFFSET as u64,
        linkedit_file_size as u64,
        1,
        1,
        0,
    );
    if imports_libsystem {
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
    if imports_libsystem {
        load_dylib(&mut bytes, "/usr/lib/libSystem.B.dylib");
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
    bytes.resize(TEXT_FILE_SIZE, 0);
    if imports_libsystem {
        bytes.resize(DATA_CONST_FILE_OFFSET, 0);
        bytes.resize(DATA_CONST_FILE_OFFSET + image.imports.len() * 8, 0);
        bytes.resize(LINKEDIT_FILE_OFFSET, 0);
        bytes.extend_from_slice(&bind_info(image));
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

fn code_offset(imports_libsystem: bool) -> usize {
    align(32 + load_commands_size(imports_libsystem), 4)
}

fn load_commands_size(_imports_libsystem: bool) -> usize {
    let base = 72 + 232 + 72 + 24 + 80 + dylinker_command_size() + 24 + 32 + 16 + 24 + 16 + 16 + 16;
    if _imports_libsystem {
        base + 152 + 48 + dylib_command_size("/usr/lib/libSystem.B.dylib")
    } else {
        base + 16 + 16
    }
}

fn load_command_count(_imports_libsystem: bool) -> u32 {
    if _imports_libsystem {
        16
    } else {
        15
    }
}

fn text_segment(
    bytes: &mut Vec<u8>,
    code_offset: usize,
    code_len: usize,
    stub_len: usize,
    _text_len: usize,
) {
    put_u32(bytes, 0x19);
    put_u32(bytes, 232);
    fixed_name(bytes, "__TEXT");
    put_u64(bytes, VM_BASE);
    put_u64(bytes, TEXT_FILE_SIZE as u64);
    put_u64(bytes, 0);
    put_u64(bytes, TEXT_FILE_SIZE as u64);
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

fn data_const_segment(bytes: &mut Vec<u8>, import_count: usize) {
    put_u32(bytes, 0x19);
    put_u32(bytes, 152);
    fixed_name(bytes, "__DATA_CONST");
    put_u64(bytes, VM_BASE + DATA_CONST_FILE_OFFSET as u64);
    put_u64(bytes, 0x4000);
    put_u64(bytes, DATA_CONST_FILE_OFFSET as u64);
    put_u64(bytes, 0x4000);
    put_u32(bytes, 3);
    put_u32(bytes, 3);
    put_u32(bytes, 1);
    put_u32(bytes, 0x10);
    section_with_segment(
        bytes,
        "__got",
        "__DATA_CONST",
        VM_BASE + DATA_CONST_FILE_OFFSET as u64,
        (import_count * 8) as u64,
        DATA_CONST_FILE_OFFSET,
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

fn linkedit_layout(image: &EncodedImage, imports_libsystem: bool) -> LinkeditLayout {
    let fixups_offset = LINKEDIT_FILE_OFFSET;
    let fixups_size = if imports_libsystem {
        bind_info(image).len()
    } else {
        empty_chained_fixups().len()
    };
    let exports_offset = fixups_offset + fixups_size;
    let symtab_offset = exports_offset;
    let symbol_count = if imports_libsystem {
        image.imports.len()
    } else {
        0
    };
    let indirect_symbol_offset = symtab_offset + symbol_count * 16;
    let indirect_symbol_count = if imports_libsystem {
        image.imports.len()
    } else {
        0
    };
    let string_offset = indirect_symbol_offset + indirect_symbol_count * 4;
    let string_size = if imports_libsystem {
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

fn bind_info(image: &EncodedImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, import) in image.imports.iter().enumerate() {
        bytes.push(0x11);
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

fn put_be_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn put_be_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}
