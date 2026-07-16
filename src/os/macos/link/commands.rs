use super::*;

pub(super) fn text_segment(
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

pub(super) fn data_const_segment(
    bytes: &mut Vec<u8>,
    file_offset: usize,
    data_const_size: usize,
    import_count: usize,
    init_count: usize,
    rodata_offset: usize,
    rodata_len: usize,
) {
    let sections = data_const_section_count(import_count, init_count, rodata_len > 0);
    put_u32(bytes, 0x19);
    put_u32(bytes, 72 + sections * 80);
    fixed_name(bytes, "__DATA_CONST");
    put_u64(bytes, VM_BASE + file_offset as u64);
    put_u64(bytes, data_const_size as u64);
    put_u64(bytes, file_offset as u64);
    put_u64(bytes, data_const_size as u64);
    put_u32(bytes, 3);
    put_u32(bytes, 3);
    put_u32(bytes, sections);
    put_u32(bytes, 0x10);
    if import_count > 0 {
        // __got: S_NON_LAZY_SYMBOL_POINTERS, one slot per import.
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
    if init_count > 0 {
        // __mod_init_func: S_MOD_INIT_FUNC_POINTERS (0x9), placed past the GOT.
        let mod_offset = file_offset + import_count * 8;
        section_with_segment(
            bytes,
            "__mod_init_func",
            "__DATA_CONST",
            VM_BASE + mod_offset as u64,
            (init_count * 8) as u64,
            mod_offset,
            0x9,
            0,
            0,
            3,
        );
    }
    if rodata_len > 0 {
        // __const: read-only program constants (string literals, error messages),
        // placed past the GOT/init pointers so `__DATA_CONST`'s SG_READ_ONLY flag
        // maps them read-only once dyld finishes fixups (bug-187). 16-byte aligned
        // (align_power 4) to match the maximum data-object alignment.
        let const_offset = file_offset + rodata_offset;
        section_with_segment(
            bytes,
            "__const",
            "__DATA_CONST",
            VM_BASE + const_offset as u64,
            rodata_len as u64,
            const_offset,
            0,
            0,
            0,
            4,
        );
    }
}

/// Writable `__DATA` segment with a single `__data` section. Holds the program's
/// constant data and the main-arena global; initprot/maxprot are RW so the global
/// can be stored to at runtime. Emitted after `__DATA_CONST` to preserve that
/// segment's index (`rebase_info` hardcodes it).
pub(super) fn data_segment(
    bytes: &mut Vec<u8>,
    file_offset: usize,
    seg_size: usize,
    data_len: usize,
) {
    put_u32(bytes, 0x19);
    put_u32(bytes, 72 + 80);
    fixed_name(bytes, "__DATA");
    put_u64(bytes, VM_BASE + file_offset as u64);
    put_u64(bytes, seg_size as u64);
    put_u64(bytes, file_offset as u64);
    put_u64(bytes, seg_size as u64);
    put_u32(bytes, 3);
    put_u32(bytes, 3);
    put_u32(bytes, 1);
    put_u32(bytes, 0);
    section_with_segment(
        bytes,
        "__data",
        "__DATA",
        VM_BASE + file_offset as u64,
        data_len as u64,
        file_offset,
        0,
        0,
        0,
        3,
    );
}

pub(super) fn mfb_sign_segment(bytes: &mut Vec<u8>, file_offset: usize, metadata_len: usize) {
    put_u32(bytes, 0x19);
    put_u32(bytes, 152);
    fixed_name(bytes, "__MFB");
    put_u64(bytes, VM_BASE + file_offset as u64);
    put_u64(bytes, align(metadata_len, PAGE_SIZE) as u64);
    put_u64(bytes, file_offset as u64);
    put_u64(bytes, metadata_len as u64);
    put_u32(bytes, 1);
    put_u32(bytes, 1);
    put_u32(bytes, 1);
    put_u32(bytes, 0);
    section_with_segment(
        bytes,
        "__sign",
        "__MFB",
        VM_BASE + file_offset as u64,
        metadata_len as u64,
        file_offset,
        0,
        0,
        0,
        0,
    );
}

pub(super) fn segment(
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

pub(super) fn section(
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

/// Narrow a `usize` Mach-O file offset / count / size to the u32 field the load
/// command stores, panicking with a clear message instead of silently truncating
/// a ≥4 GiB value into a wrong offset (bug-88 / bug-168). Reachable only for a
/// >4 GiB output image, which the linker does not otherwise support; a wrapped
/// offset would produce a structurally invalid, unloadable executable.
fn u32_field(what: &str, value: usize) -> u32 {
    u32::try_from(value).unwrap_or_else(|_| panic!("mach-o: {what} {value} exceeds u32"))
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
    put_u32(bytes, u32_field("section file offset", offset));
    put_u32(bytes, align_power);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, flags);
    put_u32(bytes, reserved1);
    put_u32(bytes, reserved2);
    put_u32(bytes, 0);
}

pub(super) fn linkedit_data(bytes: &mut Vec<u8>, command: u32, offset: usize, size: usize) {
    put_u32(bytes, command);
    put_u32(bytes, 16);
    put_u32(bytes, u32_field("linkedit_data offset", offset));
    put_u32(bytes, u32_field("linkedit_data size", size));
}

pub(super) fn symtab(bytes: &mut Vec<u8>, linkedit: &LinkeditLayout) {
    put_u32(bytes, 0x2);
    put_u32(bytes, 24);
    put_u32(bytes, u32_field("symtab offset", linkedit.symtab_offset));
    put_u32(bytes, u32_field("symtab symbol count", linkedit.symbol_count));
    put_u32(bytes, u32_field("symtab string offset", linkedit.string_offset));
    put_u32(bytes, u32_field("symtab string size", linkedit.string_size));
}

pub(super) fn dysymtab(bytes: &mut Vec<u8>, linkedit: &LinkeditLayout) {
    put_u32(bytes, 0xb);
    put_u32(bytes, 80);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, u32_field("dysymtab nundefsym", linkedit.symbol_count));
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, u32_field("dysymtab indirectsymoff", linkedit.indirect_symbol_offset));
    put_u32(bytes, u32_field("dysymtab nindirectsyms", linkedit.indirect_symbol_count));
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
}

pub(super) fn load_dylinker(bytes: &mut Vec<u8>) {
    let size = dylinker_command_size();
    put_u32(bytes, 0xe);
    put_u32(bytes, size as u32);
    put_u32(bytes, 12);
    bytes.extend_from_slice(b"/usr/lib/dyld\0");
    bytes.resize(align(bytes.len(), 8), 0);
}

pub(super) fn dylinker_command_size() -> usize {
    align(12 + b"/usr/lib/dyld\0".len(), 8)
}

pub(super) fn load_dylib(bytes: &mut Vec<u8>, name: &str) {
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

pub(super) fn dylib_command_size(name: &str) -> usize {
    align(24 + name.len() + 1, 8)
}

pub(super) fn dyld_info(bytes: &mut Vec<u8>, linkedit: &LinkeditLayout) {
    put_u32(bytes, 0x8000_0022);
    put_u32(bytes, 48);
    put_u32(bytes, u32_field("dyld_info rebase offset", linkedit.rebase_offset));
    put_u32(bytes, u32_field("dyld_info rebase size", linkedit.rebase_size));
    put_u32(bytes, u32_field("dyld_info fixups offset", linkedit.fixups_offset));
    put_u32(bytes, u32_field("dyld_info fixups size", linkedit.fixups_size));
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, 0);
    put_u32(bytes, u32_field("dyld_info exports offset", linkedit.exports_offset));
    put_u32(bytes, 0);
}

pub(super) fn uuid_command(bytes: &mut Vec<u8>) {
    put_u32(bytes, 0x1b);
    put_u32(bytes, 24);
    bytes.extend_from_slice(&[0x4d, 0x46, 0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
}

pub(super) fn build_version(bytes: &mut Vec<u8>) {
    put_u32(bytes, 0x32);
    put_u32(bytes, 32);
    put_u32(bytes, 1);
    put_u32(bytes, 11 << 16);
    put_u32(bytes, 0);
    put_u32(bytes, 1);
    put_u32(bytes, 3);
    put_u32(bytes, 0);
}

pub(super) fn source_version(bytes: &mut Vec<u8>) {
    put_u32(bytes, 0x2a);
    put_u32(bytes, 16);
    put_u64(bytes, 0);
}

pub(super) fn entry_point(bytes: &mut Vec<u8>, code_offset: usize) {
    put_u32(bytes, 0x8000_0028);
    put_u32(bytes, 24);
    put_u64(bytes, code_offset as u64);
    put_u64(bytes, 0);
}

pub(super) struct LinkeditLayout {
    pub(super) rebase_offset: usize,
    pub(super) rebase_size: usize,
    pub(super) fixups_offset: usize,
    pub(super) fixups_size: usize,
    pub(super) exports_offset: usize,
    pub(super) function_starts_offset: usize,
    pub(super) data_in_code_offset: usize,
    pub(super) symtab_offset: usize,
    pub(super) indirect_symbol_offset: usize,
    pub(super) string_offset: usize,
    pub(super) string_size: usize,
    pub(super) symbol_count: usize,
    pub(super) indirect_symbol_count: usize,
}

pub(super) fn linkedit_layout(
    image: &EncodedImage,
    libraries: &[(String, String)],
    linkedit_file_offset: usize,
) -> LinkeditLayout {
    let has_imports = !libraries.is_empty();
    let needs_data_const =
        has_imports || !image.initializers.is_empty() || rodata_len(image) > 0;
    // Rebase opcodes (for `__mod_init_func` pointers) lead the dyld_info payload,
    // followed by the bind opcodes. `rebase_offset` is 0 when there is nothing to
    // rebase, leaving the bind stream exactly where it was for imports-only images.
    let rebase_size = rebase_info(image).len();
    let rebase_offset = if rebase_size > 0 {
        linkedit_file_offset
    } else {
        0
    };
    let fixups_offset = linkedit_file_offset + rebase_size;
    let fixups_size = if needs_data_const {
        bind_info(image, libraries).len()
    } else {
        empty_chained_fixups().len()
    };
    let exports_offset = fixups_offset + fixups_size;
    let symtab_offset = exports_offset;
    let symbol_count = if needs_data_const {
        image.imports.len()
    } else {
        0
    };
    let indirect_symbol_offset = symtab_offset + symbol_count * 16;
    let indirect_symbol_count = if needs_data_const {
        image.imports.len()
    } else {
        0
    };
    let string_offset = indirect_symbol_offset + indirect_symbol_count * 4;
    let string_size = if needs_data_const {
        string_table(image).len()
    } else {
        0
    };
    let function_starts_offset = string_offset + string_size;
    let data_in_code_offset = function_starts_offset + 1;
    LinkeditLayout {
        rebase_offset,
        rebase_size,
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

pub(super) fn bind_info(image: &EncodedImage, libraries: &[(String, String)]) -> Vec<u8> {
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

pub(super) fn symbol_table(image: &EncodedImage) -> Vec<u8> {
    let strings = string_offsets(image);
    let mut bytes = Vec::new();
    for import in &image.imports {
        put_u32(
            &mut bytes,
            u32_field("symtab symbol string offset", strings[&import.symbol]),
        );
        bytes.push(0x1);
        bytes.push(0);
        put_u16(&mut bytes, 0);
        put_u64(&mut bytes, 0);
    }
    bytes
}

pub(super) fn string_table(image: &EncodedImage) -> Vec<u8> {
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

pub(super) fn empty_chained_fixups() -> Vec<u8> {
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

pub(super) fn code_signature(unsigned: &[u8], name: &str) -> Vec<u8> {
    let page_size = 4096;
    let page_count = unsigned.len().div_ceil(page_size);
    let ident = format!("mfb.{name}");
    let ident = ident.as_bytes();
    let ident_offset = 88usize;
    let hash_offset = align(ident_offset + ident.len() + 1, 4);
    let code_directory_len = hash_offset + page_count * 32;
    let superblob_len = 20 + code_directory_len;
    let mut bytes = Vec::new();
    // The Code Signature superblob is a 32-bit format: superblob/directory
    // lengths, nCodeSlots (page_count) and codeLimit (image length) are all u32
    // fields. Narrowing a ≥4 GiB image with `as u32` would silently emit an
    // under-covering, invalid ad-hoc signature, so reject it explicitly (bug-88).
    let u32_field = |what: &str, value: usize| -> u32 {
        u32::try_from(value)
            .unwrap_or_else(|_| panic!("mach-o code signature: {what} {value} exceeds u32"))
    };
    put_be_u32(&mut bytes, 0xfade_0cc0);
    put_be_u32(&mut bytes, u32_field("superblob length", superblob_len));
    put_be_u32(&mut bytes, 1);
    put_be_u32(&mut bytes, 0);
    put_be_u32(&mut bytes, 20);
    put_be_u32(&mut bytes, 0xfade_0c02);
    put_be_u32(&mut bytes, u32_field("code directory length", code_directory_len));
    put_be_u32(&mut bytes, 0x20400);
    put_be_u32(&mut bytes, 0x20002);
    put_be_u32(&mut bytes, hash_offset as u32);
    put_be_u32(&mut bytes, ident_offset as u32);
    put_be_u32(&mut bytes, 0);
    put_be_u32(&mut bytes, u32_field("code slot count", page_count));
    put_be_u32(&mut bytes, u32_field("code limit", unsigned.len()));
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
