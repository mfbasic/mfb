//! `canvas::systemFontTable` on Linux: the installed faces, through fontconfig
//! (plan-147-C).
//!
//! fontconfig is reached through `dlopen("libfontconfig.so.1")` and `dlsym`, never a
//! `DT_NEEDED`, for the same reason Vulkan is (`runtime/canvas/vulkan.rs`): a canvas
//! program must still start on a machine without the library. There — or if any
//! fontconfig call fails — the table is empty, so `listSystemFonts` answers `[]` and
//! `loadSystemFont` answers `ErrNotFound`, rather than the program failing to run.
//!
//! `FcFontList(NULL, …)` lists against fontconfig's current configuration, which a GTK
//! app has usually loaded already. A pattern is listed only when this build can draw it:
//!
//! * `fontformat` is `TrueType` — `glyf` outlines, a `.ttf` or a `.ttc` of them (CFF
//!   faces report `CFF`);
//! * `variable` is not true, and the high half of `index` is zero — fontconfig lists a
//!   variable font's named instances as patterns with `index = (instance + 1) << 16`,
//!   and the loader ignores `gvar`, so each would draw the default outlines;
//! * it has a `fullname` and a `file`. A missing `postscriptname` is written empty, and
//!   `loadSystemFont` then names the face by its full name.
//!
//! The table is built in two passes over the in-memory font set: the first sums the
//! lengths, the second copies into a block allocated once. Strings fontconfig returns
//! belong to their pattern, so they are copied before the set is destroyed.
//!
//! On x86-64 an `int` result (`FcResult`, `strcmp`) occupies only the low 32 bits of
//! `rax`; it is sign-extended before any comparison.

use super::gen_system_fonts_shared::{
    address_arg, alloc_string, call, load_arg, return_string, stack_cstring, store_result,
};
use crate::codegen::engine::builder::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// `RTLD_NOW`.
const RTLD_NOW: &str = "2";

/// The fontconfig entry points the enumeration resolves, in slot order.
const FC_FUNCTIONS: [&str; 9] = [
    "FcPatternCreate",
    "FcObjectSetCreate",
    "FcObjectSetAdd",
    "FcFontList",
    "FcPatternGetString",
    "FcPatternGetInteger",
    "FcPatternGetBool",
    "FcFontSetDestroy",
    "FcObjectSetDestroy",
];
const PATTERN_CREATE: usize = 0;
const OBJECT_SET_CREATE: usize = 1;
const OBJECT_SET_ADD: usize = 2;
const FONT_LIST: usize = 3;
const GET_STRING: usize = 4;
const GET_INTEGER: usize = 5;
const GET_BOOL: usize = 6;
const FONT_SET_DESTROY: usize = 7;
const OBJECT_SET_DESTROY: usize = 8;
/// `FcPatternDestroy` is resolved separately: it is the one cleanup call made on a
/// path where the others may not all exist.
const PATTERN_DESTROY: &str = "FcPatternDestroy";

/// The pattern properties read, which are also the ones the object set asks for.
const OBJECTS: [&str; 6] = [
    "fullname",
    "postscriptname",
    "file",
    "fontformat",
    "variable",
    "index",
];

struct Slots {
    handle: usize,
    functions: Vec<usize>,
    pattern_destroy: usize,
    objects: Vec<usize>,
    true_type: usize,
    pattern: usize,
    object_set: usize,
    font_set: usize,
    count: usize,
    fonts: usize,
    index: usize,
    font: usize,
    pass: usize,
    total: usize,
    result: usize,
    cursor: usize,
    out_string: usize,
    out_int: usize,
    failed: usize,
    full: usize,
    post_script: usize,
    file: usize,
    length: usize,
}

/// Call the fontconfig function held in `slots.functions[which]`; arguments must
/// already be staged. The pointer is loaded after staging: vregs are never allocated to
/// argument registers, so staging cannot be disturbed.
fn call_fc(builder: &mut CodeBuilder, slots: &Slots, which: usize) {
    let target = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &target,
        abi::stack_pointer(),
        slots.functions[which],
    ));
    builder.emit(abi::branch_link_register(&target));
}

/// Sign-extend the `int` result into a vreg and branch to `target` when it is not 0.
fn branch_if_int_result_nonzero(builder: &mut CodeBuilder, target: &str) {
    let result = builder.temporary_vreg();
    builder.emit(abi::sign_extend_word(&result, abi::c_return(0)));
    builder.emit(abi::compare_immediate(&result, "0"));
    builder.emit(abi::branch_ne(target));
}

