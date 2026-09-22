//! `canvas::systemFontTable` on macOS: the installed faces, through CoreText
//! (plan-147-B).
//!
//! CoreText reports one font URL per face — a collection's URL repeats once per face —
//! and gives no face *index*, so each record carries the face's PostScript name and
//! `canvas::loadFont(path, face)` finds the face by reading the file's own `name`
//! tables. A face is listed only when this build can draw it:
//!
//! * it has a `glyf` table — TrueType outlines, not CFF (`CTFontCopyTable`);
//! * its full name does not start with `.` — macOS's private UI faces;
//! * it is not a variable font (`CTFontCopyVariation` answers nothing). CoreText lists
//!   each *named instance* of a variable font as its own face, under a synthetic
//!   PostScript name (`Skia-Regular_Bold`) no face in the file carries, and the loader
//!   ignores `gvar` — every instance would draw the default outlines under the wrong
//!   name.
//!
//! The table is assembled inside CoreFoundation — a `CFMutableString` the names and
//! paths are appended to — and turned into an MFBASIC `String` once at the end, so no
//! per-field buffer exists. Records end in U+001E and fields end in U+001F; neither is
//! a character a font name or a font path carries.
//!
//! Every value needed after an external call lives in a stack slot: a CoreText or
//! CoreFoundation call clobbers every caller-saved register (`.ai/compiler.md`, the
//! register-lifetime rules).

use super::gen_system_fonts_shared::{call, load_arg, store_result};
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// `kCFStringEncodingUTF8`.
const CF_UTF8: &str = "134217984";
/// `kCFURLPOSIXPathStyle`.
const CF_POSIX_PATH: &str = "0";
/// The `glyf` table tag, `'glyf'` as a big-endian `u32`.
const TAG_GLYF: &str = "1735162214";
/// `.`, which starts the name of a private system face.
const PRIVATE_PREFIX: &str = "46";
/// U+001F, which ends a field.
const FIELD_END: &str = "31";
/// U+001E, which ends a record.
const RECORD_END: &str = "30";

/// Stack slots the enumeration keeps across calls.
struct Slots {
    table: usize,
    urls: usize,
    count: usize,
    index: usize,
    url: usize,
    descriptors: usize,
    descriptor_count: usize,
    descriptor_index: usize,
    font: usize,
    full: usize,
    post_script: usize,
    path: usize,
    unichar: usize,
    result: usize,
    capacity: usize,
}

/// `CFRelease(slot)` when the slot holds a non-NULL reference.
fn release_slot(builder: &mut CodeBuilder, ctx: &AbiCtx, slot: usize) -> Result<(), String> {
    let skip = builder.label("canvas_sysfont_release_skip");
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
    builder.emit(abi::compare_immediate(&value, "0"));
    builder.emit(abi::branch_eq(&skip));
    load_arg(builder, 0, slot);
    call(builder, ctx, "CFRelease")?;
    builder.emit(abi::label(&skip));
    Ok(())
}

/// Append one UTF-16 code unit to the table: `CFStringAppendCharacters(table, &unit, 1)`.
fn append_unit(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    slots: &Slots,
    unit: &str,
) -> Result<(), String> {
    let value = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&value, "Integer", unit));
    builder.emit(abi::store_u64(&value, abi::stack_pointer(), slots.unichar));
    load_arg(builder, 0, slots.table);
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        slots.unichar,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "1"));
    call(builder, ctx, "CFStringAppendCharacters")
}

/// `CFStringAppend(table, slot)` when the slot holds a string, then the field end.
fn append_field(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    slots: &Slots,
    slot: usize,
    end: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_sysfont_field_empty");
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
    builder.emit(abi::compare_immediate(&value, "0"));
    builder.emit(abi::branch_eq(&empty));
    load_arg(builder, 0, slots.table);
    load_arg(builder, 1, slot);
    call(builder, ctx, "CFStringAppend")?;
    builder.emit(abi::label(&empty));
    append_unit(builder, ctx, slots, end)
}

