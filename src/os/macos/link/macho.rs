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
    let rodata_size = rodata_len(image);
    let has_rodata = rodata_size > 0;
    // `__DATA` (writable) holds only the data past the read-only constant prefix —
    // the arena global and other runtime globals (bug-187).
    let has_writable = data.len() > rodata_size;
    // A `__DATA_CONST` segment (and the `LC_DYLD_INFO_ONLY` path) is needed when
    // the image has a GOT (imports), a `__mod_init_func` (initializers), and/or a
    // read-only `__const` block (constants, bug-187).
    let needs_data_const = has_imports || has_init || has_rodata;
    let dc_size = data_const_size(image);
    let signing_metadata = image.signing_metadata.as_deref();
    let signing_metadata_len = signing_metadata.map_or(0, |metadata| metadata.len());
    let layout = macho_layout(
        code_offset,
        code.len(),
        data.len(),
        rodata_size,
        dc_size,
        signing_metadata_len,
    );
    let linkedit = linkedit_layout(image, libraries, layout.linkedit_file_offset);
    let signature_offset = align(linkedit.data_in_code_offset, 16);
    let linkedit_file_size = signature_offset + signature_size - layout.linkedit_file_offset;
    let load_commands_size = load_commands_size(
        libraries,
        signing_metadata.is_some(),
        has_init,
        has_writable,
        has_rodata,
    );
    let mut bytes = Vec::new();

    put_u32(&mut bytes, 0xfeed_facf);
    put_u32(&mut bytes, 0x0100_000c);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 2);
    put_u32(
        &mut bytes,
        load_command_count(
            libraries,
            signing_metadata.is_some(),
            has_init,
            has_writable,
        ),
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
            rodata_offset_in_data_const(image),
            rodata_size,
        );
    }
    if has_writable {
        data_segment(
            &mut bytes,
            layout.data_seg_file_offset,
            layout.data_seg_size,
            data.len() - rodata_size,
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
    // The `MFBasic\0` provenance marker (plan-43), before `LC_CODE_SIGNATURE` so
    // its payload is inside the signed prefix.
    note_command(
        &mut bytes,
        layout.note_file_offset,
        MFB_NOTE_DESCRIPTOR_SIZE,
    );
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
        if has_rodata {
            // Read-only `__const`: the program's constant data (string literals,
            // error messages), past the GOT/init pointers (bug-187).
            bytes.resize(
                layout.data_const_file_offset + rodata_offset_in_data_const(image),
                0,
            );
            bytes.extend_from_slice(&data[..rodata_size]);
        }
    }
    if has_writable {
        // Writable `__DATA`: the runtime globals past the read-only constant prefix
        // (the zero-initialized main-arena global etc.). Padded to the page-aligned
        // segment size.
        bytes.resize(layout.data_seg_file_offset, 0);
        bytes.extend_from_slice(&data[rodata_size..]);
        bytes.resize(layout.data_seg_file_offset + layout.data_seg_size, 0);
    }
    if let Some(metadata) = signing_metadata {
        bytes.resize(layout.mfb_sign_file_offset, 0);
        bytes.extend_from_slice(metadata);
    }
    // The out-of-line `LC_NOTE` payload (plan-43), in the gap before `__LINKEDIT`.
    bytes.resize(layout.note_file_offset, 0);
    bytes.extend_from_slice(&mfb_note_descriptor());
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
    /// Writable `__DATA` segment (the main-arena global and other runtime globals,
    /// i.e. `image.data` past its read-only constant prefix — bug-187). Placed
    /// after `__DATA_CONST` so `__DATA_CONST` keeps segment index 2, which
    /// `rebase_info` hardcodes. Zero `data_seg_size` means the image has no
    /// writable data and no `__DATA` segment is emitted.
    pub(super) data_seg_file_offset: usize,
    pub(super) data_seg_size: usize,
    pub(super) mfb_sign_file_offset: usize,
    /// The `LC_NOTE` descriptor payload (plan-43): a 16-byte-aligned region after
    /// `__DATA` / the optional `__MFB` sign block and before `__LINKEDIT`. It is
    /// a bare file region owned by no segment — `LC_NOTE` addresses it by file
    /// offset — and sits below `codeLimit`, so the ad-hoc signature covers it.
    pub(super) note_file_offset: usize,
    pub(super) linkedit_file_offset: usize,
}

