//! `canvas::systemFontTable` on Windows: the installed faces, through DirectWrite's
//! system font collection (plan-147-D).
//!
//! `DWriteCreateFactory` is `dwrite.dll`'s one flat export; everything after it is a
//! COM vtable call. The slot numbers below are the method's index in its interface's
//! `…Vtbl` struct in mingw-w64's `dwrite.h` / `dwrite_3.h` (cited per constant), counting
//! `QueryInterface`, `AddRef`, `Release` as 0, 1, 2.
//!
//! A font is listed only when this build can draw it:
//!
//! * `GetSimulations` is `DWRITE_FONT_SIMULATIONS_NONE` — DirectWrite lists a *bold* or
//!   *oblique* it would synthesise from another face, which no file carries;
//! * its face has a `glyf` table (`TryGetFontTable`) — TrueType outlines, not CFF;
//! * its face has no variations (`IDWriteFontFace5::HasVariations`, when the system
//!   offers that interface) — Windows lists each named instance of a variable font,
//!   and the loader ignores `gvar`;
//! * it is backed by a local file (`IDWriteLocalFontFileLoader`) with a full name.
//!
//! Names are taken in `en-us` when the font has that locale, else its first string,
//! which is how CoreText and fontconfig report them. The table is built in two passes
//! over the same collection object: the first sums the UTF-16 lengths, the second writes
//! each string straight into one wide buffer (`GetString` and `GetFilePathFromKey` write
//! into caller memory), and a single `WideCharToMultiByte` makes it an MFBASIC `String`.
//!
//! Every `HRESULT`/`UINT32`/`BOOL` result occupies only the low 32 bits of `rax` and is
//! sign-extended before it is compared.