/// Emit the whole body. The result is an MFBASIC `String` in the result registers.
pub(crate) fn emit_system_font_table(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
) -> Result<(), String> {
    let slots = Slots {
        table: builder.allocate_stack_object("canvas_sysfont_table", 8),
        urls: builder.allocate_stack_object("canvas_sysfont_urls", 8),
        count: builder.allocate_stack_object("canvas_sysfont_count", 8),
        index: builder.allocate_stack_object("canvas_sysfont_index", 8),
        url: builder.allocate_stack_object("canvas_sysfont_url", 8),
        descriptors: builder.allocate_stack_object("canvas_sysfont_descs", 8),
        descriptor_count: builder.allocate_stack_object("canvas_sysfont_desc_count", 8),
        descriptor_index: builder.allocate_stack_object("canvas_sysfont_desc_index", 8),
        font: builder.allocate_stack_object("canvas_sysfont_font", 8),
        full: builder.allocate_stack_object("canvas_sysfont_full", 8),
        post_script: builder.allocate_stack_object("canvas_sysfont_ps", 8),
        path: builder.allocate_stack_object("canvas_sysfont_path", 8),
        unichar: builder.allocate_stack_object("canvas_sysfont_unichar", 8),
        result: builder.allocate_stack_object("canvas_sysfont_result", 8),
        capacity: builder.allocate_stack_object("canvas_sysfont_capacity", 8),
    };
    let url_loop = builder.label("canvas_sysfont_url_loop");
    let url_next = builder.label("canvas_sysfont_url_next");
    let urls_done = builder.label("canvas_sysfont_urls_done");
    let face_loop = builder.label("canvas_sysfont_face_loop");
    let face_next = builder.label("canvas_sysfont_face_next");
    let face_skip = builder.label("canvas_sysfont_face_skip");
    let faces_done = builder.label("canvas_sysfont_faces_done");
    let convert = builder.label("canvas_sysfont_convert");
    let alloc_ok = builder.label("canvas_sysfont_alloc_ok");
    let table_ok = builder.label("canvas_sysfont_table_ok");
    let done = builder.label("canvas_sysfont_done");

    // The table first, so every later path — including "no fonts at all" — ends in the
    // same conversion and answers a String, empty or not.
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    call(builder, ctx, "CFStringCreateMutable")?;
    store_result(builder, slots.table);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&table_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&table_ok));

    for slot in [slots.urls, slots.descriptors, slots.font, slots.full]
        .into_iter()
        .chain([slots.post_script, slots.path])
    {
        builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
    }
    call(builder, ctx, "CTFontManagerCopyAvailableFontURLs")?;
    store_result(builder, slots.urls);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&convert));
    load_arg(builder, 0, slots.urls);
    call(builder, ctx, "CFArrayGetCount")?;
    store_result(builder, slots.count);
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.index));

    // for each URL
    builder.emit(abi::label(&url_loop));
    let index = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&index, abi::stack_pointer(), slots.index));
    builder.emit(abi::load_u64(&count, abi::stack_pointer(), slots.count));
    builder.emit(abi::compare_registers(&index, &count));
    builder.emit(abi::branch_ge(&urls_done));
    load_arg(builder, 0, slots.urls);
    load_arg(builder, 1, slots.index);
    call(builder, ctx, "CFArrayGetValueAtIndex")?;
    store_result(builder, slots.url);
    load_arg(builder, 0, slots.url);
    call(builder, ctx, "CTFontManagerCreateFontDescriptorsFromURL")?;
    store_result(builder, slots.descriptors);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&url_next));
    load_arg(builder, 0, slots.descriptors);
    call(builder, ctx, "CFArrayGetCount")?;
    store_result(builder, slots.descriptor_count);
    builder.emit(abi::store_u64(
        abi::ZERO,
        abi::stack_pointer(),
        slots.descriptor_index,
    ));

    // for each face the URL holds
    builder.emit(abi::label(&face_loop));
    let face_index = builder.temporary_vreg();
    let face_count = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &face_index,
        abi::stack_pointer(),
        slots.descriptor_index,
    ));
    builder.emit(abi::load_u64(
        &face_count,
        abi::stack_pointer(),
        slots.descriptor_count,
    ));
    builder.emit(abi::compare_registers(&face_index, &face_count));
    builder.emit(abi::branch_ge(&faces_done));
    load_arg(builder, 0, slots.descriptors);
    load_arg(builder, 1, slots.descriptor_index);
    call(builder, ctx, "CFArrayGetValueAtIndex")?;
    // CTFontCreateWithFontDescriptor(descriptor, size, NULL): the size is a CGFloat,
    // passed in d0, not an integer register. 0.0 asks for the default size; nothing
    // here measures the font, only reads its names and tables.
    builder.emit(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    builder.emit(abi::float_move_d_from_x(
        abi::fp_argument_register(0)?,
        abi::ZERO,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    call(builder, ctx, "CTFontCreateWithFontDescriptor")?;
    store_result(builder, slots.font);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&face_next));

    // TrueType outlines only.
    load_arg(builder, 0, slots.font);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", TAG_GLYF));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    call(builder, ctx, "CTFontCopyTable")?;
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&face_skip));
    builder.emit(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    call(builder, ctx, "CFRelease")?;
    // Not a variable font.
    load_arg(builder, 0, slots.font);
    call(builder, ctx, "CTFontCopyVariation")?;
    let variation_none = builder.label("canvas_sysfont_variation_none");
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&variation_none));
    builder.emit(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    call(builder, ctx, "CFRelease")?;
    builder.emit(abi::branch(&face_skip));
    builder.emit(abi::label(&variation_none));

    load_arg(builder, 0, slots.font);
    call(builder, ctx, "CTFontCopyFullName")?;
    store_result(builder, slots.full);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&face_skip));
    // Not a private system face: a name that starts with `.` is macOS's own UI font
    // (`.SF NS`, `.Al Bayan PUA`), hidden from every font menu and not a name a
    // program is meant to ask for.
    load_arg(builder, 0, slots.full);
    call(builder, ctx, "CFStringGetLength")?;
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&face_skip));
    load_arg(builder, 0, slots.full);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    call(builder, ctx, "CFStringGetCharacterAtIndex")?;
    builder.emit(abi::compare_immediate(abi::c_return(0), PRIVATE_PREFIX));
    builder.emit(abi::branch_eq(&face_skip));
    load_arg(builder, 0, slots.font);
    call(builder, ctx, "CTFontCopyPostScriptName")?;
    store_result(builder, slots.post_script);
    load_arg(builder, 0, slots.url);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", CF_POSIX_PATH));
    call(builder, ctx, "CFURLCopyFileSystemPath")?;
    store_result(builder, slots.path);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&face_skip));

    append_field(builder, ctx, &slots, slots.full, FIELD_END)?;
    append_field(builder, ctx, &slots, slots.post_script, FIELD_END)?;
    append_field(builder, ctx, &slots, slots.path, RECORD_END)?;

    // Release this face's objects, whichever of them exist.
    builder.emit(abi::label(&face_skip));
    for slot in [slots.full, slots.post_script, slots.path, slots.font] {
        release_slot(builder, ctx, slot)?;
        builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
    }
    builder.emit(abi::label(&face_next));
    let next_face = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &next_face,
        abi::stack_pointer(),
        slots.descriptor_index,
    ));
    builder.emit(abi::add_immediate(&next_face, &next_face, 1));
    builder.emit(abi::store_u64(
        &next_face,
        abi::stack_pointer(),
        slots.descriptor_index,
    ));
    builder.emit(abi::branch(&face_loop));

    builder.emit(abi::label(&faces_done));
    release_slot(builder, ctx, slots.descriptors)?;
    builder.emit(abi::store_u64(
        abi::ZERO,
        abi::stack_pointer(),
        slots.descriptors,
    ));
    builder.emit(abi::label(&url_next));
    let next_url = builder.temporary_vreg();
    builder.emit(abi::load_u64(&next_url, abi::stack_pointer(), slots.index));
    builder.emit(abi::add_immediate(&next_url, &next_url, 1));
    builder.emit(abi::store_u64(&next_url, abi::stack_pointer(), slots.index));
    builder.emit(abi::branch(&url_loop));

    builder.emit(abi::label(&urls_done));
    release_slot(builder, ctx, slots.urls)?;

    // CFString → MFBASIC String: `[length u64][UTF-8 bytes][NUL]`, sized for the
    // worst-case encoding and then stamped with the real length.
    builder.emit(abi::label(&convert));
    load_arg(builder, 0, slots.table);
    call(builder, ctx, "CFStringGetLength")?;
    builder.emit(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", CF_UTF8));
    call(builder, ctx, "CFStringGetMaximumSizeForEncoding")?;
    builder.emit(abi::add_immediate(abi::c_return(0), abi::c_return(0), 1));
    store_result(builder, slots.capacity);
    let size = builder.temporary_vreg();
    builder.emit(abi::load_u64(&size, abi::stack_pointer(), slots.capacity));
    builder.emit(abi::add_immediate(abi::c_arg(0), &size, 8));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    release_slot(builder, ctx, slots.table)?;
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        slots.result,
    ));
    let buffer = builder.temporary_vreg();
    builder.emit(abi::load_u64(&buffer, abi::stack_pointer(), slots.result));
    builder.emit(abi::add_immediate(abi::c_arg(1), &buffer, 8));
    load_arg(builder, 0, slots.table);
    load_arg(builder, 2, slots.capacity);
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", CF_UTF8));
    call(builder, ctx, "CFStringGetCString")?;
    // A failed conversion leaves the buffer unspecified; an empty string is the honest
    // answer then, and `strlen` of a NUL-first buffer gives exactly that.
    let converted = builder.label("canvas_sysfont_converted");
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&converted));
    let first = builder.temporary_vreg();
    builder.emit(abi::load_u64(&first, abi::stack_pointer(), slots.result));
    builder.emit(abi::store_u8(abi::ZERO, &first, 8));
    builder.emit(abi::label(&converted));
    let text = builder.temporary_vreg();
    builder.emit(abi::load_u64(&text, abi::stack_pointer(), slots.result));
    builder.emit(abi::add_immediate(abi::c_arg(0), &text, 8));
    call(builder, ctx, "strlen")?;
    let length = builder.temporary_vreg();
    builder.emit(abi::move_register(&length, abi::c_return(0)));
    let block = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, abi::stack_pointer(), slots.result));
    builder.emit(abi::store_u64(&length, &block, 0));
    release_slot(builder, ctx, slots.table)?;

    let answer = builder.temporary_vreg();
    builder.emit(abi::load_u64(&answer, abi::stack_pointer(), slots.result));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &answer));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());
    Ok(())
}