pub(super) fn macho_layout(
    code_offset: usize,
    code_len: usize,
    data_len: usize,
    rodata_size: usize,
    data_const_size: usize,
    signing_metadata_len: usize,
) -> MachOLayout {
    // Only the writable suffix (`data_len - rodata_size`) lands in `__DATA`; the
    // read-only constant prefix rides in `__DATA_CONST,__const` (bug-187).
    let writable_len = data_len.saturating_sub(rodata_size);
    let has_writable = writable_len > 0;
    // `__TEXT` holds only code (+ import stubs); the writable data moved to the
    // `__DATA` segment so the main-arena global can be stored to.
    let text_file_size = align(code_offset + code_len, PAGE_SIZE);
    let data_const_file_offset = text_file_size;
    let after_data_const = data_const_file_offset + data_const_size;
    let data_seg_file_offset = if has_writable {
        align(after_data_const, PAGE_SIZE)
    } else {
        after_data_const
    };
    let data_seg_size = if has_writable {
        align(writable_len, PAGE_SIZE)
    } else {
        0
    };
    let after_data = data_seg_file_offset + data_seg_size;
    let mfb_sign_file_offset = if signing_metadata_len == 0 {
        after_data
    } else {
        align(after_data, 16)
    };
    let after_sign = if signing_metadata_len == 0 {
        after_data
    } else {
        mfb_sign_file_offset + signing_metadata_len
    };
    // The `LC_NOTE` descriptor (plan-43) follows the signing block (or `__DATA`
    // when there is none); `__LINKEDIT` starts on the next page after it, so the
    // note is never inside a mapped segment but is still hashed by the signature.
    let note_file_offset = align(after_sign, 16);
    let linkedit_file_offset = align(note_file_offset + MFB_NOTE_DESCRIPTOR_SIZE, PAGE_SIZE);
    MachOLayout {
        text_file_size,
        data_const_file_offset,
        data_seg_file_offset,
        data_seg_size,
        mfb_sign_file_offset,
        note_file_offset,
        linkedit_file_offset,
    }
}

pub(super) fn code_offset(
    libraries: &[(String, String)],
    has_signing_metadata: bool,
    has_init: bool,
    has_writable: bool,
    has_rodata: bool,
) -> usize {
    align(
        32 + load_commands_size(
            libraries,
            has_signing_metadata,
            has_init,
            has_writable,
            has_rodata,
        ),
        4,
    )
}

fn load_commands_size(
    libraries: &[(String, String)],
    has_signing_metadata: bool,
    has_init: bool,
    has_writable: bool,
    has_rodata: bool,
) -> usize {
    // The trailing `NOTE_COMMAND_SIZE` is the unconditional `LC_NOTE` provenance
    // marker (plan-43) — present in every image, signed or not.
    let base = 72
        + 232
        + 72
        + 24
        + 80
        + dylinker_command_size()
        + 24
        + 32
        + 16
        + 24
        + 16
        + 16
        + 16
        + NOTE_COMMAND_SIZE;
    let signing = if has_signing_metadata { 152 } else { 0 };
    // The writable `__DATA` segment adds its segment header plus one `__data`
    // section header.
    let data = if has_writable { 72 + 80 } else { 0 };
    let needs_data_const = !libraries.is_empty() || has_init || has_rodata;
    (if !needs_data_const {
        base + 16 + 16
    } else {
        // __DATA_CONST segment (72 + one section header per
        // __got/__mod_init_func/__const) + LC_DYLD_INFO_ONLY + one LC_LOAD_DYLIB
        // per library.
        let sections =
            data_const_section_count(libraries.len(), has_init as usize, has_rodata) as usize;
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
    has_writable: bool,
) -> u32 {
    let signing = if has_signing_metadata { 1 } else { 0 };
    // The chained-fixups path (no data-const segment) and the dyld_info path both
    // total 16 base commands (15 plus the unconditional `LC_NOTE` provenance
    // marker, plan-43); the data-const path swaps two LINKEDIT commands for a
    // __DATA_CONST segment + LC_DYLD_INFO_ONLY. A __mod_init_func/__const block adds
    // a section, not a command, so only extra dylibs grow the count. The writable
    // `__DATA` segment adds one command when the image has writable data.
    let _ = has_init;
    16 + libraries.len() as u32 + signing + has_writable as u32
}