use super::gen_system_fonts_shared::{
    address_arg, alloc_string, call, load_arg, return_string, stack_bytes,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// `IUnknown::QueryInterface` / `Release`.
const QUERY_INTERFACE: usize = 0;
const RELEASE: usize = 2;
/// dwrite.h:5247.
const FACTORY_GET_SYSTEM_FONT_COLLECTION: usize = 3;
/// dwrite.h:2360, 2363.
const COLLECTION_GET_FONT_FAMILY_COUNT: usize = 3;
const COLLECTION_GET_FONT_FAMILY: usize = 4;
/// dwrite.h:2225, 2228 (inherited from `IDWriteFontList`).
const FAMILY_GET_FONT_COUNT: usize = 4;
const FAMILY_GET_FONT: usize = 5;
/// dwrite.h:1974, 1980, 1992.
const FONT_GET_INFORMATIONAL_STRINGS: usize = 9;
const FONT_GET_SIMULATIONS: usize = 10;
const FONT_CREATE_FONT_FACE: usize = 13;
/// dwrite.h:1709, 1743, 1751.
const FACE_GET_FILES: usize = 4;
const FACE_TRY_GET_FONT_TABLE: usize = 12;
const FACE_RELEASE_FONT_TABLE: usize = 13;
/// dwrite_3.h:9602.
const FACE5_HAS_VARIATIONS: usize = 55;
/// dwrite.h:1102, 1107.
const FILE_GET_REFERENCE_KEY: usize = 3;
const FILE_GET_LOADER: usize = 4;
/// dwrite.h:986, 992.
const LOCAL_LOADER_GET_FILE_PATH_LENGTH: usize = 4;
const LOCAL_LOADER_GET_FILE_PATH: usize = 5;
/// dwrite.h:1396, 1413, 1418.
const STRINGS_FIND_LOCALE_NAME: usize = 4;
const STRINGS_GET_STRING_LENGTH: usize = 7;
const STRINGS_GET_STRING: usize = 8;

/// dwrite.h:381-382.
const INFORMATIONAL_STRING_FULL_NAME: &str = "16";
const INFORMATIONAL_STRING_POSTSCRIPT_NAME: &str = "17";
/// `DWRITE_MAKE_OPENTYPE_TAG('g','l','y','f')` — little-endian.
const TAG_GLYF: &str = "1719233639";
/// `CP_UTF8`.
const CP_UTF8: &str = "65001";

/// dwrite.h:5107.
const IID_IDWRITE_FACTORY: (u32, u16, u16, [u8; 8]) = (
    0xb859_ee5a,
    0xd838,
    0x4b5b,
    [0xa2, 0xe8, 0x1a, 0xdc, 0x7d, 0x93, 0xdb, 0x48],
);
/// dwrite.h:937.
const IID_IDWRITE_LOCAL_FONT_FILE_LOADER: (u32, u16, u16, [u8; 8]) = (
    0xb2d9_f3ec,
    0xc9fe,
    0x4a11,
    [0xa2, 0xec, 0xd8, 0x62, 0x08, 0xf7, 0xc0, 0xa2],
);
/// dwrite_3.h:9269.
const IID_IDWRITE_FONT_FACE5: (u32, u16, u16, [u8; 8]) = (
    0x98ef_f3a5,
    0xb667,
    0x479a,
    [0xb1, 0x45, 0xe2, 0xfa, 0x5b, 0x9f, 0xdc, 0x29],
);

fn guid_bytes(guid: (u32, u16, u16, [u8; 8])) -> Vec<u8> {
    let mut out = guid.0.to_le_bytes().to_vec();
    out.extend(guid.1.to_le_bytes());
    out.extend(guid.2.to_le_bytes());
    out.extend(guid.3);
    out
}

struct Slots {
    factory: usize,
    collection: usize,
    family: usize,
    font: usize,
    face: usize,
    face5: usize,
    full: usize,
    post_script: usize,
    file: usize,
    loader: usize,
    local: usize,
    family_count: usize,
    family_index: usize,
    font_count: usize,
    font_index: usize,
    pass: usize,
    total: usize,
    buffer: usize,
    cursor: usize,
    result: usize,
    wide_length: usize,
    utf8_length: usize,
    key: usize,
    key_size: usize,
    path_length: usize,
    out_u32: usize,
    out_bool: usize,
    out_pointer: usize,
    table_size: usize,
    table_context: usize,
    full_index: usize,
    full_length: usize,
    ps_index: usize,
    ps_length: usize,
    locale: usize,
    iid_factory: usize,
    iid_local: usize,
    iid_face5: usize,
}

/// The objects one font iteration may hold, released at the end of each.
fn per_font(slots: &Slots) -> [usize; 8] {
    [
        slots.local,
        slots.loader,
        slots.file,
        slots.post_script,
        slots.full,
        slots.face5,
        slots.face,
        slots.font,
    ]
}

/// A vtable call on the object in `this_slot`: `this` into the first argument, the
/// method from `[[this] + slot * 8]`. Other arguments — registers and the outgoing
/// stack tail — must already be staged. Leaves the sign-extended 32-bit result in a
/// fresh vreg, which it returns.
fn com(builder: &mut CodeBuilder, this_slot: usize, slot: usize) -> VirtualRegister {
    let method = builder.temporary_vreg();
    let result = builder.temporary_vreg();
    crate::codegen::os::ffi::emit_com_call(
        &mut builder.instructions,
        this_slot,
        slot,
        &method,
        &result,
    );
    result
}

/// Branch to `target` when an `HRESULT` failed (is negative).
fn on_failure(builder: &mut CodeBuilder, hr: &VirtualRegister, target: &str) {
    builder.emit(abi::compare_immediate(hr, "0"));
    builder.emit(abi::branch_lt(target));
}

/// `Release` the object in `slot` if there is one, and clear the slot.
fn release(builder: &mut CodeBuilder, slot: usize) {
    let skip = builder.label("canvas_sysfont_release_skip");
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
    builder.emit(abi::compare_immediate(&value, "0"));
    builder.emit(abi::branch_eq(&skip));
    com(builder, slot, RELEASE);
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
    builder.emit(abi::label(&skip));
}

/// Stack slot `slot` ← a zero-extended `u32` read from stack slot `from`.
fn copy_u32(builder: &mut CodeBuilder, from: usize, slot: usize) {
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u32(&value, abi::stack_pointer(), from));
    builder.emit(abi::store_u64(&value, abi::stack_pointer(), slot));
}

/// Branch to `copy` on the second pass.
fn branch_if_copying(builder: &mut CodeBuilder, slots: &Slots, copy: &str) {
    let pass = builder.temporary_vreg();
    builder.emit(abi::load_u64(&pass, abi::stack_pointer(), slots.pass));
    builder.emit(abi::compare_immediate(&pass, "0"));
    builder.emit(abi::branch_ne(copy));
}

