use crate::arch::aarch64;
use crate::bytecode;
use crate::ir::IrProject;
use crate::target::BuildTarget;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const VM_BASE: u64 = 0x1_0000_0000;
const TEXT_FILE_SIZE: usize = 0x4000;
const LINKEDIT_FILE_OFFSET: usize = 0x4000;

pub fn write_executable(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
) -> Result<PathBuf, String> {
    if target.os != "macos" || target.arch != "aarch64" {
        return Err(format!(
            "native executable output only supports macOS aarch64 for now, got {} {}",
            target.os, target.arch
        ));
    }

    let program = bytecode::native_program(ir)?;
    let code_offset = code_offset();
    let image = aarch64::encode(&program, VM_BASE + code_offset as u64)?;
    let bytes = encode_mach_o(&ir.name, code_offset, &image.code, &image.data);
    let path = project_dir.join(format!("{}.out", ir.name));

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

fn encode_mach_o(name: &str, code_offset: usize, code: &[u8], data: &[u8]) -> Vec<u8> {
    let unsigned = encode_unsigned_mach_o(code_offset, code, data, 0);
    let signature = code_signature(&unsigned, name);
    let unsigned = encode_unsigned_mach_o(code_offset, code, data, signature.len());
    let signature = code_signature(&unsigned, name);
    let mut bytes = unsigned;
    bytes.extend_from_slice(&signature);
    bytes
}

fn encode_unsigned_mach_o(
    code_offset: usize,
    code: &[u8],
    data: &[u8],
    signature_size: usize,
) -> Vec<u8> {
    let linkedit = linkedit_layout();
    let signature_offset = align(linkedit.data_in_code_offset, 16);
    let linkedit_file_size = signature_offset + signature_size - LINKEDIT_FILE_OFFSET;
    let load_commands_size = load_commands_size();
    let mut bytes = Vec::new();

    put_u32(&mut bytes, 0xfeed_facf);
    put_u32(&mut bytes, 0x0100_000c);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 2);
    put_u32(&mut bytes, 15);
    put_u32(&mut bytes, load_commands_size as u32);
    put_u32(&mut bytes, 0x0020_0085);
    put_u32(&mut bytes, 0);

    segment(&mut bytes, "__PAGEZERO", 0, VM_BASE, 0, 0, 0, 0, 0);
    text_segment(&mut bytes, code_offset, code.len(), code.len() + data.len());
    segment(
        &mut bytes,
        "__LINKEDIT",
        VM_BASE + TEXT_FILE_SIZE as u64,
        0x4000,
        LINKEDIT_FILE_OFFSET as u64,
        linkedit_file_size as u64,
        1,
        1,
        0,
    );

    linkedit_data(
        &mut bytes,
        0x8000_0034,
        linkedit.fixups_offset,
        linkedit.fixups_size,
    );
    linkedit_data(&mut bytes, 0x8000_0033, linkedit.exports_offset, 0);
    symtab(&mut bytes, linkedit.symtab_offset, linkedit.string_offset);
    dysymtab(&mut bytes);
    load_dylinker(&mut bytes);
    uuid_command(&mut bytes);
    build_version(&mut bytes);
    source_version(&mut bytes);
    entry_point(&mut bytes, code_offset);
    linkedit_data(&mut bytes, 0x26, linkedit.function_starts_offset, 1);
    linkedit_data(&mut bytes, 0x29, linkedit.data_in_code_offset, 0);
    linkedit_data(&mut bytes, 0x1d, signature_offset, signature_size);

    debug_assert_eq!(bytes.len(), code_offset);
    bytes.extend_from_slice(code);
    bytes.extend_from_slice(data);
    bytes.resize(TEXT_FILE_SIZE, 0);
    bytes.extend_from_slice(&empty_chained_fixups());
    bytes.push(0);
    bytes.resize(signature_offset, 0);
    bytes
}

fn code_offset() -> usize {
    align(32 + load_commands_size(), 4)
}

fn load_commands_size() -> usize {
    72 + 232 + 72 + 16 + 16 + 24 + 80 + dylinker_command_size() + 24 + 32 + 16 + 24 + 16 + 16 + 16
}

fn text_segment(bytes: &mut Vec<u8>, code_offset: usize, code_len: usize, text_len: usize) {
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
    );
    section(
        bytes,
        "__unwind_info",
        VM_BASE + (code_offset + text_len) as u64,
        0,
        code_offset + text_len,
        0,
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

fn section(bytes: &mut Vec<u8>, name: &str, addr: u64, size: u64, offset: usize, flags: u32) {
    fixed_name(bytes, name);
    fixed_name(bytes, "__TEXT");
    put_u64(bytes, addr);
    put_u64(bytes, size);
    put_u32(bytes, offset as u32);
    put_u32(bytes, 2);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, flags);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
}

fn linkedit_data(bytes: &mut Vec<u8>, command: u32, offset: usize, size: usize) {
    put_u32(bytes, command);
    put_u32(bytes, 16);
    put_u32(bytes, offset as u32);
    put_u32(bytes, size as u32);
}

fn symtab(bytes: &mut Vec<u8>, symtab_offset: usize, string_offset: usize) {
    put_u32(bytes, 0x2);
    put_u32(bytes, 24);
    put_u32(bytes, symtab_offset as u32);
    put_u32(bytes, 0);
    put_u32(bytes, string_offset as u32);
    put_u32(bytes, 0);
}

fn dysymtab(bytes: &mut Vec<u8>) {
    put_u32(bytes, 0xb);
    put_u32(bytes, 80);
    bytes.resize(bytes.len() + 72, 0);
}

fn load_dylinker(bytes: &mut Vec<u8>) {
    let path = b"/usr/lib/dyld\0";
    let size = dylinker_command_size();
    put_u32(bytes, 0xe);
    put_u32(bytes, size as u32);
    put_u32(bytes, 12);
    bytes.extend_from_slice(path);
    bytes.resize(align(bytes.len(), 8), 0);
}

fn dylinker_command_size() -> usize {
    align(12 + b"/usr/lib/dyld\0".len(), 8)
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
    string_offset: usize,
}

fn linkedit_layout() -> LinkeditLayout {
    let fixups_offset = LINKEDIT_FILE_OFFSET;
    let fixups_size = empty_chained_fixups().len();
    let exports_offset = fixups_offset + fixups_size;
    let function_starts_offset = exports_offset;
    let data_in_code_offset = function_starts_offset + 1;
    let symtab_offset = data_in_code_offset;
    let string_offset = symtab_offset;
    LinkeditLayout {
        fixups_offset,
        fixups_size,
        exports_offset,
        function_starts_offset,
        data_in_code_offset,
        symtab_offset,
        string_offset,
    }
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
    let start = bytes.len();
    bytes.extend_from_slice(name.as_bytes());
    bytes.resize(start + 16, 0);
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_be_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn put_be_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}