/// `FcPatternGetString(font, object, 0, &out_string)`; branch to `missing` when it
/// does not answer `FcResultMatch`.
fn get_string(builder: &mut CodeBuilder, slots: &Slots, object: usize, missing: &str) {
    load_arg(builder, 0, slots.font);
    address_arg(builder, 1, slots.objects[object]);
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    address_arg(builder, 3, slots.out_string);
    call_fc(builder, slots, GET_STRING);
    branch_if_int_result_nonzero(builder, missing);
}

/// `strlen` of the C string in `slot`, or 0 when the slot holds NULL, into `length`.
fn length_of(builder: &mut CodeBuilder, ctx: &AbiCtx, slots: &Slots, slot: usize) -> Result<(), String> {
    let null = builder.label("canvas_sysfont_len_null");
    let done = builder.label("canvas_sysfont_len_done");
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
    builder.emit(abi::compare_immediate(&value, "0"));
    builder.emit(abi::branch_eq(&null));
    load_arg(builder, 0, slot);
    call(builder, ctx, "strlen")?;
    store_result(builder, slots.length);
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&null));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.length));
    builder.emit(abi::label(&done));
    Ok(())
}

/// Pass 0: `total += strlen(slot) + 1`. Pass 1: copy the string to the cursor, then the
/// separator byte `end`, and advance the cursor.
fn emit_field(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    slots: &Slots,
    slot: usize,
    end: &str,
) -> Result<(), String> {
    let copy = builder.label("canvas_sysfont_field_copy");
    let done = builder.label("canvas_sysfont_field_done");
    length_of(builder, ctx, slots, slot)?;
    let pass = builder.temporary_vreg();
    builder.emit(abi::load_u64(&pass, abi::stack_pointer(), slots.pass));
    builder.emit(abi::compare_immediate(&pass, "0"));
    builder.emit(abi::branch_ne(&copy));
    let total = builder.temporary_vreg();
    let length = builder.temporary_vreg();
    builder.emit(abi::load_u64(&total, abi::stack_pointer(), slots.total));
    builder.emit(abi::load_u64(&length, abi::stack_pointer(), slots.length));
    builder.emit(abi::add_registers(&total, &total, &length));
    builder.emit(abi::add_immediate(&total, &total, 1));
    builder.emit(abi::store_u64(&total, abi::stack_pointer(), slots.total));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&copy));
    // memcpy(cursor, string, length) — a NULL string has length 0 and copies nothing.
    let empty = builder.label("canvas_sysfont_field_empty");
    let length = builder.temporary_vreg();
    builder.emit(abi::load_u64(&length, abi::stack_pointer(), slots.length));
    builder.emit(abi::compare_immediate(&length, "0"));
    builder.emit(abi::branch_eq(&empty));
    load_arg(builder, 0, slots.cursor);
    load_arg(builder, 1, slot);
    load_arg(builder, 2, slots.length);
    call(builder, ctx, "memcpy")?;
    builder.emit(abi::label(&empty));
    let cursor = builder.temporary_vreg();
    let length = builder.temporary_vreg();
    let separator = builder.temporary_vreg();
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), slots.cursor));
    builder.emit(abi::load_u64(&length, abi::stack_pointer(), slots.length));
    builder.emit(abi::add_registers(&cursor, &cursor, &length));
    builder.emit(abi::move_immediate(&separator, "Integer", end));
    builder.emit(abi::store_u8(&separator, &cursor, 0));
    builder.emit(abi::add_immediate(&cursor, &cursor, 1));
    builder.emit(abi::store_u64(&cursor, abi::stack_pointer(), slots.cursor));
    builder.emit(abi::label(&done));
    Ok(())
}