/// Pass 1, after the string's `length` units were written at the cursor: the
/// separator `end` over the terminating NUL, and the cursor past it.
fn advance_cursor(builder: &mut CodeBuilder, slots: &Slots, length_slot: usize, end: &str) {
    let cursor = builder.temporary_vreg();
    let length = builder.temporary_vreg();
    let separator = builder.temporary_vreg();
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), slots.cursor));
    builder.emit(abi::load_u64(&length, abi::stack_pointer(), length_slot));
    builder.emit(abi::shift_left_immediate(&length, &length, 1));
    builder.emit(abi::add_registers(&cursor, &cursor, &length));
    builder.emit(abi::move_immediate(&separator, "Integer", end));
    builder.emit(abi::store_u16(&separator, &cursor, 0));
    builder.emit(abi::add_immediate(&cursor, &cursor, 2));
    builder.emit(abi::store_u64(&cursor, abi::stack_pointer(), slots.cursor));
}

/// Measure one localized-strings object: which string to take (`en-us` if the font has
/// it, else the first) into `index`, and its length in UTF-16 units into `length`. A
/// missing object, or one that will not answer, measures 0.
fn measure_strings(
    builder: &mut CodeBuilder,
    slots: &Slots,
    strings: usize,
    index: usize,
    length: usize,
) {
    let measured = builder.label("canvas_sysfont_strings_measured");
    let use_first = builder.label("canvas_sysfont_strings_first");
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), length));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index));
    let present = builder.temporary_vreg();
    builder.emit(abi::load_u64(&present, abi::stack_pointer(), strings));
    builder.emit(abi::compare_immediate(&present, "0"));
    builder.emit(abi::branch_eq(&measured));
    // FindLocaleName(strings, L"en-us", &index, &exists)
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_u32));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_bool));
    address_arg(builder, 1, slots.locale);
    address_arg(builder, 2, slots.out_u32);
    address_arg(builder, 3, slots.out_bool);
    let hr = com(builder, strings, STRINGS_FIND_LOCALE_NAME);
    on_failure(builder, &hr, &use_first);
    let exists = builder.temporary_vreg();
    builder.emit(abi::load_u32(&exists, abi::stack_pointer(), slots.out_bool));
    builder.emit(abi::compare_immediate(&exists, "0"));
    builder.emit(abi::branch_eq(&use_first));
    copy_u32(builder, slots.out_u32, index);
    builder.emit(abi::label(&use_first));
    // GetStringLength(strings, index, &length)
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_u32));
    load_arg(builder, 1, index);
    address_arg(builder, 2, slots.out_u32);
    let hr = com(builder, strings, STRINGS_GET_STRING_LENGTH);
    on_failure(builder, &hr, &measured);
    copy_u32(builder, slots.out_u32, length);
    builder.emit(abi::label(&measured));
}

/// Pass 1: `GetString(strings, index, cursor, length + 1)` when there is text, then the
/// separator.
fn write_strings(
    builder: &mut CodeBuilder,
    slots: &Slots,
    strings: usize,
    index: usize,
    length: usize,
    end: &str,
) {
    let written = builder.label("canvas_sysfont_strings_written");
    let units = builder.temporary_vreg();
    builder.emit(abi::load_u64(&units, abi::stack_pointer(), length));
    builder.emit(abi::compare_immediate(&units, "0"));
    builder.emit(abi::branch_eq(&written));
    load_arg(builder, 1, index);
    load_arg(builder, 2, slots.cursor);
    let size = builder.temporary_vreg();
    builder.emit(abi::load_u64(&size, abi::stack_pointer(), length));
    builder.emit(abi::add_immediate(abi::c_arg(3), &size, 1));
    com(builder, strings, STRINGS_GET_STRING);
    builder.emit(abi::label(&written));
    advance_cursor(builder, slots, length, end);
}

