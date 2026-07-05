use super::commands::*;
use super::*;

pub(super) fn encode_mach_o(
    name: &str,
    code_offset: usize,
    entry_offset: usize,
    code: &[u8],
    data: &[u8],
    libraries: &[(String, String)],
    image: &EncodedImage,
) -> Vec<u8> {
    let unsigned =
        encode_unsigned_mach_o(code_offset, entry_offset, code, data, 0, libraries, image);
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
    let has_init = !image.initializers.is_empty();
    let has_data = !data.is_empty();
    // A `__DATA_CONST` segment (and the `LC_DYLD_INFO_ONLY` path) is needed when
    // the image has a GOT (imports) and/or a `__mod_init_func` (initializers).
    let needs_data_const = has_imports || has_init;
    let dc_size = data_const_size(image);
    let signing_metadata = image.signing_metadata.as_deref();
    let signing_metadata_len = signing_metadata.map_or(0, |metadata| metadata.len());
    let layout = macho_layout(
        code_offset,
        code.len(),
        data.len(),
        dc_size,
        signing_metadata_len,
    );
    let linkedit = linkedit_layout(image, libraries, layout.linkedit_file_offset);
    let signature_offset = align(linkedit.data_in_code_offset, 16);
    let linkedit_file_size = signature_offset + signature_size - layout.linkedit_file_offset;
    let load_commands_size =
        load_commands_size(libraries, signing_metadata.is_some(), has_init, has_data);
    let mut bytes = Vec::new();

    put_u32(&mut bytes, 0xfeed_facf);
    put_u32(&mut bytes, 0x0100_000c);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 2);
    put_u32(
        &mut bytes,
        load_command_count(libraries, signing_metadata.is_some(), has_init, has_data),
    );
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
    if needs_data_const {
        data_const_segment(
            &mut bytes,
            layout.data_const_file_offset,
            dc_size,
            image.imports.len(),
            image.initializers.len(),
        );
    }
    if has_data {
        data_segment(
            &mut bytes,
            layout.data_seg_file_offset,
            layout.data_seg_size,
            data.len(),
        );
    }
    if let Some(metadata) = signing_metadata {
        mfb_sign_segment(&mut bytes, layout.mfb_sign_file_offset, metadata.len());
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
    if needs_data_const {
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
    bytes.resize(layout.text_file_size, 0);
    if needs_data_const {
        // GOT slots (zero-filled, bound by dyld) followed by the `__mod_init_func`
        // pointer array. The init pointers hold unslid text addresses; `rebase_info`
        // tells dyld to slide them before running them (plan-linker.md §7.5).
        bytes.resize(layout.data_const_file_offset, 0);
        bytes.resize(layout.data_const_file_offset + image.imports.len() * 8, 0);
        for name in &image.initializers {
            put_u64(&mut bytes, initializer_addr(image, name, code_offset));
        }
    }
    if has_data {
        // Writable `__DATA`: the program's constant data and the zero-initialized
        // main-arena global. Padded to the page-aligned segment size.
        bytes.resize(layout.data_seg_file_offset, 0);
        bytes.extend_from_slice(data);
        bytes.resize(layout.data_seg_file_offset + layout.data_seg_size, 0);
    }
    if let Some(metadata) = signing_metadata {
        bytes.resize(layout.mfb_sign_file_offset, 0);
        bytes.extend_from_slice(metadata);
    }
    bytes.resize(layout.linkedit_file_offset, 0);
    if needs_data_const {
        bytes.extend_from_slice(&rebase_info(image));
        bytes.resize(linkedit.fixups_offset, 0);
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
pub(super) struct MachOLayout {
    pub(super) text_file_size: usize,
    pub(super) data_const_file_offset: usize,
    /// Writable `__DATA` segment (the main-arena global plus the program's
    /// constant data). Placed after `__DATA_CONST` so `__DATA_CONST` keeps
    /// segment index 2, which `rebase_info` hardcodes. Zero `data_seg_size` means
    /// the image has no data and no `__DATA` segment is emitted.
    pub(super) data_seg_file_offset: usize,
    pub(super) data_seg_size: usize,
    pub(super) mfb_sign_file_offset: usize,
    pub(super) linkedit_file_offset: usize,
}

pub(super) fn macho_layout(
    code_offset: usize,
    code_len: usize,
    data_len: usize,
    data_const_size: usize,
    signing_metadata_len: usize,
) -> MachOLayout {
    let has_data = data_len > 0;
    // `__TEXT` now holds only code (+ import stubs); the constant data moved to
    // the writable `__DATA` segment so the main-arena global can be stored to.
    let text_file_size = align(code_offset + code_len, PAGE_SIZE);
    let data_const_file_offset = text_file_size;
    let after_data_const = data_const_file_offset + data_const_size;
    let data_seg_file_offset = if has_data {
        align(after_data_const, PAGE_SIZE)
    } else {
        after_data_const
    };
    let data_seg_size = if has_data {
        align(data_len, PAGE_SIZE)
    } else {
        0
    };
    let after_data = data_seg_file_offset + data_seg_size;
    let mfb_sign_file_offset = if signing_metadata_len == 0 {
        after_data
    } else {
        align(after_data, 16)
    };
    let linkedit_file_offset = if signing_metadata_len == 0 {
        after_data
    } else {
        align(mfb_sign_file_offset + signing_metadata_len, PAGE_SIZE)
    };
    MachOLayout {
        text_file_size,
        data_const_file_offset,
        data_seg_file_offset,
        data_seg_size,
        mfb_sign_file_offset,
        linkedit_file_offset,
    }
}

pub(super) fn code_offset(
    libraries: &[(String, String)],
    has_signing_metadata: bool,
    has_init: bool,
    has_data: bool,
) -> usize {
    align(
        32 + load_commands_size(libraries, has_signing_metadata, has_init, has_data),
        4,
    )
}

fn load_commands_size(
    libraries: &[(String, String)],
    has_signing_metadata: bool,
    has_init: bool,
    has_data: bool,
) -> usize {
    let base = 72 + 232 + 72 + 24 + 80 + dylinker_command_size() + 24 + 32 + 16 + 24 + 16 + 16 + 16;
    let signing = if has_signing_metadata { 152 } else { 0 };
    // The writable `__DATA` segment adds its segment header plus one `__data`
    // section header.
    let data = if has_data { 72 + 80 } else { 0 };
    let needs_data_const = !libraries.is_empty() || has_init;
    (if !needs_data_const {
        base + 16 + 16
    } else {
        // __DATA_CONST segment (72 + one section header per __got/__mod_init_func)
        // + LC_DYLD_INFO_ONLY + one LC_LOAD_DYLIB per library.
        let sections = data_const_section_count(libraries.len(), has_init as usize) as usize;
        let dylibs: usize = libraries
            .iter()
            .map(|(_, path)| dylib_command_size(path))
            .sum();
        base + (72 + sections * 80) + 48 + dylibs
    }) + data
        + signing
}

fn load_command_count(
    libraries: &[(String, String)],
    has_signing_metadata: bool,
    has_init: bool,
    has_data: bool,
) -> u32 {
    let signing = if has_signing_metadata { 1 } else { 0 };
    // The chained-fixups path (no data-const segment) and the dyld_info path both
    // total 15 base commands; the data-const path swaps two LINKEDIT commands for a
    // __DATA_CONST segment + LC_DYLD_INFO_ONLY. A non-empty __mod_init_func adds a
    // section, not a command, so only extra dylibs grow the count. The writable
    // `__DATA` segment adds one command when the image has any data.
    let _ = has_init;
    15 + libraries.len() as u32 + signing + has_data as u32
}