/// Emit the whole body. The result is an MFBASIC `String` in the result registers.
pub(crate) fn emit_system_font_table(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
) -> Result<(), String> {
    let slot = |builder: &mut CodeBuilder, name: &str| builder.allocate_stack_object(name, 8);
    let handle = slot(builder, "canvas_sysfont_handle");
    let functions = (0..FC_FUNCTIONS.len())
        .map(|i| slot(builder, &format!("canvas_sysfont_fn{i}")))
        .collect();
    let pattern_destroy = slot(builder, "canvas_sysfont_pattern_destroy");
    let objects = OBJECTS
        .iter()
        .map(|object| stack_cstring(builder, &format!("canvas_sysfont_obj_{object}"), object))
        .collect();
    let true_type = stack_cstring(builder, "canvas_sysfont_truetype", "TrueType");
    let slots = Slots {
        handle,
        functions,
        pattern_destroy,
        objects,
        true_type,
        pattern: slot(builder, "canvas_sysfont_pattern"),
        object_set: slot(builder, "canvas_sysfont_object_set"),
        font_set: slot(builder, "canvas_sysfont_font_set"),
        count: slot(builder, "canvas_sysfont_count"),
        fonts: slot(builder, "canvas_sysfont_fonts"),
        index: slot(builder, "canvas_sysfont_index"),
        font: slot(builder, "canvas_sysfont_font"),
        pass: slot(builder, "canvas_sysfont_pass"),
        total: slot(builder, "canvas_sysfont_total"),
        result: slot(builder, "canvas_sysfont_result"),
        cursor: slot(builder, "canvas_sysfont_cursor"),
        out_string: slot(builder, "canvas_sysfont_out_string"),
        out_int: slot(builder, "canvas_sysfont_out_int"),
        failed: slot(builder, "canvas_sysfont_failed"),
        full: slot(builder, "canvas_sysfont_full"),
        post_script: slot(builder, "canvas_sysfont_ps"),
        file: slot(builder, "canvas_sysfont_file"),
        length: slot(builder, "canvas_sysfont_length"),
    };
    let empty = builder.label("canvas_sysfont_empty");
    let cleanup = builder.label("canvas_sysfont_cleanup");
    let pass_loop = builder.label("canvas_sysfont_pass_loop");
    let font_loop = builder.label("canvas_sysfont_font_loop");
    let font_next = builder.label("canvas_sysfont_font_next");
    let pass_end = builder.label("canvas_sysfont_pass_end");
    let finish = builder.label("canvas_sysfont_finish");
    let done = builder.label("canvas_sysfont_done");

    for s in [
        slots.pattern,
        slots.object_set,
        slots.font_set,
        slots.result,
        slots.failed,
    ] {
        builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), s));
    }

    // dlopen + dlsym. A missing library or symbol is an empty table, not an error.
    let library = stack_cstring(builder, "canvas_sysfont_lib", "libfontconfig.so.1");
    address_arg(builder, 0, library);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    call(builder, ctx, "dlopen")?;
    store_result(builder, slots.handle);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&empty));
    let names: Vec<(&str, usize)> = FC_FUNCTIONS
        .iter()
        .copied()
        .zip(slots.functions.iter().copied())
        .chain([(PATTERN_DESTROY, slots.pattern_destroy)])
        .collect();
    for (name, target) in names {
        let symbol = stack_cstring(builder, &format!("canvas_sysfont_sym_{name}"), name);
        load_arg(builder, 0, slots.handle);
        address_arg(builder, 1, symbol);
        call(builder, ctx, "dlsym")?;
        store_result(builder, target);
        builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
        builder.emit(abi::branch_eq(&empty));
    }

    // pattern (matches everything), the object set, and the list.
    call_fc(builder, &slots, PATTERN_CREATE);
    store_result(builder, slots.pattern);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&cleanup));
    call_fc(builder, &slots, OBJECT_SET_CREATE);
    store_result(builder, slots.object_set);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&cleanup));
    for object in slots.objects.clone() {
        load_arg(builder, 0, slots.object_set);
        address_arg(builder, 1, object);
        call_fc(builder, &slots, OBJECT_SET_ADD);
    }
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    load_arg(builder, 1, slots.pattern);
    load_arg(builder, 2, slots.object_set);
    call_fc(builder, &slots, FONT_LIST);
    store_result(builder, slots.font_set);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&cleanup));
    // FcFontSet { int nfont; int sfont; FcPattern **fonts; }
    let set = builder.temporary_vreg();
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u64(&set, abi::stack_pointer(), slots.font_set));
    builder.emit(abi::load_u32(&value, &set, 0));
    builder.emit(abi::store_u64(&value, abi::stack_pointer(), slots.count));
    let fonts = builder.temporary_vreg();
    builder.emit(abi::load_u64(&fonts, &set, 8));
    builder.emit(abi::store_u64(&fonts, abi::stack_pointer(), slots.fonts));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.pass));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.total));

    // Two passes over the set: size, then copy.
    builder.emit(abi::label(&pass_loop));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.index));
    builder.emit(abi::label(&font_loop));
    let index = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&index, abi::stack_pointer(), slots.index));
    builder.emit(abi::load_u64(&count, abi::stack_pointer(), slots.count));
    builder.emit(abi::compare_registers(&index, &count));
    builder.emit(abi::branch_ge(&pass_end));
    let fonts = builder.temporary_vreg();
    let offset = builder.temporary_vreg();
    let font = builder.temporary_vreg();
    builder.emit(abi::load_u64(&fonts, abi::stack_pointer(), slots.fonts));
    builder.emit(abi::load_u64(&offset, abi::stack_pointer(), slots.index));
    builder.emit(abi::shift_left_immediate(&offset, &offset, 3));
    builder.emit(abi::add_registers(&fonts, &fonts, &offset));
    builder.emit(abi::load_u64(&font, &fonts, 0));
    builder.emit(abi::store_u64(&font, abi::stack_pointer(), slots.font));

    // fontformat == "TrueType"
    get_string(builder, &slots, 3, &font_next);
    load_arg(builder, 0, slots.out_string);
    address_arg(builder, 1, slots.true_type);
    call(builder, ctx, "strcmp")?;
    branch_if_int_result_nonzero(builder, &font_next);
    // not variable
    let not_variable = builder.label("canvas_sysfont_not_variable");
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_int));
    load_arg(builder, 0, slots.font);
    address_arg(builder, 1, slots.objects[4]);
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    address_arg(builder, 3, slots.out_int);
    call_fc(builder, &slots, GET_BOOL);
    branch_if_int_result_nonzero(builder, &not_variable);
    let flag = builder.temporary_vreg();
    builder.emit(abi::load_u32(&flag, abi::stack_pointer(), slots.out_int));
    builder.emit(abi::compare_immediate(&flag, "0"));
    builder.emit(abi::branch_ne(&font_next));
    builder.emit(abi::label(&not_variable));
    // not a named instance: index >> 16 == 0
    let not_instance = builder.label("canvas_sysfont_not_instance");
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.out_int));
    load_arg(builder, 0, slots.font);
    address_arg(builder, 1, slots.objects[5]);
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    address_arg(builder, 3, slots.out_int);
    call_fc(builder, &slots, GET_INTEGER);
    branch_if_int_result_nonzero(builder, &not_instance);
    let face_index = builder.temporary_vreg();
    builder.emit(abi::load_u32(&face_index, abi::stack_pointer(), slots.out_int));
    builder.emit(abi::shift_right_immediate(&face_index, &face_index, 16));
    builder.emit(abi::compare_immediate(&face_index, "0"));
    builder.emit(abi::branch_ne(&font_next));
    builder.emit(abi::label(&not_instance));
    // fullname and file are required; postscriptname may be missing.
    get_string(builder, &slots, 0, &font_next);
    let full = builder.temporary_vreg();
    builder.emit(abi::load_u64(&full, abi::stack_pointer(), slots.out_string));
    builder.emit(abi::store_u64(&full, abi::stack_pointer(), slots.full));
    get_string(builder, &slots, 2, &font_next);
    let file = builder.temporary_vreg();
    builder.emit(abi::load_u64(&file, abi::stack_pointer(), slots.out_string));
    builder.emit(abi::store_u64(&file, abi::stack_pointer(), slots.file));
    let no_post_script = builder.label("canvas_sysfont_no_ps");
    let have_post_script = builder.label("canvas_sysfont_have_ps");
    get_string(builder, &slots, 1, &no_post_script);
    let post_script = builder.temporary_vreg();
    builder.emit(abi::load_u64(&post_script, abi::stack_pointer(), slots.out_string));
    builder.emit(abi::store_u64(&post_script, abi::stack_pointer(), slots.post_script));
    builder.emit(abi::branch(&have_post_script));
    builder.emit(abi::label(&no_post_script));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.post_script));
    builder.emit(abi::label(&have_post_script));

    emit_field(builder, ctx, &slots, slots.full, "31")?;
    emit_field(builder, ctx, &slots, slots.post_script, "31")?;
    emit_field(builder, ctx, &slots, slots.file, "30")?;

    builder.emit(abi::label(&font_next));
    let next = builder.temporary_vreg();
    builder.emit(abi::load_u64(&next, abi::stack_pointer(), slots.index));
    builder.emit(abi::add_immediate(&next, &next, 1));
    builder.emit(abi::store_u64(&next, abi::stack_pointer(), slots.index));
    builder.emit(abi::branch(&font_loop));

    // After the sizing pass, allocate once and run the loop again to copy.
    builder.emit(abi::label(&pass_end));
    let pass = builder.temporary_vreg();
    builder.emit(abi::load_u64(&pass, abi::stack_pointer(), slots.pass));
    builder.emit(abi::compare_immediate(&pass, "0"));
    builder.emit(abi::branch_ne(&finish));
    let out_of_memory = builder.label("canvas_sysfont_oom");
    let allocated = builder.label("canvas_sysfont_allocated");
    alloc_string(builder, slots.total, slots.result, &out_of_memory);
    builder.emit(abi::branch(&allocated));
    // Release the fontconfig objects, then raise.
    builder.emit(abi::label(&out_of_memory));
    let one = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&one, "Integer", "1"));
    builder.emit(abi::store_u64(&one, abi::stack_pointer(), slots.failed));
    builder.emit(abi::branch(&cleanup));
    builder.emit(abi::label(&allocated));
    let text = builder.temporary_vreg();
    builder.emit(abi::load_u64(&text, abi::stack_pointer(), slots.result));
    builder.emit(abi::add_immediate(&text, &text, 8));
    builder.emit(abi::store_u64(&text, abi::stack_pointer(), slots.cursor));
    let one = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&one, "Integer", "1"));
    builder.emit(abi::store_u64(&one, abi::stack_pointer(), slots.pass));
    builder.emit(abi::branch(&pass_loop));

    // Stamp the length (cursor - text) and the NUL.
    builder.emit(abi::label(&finish));
    let block = builder.temporary_vreg();
    let cursor = builder.temporary_vreg();
    let length = builder.temporary_vreg();
    let text = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, abi::stack_pointer(), slots.result));
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), slots.cursor));
    builder.emit(abi::add_immediate(&text, &block, 8));
    builder.emit(abi::subtract_registers(&length, &cursor, &text));
    builder.emit(abi::store_u64(&length, &block, 0));
    builder.emit(abi::store_u8(abi::ZERO, &cursor, 0));

    // Destroy whatever was created. `cleanup` is also where a failure lands; with no
    // result block by then, the table is empty.
    builder.emit(abi::label(&cleanup));
    for (slot, destroy) in [
        (slots.font_set, Some(FONT_SET_DESTROY)),
        (slots.object_set, Some(OBJECT_SET_DESTROY)),
        (slots.pattern, None),
    ] {
        let skip = builder.label("canvas_sysfont_destroy_skip");
        let value = builder.temporary_vreg();
        builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        builder.emit(abi::compare_immediate(&value, "0"));
        builder.emit(abi::branch_eq(&skip));
        load_arg(builder, 0, slot);
        match destroy {
            Some(which) => call_fc(builder, &slots, which),
            None => {
                let target = builder.temporary_vreg();
                builder.emit(abi::load_u64(
                    &target,
                    abi::stack_pointer(),
                    slots.pattern_destroy,
                ));
                builder.emit(abi::branch_link_register(&target));
            }
        }
        builder.emit(abi::label(&skip));
    }
    let not_failed = builder.label("canvas_sysfont_not_failed");
    let failed = builder.temporary_vreg();
    builder.emit(abi::load_u64(&failed, abi::stack_pointer(), slots.failed));
    builder.emit(abi::compare_immediate(&failed, "0"));
    builder.emit(abi::branch_eq(&not_failed));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&not_failed));
    let have_result = builder.label("canvas_sysfont_have_result");
    let result = builder.temporary_vreg();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), slots.result));
    builder.emit(abi::compare_immediate(&result, "0"));
    builder.emit(abi::branch_ne(&have_result));

    // The empty table: a zero-length String.
    builder.emit(abi::label(&empty));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.total));
    let empty_failed = builder.label("canvas_sysfont_empty_oom");
    let empty_ok = builder.label("canvas_sysfont_empty_ok");
    alloc_string(builder, slots.total, slots.result, &empty_failed);
    builder.emit(abi::branch(&empty_ok));
    builder.emit(abi::label(&empty_failed));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&empty_ok));
    let block = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, abi::stack_pointer(), slots.result));
    builder.emit(abi::store_u64(abi::ZERO, &block, 0));
    builder.emit(abi::store_u8(abi::ZERO, &block, 8));

    builder.emit(abi::label(&have_result));
    return_string(builder, slots.result);
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());
    Ok(())
}