/// Emit the whole body. The result is an MFBASIC `String` in the result registers.
pub(crate) fn emit_system_font_table(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
) -> Result<(), String> {
    let slot = |builder: &mut CodeBuilder, name: &str| builder.allocate_stack_object(name, 8);
    let locale: Vec<u8> = "en-us\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let slots = Slots {
        factory: slot(builder, "canvas_sysfont_factory"),
        collection: slot(builder, "canvas_sysfont_collection"),
        family: slot(builder, "canvas_sysfont_family"),
        font: slot(builder, "canvas_sysfont_font"),
        face: slot(builder, "canvas_sysfont_face"),
        face5: slot(builder, "canvas_sysfont_face5"),
        full: slot(builder, "canvas_sysfont_full"),
        post_script: slot(builder, "canvas_sysfont_ps"),
        file: slot(builder, "canvas_sysfont_file"),
        loader: slot(builder, "canvas_sysfont_loader"),
        local: slot(builder, "canvas_sysfont_local"),
        family_count: slot(builder, "canvas_sysfont_family_count"),
        family_index: slot(builder, "canvas_sysfont_family_index"),
        font_count: slot(builder, "canvas_sysfont_font_count"),
        font_index: slot(builder, "canvas_sysfont_font_index"),
        pass: slot(builder, "canvas_sysfont_pass"),
        total: slot(builder, "canvas_sysfont_total"),
        buffer: slot(builder, "canvas_sysfont_buffer"),
        cursor: slot(builder, "canvas_sysfont_cursor"),
        result: slot(builder, "canvas_sysfont_result"),
        wide_length: slot(builder, "canvas_sysfont_wide_length"),
        utf8_length: slot(builder, "canvas_sysfont_utf8_length"),
        key: slot(builder, "canvas_sysfont_key"),
        key_size: slot(builder, "canvas_sysfont_key_size"),
        path_length: slot(builder, "canvas_sysfont_path_length"),
        out_u32: slot(builder, "canvas_sysfont_out_u32"),
        out_bool: slot(builder, "canvas_sysfont_out_bool"),
        out_pointer: slot(builder, "canvas_sysfont_out_pointer"),
        table_size: slot(builder, "canvas_sysfont_table_size"),
        table_context: slot(builder, "canvas_sysfont_table_context"),
        full_index: slot(builder, "canvas_sysfont_full_index"),
        full_length: slot(builder, "canvas_sysfont_full_length"),
        ps_index: slot(builder, "canvas_sysfont_ps_index"),
        ps_length: slot(builder, "canvas_sysfont_ps_length"),
        locale: stack_bytes(builder, "canvas_sysfont_locale", &locale),
        iid_factory: stack_bytes(
            builder,
            "canvas_sysfont_iid_factory",
            &guid_bytes(IID_IDWRITE_FACTORY),
        ),
        iid_local: stack_bytes(
            builder,
            "canvas_sysfont_iid_local",
            &guid_bytes(IID_IDWRITE_LOCAL_FONT_FILE_LOADER),
        ),
        iid_face5: stack_bytes(
            builder,
            "canvas_sysfont_iid_face5",
            &guid_bytes(IID_IDWRITE_FONT_FACE5),
        ),
    };
    let pass_loop = builder.label("canvas_sysfont_pass_loop");
    let family_loop = builder.label("canvas_sysfont_family_loop");
    let family_next = builder.label("canvas_sysfont_family_next");
    let families_done = builder.label("canvas_sysfont_families_done");
    let font_loop = builder.label("canvas_sysfont_font_loop");
    let font_done = builder.label("canvas_sysfont_font_done");
    let fonts_done = builder.label("canvas_sysfont_fonts_done");
    let finish = builder.label("canvas_sysfont_finish");
    let cleanup = builder.label("canvas_sysfont_cleanup");
    let out_of_memory = builder.label("canvas_sysfont_oom");
    let done = builder.label("canvas_sysfont_done");

    for s in [
        slots.factory,
        slots.collection,
        slots.family,
        slots.buffer,
        slots.result,
        slots.pass,
        slots.total,
    ]
    .into_iter()
    .chain(per_font(&slots))
    {
        builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), s));
    }

    // DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, &IID_IDWriteFactory, &factory)
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    address_arg(builder, 1, slots.iid_factory);
    address_arg(builder, 2, slots.factory);
    call(builder, ctx, "DWriteCreateFactory")?;
    let hr = builder.temporary_vreg();
    builder.emit(abi::sign_extend_word(&hr, abi::c_return(0)));
    on_failure(builder, &hr, &finish);
    // GetSystemFontCollection(factory, &collection, FALSE)
    address_arg(builder, 1, slots.collection);
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    let hr = com(builder, slots.factory, FACTORY_GET_SYSTEM_FONT_COLLECTION);
    on_failure(builder, &hr, &finish);
    let count = com(builder, slots.collection, COLLECTION_GET_FONT_FAMILY_COUNT);
    builder.emit(abi::store_u64(&count, abi::stack_pointer(), slots.family_count));

    // Two passes over the same collection: size, then copy.
    builder.emit(abi::label(&pass_loop));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.family_index));
    builder.emit(abi::label(&family_loop));
    let index = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&index, abi::stack_pointer(), slots.family_index));
    builder.emit(abi::load_u64(&count, abi::stack_pointer(), slots.family_count));
    builder.emit(abi::compare_registers(&index, &count));
    builder.emit(abi::branch_ge(&families_done));
    load_arg(builder, 1, slots.family_index);
    address_arg(builder, 2, slots.family);
    let hr = com(builder, slots.collection, COLLECTION_GET_FONT_FAMILY);
    on_failure(builder, &hr, &family_next);
    let fonts = com(builder, slots.family, FAMILY_GET_FONT_COUNT);
    builder.emit(abi::store_u64(&fonts, abi::stack_pointer(), slots.font_count));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.font_index));

    builder.emit(abi::label(&font_loop));
    let index = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&index, abi::stack_pointer(), slots.font_index));
    builder.emit(abi::load_u64(&count, abi::stack_pointer(), slots.font_count));
    builder.emit(abi::compare_registers(&index, &count));
    builder.emit(abi::branch_ge(&fonts_done));
    load_arg(builder, 1, slots.font_index);
    address_arg(builder, 2, slots.font);
    let hr = com(builder, slots.family, FAMILY_GET_FONT);
    on_failure(builder, &hr, &font_done);
    // Not simulated.
    let simulations = com(builder, slots.font, FONT_GET_SIMULATIONS);
    builder.emit(abi::compare_immediate(&simulations, "0"));
    builder.emit(abi::branch_ne(&font_done));
    // A face with TrueType outlines.
    address_arg(builder, 1, slots.face);
    let hr = com(builder, slots.font, FONT_CREATE_FONT_FACE);
    on_failure(builder, &hr, &font_done);
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_bool));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.table_context));
    let context = builder.temporary_vreg();
    let exists = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&context, abi::stack_pointer(), slots.table_context));
    builder.emit(abi::add_immediate(&exists, abi::stack_pointer(), slots.out_bool));
    builder.emit(abi::outgoing_stack_arg_store(&context, 0));
    builder.emit(abi::outgoing_stack_arg_store(&exists, 1));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", TAG_GLYF));
    address_arg(builder, 2, slots.out_pointer);
    address_arg(builder, 3, slots.table_size);
    let hr = com(builder, slots.face, FACE_TRY_GET_FONT_TABLE);
    on_failure(builder, &hr, &font_done);
    let has_glyf = builder.temporary_vreg();
    builder.emit(abi::load_u32(&has_glyf, abi::stack_pointer(), slots.out_bool));
    builder.emit(abi::compare_immediate(&has_glyf, "0"));
    builder.emit(abi::branch_eq(&font_done));
    load_arg(builder, 1, slots.table_context);
    com(builder, slots.face, FACE_RELEASE_FONT_TABLE);
    // Not a variable font, when the system can say.
    let no_face5 = builder.label("canvas_sysfont_no_face5");
    address_arg(builder, 1, slots.iid_face5);
    address_arg(builder, 2, slots.face5);
    let hr = com(builder, slots.face, QUERY_INTERFACE);
    on_failure(builder, &hr, &no_face5);
    let variations = com(builder, slots.face5, FACE5_HAS_VARIATIONS);
    builder.emit(abi::compare_immediate(&variations, "0"));
    builder.emit(abi::branch_ne(&font_done));
    builder.emit(abi::label(&no_face5));
    // The full name is required; the PostScript name may be missing.
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_bool));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        INFORMATIONAL_STRING_FULL_NAME,
    ));
    address_arg(builder, 2, slots.full);
    address_arg(builder, 3, slots.out_bool);
    let hr = com(builder, slots.font, FONT_GET_INFORMATIONAL_STRINGS);
    on_failure(builder, &hr, &font_done);
    let full = builder.temporary_vreg();
    builder.emit(abi::load_u64(&full, abi::stack_pointer(), slots.full));
    builder.emit(abi::compare_immediate(&full, "0"));
    builder.emit(abi::branch_eq(&font_done));
    let no_post_script = builder.label("canvas_sysfont_no_ps");
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        INFORMATIONAL_STRING_POSTSCRIPT_NAME,
    ));
    address_arg(builder, 2, slots.post_script);
    address_arg(builder, 3, slots.out_bool);
    let hr = com(builder, slots.font, FONT_GET_INFORMATIONAL_STRINGS);
    on_failure(builder, &hr, &no_post_script);
    builder.emit(abi::label(&no_post_script));
    // The local file: GetFiles(face, &one, &file), its key, its loader as a local
    // loader, and the path length.
    let one = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&one, "Integer", "1"));
    builder.emit(abi::store_u64(&one, abi::stack_pointer(), slots.out_u32));
    address_arg(builder, 1, slots.out_u32);
    address_arg(builder, 2, slots.file);
    let hr = com(builder, slots.face, FACE_GET_FILES);
    on_failure(builder, &hr, &font_done);
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.key_size));
    address_arg(builder, 1, slots.key);
    address_arg(builder, 2, slots.key_size);
    let hr = com(builder, slots.file, FILE_GET_REFERENCE_KEY);
    on_failure(builder, &hr, &font_done);
    address_arg(builder, 1, slots.loader);
    let hr = com(builder, slots.file, FILE_GET_LOADER);
    on_failure(builder, &hr, &font_done);
    address_arg(builder, 1, slots.iid_local);
    address_arg(builder, 2, slots.local);
    let hr = com(builder, slots.loader, QUERY_INTERFACE);
    on_failure(builder, &hr, &font_done);
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_u32));
    load_arg(builder, 1, slots.key);
    let key_size = builder.temporary_vreg();
    builder.emit(abi::load_u32(&key_size, abi::stack_pointer(), slots.key_size));
    builder.emit(abi::move_register(abi::c_arg(2), &key_size));
    address_arg(builder, 3, slots.out_u32);
    let hr = com(builder, slots.local, LOCAL_LOADER_GET_FILE_PATH_LENGTH);
    on_failure(builder, &hr, &font_done);
    copy_u32(builder, slots.out_u32, slots.path_length);

    measure_strings(builder, &slots, slots.full, slots.full_index, slots.full_length);
    measure_strings(builder, &slots, slots.post_script, slots.ps_index, slots.ps_length);
    // The record is full + PostScript + path + three separators, in UTF-16 units.
    let record = builder.temporary_vreg();
    let part = builder.temporary_vreg();
    builder.emit(abi::load_u64(&record, abi::stack_pointer(), slots.full_length));
    builder.emit(abi::load_u64(&part, abi::stack_pointer(), slots.ps_length));
    builder.emit(abi::add_registers(&record, &record, &part));
    builder.emit(abi::load_u64(&part, abi::stack_pointer(), slots.path_length));
    builder.emit(abi::add_registers(&record, &record, &part));
    builder.emit(abi::add_immediate(&record, &record, 3));
    let copy = builder.label("canvas_sysfont_record_copy");
    branch_if_copying(builder, &slots, &copy);
    let total = builder.temporary_vreg();
    builder.emit(abi::load_u64(&total, abi::stack_pointer(), slots.total));
    builder.emit(abi::add_registers(&total, &total, &record));
    builder.emit(abi::store_u64(&total, abi::stack_pointer(), slots.total));
    builder.emit(abi::branch(&font_done));
    // Pass 1 writes a record only if it fits what pass 0 sized: the collection is one
    // snapshot, but a COM call that failed only the first time must not write past the
    // buffer.
    builder.emit(abi::label(&copy));
    let end = builder.temporary_vreg();
    let need = builder.temporary_vreg();
    builder.emit(abi::load_u64(&end, abi::stack_pointer(), slots.total));
    builder.emit(abi::shift_left_immediate(&end, &end, 1));
    builder.emit(abi::load_u64(&part, abi::stack_pointer(), slots.buffer));
    builder.emit(abi::add_registers(&end, &end, &part));
    builder.emit(abi::shift_left_immediate(&need, &record, 1));
    builder.emit(abi::load_u64(&part, abi::stack_pointer(), slots.cursor));
    builder.emit(abi::add_registers(&need, &need, &part));
    builder.emit(abi::compare_registers(&need, &end));
    builder.emit(abi::branch_gt(&font_done));
    write_strings(builder, &slots, slots.full, slots.full_index, slots.full_length, "31");
    write_strings(builder, &slots, slots.post_script, slots.ps_index, slots.ps_length, "31");
    // GetFilePathFromKey(local, key, keySize, cursor, length + 1)
    let size = builder.temporary_vreg();
    builder.emit(abi::load_u64(&size, abi::stack_pointer(), slots.path_length));
    builder.emit(abi::add_immediate(&size, &size, 1));
    builder.emit(abi::outgoing_stack_arg_store(&size, 0));
    load_arg(builder, 1, slots.key);
    let key_size = builder.temporary_vreg();
    builder.emit(abi::load_u32(&key_size, abi::stack_pointer(), slots.key_size));
    builder.emit(abi::move_register(abi::c_arg(2), &key_size));
    load_arg(builder, 3, slots.cursor);
    com(builder, slots.local, LOCAL_LOADER_GET_FILE_PATH);
    advance_cursor(builder, &slots, slots.path_length, "30");

    builder.emit(abi::label(&font_done));
    for s in per_font(&slots) {
        release(builder, s);
    }
    let next = builder.temporary_vreg();
    builder.emit(abi::load_u64(&next, abi::stack_pointer(), slots.font_index));
    builder.emit(abi::add_immediate(&next, &next, 1));
    builder.emit(abi::store_u64(&next, abi::stack_pointer(), slots.font_index));
    builder.emit(abi::branch(&font_loop));

    builder.emit(abi::label(&fonts_done));
    release(builder, slots.family);
    builder.emit(abi::label(&family_next));
    let next = builder.temporary_vreg();
    builder.emit(abi::load_u64(&next, abi::stack_pointer(), slots.family_index));
    builder.emit(abi::add_immediate(&next, &next, 1));
    builder.emit(abi::store_u64(&next, abi::stack_pointer(), slots.family_index));
    builder.emit(abi::branch(&family_loop));

    // After the sizing pass, one wide buffer for every string, then the copy pass.
    builder.emit(abi::label(&families_done));
    let pass = builder.temporary_vreg();
    builder.emit(abi::load_u64(&pass, abi::stack_pointer(), slots.pass));
    builder.emit(abi::compare_immediate(&pass, "0"));
    builder.emit(abi::branch_ne(&finish));
    let total = builder.temporary_vreg();
    let buffer_ok = builder.label("canvas_sysfont_buffer_ok");
    builder.emit(abi::load_u64(&total, abi::stack_pointer(), slots.total));
    builder.emit(abi::shift_left_immediate(&total, &total, 1));
    builder.emit(abi::add_immediate(abi::c_arg(0), &total, 2));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&buffer_ok));
    builder.emit(abi::branch(&out_of_memory));
    builder.emit(abi::label(&buffer_ok));
    builder.emit(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), slots.buffer));
    builder.emit(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), slots.cursor));
    let one = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&one, "Integer", "1"));
    builder.emit(abi::store_u64(&one, abi::stack_pointer(), slots.pass));
    builder.emit(abi::branch(&pass_loop));

    // UTF-16 → the MFBASIC String. An empty table, or any failure before the copy
    // pass, has no buffer and converts to "".
    builder.emit(abi::label(&finish));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.utf8_length));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.wide_length));
    let measured = builder.label("canvas_sysfont_utf8_measured");
    let buffer = builder.temporary_vreg();
    builder.emit(abi::load_u64(&buffer, abi::stack_pointer(), slots.buffer));
    builder.emit(abi::compare_immediate(&buffer, "0"));
    builder.emit(abi::branch_eq(&measured));
    let cursor = builder.temporary_vreg();
    let wide = builder.temporary_vreg();
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), slots.cursor));
    builder.emit(abi::subtract_registers(&wide, &cursor, &buffer));
    builder.emit(abi::shift_right_immediate(&wide, &wide, 1));
    builder.emit(abi::store_u64(&wide, abi::stack_pointer(), slots.wide_length));
    builder.emit(abi::compare_immediate(&wide, "0"));
    builder.emit(abi::branch_eq(&measured));
    for k in 0..4 {
        builder.emit(abi::outgoing_stack_arg_store(abi::ZERO, k));
    }
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", CP_UTF8));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    load_arg(builder, 2, slots.buffer);
    load_arg(builder, 3, slots.wide_length);
    call(builder, ctx, "WideCharToMultiByte")?;
    let length = builder.temporary_vreg();
    builder.emit(abi::sign_extend_word(&length, abi::c_return(0)));
    builder.emit(abi::store_u64(&length, abi::stack_pointer(), slots.utf8_length));
    builder.emit(abi::label(&measured));
    alloc_string(builder, slots.utf8_length, slots.result, &out_of_memory);
    let converted = builder.label("canvas_sysfont_converted");
    let length = builder.temporary_vreg();
    builder.emit(abi::load_u64(&length, abi::stack_pointer(), slots.utf8_length));
    builder.emit(abi::compare_immediate(&length, "0"));
    builder.emit(abi::branch_eq(&converted));
    let text = builder.temporary_vreg();
    builder.emit(abi::load_u64(&text, abi::stack_pointer(), slots.result));
    builder.emit(abi::add_immediate(&text, &text, 8));
    builder.emit(abi::outgoing_stack_arg_store(&text, 0));
    let capacity = builder.temporary_vreg();
    builder.emit(abi::load_u64(&capacity, abi::stack_pointer(), slots.utf8_length));
    builder.emit(abi::outgoing_stack_arg_store(&capacity, 1));
    builder.emit(abi::outgoing_stack_arg_store(abi::ZERO, 2));
    builder.emit(abi::outgoing_stack_arg_store(abi::ZERO, 3));
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", CP_UTF8));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    load_arg(builder, 2, slots.buffer);
    load_arg(builder, 3, slots.wide_length);
    call(builder, ctx, "WideCharToMultiByte")?;
    builder.emit(abi::label(&converted));
    let block = builder.temporary_vreg();
    let length = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, abi::stack_pointer(), slots.result));
    builder.emit(abi::load_u64(&length, abi::stack_pointer(), slots.utf8_length));
    builder.emit(abi::store_u64(&length, &block, 0));
    builder.emit(abi::add_registers(&block, &block, &length));
    builder.emit(abi::store_u8(abi::ZERO, &block, 8));
    builder.emit(abi::branch(&cleanup));

    builder.emit(abi::label(&out_of_memory));
    for s in per_font(&slots) {
        release(builder, s);
    }
    release(builder, slots.family);

    // Free the wide buffer and release the collection and factory, on every path.
    builder.emit(abi::label(&cleanup));
    let no_buffer = builder.label("canvas_sysfont_no_buffer");
    let buffer = builder.temporary_vreg();
    builder.emit(abi::load_u64(&buffer, abi::stack_pointer(), slots.buffer));
    builder.emit(abi::compare_immediate(&buffer, "0"));
    builder.emit(abi::branch_eq(&no_buffer));
    let size = builder.temporary_vreg();
    builder.emit(abi::load_u64(&size, abi::stack_pointer(), slots.total));
    builder.emit(abi::shift_left_immediate(&size, &size, 1));
    builder.emit(abi::add_immediate(abi::c_arg(1), &size, 2));
    load_arg(builder, 0, slots.buffer);
    builder.emit_arena_free_call();
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.buffer));
    builder.emit(abi::label(&no_buffer));
    release(builder, slots.collection);
    release(builder, slots.factory);
    let have_result = builder.label("canvas_sysfont_have_result");
    let result = builder.temporary_vreg();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), slots.result));
    builder.emit(abi::compare_immediate(&result, "0"));
    builder.emit(abi::branch_ne(&have_result));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&have_result));
    return_string(builder, slots.result);
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());
    Ok(())
}
