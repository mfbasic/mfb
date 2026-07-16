//! macOS app-mode TermView backend: draw/init/clear/scroll/write/keydown
//! emitters plus color-from-packed and is-flipped (plan-11 split, pure relocation).

use super::*;

/// `void _mfb_macapp_key_down(id self /*x0 = MFBTextView*/, SEL _cmd, NSEvent
/// *event /*x2*/)`: terminal-style input (plan §5.6). The transcript view itself
/// receives keys; each printable key is echoed into the transcript and appended
/// to the input-line buffer, Backspace deletes the last character from both, and
/// Return commits the buffered line (UTF-8 bytes + newline) to the input pipe so
/// the program's reads on fd 0 receive it. Runs on the main thread, so the
/// synchronous transcript appends do not deadlock.
pub(super) fn emit_key_down_helper() -> CodeFunction {
    let mut asm = Asm::new(KEY_DOWN_SYMBOL);
    // Frame: lr@0, x19(self)@8, x20(app)@16, x21(chars/cstr)@24,
    // x22(textStorage)@32, x23(event/scratch)@40, x24(char code)@48,
    // x25(input line)@56, x26(input mode)@64, newline byte@72.
    let frame = 96;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::store_u64("x23", abi::stack_pointer(), 40));
    asm.push(abi::store_u64("x24", abi::stack_pointer(), 48));
    asm.push(abi::store_u64("x25", abi::stack_pointer(), 56));
    asm.push(abi::store_u64("x26", abi::stack_pointer(), 64));
    asm.push(abi::move_register("x19", "x0")); // self (text view)
    asm.push(abi::move_register("x23", "x2")); // event

    // chars = [event characters]; if [chars length] == 0 (modifier-only) -> done
    asm.load_selector(SEL_CHARACTERS.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // chars
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    // c = [chars characterAtIndex:0]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0")); // char code

    // app, input line buffer, text storage.
    asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // app
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", INPUT_LINE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // input line buffer
    asm.load_selector(SEL_TEXT_STORAGE.0);
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0")); // text storage
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", INPUT_MODE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0")); // input mode

    // Dispatch on the key.
    asm.push(abi::compare_immediate("x26", INPUT_MODE_RAW_NO_ECHO));
    asm.push(abi::branch_eq("kd_raw"));
    asm.push(abi::compare_immediate("x24", "13")); // CR
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate("x24", "10")); // LF
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate("x24", "3")); // Enter
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate("x24", "127")); // Delete
    asm.push(abi::branch_eq("kd_backspace"));
    asm.push(abi::compare_immediate("x24", "8")); // Backspace
    asm.push(abi::branch_eq("kd_backspace"));

    // Default: [inputLine appendString:chars]; echo only for io.input mode.
    asm.load_selector(SEL_APPEND_STRING.0);
    asm.push(abi::move_register("x2", "x21"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_done"));
    asm.push(abi::move_register("x0", "x19"));
    asm.push(abi::move_register("x1", "x21"));
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::branch("kd_done"));

    // Commit: deliver the buffered line + newline to the pipe, echo a newline,
    // clear the buffer.
    asm.push(abi::label("kd_commit"));
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // UTF-8 bytes of the line
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::move_register("x22", "x0")); // bytes still to deliver
    // Deliver the whole line, resuming after a partial write (bug-241). A pipe
    // write is atomic only up to PIPE_BUF, so a line longer than that splits
    // when the reader is behind; writing the remainder off and still sending the
    // newline below would hand the program a truncated line as a complete one.
    // x22 (text storage) is dead on this path — only `kd_backspace` reads it and
    // `kd_commit` branches straight to `kd_done` — so it carries the remaining
    // count across the `_write` calls, which clobber x0-x17.
    asm.push(abi::label("kd_commit_write"));
    asm.push(abi::compare_immediate("x22", "0"));
    asm.push(abi::branch_eq("kd_commit_newline"));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::move_register("x1", "x21"));
    asm.push(abi::move_register("x2", "x22"));
    asm.call_external("_write", LIB_SYSTEM);
    // The pipe write end is O_NONBLOCK (bug-114): if the pipe buffer is full the
    // worker hasn't drained stdin, so write() returns -1/EAGAIN instead of
    // blocking the UI thread forever. Give up on the line then, skipping the
    // trailing newline so the program never sees a partial line terminated as a
    // whole one; still echo + clear below. Testing `<= 0` rather than `< 0` also
    // makes the loop provably terminate: each iteration either delivers at least
    // one byte or leaves the loop, so it can never spin on the UI thread.
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_le("kd_commit_echo"));
    asm.push(abi::add_registers("x21", "x21", "x0"));
    asm.push(abi::subtract_registers("x22", "x22", "x0"));
    asm.push(abi::branch("kd_commit_write"));
    asm.push(abi::label("kd_commit_newline"));
    asm.push(abi::move_immediate("x9", "Integer", "10"));
    asm.push(abi::store_u8("x9", abi::stack_pointer(), 72));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 72));
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::label("kd_commit_echo"));
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, "x21", STR_NEWLINE.0);
    asm.push(abi::move_register("x1", "x0"));
    asm.push(abi::move_register("x0", "x19"));
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::label("kd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, "x21", STR_EMPTY.0);
    asm.push(abi::move_register("x24", "x0")); // empty string (callee-saved; survives
                                               // the sel_registerName in load_selector)
    asm.load_selector(SEL_SET_STRING.0);
    asm.push(abi::move_register("x2", "x24"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("kd_done"));

    // Backspace: drop the last character from the buffer and the transcript.
    asm.push(abi::label("kd_backspace"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register("x23", "x0")); // buffer length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", "x23", 1)); // range.location = len - 1
    asm.push(abi::move_immediate("x3", "Integer", "1")); // range.length = 1
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_done"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register("x23", "x0")); // transcript length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", "x23", 1));
    asm.push(abi::move_immediate("x3", "Integer", "1"));
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // Terminate the line-echo backspace path here; without this the block falls
    // through into `kd_raw` and injects the DEL/BS key byte into the input pipe.
    // Mirrors `tkd_backspace`'s terminating branch (bug-46).
    asm.push(abi::branch("kd_done"));

    // Raw read mode: write this key event's UTF-8 bytes to the input pipe now,
    // with no transcript echo and no line buffering.
    asm.push(abi::label("kd_raw"));
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // UTF-8 bytes for chars
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register("x2", "x0"));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::move_register("x1", "x21"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::branch("kd_done"));

    asm.push(abi::label("kd_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::load_u64("x23", abi::stack_pointer(), 40));
    asm.push(abi::load_u64("x24", abi::stack_pointer(), 48));
    asm.push(abi::load_u64("x25", abi::stack_pointer(), 56));
    asm.push(abi::load_u64("x26", abi::stack_pointer(), 64));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.keyDown".to_string(),
        symbol: KEY_DOWN_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// IMP for `TermView`'s `isFlipped` override — returns YES so row 0 is at the
/// top of the view and cell `(row, col)` maps to `(col*cellW, row*cellH)` in the
/// flipped coordinate space (plan-01-term.md §6.3).
pub(super) fn emit_term_view_is_flipped() -> CodeFunction {
    let mut asm = Asm::new(TERM_VIEW_IS_FLIPPED_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x0", "Integer", "1")); // YES
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.term.isFlipped".to_string(),
        symbol: TERM_VIEW_IS_FLIPPED_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Boolean".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// Build an `NSColor` from a packed `r|g<<8|b<<16` value (in `x11`) into `x0`.
/// The class is in `x26` and the `colorWithCalibratedRed:green:blue:alpha:`
/// selector is spilled at `sp+sel_off` (both pre-resolved so no `sel_registerName`
/// call clobbers the d0..d3 colour-component arguments). Clobbers x9/x10/d0..d4.
fn emit_color_from_packed(asm: &mut Asm, sel_off: usize) {
    asm.push(abi::move_immediate("x10", "Integer", "255"));
    asm.push(abi::signed_convert_to_float_d("d4", "x10")); // 255.0 divisor
    asm.push(abi::and_registers("x9", "x11", "x10"));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d4")); // r
    asm.push(abi::shift_right_immediate("x9", "x11", 8));
    asm.push(abi::and_registers("x9", "x9", "x10"));
    asm.push(abi::signed_convert_to_float_d("d1", "x9"));
    asm.push(abi::float_divide_d("d1", "d1", "d4")); // g
    asm.push(abi::shift_right_immediate("x9", "x11", 16));
    asm.push(abi::and_registers("x9", "x9", "x10"));
    asm.push(abi::signed_convert_to_float_d("d2", "x9"));
    asm.push(abi::float_divide_d("d2", "d2", "d4")); // b
    asm.push(abi::move_immediate("x9", "Integer", "1"));
    asm.push(abi::signed_convert_to_float_d("d3", "x9")); // alpha 1.0
    asm.push(abi::move_register("x0", "x26")); // NSColor class
    asm.push(abi::load_u64("x1", abi::stack_pointer(), sel_off));
    asm.call_external("_objc_msgSend", LIB_OBJC);
}

/// IMP for `TermView`'s `drawRect:` (`void drawRect:(NSRect dirty)`; self in x0,
/// `_cmd` in x1, the rect in d0..d3).
///
/// Fills the dirty rect black, then for each cell paints its background rect (when
/// non-black) and its glyph in the cell's foreground colour and the monospaced
/// font (plan-01-term.md §6.3).
pub(super) fn emit_term_view_draw_rect() -> CodeFunction {
    let mut asm = Asm::new(TERM_VIEW_DRAW_RECT_SYMBOL);
    // Frame: lr@0; callee-saved x19(state)@8, x20(cells)@16, x21(rows)@24,
    // x22(cols)@32, x23(row)@40, x24(col)@48, x25(attrs)@56, x26(NSColor class)@64,
    // x27(cell ptr)@72, x28(drawAtPoint sel)@80; rect@88..112; colorWithRGBA
    // sel@120; set sel@128; setObject:forKey: sel@136; fg key@144;
    // stringWithChars sel@152; glyph buffer@160; bold NSNumber@168; underline
    // NSNumber@176; stroke-width key@184; underline-style key@192;
    // removeObjectForKey: sel@200.
    let frame = 224;
    let (off_rx, off_ry, off_rw, off_rh) = (88, 96, 104, 112);
    let off_color_sel = 120;
    let off_set_sel = 128;
    let off_setobj_sel = 136;
    let off_fgkey = 144;
    let off_swc_sel = 152;
    let off_glyph = 160;
    let off_numbold = 168;
    let off_numul = 176;
    let off_strokekey = 184;
    let off_ulkey = 192;
    let off_removeobj_sel = 200;
    let saved: [(&str, usize); 10] = [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x27", 72),
        ("x28", 80),
    ];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in saved {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    // Spill the dirty rect (d0..d3) before any call clobbers the FP arg regs.
    for (reg, off) in [
        ("d0", off_rx),
        ("d1", off_ry),
        ("d2", off_rw),
        ("d3", off_rh),
    ] {
        asm.push(abi::float_move_x_from_d("x9", reg));
        asm.push(abi::store_u64("x9", abi::stack_pointer(), off));
    }

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)  (self in x0)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x19", "x0")); // state (or nil)

    // Pre-resolve the colour primitives so the per-cell colour build avoids any
    // sel_registerName (which would clobber the d0..d3 component arguments).
    asm.external_data("x26", CLASS_NS_COLOR, LIB_APPKIT); // NSColor class
    asm.load_selector(SEL_COLOR_WITH_RGBA.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_color_sel));
    asm.load_selector(SEL_SET.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_set_sel));

    // Fill the dirty rect black: [[NSColor blackColor] set]; NSRectFill(rect).
    asm.load_selector(SEL_BLACK_COLOR.0);
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_set_sel));
    asm.call_external("_objc_msgSend", LIB_OBJC); // [black set]
    for (reg, off) in [
        ("d0", off_rx),
        ("d1", off_ry),
        ("d2", off_rw),
        ("d3", off_rh),
    ] {
        asm.push(abi::load_u64("x9", abi::stack_pointer(), off));
        asm.push(abi::float_move_d_from_x(reg, "x9"));
    }
    asm.call_external(NS_RECT_FILL, LIB_APPKIT);

    // No state / no grid yet -> nothing more to paint.
    asm.push(abi::compare_immediate("x19", "0"));
    asm.push(abi::branch_eq("draw_done"));
    asm.push(abi::load_u64("x20", "x19", TV_CELLS_OFFSET)); // cells
    asm.push(abi::compare_immediate("x20", "0"));
    asm.push(abi::branch_eq("draw_done"));
    asm.push(abi::load_u64("x21", "x19", TV_ROWS_OFFSET));
    asm.push(abi::load_u64("x22", "x19", TV_COLS_OFFSET));

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data("x25", CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, "d0", TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // font

    // attrs = [NSMutableDictionary dictionary]; [attrs setObject:font forKey:NSFontAttributeName]
    // (the foreground colour key is set per cell below).
    asm.load_selector(SEL_DICTIONARY.0);
    asm.external_data("x0", CLASS_NS_MUTABLE_DICTIONARY, LIB_FOUNDATION);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x27", "x0")); // attrs dict (temp in x27)
    asm.load_selector(SEL_SET_OBJECT_FOR_KEY.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::move_register("x2", "x25")); // font
    asm.external_data("x3", NS_FONT_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::move_register("x0", "x27"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x25", "x27")); // attrs dict -> x25

    // Pre-resolve drawAtPoint: (x28) + stringWithChars: (spilled); cache the
    // foreground-colour attribute key (an NSString global) on the stack.
    asm.load_selector(SEL_DRAW_AT_POINT.0);
    asm.push(abi::move_register("x28", "x1"));
    asm.load_selector(SEL_STRING_WITH_CHARS.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_swc_sel));
    asm.external_data("x3", NS_FOREGROUND_COLOR_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::store_u64("x3", abi::stack_pointer(), off_fgkey));

    // Bold/underline attribute values + keys (set/removed per cell below).
    // numberBold = [NSNumber numberWithDouble:-3.0]  (negative stroke width = faux bold)
    asm.load_selector(SEL_NUMBER_WITH_DOUBLE.0);
    asm.external_data("x0", CLASS_NS_NUMBER, LIB_FOUNDATION);
    asm.push(abi::move_immediate("x9", "Integer", "3"));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::float_negate_d("d0", "d0")); // -3.0
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64("x0", abi::stack_pointer(), off_numbold));
    // numberUnderline = [NSNumber numberWithInt:1]  (NSUnderlineStyleSingle)
    asm.load_selector(SEL_NUMBER_WITH_INT.0);
    asm.external_data("x0", CLASS_NS_NUMBER, LIB_FOUNDATION);
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64("x0", abi::stack_pointer(), off_numul));
    // stroke-width + underline-style attribute keys (NSString globals).
    asm.external_data("x3", NS_STROKE_WIDTH_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::store_u64("x3", abi::stack_pointer(), off_strokekey));
    asm.external_data("x3", NS_UNDERLINE_STYLE_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::store_u64("x3", abi::stack_pointer(), off_ulkey));
    asm.load_selector(SEL_REMOVE_OBJECT_FOR_KEY.0);
    asm.push(abi::store_u64(
        "x1",
        abi::stack_pointer(),
        off_removeobj_sel,
    ));

    // for row in 0..rows: for col in 0..cols
    asm.push(abi::move_immediate("x23", "Integer", "0"));
    asm.push(abi::label("draw_row"));
    asm.push(abi::compare_registers("x23", "x21"));
    asm.push(abi::branch_ge("draw_done"));
    asm.push(abi::move_immediate("x24", "Integer", "0"));
    asm.push(abi::label("draw_col"));
    asm.push(abi::compare_registers("x24", "x22"));
    asm.push(abi::branch_ge("draw_row_next"));

    // cell = cells + (row*cols + col) * CELL_SIZE
    asm.push(abi::multiply_registers("x9", "x23", "x22"));
    asm.push(abi::add_registers("x9", "x9", "x24"));
    asm.push(abi::shift_left_immediate("x9", "x9", 4)); // * CELL_SIZE (16)
    asm.push(abi::add_registers("x27", "x20", "x9")); // cell ptr (callee-saved)

    // --- background: fill the cell rect when bg is non-black ---
    asm.push(abi::load_u32("x11", "x27", CELL_BG_OFFSET));
    asm.push(abi::compare_immediate("x11", "0"));
    asm.push(abi::branch_eq("draw_skip_bg"));
    emit_color_from_packed(&mut asm, off_color_sel); // x0 = bg colour
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_set_sel));
    asm.call_external("_objc_msgSend", LIB_OBJC); // [bgColor set]
    asm.push(abi::load_u64("x9", "x19", TV_CELL_W_OFFSET));
    asm.push(abi::float_move_d_from_x("d2", "x9")); // cellW
    asm.push(abi::load_u64("x9", "x19", TV_CELL_H_OFFSET));
    asm.push(abi::float_move_d_from_x("d3", "x9")); // cellH
    asm.push(abi::signed_convert_to_float_d("d4", "x24"));
    asm.push(abi::float_multiply_d("d0", "d4", "d2")); // px
    asm.push(abi::signed_convert_to_float_d("d5", "x23"));
    asm.push(abi::float_multiply_d("d1", "d5", "d3")); // py
    asm.call_external(NS_RECT_FILL, LIB_APPKIT);
    asm.push(abi::label("draw_skip_bg"));

    // --- glyph: paint in the cell foreground colour when non-blank ---
    asm.push(abi::load_u32("x9", "x27", CELL_GLYPH_OFFSET));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("draw_col_next"));
    asm.push(abi::compare_immediate("x9", "32")); // space = blank
    asm.push(abi::branch_eq("draw_col_next"));
    // [attrs setObject:[color from cell.fg] forKey:NSForegroundColorAttributeName]
    asm.push(abi::load_u32("x11", "x27", CELL_FG_OFFSET));
    emit_color_from_packed(&mut asm, off_color_sel); // x0 = fg colour
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::move_register("x2", "x0")); // colour (x2 set after the sel load)
    asm.push(abi::load_u64("x3", abi::stack_pointer(), off_fgkey));
    asm.push(abi::move_register("x0", "x25")); // attrs dict
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // bold: set/remove the faux-bold stroke-width attribute for this cell.
    asm.push(abi::load_u8("x9", "x27", CELL_BOLD_OFFSET));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("draw_bold_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_numbold));
    asm.push(abi::load_u64("x3", abi::stack_pointer(), off_strokekey));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("draw_bold_done"));
    asm.push(abi::label("draw_bold_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_removeobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_strokekey));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("draw_bold_done"));
    // underline: set/remove the underline-style attribute for this cell.
    asm.push(abi::load_u8("x9", "x27", CELL_UNDERLINE_OFFSET));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("draw_ul_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_numul));
    asm.push(abi::load_u64("x3", abi::stack_pointer(), off_ulkey));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("draw_ul_done"));
    asm.push(abi::label("draw_ul_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_removeobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_ulkey));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("draw_ul_done"));
    // s = [NSString stringWithCharacters:&glyph length:1]
    asm.push(abi::load_u32("x9", "x27", CELL_GLYPH_OFFSET));
    asm.push(abi::store_u32("x9", abi::stack_pointer(), off_glyph));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_swc_sel));
    asm.external_data("x0", CLASS_NS_STRING, LIB_FOUNDATION);
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), off_glyph));
    asm.push(abi::move_immediate("x3", "Integer", "1"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [s drawAtPoint:(col*cellW, row*cellH) withAttributes:attrs]
    asm.push(abi::load_u64("x9", "x19", TV_CELL_W_OFFSET));
    asm.push(abi::float_move_d_from_x("d4", "x9"));
    asm.push(abi::load_u64("x9", "x19", TV_CELL_H_OFFSET));
    asm.push(abi::float_move_d_from_x("d5", "x9"));
    asm.push(abi::signed_convert_to_float_d("d6", "x24"));
    asm.push(abi::float_multiply_d("d0", "d6", "d4")); // px
    asm.push(abi::signed_convert_to_float_d("d7", "x23"));
    asm.push(abi::float_multiply_d("d1", "d7", "d5")); // py
    asm.push(abi::move_register("x2", "x25")); // attrs
    asm.push(abi::move_register("x1", "x28")); // drawAtPoint:withAttributes: sel
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("draw_col_next"));
    asm.push(abi::add_immediate("x24", "x24", 1));
    asm.push(abi::branch("draw_col"));
    asm.push(abi::label("draw_row_next"));
    asm.push(abi::add_immediate("x23", "x23", 1));
    asm.push(abi::branch("draw_row"));

    asm.push(abi::label("draw_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.drawRect".to_string(),
        symbol: TERM_VIEW_DRAW_RECT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// `void _mfb_macapp_term_init(id termView /*x0*/)`: size the TermView's cell
/// grid from the monospaced font metrics and the initial view frame, then
/// `calloc` the (zero-initialized = cleared) grid into the state struct held in
/// the view's extra bytes (plan-01-term.md §6.3). Called once from the bootstrap.
pub(super) fn emit_term_init_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_INIT_SYMBOL);
    // Frame: lr@0, x19(termView)@8, x20(state)@16, x21(font)@24, x22(scratch)@32,
    // cellW bits@40, cellH bits@48.
    let frame = 64;
    let (off_cw, off_lh) = (40, 48);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::move_register("x19", "x0")); // termView

    // state = calloc(1, TV_STATE_SIZE) — zero-initialized grid state struct.
    asm.push(abi::move_immediate("x0", "Integer", "1"));
    asm.push(abi::move_immediate(
        "x1",
        "Integer",
        &TV_STATE_SIZE.to_string(),
    ));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::move_register("x20", "x0")); // state struct ptr

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data("x21", CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, "d0", TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // font

    // cellW = [font maximumAdvancement].width (d0); spill bits.
    asm.load_selector(SEL_MAX_ADVANCEMENT.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_cw));

    // lm = [[NSLayoutManager alloc] init]; cellH = [lm defaultLineHeightForFont:font].
    asm.external_data("x22", CLASS_NS_LAYOUT_MANAGER, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0")); // layout manager
    asm.load_selector(SEL_DEFAULT_LINE_HEIGHT.0);
    asm.push(abi::move_register("x2", "x21")); // font
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_lh));

    // cols = floor(WIDTH / cellW); rows = floor(HEIGHT / cellH).
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_cw));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &TERM_VIEW_WIDTH.to_string(),
    ));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x9", "d0"));
    asm.push(abi::store_u64("x9", "x20", TV_COLS_OFFSET));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_lh));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &TERM_VIEW_HEIGHT.to_string(),
    ));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x9", "d0"));
    asm.push(abi::store_u64("x9", "x20", TV_ROWS_OFFSET));

    // Persist the cell pixel dimensions for drawRect: / cursor positioning.
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_cw));
    asm.push(abi::store_u64("x9", "x20", TV_CELL_W_OFFSET));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_lh));
    asm.push(abi::store_u64("x9", "x20", TV_CELL_H_OFFSET));

    // cells = calloc(rows*cols, CELL_SIZE) — zero-initialized = cleared grid.
    asm.push(abi::load_u64("x9", "x20", TV_COLS_OFFSET));
    asm.push(abi::load_u64("x10", "x20", TV_ROWS_OFFSET));
    asm.push(abi::multiply_registers("x0", "x9", "x10"));
    asm.push(abi::move_immediate("x1", "Integer", &CELL_SIZE.to_string()));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::store_u64("x0", "x20", TV_CELLS_OFFSET));

    // cursor (0,0; calloc already zeroed); cursor visible; current fg = white
    // (bg/bold/underline default to 0 from calloc).
    asm.push(abi::move_immediate("x9", "Integer", "1"));
    asm.push(abi::store_u64("x9", "x20", TV_CURSOR_VISIBLE_OFFSET));
    asm.push(abi::move_immediate("x9", "Integer", TERM_DEFAULT_FG_PACKED));
    asm.push(abi::store_u64("x9", "x20", TV_CUR_FG_OFFSET));

    // objc_setAssociatedObject(termView, &TVSTATE_KEY, state, ASSIGN)
    asm.push(abi::move_register("x0", "x19"));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.push(abi::move_register("x2", "x20"));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.init".to_string(),
        symbol: TERM_INIT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// `void _mfb_macapp_term_clear(id termView /*x0*/)`: zero every grid cell (the
/// cleared-to-background-black, blank-glyph state) and home the cursor. Pure data
/// mutation on our own heap, safe from the worker thread (plan-01-term.md §6.4).
pub(super) fn emit_term_clear_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_CLEAR_SYMBOL);
    // Frame: lr@0, x19(state, after spilling the caller's arena base)@8.
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));

    // state = objc_getAssociatedObject(termView, &TVSTATE_KEY)  (x0 = termView)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x19", "x0")); // state struct ptr
    asm.push(abi::compare_immediate("x19", "0"));
    asm.push(abi::branch_eq("clr_done")); // no state attached yet

    // bzero(cells, rows*cols*CELL_SIZE) when a grid is allocated.
    asm.push(abi::load_u64("x9", "x19", TV_CELLS_OFFSET));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("clr_cursor"));
    asm.push(abi::load_u64("x10", "x19", TV_ROWS_OFFSET));
    asm.push(abi::load_u64("x11", "x19", TV_COLS_OFFSET));
    asm.push(abi::multiply_registers("x10", "x10", "x11"));
    asm.push(abi::shift_left_immediate("x10", "x10", 4)); // * CELL_SIZE (16)
    asm.push(abi::move_register("x0", "x9"));
    asm.push(abi::move_register("x1", "x10"));
    asm.call_external("_bzero", LIB_SYSTEM);

    asm.push(abi::label("clr_cursor"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x19", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::store_u64("x9", "x19", TV_CURSOR_COL_OFFSET));

    asm.push(abi::label("clr_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.clear".to_string(),
        symbol: TERM_CLEAR_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// `void _mfb_macapp_term_scroll(void *state /*x0*/)`: scroll the grid up one row
/// (memmove rows 1.. to 0.., then clear the new bottom row). Main-thread only.
pub(super) fn emit_term_scroll_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_SCROLL_SYMBOL);
    // Frame: lr@0, x19(rowBytes)@8, x20(cells)@16, x21(rows)@24.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));

    asm.push(abi::load_u64("x20", "x0", TV_CELLS_OFFSET)); // cells
    asm.push(abi::load_u64("x21", "x0", TV_ROWS_OFFSET)); // rows
    asm.push(abi::load_u64("x9", "x0", TV_COLS_OFFSET)); // cols
    asm.push(abi::shift_left_immediate("x19", "x9", 4)); // rowBytes = cols*CELL_SIZE

    // memmove(cells, cells + rowBytes, (rows-1)*rowBytes)
    asm.push(abi::subtract_immediate("x9", "x21", 1));
    asm.push(abi::multiply_registers("x2", "x9", "x19")); // len
    asm.push(abi::move_register("x0", "x20")); // dst
    asm.push(abi::add_registers("x1", "x20", "x19")); // src
    asm.call_external("_memmove", LIB_SYSTEM);

    // bzero(cells + (rows-1)*rowBytes, rowBytes) — clear the new bottom row.
    asm.push(abi::subtract_immediate("x9", "x21", 1));
    asm.push(abi::multiply_registers("x9", "x9", "x19"));
    asm.push(abi::add_registers("x0", "x20", "x9"));
    asm.push(abi::move_register("x1", "x19"));
    asm.call_external("_bzero", LIB_SYSTEM);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.scroll".to_string(),
        symbol: TERM_SCROLL_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// IMP for `TermView mfbWriteString:` (`void mfbWriteString:(id self, SEL _cmd,
/// NSString *str)`): write `str` into the grid at the cursor using the current
/// attributes, honouring `\n`/`\r`/`\t`, wrapping at the right edge and scrolling
/// at the bottom (plan-01-term.md §4.8). Main-thread only (invoked via
/// performSelectorOnMainThread), so grid mutation is serialized in program order
/// with the other surface ops (§6.4). The write does **not** request a redraw —
/// the surface repaints only on the next present (`term::sync`/`io::flush`), so
/// redraw is present-driven (plan-35-D §3, mandatory present).
pub(super) fn emit_term_write_string_helper() -> CodeFunction {
    let mut asm = Asm::new(MFB_WRITE_STRING_SYMBOL);
    // Frame: lr@0, x19(self)@8, x20(str)@16, x21(state)@24, x22(cells)@32,
    // x23(i)@40, x24(n)@48, x25(cols)@56, x26(rows)@64, x27(char)@72.
    let frame = 96;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x27", 72),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register("x19", "x0")); // self
    asm.push(abi::move_register("x20", "x2")); // str

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0"));
    asm.push(abi::compare_immediate("x21", "0"));
    asm.push(abi::branch_eq("w_done"));
    asm.push(abi::load_u64("x22", "x21", TV_CELLS_OFFSET)); // cells
    asm.push(abi::compare_immediate("x22", "0"));
    asm.push(abi::branch_eq("w_done"));
    asm.push(abi::load_u64("x25", "x21", TV_COLS_OFFSET));
    asm.push(abi::load_u64("x26", "x21", TV_ROWS_OFFSET));

    // n = [str length]; i = 0
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0"));
    asm.push(abi::move_immediate("x23", "Integer", "0"));

    asm.push(abi::label("w_loop"));
    asm.push(abi::compare_registers("x23", "x24"));
    asm.push(abi::branch_ge("w_done"));
    // c = [str characterAtIndex:i]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x27", "x0")); // char code

    asm.push(abi::compare_immediate("x27", "10")); // \n
    asm.push(abi::branch_eq("w_newline"));
    asm.push(abi::compare_immediate("x27", "13")); // \r
    asm.push(abi::branch_eq("w_cr"));
    asm.push(abi::compare_immediate("x27", "9")); // \t
    asm.push(abi::branch_eq("w_tab"));

    // printable: wrap if cursor_col >= cols
    asm.push(abi::load_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::compare_registers("x9", "x25"));
    asm.push(abi::branch_lt("w_col_ok"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::add_immediate("x10", "x10", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::label("w_col_ok"));
    // scroll if cursor_row >= rows
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::compare_registers("x10", "x26"));
    asm.push(abi::branch_lt("w_row_ok"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate("x10", "x26", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::label("w_row_ok"));
    // cell = cells + (row*cols + col)*CELL_SIZE
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::load_u64("x11", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::multiply_registers("x12", "x10", "x25"));
    asm.push(abi::add_registers("x12", "x12", "x11"));
    asm.push(abi::shift_left_immediate("x12", "x12", 4));
    asm.push(abi::add_registers("x12", "x22", "x12")); // cell ptr
    asm.push(abi::store_u32("x27", "x12", CELL_GLYPH_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_FG_OFFSET));
    asm.push(abi::store_u32("x13", "x12", CELL_FG_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_BG_OFFSET));
    asm.push(abi::store_u32("x13", "x12", CELL_BG_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_BOLD_OFFSET));
    asm.push(abi::store_u8("x13", "x12", CELL_BOLD_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_UNDERLINE_OFFSET));
    asm.push(abi::store_u8("x13", "x12", CELL_UNDERLINE_OFFSET));
    // cursor_col++
    asm.push(abi::load_u64("x11", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::add_immediate("x11", "x11", 1));
    asm.push(abi::store_u64("x11", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::branch("w_next"));

    // \n: col = 0, row++ (scroll if needed)
    asm.push(abi::label("w_newline"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::add_immediate("x10", "x10", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::compare_registers("x10", "x26"));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate("x10", "x26", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::branch("w_next"));

    // \r: col = 0
    asm.push(abi::label("w_cr"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::branch("w_next"));

    // \t: col = (col & ~3) + 4, wrapping to a new line if it runs off the edge
    asm.push(abi::label("w_tab"));
    asm.push(abi::load_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::shift_right_immediate("x9", "x9", 2));
    asm.push(abi::shift_left_immediate("x9", "x9", 2));
    asm.push(abi::add_immediate("x9", "x9", 4));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::compare_registers("x9", "x25"));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::add_immediate("x10", "x10", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::compare_registers("x10", "x26"));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate("x10", "x26", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));

    asm.push(abi::label("w_next"));
    asm.push(abi::add_immediate("x23", "x23", 1));
    asm.push(abi::branch("w_loop"));

    // Grid mutation is complete. Redraw is present-driven (plan-35-D §3): the
    // surface repaints only on the next `term::sync`/`io::flush`, never per write,
    // so a program that draws without a following present shows nothing new
    // (mandatory present, plan-35 D1).
    asm.push(abi::label("w_done"));

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x27", 72),
    ] {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.writeString".to_string(),
        symbol: MFB_WRITE_STRING_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// IMP for TermView `acceptsFirstResponder` — returns YES so the surface can take
/// keyboard focus while TUI mode is active.
pub(super) fn emit_term_accepts_first_responder() -> CodeFunction {
    let mut asm = Asm::new(TERM_ACCEPTS_FR_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x0", "Integer", "1")); // YES
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.term.acceptsFirstResponder".to_string(),
        symbol: TERM_ACCEPTS_FR_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Boolean".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// IMP for TermView `keyDown:` (`void keyDown:(id self, SEL, NSEvent *event)`):
/// the TUI-surface analogue of the transcript's keyDown: (plan-01-term.md §4.8 /
/// §3 — input stays an `io::` concern). Raw mode writes the key's UTF-8 to the
/// window input pipe immediately; line mode buffers until Return then delivers
/// the line, echoing typed characters into the surface itself. Runs on the main
/// thread.
pub(super) fn emit_term_key_down_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_KEY_DOWN_SYMBOL);
    // Frame: lr@0, x19(self)@8, x20(app)@16, x21(chars/cstr)@24,
    // x22(write remainder)@32, x23(event/wfd/scratch)@40, x24(char/scratch)@48,
    // x25(input line)@56, x26(input mode)@64, newline byte@72.
    let frame = 96;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        // x22 carries `tkd_commit`'s remaining-byte count across `_write`
        // (bug-241). Unlike the `kd_*` sibling this helper had no other use for
        // it, so it must be saved here before being clobbered.
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register("x19", "x0")); // self (TermView)
    asm.push(abi::move_register("x23", "x2")); // event

    // chars = [event characters]; if [chars length] == 0 -> done
    asm.load_selector(SEL_CHARACTERS.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // chars
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("tkd_done"));
    // c = [chars characterAtIndex:0]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0")); // char code

    // app, input line buffer, input mode.
    asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // app
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", INPUT_LINE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // input line buffer
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", INPUT_MODE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0")); // input mode

    // Dispatch on the key.
    asm.push(abi::compare_immediate("x26", INPUT_MODE_RAW_NO_ECHO));
    asm.push(abi::branch_eq("tkd_raw"));
    asm.push(abi::compare_immediate("x24", "13")); // CR
    asm.push(abi::branch_eq("tkd_commit"));
    asm.push(abi::compare_immediate("x24", "10")); // LF
    asm.push(abi::branch_eq("tkd_commit"));
    asm.push(abi::compare_immediate("x24", "3")); // Enter
    asm.push(abi::branch_eq("tkd_commit"));
    asm.push(abi::compare_immediate("x24", "127")); // Delete
    asm.push(abi::branch_eq("tkd_backspace"));
    asm.push(abi::compare_immediate("x24", "8")); // Backspace
    asm.push(abi::branch_eq("tkd_backspace"));

    // Default: [inputLine appendString:chars]; echo to the surface for io.input.
    asm.load_selector(SEL_APPEND_STRING.0);
    asm.push(abi::move_register("x2", "x21"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("tkd_done"));
    // [self mfbWriteString:chars]
    asm.load_selector(SEL_MFB_WRITE_STRING.0);
    asm.push(abi::move_register("x2", "x21"));
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("tkd_done"));

    // Commit: deliver the buffered line + newline to the pipe; echo a newline.
    asm.push(abi::label("tkd_commit"));
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // UTF-8 bytes of the line
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::move_register("x22", "x0")); // bytes still to deliver
    // Deliver the whole line, resuming after a partial write (bug-241) — see
    // `kd_commit`, which this mirrors.
    asm.push(abi::label("tkd_commit_write"));
    asm.push(abi::compare_immediate("x22", "0"));
    asm.push(abi::branch_eq("tkd_commit_newline"));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::move_register("x1", "x21"));
    asm.push(abi::move_register("x2", "x22"));
    asm.call_external("_write", LIB_SYSTEM);
    // O_NONBLOCK write end (bug-114): on -1/EAGAIN (pipe full, worker not
    // reading) give up on the line rather than block the UI thread; skip the
    // trailing newline write so a partial line is never terminated as a whole
    // one, and fall through to echo + clear. `<= 0` also makes the loop provably
    // terminate — each pass delivers at least one byte or leaves.
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_le("tkd_commit_echo"));
    asm.push(abi::add_registers("x21", "x21", "x0"));
    asm.push(abi::subtract_registers("x22", "x22", "x0"));
    asm.push(abi::branch("tkd_commit_write"));
    asm.push(abi::label("tkd_commit_newline"));
    asm.push(abi::move_immediate("x9", "Integer", "10"));
    asm.push(abi::store_u8("x9", abi::stack_pointer(), 72));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 72));
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::label("tkd_commit_echo"));
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("tkd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, "x21", STR_NEWLINE.0);
    asm.push(abi::move_register("x24", "x0")); // "\n" (callee-saved across load_selector)
    asm.load_selector(SEL_MFB_WRITE_STRING.0);
    asm.push(abi::move_register("x2", "x24"));
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("tkd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, "x21", STR_EMPTY.0);
    asm.push(abi::move_register("x24", "x0")); // empty string
    asm.load_selector(SEL_SET_STRING.0);
    asm.push(abi::move_register("x2", "x24"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("tkd_done"));

    // Backspace: drop the last character from the buffer.
    asm.push(abi::label("tkd_backspace"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("tkd_done"));
    asm.push(abi::move_register("x23", "x0")); // buffer length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", "x23", 1));
    asm.push(abi::move_immediate("x3", "Integer", "1"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("tkd_done"));

    // Raw read mode: write this key's UTF-8 to the input pipe; no echo/buffering.
    asm.push(abi::label("tkd_raw"));
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // UTF-8 bytes for chars
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("tkd_done"));
    asm.push(abi::move_register("x2", "x0"));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::move_register("x1", "x21"));
    asm.call_external("_write", LIB_SYSTEM);

    asm.push(abi::label("tkd_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
    ] {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.keyDown".to_string(),
        symbol: TERM_KEY_DOWN_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// IMP for TermView `setFrameSize:` (`void setFrameSize:(NSSize newSize)`; self
/// x0, `_cmd` x1, width d0, height d1): the live-window-resize hook (plan-35-D
/// Phase 2). Calls `super` to actually resize the view, then recomputes
/// `cols = floor(w/cellW)` / `rows = floor(h/cellH)` from the cached cell
/// metrics, reallocs the `TermCell[]` grid preserving the top-left overlap,
/// updates `TVSTATE` rows/cols, clamps the cursor, and forces a full redraw.
/// `term::terminalSize` reads `TV_ROWS`/`TV_COLS`, so a program re-querying its
/// size sees the new extent. AppKit geometry changes run on the main thread, the
/// same thread as `drawRect:` and the marshaled grid writes, so the realloc
/// cannot tear a concurrent draw.
pub(super) fn emit_term_set_frame_size_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_SET_FRAME_SIZE_SYMBOL);
    // Frame: lr@0, x19(self)@8, x20(state)@16, x21(oldCells)@24, x22(oldRows)@32,
    // x23(oldCols)@40, x24(newRows)@48, x25(newCols)@56, x26(newCells)@64,
    // x27(loop r)@72, width bits@80, height bits@88, objc_super{receiver@96,
    // super_class@104}, minRows@112, minCols@120.
    let frame = 128;
    let (off_w, off_h) = (80, 88);
    let (off_super_recv, off_super_cls) = (96, 104);
    let (off_min_rows, off_min_cols) = (112, 120);
    let saved: [(&str, usize); 9] = [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x27", 72),
    ];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in saved {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register("x19", "x0")); // self
    // Spill the NSSize args (d0 = width, d1 = height); the super call clobbers them.
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_w));
    asm.push(abi::float_move_x_from_d("x9", "d1"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_h));

    // [super setFrameSize:newSize] — actually resize the NSView. Build the
    // objc_super { receiver = self; super_class = NSView } record on the stack.
    asm.push(abi::store_u64("x19", abi::stack_pointer(), off_super_recv));
    asm.external_data("x9", CLASS_NS_VIEW, LIB_APPKIT);
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_super_cls));
    asm.load_selector(SEL_SET_FRAME_SIZE.0); // sel -> x1 (clobbers x0)
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), off_super_recv)); // &super
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_w));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_h));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.call_external("_objc_msgSendSuper", LIB_OBJC);

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY); nil -> no grid yet.
    asm.push(abi::move_register("x0", "x19"));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0"));
    asm.push(abi::compare_immediate("x20", "0"));
    asm.push(abi::branch_eq("sfs_done"));

    // newCols = floor(width / cellW); newRows = floor(height / cellH); each >= 1.
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_w));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::load_u64("x9", "x20", TV_CELL_W_OFFSET));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x25", "d0")); // newCols
    asm.push(abi::compare_immediate("x25", "1"));
    asm.push(abi::branch_ge("sfs_cols_ok"));
    asm.push(abi::move_immediate("x25", "Integer", "1"));
    asm.push(abi::label("sfs_cols_ok"));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_h));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::load_u64("x9", "x20", TV_CELL_H_OFFSET));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x24", "d0")); // newRows
    asm.push(abi::compare_immediate("x24", "1"));
    asm.push(abi::branch_ge("sfs_rows_ok"));
    asm.push(abi::move_immediate("x24", "Integer", "1"));
    asm.push(abi::label("sfs_rows_ok"));

    // old geometry.
    asm.push(abi::load_u64("x21", "x20", TV_CELLS_OFFSET)); // oldCells
    asm.push(abi::load_u64("x22", "x20", TV_ROWS_OFFSET)); // oldRows
    asm.push(abi::load_u64("x23", "x20", TV_COLS_OFFSET)); // oldCols

    // Unchanged geometry -> nothing to do (AppKit already marks the resize dirty).
    asm.push(abi::compare_registers("x24", "x22"));
    asm.push(abi::branch_ne("sfs_resize"));
    asm.push(abi::compare_registers("x25", "x23"));
    asm.push(abi::branch_eq("sfs_done"));
    asm.push(abi::label("sfs_resize"));

    // newCells = calloc(newRows*newCols, CELL_SIZE); leave the grid intact on OOM.
    asm.push(abi::multiply_registers("x0", "x24", "x25"));
    asm.push(abi::move_immediate("x1", "Integer", &CELL_SIZE.to_string()));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::move_register("x26", "x0")); // newCells
    asm.push(abi::compare_immediate("x26", "0"));
    asm.push(abi::branch_eq("sfs_done"));

    // Preserve the top-left overlap: for r in 0..min(oldRows,newRows) copy
    // min(oldCols,newCols) cells (row strides differ, so copy row by row).
    asm.push(abi::move_register("x9", "x22")); // minRows = min(oldRows, newRows)
    asm.push(abi::compare_registers("x22", "x24"));
    asm.push(abi::branch_le("sfs_minrows"));
    asm.push(abi::move_register("x9", "x24"));
    asm.push(abi::label("sfs_minrows"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_min_rows));
    asm.push(abi::move_register("x9", "x23")); // minCols = min(oldCols, newCols)
    asm.push(abi::compare_registers("x23", "x25"));
    asm.push(abi::branch_le("sfs_mincols"));
    asm.push(abi::move_register("x9", "x25"));
    asm.push(abi::label("sfs_mincols"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_min_cols));
    // No old grid -> nothing to copy.
    asm.push(abi::compare_immediate("x21", "0"));
    asm.push(abi::branch_eq("sfs_copy_done"));

    asm.push(abi::move_immediate("x27", "Integer", "0")); // r = 0
    asm.push(abi::label("sfs_copy_loop"));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_min_rows));
    asm.push(abi::compare_registers("x27", "x9"));
    asm.push(abi::branch_ge("sfs_copy_done"));
    // dst = newCells + (r*newCols)*CELL_SIZE
    asm.push(abi::multiply_registers("x9", "x27", "x25"));
    asm.push(abi::shift_left_immediate("x9", "x9", 4));
    asm.push(abi::add_registers("x0", "x26", "x9"));
    // src = oldCells + (r*oldCols)*CELL_SIZE
    asm.push(abi::multiply_registers("x10", "x27", "x23"));
    asm.push(abi::shift_left_immediate("x10", "x10", 4));
    asm.push(abi::add_registers("x1", "x21", "x10"));
    // len = minCols * CELL_SIZE
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_min_cols));
    asm.push(abi::shift_left_immediate("x2", "x9", 4));
    asm.call_external("_memcpy", LIB_SYSTEM);
    asm.push(abi::add_immediate("x27", "x27", 1));
    asm.push(abi::branch("sfs_copy_loop"));
    asm.push(abi::label("sfs_copy_done"));

    // Publish the new grid + geometry, then free the old buffer.
    asm.push(abi::store_u64("x26", "x20", TV_CELLS_OFFSET));
    asm.push(abi::store_u64("x24", "x20", TV_ROWS_OFFSET));
    asm.push(abi::store_u64("x25", "x20", TV_COLS_OFFSET));
    asm.push(abi::compare_immediate("x21", "0"));
    asm.push(abi::branch_eq("sfs_freed"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_free", LIB_SYSTEM);
    asm.push(abi::label("sfs_freed"));

    // Clamp the cursor into the new extent.
    asm.push(abi::load_u64("x9", "x20", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::compare_registers("x9", "x24"));
    asm.push(abi::branch_lt("sfs_cur_row_ok"));
    asm.push(abi::subtract_immediate("x9", "x24", 1));
    asm.push(abi::store_u64("x9", "x20", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::label("sfs_cur_row_ok"));
    asm.push(abi::load_u64("x9", "x20", TV_CURSOR_COL_OFFSET));
    asm.push(abi::compare_registers("x9", "x25"));
    asm.push(abi::branch_lt("sfs_cur_col_ok"));
    asm.push(abi::subtract_immediate("x9", "x25", 1));
    asm.push(abi::store_u64("x9", "x20", TV_CURSOR_COL_OFFSET));
    asm.push(abi::label("sfs_cur_col_ok"));

    // Full redraw of the resized surface. setFrameSize: runs on the main thread,
    // so message the view directly (no marshaling needed).
    asm.load_selector(SEL_SET_NEEDS_DISPLAY.0);
    asm.push(abi::move_immediate("x2", "Integer", "1")); // YES
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("sfs_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.setFrameSize".to_string(),
        symbol: TERM_SET_FRAME_SIZE_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;

    /// Index of the first `label` instruction with the given name.
    fn label_index(ins: &[CodeInstruction], name: &str) -> usize {
        ins.iter()
            .position(|i| i.op == CodeOp::Label && i.get("name") == Some(name))
            .unwrap_or_else(|| panic!("label {name:?} not found"))
    }

    /// bug-46: in the transcript keyDown handler the line-echo backspace block
    /// must terminate with an unconditional branch to `kd_done`. Without it,
    /// control falls through into the `kd_raw` block and writes the Backspace
    /// key's own UTF-8 byte (DEL/BS) into the input pipe. The instruction
    /// immediately preceding the `kd_raw` label must be `b kd_done`.
    #[test]
    fn kd_backspace_does_not_fall_through_into_kd_raw() {
        let func = emit_key_down_helper();
        let ins = &func.instructions;

        let bs = label_index(ins, "kd_backspace");
        let raw = label_index(ins, "kd_raw");
        assert!(bs < raw, "kd_backspace must precede kd_raw");

        let last = &ins[raw - 1];
        assert_eq!(
            last.op,
            CodeOp::Branch,
            "kd_backspace must end with an unconditional branch (found {:?}), \
             else it falls through into kd_raw and leaks the Backspace byte",
            last.op
        );
        assert_eq!(
            last.get("target"),
            Some("kd_done"),
            "the terminating branch must target kd_done"
        );
    }

    /// Sibling anchor: the structurally identical TUI handler was already
    /// correct and is the template for the fix above.
    #[test]
    fn tkd_backspace_does_not_fall_through_into_tkd_raw() {
        let func = emit_term_key_down_helper();
        let ins = &func.instructions;

        let raw = label_index(ins, "tkd_raw");
        let last = &ins[raw - 1];
        assert_eq!(last.op, CodeOp::Branch);
        assert_eq!(last.get("target"), Some("tkd_done"));
    }
}
