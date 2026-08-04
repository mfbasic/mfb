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
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
    asm.push(abi::store_u64(abi::LOCAL[4], abi::stack_pointer(), 40));
    asm.push(abi::store_u64(abi::LOCAL[5], abi::stack_pointer(), 48));
    asm.push(abi::store_u64(abi::LOCAL[6], abi::stack_pointer(), 56));
    asm.push(abi::store_u64(abi::LOCAL[7], abi::stack_pointer(), 64));
    asm.push(abi::move_register(abi::LOCAL[0], "x0")); // self (text view)
    asm.push(abi::move_register(abi::LOCAL[4], "x2")); // event

    // chars = [event characters]; if [chars length] == 0 (modifier-only) -> done
    asm.load_selector(SEL_CHARACTERS.0);
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // chars
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    // c = [chars characterAtIndex:0]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[5], "x0")); // char code

    // app, input line buffer, text storage.
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // app
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", INPUT_LINE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[6], "x0")); // input line buffer
    asm.load_selector(SEL_TEXT_STORAGE.0);
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // text storage
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", INPUT_MODE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[7], "x0")); // input mode

    // Dispatch on the key.
    asm.push(abi::compare_immediate(
        abi::LOCAL[7],
        INPUT_MODE_RAW_NO_ECHO,
    ));
    asm.push(abi::branch_eq("kd_raw"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "13")); // CR
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "10")); // LF
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "3")); // Enter
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "127")); // Delete
    asm.push(abi::branch_eq("kd_backspace"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "8")); // Backspace
    asm.push(abi::branch_eq("kd_backspace"));

    // Default: [inputLine appendString:chars]; echo only for io.input mode.
    asm.load_selector(SEL_APPEND_STRING.0);
    asm.push(abi::move_register("x2", abi::LOCAL[2]));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::LOCAL[7], INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_done"));
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
    asm.push(abi::move_register("x1", abi::LOCAL[2]));
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::branch("kd_done"));

    // Commit: deliver the buffered line + newline to the pipe, echo a newline,
    // clear the buffer.
    asm.push(abi::label("kd_commit"));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[4], "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // UTF-8 bytes of the line
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // bytes still to deliver
                                                       // Deliver the whole line, resuming after a partial write (bug-241). A pipe
                                                       // write is atomic only up to PIPE_BUF, so a line longer than that splits
                                                       // when the reader is behind; writing the remainder off and still sending the
                                                       // newline below would hand the program a truncated line as a complete one.
                                                       // x22 (text storage) is dead on this path — only `kd_backspace` reads it and
                                                       // `kd_commit` branches straight to `kd_done` — so it carries the remaining
                                                       // count across the `_write` calls, which clobber x0-x17.
    asm.push(abi::label("kd_commit_write"));
    asm.push(abi::compare_immediate(abi::LOCAL[3], "0"));
    asm.push(abi::branch_eq("kd_commit_newline"));
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.push(abi::move_register("x1", abi::LOCAL[2]));
    asm.push(abi::move_register("x2", abi::LOCAL[3]));
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
    asm.push(abi::add_registers(abi::LOCAL[2], abi::LOCAL[2], "x0"));
    asm.push(abi::subtract_registers(abi::LOCAL[3], abi::LOCAL[3], "x0"));
    asm.push(abi::branch("kd_commit_write"));
    asm.push(abi::label("kd_commit_newline"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "10"));
    asm.push(abi::store_u8(abi::SCRATCH[0], abi::stack_pointer(), 72));
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 72));
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::label("kd_commit_echo"));
    asm.push(abi::compare_immediate(abi::LOCAL[7], INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, abi::LOCAL[2], STR_NEWLINE.0);
    asm.push(abi::move_register("x1", "x0"));
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::label("kd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, abi::LOCAL[2], STR_EMPTY.0);
    asm.push(abi::move_register(abi::LOCAL[5], "x0")); // empty string (callee-saved; survives
                                                       // the sel_registerName in load_selector)
    asm.load_selector(SEL_SET_STRING.0);
    asm.push(abi::move_register("x2", abi::LOCAL[5]));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("kd_done"));

    // Backspace: drop the last character from the buffer and the transcript.
    asm.push(abi::label("kd_backspace"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register(abi::LOCAL[4], "x0")); // buffer length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", abi::LOCAL[4], 1)); // range.location = len - 1
    asm.push(abi::move_immediate("x3", "Integer", "1")); // range.length = 1
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::LOCAL[7], INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_done"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register(abi::LOCAL[4], "x0")); // transcript length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", abi::LOCAL[4], 1));
    asm.push(abi::move_immediate("x3", "Integer", "1"));
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // Terminate the line-echo backspace path here; without this the block falls
    // through into `kd_raw` and injects the DEL/BS key byte into the input pipe.
    // Mirrors `tkd_backspace`'s terminating branch (bug-46).
    asm.push(abi::branch("kd_done"));

    // Raw read mode: write this key event's UTF-8 bytes to the input pipe now,
    // with no transcript echo and no line buffering.
    asm.push(abi::label("kd_raw"));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[4], "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // UTF-8 bytes for chars
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register("x2", "x0"));
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.push(abi::move_register("x1", abi::LOCAL[2]));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::branch("kd_done"));

    asm.push(abi::label("kd_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
    asm.push(abi::load_u64(abi::LOCAL[4], abi::stack_pointer(), 40));
    asm.push(abi::load_u64(abi::LOCAL[5], abi::stack_pointer(), 48));
    asm.push(abi::load_u64(abi::LOCAL[6], abi::stack_pointer(), 56));
    asm.push(abi::load_u64(abi::LOCAL[7], abi::stack_pointer(), 64));
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
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "255"));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[4],
        abi::SCRATCH[1],
    )); // 255.0 divisor
    asm.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[4],
    )); // r
    asm.push(abi::shift_right_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[2],
        8,
    ));
    asm.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[4],
    )); // g
    asm.push(abi::shift_right_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[2],
        16,
    ));
    asm.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[2],
        abi::FP_SCRATCH[2],
        abi::FP_SCRATCH[4],
    )); // b
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[0],
    )); // alpha 1.0
    asm.push(abi::move_register("x0", abi::LOCAL[7])); // NSColor class
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
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
        (abi::LOCAL[6], 56),
        (abi::LOCAL[7], 64),
        (abi::LOCAL[8], 72),
        (abi::LOCAL[9], 80),
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
        (abi::FP_SCRATCH[0], off_rx),
        (abi::FP_SCRATCH[1], off_ry),
        (abi::FP_SCRATCH[2], off_rw),
        (abi::FP_SCRATCH[3], off_rh),
    ] {
        asm.push(abi::float_move_x_from_d(abi::SCRATCH[0], reg));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), off));
    }

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)  (self in x0)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], "x0")); // state (or nil)

    // Pre-resolve the colour primitives so the per-cell colour build avoids any
    // sel_registerName (which would clobber the d0..d3 component arguments).
    asm.external_data(abi::LOCAL[7], CLASS_NS_COLOR, LIB_APPKIT); // NSColor class
    asm.load_selector(SEL_COLOR_WITH_RGBA.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_color_sel));
    asm.load_selector(SEL_SET.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_set_sel));

    // Fill the dirty rect black: [[NSColor blackColor] set]; NSRectFill(rect).
    asm.load_selector(SEL_BLACK_COLOR.0);
    asm.push(abi::move_register("x0", abi::LOCAL[7]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_set_sel));
    asm.call_external("_objc_msgSend", LIB_OBJC); // [black set]
    for (reg, off) in [
        (abi::FP_SCRATCH[0], off_rx),
        (abi::FP_SCRATCH[1], off_ry),
        (abi::FP_SCRATCH[2], off_rw),
        (abi::FP_SCRATCH[3], off_rh),
    ] {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off));
        asm.push(abi::float_move_d_from_x(reg, abi::SCRATCH[0]));
    }
    asm.call_external(NS_RECT_FILL, LIB_APPKIT);

    // No state / no grid yet -> nothing more to paint.
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq("draw_done"));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::LOCAL[0], TV_CELLS_OFFSET)); // cells
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq("draw_done"));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::LOCAL[0], TV_ROWS_OFFSET));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::LOCAL[0], TV_COLS_OFFSET));

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data(abi::LOCAL[6], CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, abi::FP_SCRATCH[0], TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[6], "x0")); // font

    // attrs = [NSMutableDictionary dictionary]; [attrs setObject:font forKey:NSFontAttributeName]
    // (the foreground colour key is set per cell below).
    asm.load_selector(SEL_DICTIONARY.0);
    asm.external_data("x0", CLASS_NS_MUTABLE_DICTIONARY, LIB_FOUNDATION);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[8], "x0")); // attrs dict (temp in x27)
    asm.load_selector(SEL_SET_OBJECT_FOR_KEY.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::move_register("x2", abi::LOCAL[6])); // font
    asm.external_data("x3", NS_FONT_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::move_register("x0", abi::LOCAL[8]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[6], abi::LOCAL[8])); // attrs dict -> x25

    // Pre-resolve drawAtPoint: (x28) + stringWithChars: (spilled); cache the
    // foreground-colour attribute key (an NSString global) on the stack.
    asm.load_selector(SEL_DRAW_AT_POINT.0);
    asm.push(abi::move_register(abi::LOCAL[9], "x1"));
    asm.load_selector(SEL_STRING_WITH_CHARS.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_swc_sel));
    asm.external_data("x3", NS_FOREGROUND_COLOR_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::store_u64("x3", abi::stack_pointer(), off_fgkey));

    // Bold/underline attribute values + keys (set/removed per cell below).
    // numberBold = [NSNumber numberWithDouble:-3.0]  (negative stroke width = faux bold)
    asm.load_selector(SEL_NUMBER_WITH_DOUBLE.0);
    asm.external_data("x0", CLASS_NS_NUMBER, LIB_FOUNDATION);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "3"));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_negate_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0])); // -3.0
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
    asm.push(abi::move_immediate(abi::LOCAL[4], "Integer", "0"));
    asm.push(abi::label("draw_row"));
    asm.push(abi::compare_registers(abi::LOCAL[4], abi::LOCAL[2]));
    asm.push(abi::branch_ge("draw_done"));
    asm.push(abi::move_immediate(abi::LOCAL[5], "Integer", "0"));
    asm.push(abi::label("draw_col"));
    asm.push(abi::compare_registers(abi::LOCAL[5], abi::LOCAL[3]));
    asm.push(abi::branch_ge("draw_row_next"));

    // cell = cells + (row*cols + col) * CELL_SIZE
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::LOCAL[4],
        abi::LOCAL[3],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::LOCAL[5],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        4,
    )); // * CELL_SIZE (16)
    asm.push(abi::add_registers(
        abi::LOCAL[8],
        abi::LOCAL[1],
        abi::SCRATCH[0],
    )); // cell ptr (callee-saved)

    // --- background: fill the cell rect when bg is non-black ---
    asm.push(abi::load_u32(
        abi::SCRATCH[2],
        abi::LOCAL[8],
        CELL_BG_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
    asm.push(abi::branch_eq("draw_skip_bg"));
    emit_color_from_packed(&mut asm, off_color_sel); // x0 = bg colour
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_set_sel));
    asm.call_external("_objc_msgSend", LIB_OBJC); // [bgColor set]
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        TV_CELL_W_OFFSET,
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[0],
    )); // cellW
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        TV_CELL_H_OFFSET,
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[0],
    )); // cellH
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[4],
        abi::LOCAL[5],
    ));
    asm.push(abi::float_multiply_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[4],
        abi::FP_SCRATCH[2],
    )); // px
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[5],
        abi::LOCAL[4],
    ));
    asm.push(abi::float_multiply_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[5],
        abi::FP_SCRATCH[3],
    )); // py
    asm.call_external(NS_RECT_FILL, LIB_APPKIT);
    asm.push(abi::label("draw_skip_bg"));

    // --- glyph: paint in the cell foreground colour when non-blank ---
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::LOCAL[8],
        CELL_GLYPH_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("draw_col_next"));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "32")); // space = blank
    asm.push(abi::branch_eq("draw_col_next"));
    // plan-70-D: a wide-trailing sentinel draws nothing — the wide primary's glyph
    // already spans this column (its background was filled above). Skip the glyph.
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        APP_WIDE_TRAIL,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_eq("draw_col_next"));
    // [attrs setObject:[color from cell.fg] forKey:NSForegroundColorAttributeName]
    asm.push(abi::load_u32(
        abi::SCRATCH[2],
        abi::LOCAL[8],
        CELL_FG_OFFSET,
    ));
    emit_color_from_packed(&mut asm, off_color_sel); // x0 = fg colour
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::move_register("x2", "x0")); // colour (x2 set after the sel load)
    asm.push(abi::load_u64("x3", abi::stack_pointer(), off_fgkey));
    asm.push(abi::move_register("x0", abi::LOCAL[6])); // attrs dict
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // bold: set/remove the faux-bold stroke-width attribute for this cell.
    asm.push(abi::load_u8(
        abi::SCRATCH[0],
        abi::LOCAL[8],
        CELL_BOLD_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("draw_bold_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_numbold));
    asm.push(abi::load_u64("x3", abi::stack_pointer(), off_strokekey));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("draw_bold_done"));
    asm.push(abi::label("draw_bold_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_removeobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_strokekey));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("draw_bold_done"));
    // underline: set/remove the underline-style attribute for this cell.
    asm.push(abi::load_u8(
        abi::SCRATCH[0],
        abi::LOCAL[8],
        CELL_UNDERLINE_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("draw_ul_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_setobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_numul));
    asm.push(abi::load_u64("x3", abi::stack_pointer(), off_ulkey));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("draw_ul_done"));
    asm.push(abi::label("draw_ul_off"));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_removeobj_sel));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), off_ulkey));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("draw_ul_done"));
    // s = [NSString stringWithCharacters:&units length:len]
    // plan-70-D: the cell glyph holds either a full Unicode scalar (inline) or a
    // pooled-cluster tag (top byte 0xC0) whose UTF-16 units live in this cell's EGC
    // pool slot. A BMP scalar is one UTF-16 unit; an astral scalar (>= U+10000) is
    // written back as its surrogate pair (two units) so it renders as one glyph
    // instead of tofu. The two u16 units pack into one u32 (hi in bytes 0-1, lo in
    // bytes 2-3, LE).
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::LOCAL[8],
        CELL_GLYPH_OFFSET,
    ));
    // plan-70-D Phase 2: a pooled multi-scalar cluster (combining marks, ZWJ emoji)
    // rebuilds its whole grapheme from the pool slot; lone scalars fall through to
    // the inline surrogate build below.
    asm.push(abi::shift_right_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        24,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "192")); // 0xC0 pooled tag
    asm.push(abi::branch_ne("draw_not_pooled"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        APP_GLYPH_POOLED_LEN_MASK,
    ));
    asm.push(abi::and_registers(
        abi::SCRATCH[4],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    )); // unit count
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::LOCAL[0],
        TV_POOL_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
    asm.push(abi::branch_eq("draw_col_next")); // no pool -> nothing to draw
    asm.push(abi::multiply_registers(
        abi::SCRATCH[3],
        abi::LOCAL[4],
        abi::LOCAL[3],
    )); // row*cols
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        abi::LOCAL[5],
    )); // +col
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        6,
    )); // *POOL(64)
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[3],
    )); // pool slot
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_swc_sel));
    asm.external_data("x0", CLASS_NS_STRING, LIB_FOUNDATION);
    asm.push(abi::move_register("x2", abi::SCRATCH[2])); // units buffer
    asm.push(abi::move_register("x3", abi::SCRATCH[4])); // unit count
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("draw_have_string"));
    asm.push(abi::label("draw_not_pooled"));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "65536")); // 0x10000
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_lt("draw_bmp"));
    // astral: cp -= 0x10000; hi = 0xD800 + (cp>>10); lo = 0xDC00 + (cp & 0x3FF)
    asm.push(abi::subtract_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::shift_right_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[0],
        10,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "55296"));
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    )); // hi
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "1023")); // 0x3FF
    asm.push(abi::and_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "56320"));
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        abi::SCRATCH[1],
    )); // lo
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        16,
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[2],
        abi::SCRATCH[3],
    )); // hi | lo<<16
    asm.push(abi::move_immediate(abi::SCRATCH[4], "Integer", "2"));
    asm.push(abi::branch("draw_len_set"));
    asm.push(abi::label("draw_bmp"));
    asm.push(abi::move_immediate(abi::SCRATCH[4], "Integer", "1"));
    asm.push(abi::label("draw_len_set"));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_glyph,
    ));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_swc_sel));
    asm.external_data("x0", CLASS_NS_STRING, LIB_FOUNDATION);
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), off_glyph));
    asm.push(abi::move_register("x3", abi::SCRATCH[4]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("draw_have_string")); // inline + pooled paths converge (x0 = string)
                                              // [s drawAtPoint:(col*cellW, row*cellH) withAttributes:attrs]
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        TV_CELL_W_OFFSET,
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[4],
        abi::SCRATCH[0],
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        TV_CELL_H_OFFSET,
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[5],
        abi::SCRATCH[0],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[6],
        abi::LOCAL[5],
    ));
    asm.push(abi::float_multiply_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[6],
        abi::FP_SCRATCH[4],
    )); // px
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[7],
        abi::LOCAL[4],
    ));
    asm.push(abi::float_multiply_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[7],
        abi::FP_SCRATCH[5],
    )); // py
    asm.push(abi::move_register("x2", abi::LOCAL[6])); // attrs
    asm.push(abi::move_register("x1", abi::LOCAL[9])); // drawAtPoint:withAttributes: sel
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("draw_col_next"));
    asm.push(abi::add_immediate(abi::LOCAL[5], abi::LOCAL[5], 1));
    asm.push(abi::branch("draw_col"));
    asm.push(abi::label("draw_row_next"));
    asm.push(abi::add_immediate(abi::LOCAL[4], abi::LOCAL[4], 1));
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
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
    asm.push(abi::move_register(abi::LOCAL[0], "x0")); // termView

    // state = calloc(1, TV_STATE_SIZE) — zero-initialized grid state struct.
    asm.push(abi::move_immediate("x0", "Integer", "1"));
    asm.push(abi::move_immediate(
        "x1",
        "Integer",
        &TV_STATE_SIZE.to_string(),
    ));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // state struct ptr

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data(abi::LOCAL[2], CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, abi::FP_SCRATCH[0], TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // font

    // cellW = [font maximumAdvancement].width (d0); spill bits.
    asm.load_selector(SEL_MAX_ADVANCEMENT.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[0],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_cw,
    ));

    // lm = [[NSLayoutManager alloc] init]; cellH = [lm defaultLineHeightForFont:font].
    asm.external_data(abi::LOCAL[3], CLASS_NS_LAYOUT_MANAGER, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // layout manager
    asm.load_selector(SEL_DEFAULT_LINE_HEIGHT.0);
    asm.push(abi::move_register("x2", abi::LOCAL[2])); // font
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[0],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_lh,
    ));

    // cols = floor(WIDTH / cellW); rows = floor(HEIGHT / cellH).
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_cw));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &TERM_VIEW_WIDTH.to_string(),
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::float_floor_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[0],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_COLS_OFFSET,
    ));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_lh));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &TERM_VIEW_HEIGHT.to_string(),
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::float_floor_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[0],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_ROWS_OFFSET,
    ));

    // Persist the cell pixel dimensions for drawRect: / cursor positioning.
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_cw));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CELL_W_OFFSET,
    ));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_lh));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CELL_H_OFFSET,
    ));

    // cells = calloc(rows*cols, CELL_SIZE) — zero-initialized = cleared grid.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_COLS_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[1],
        TV_ROWS_OFFSET,
    ));
    asm.push(abi::multiply_registers(
        "x0",
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::move_immediate("x1", "Integer", &CELL_SIZE.to_string()));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::store_u64("x0", abi::LOCAL[1], TV_CELLS_OFFSET));

    // plan-70-D Phase 2: pool = calloc(rows*cols, APP_POOL_BYTES_PER_CELL) — the
    // per-cell EGC byte arena for multi-scalar clusters.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_COLS_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[1],
        TV_ROWS_OFFSET,
    ));
    asm.push(abi::multiply_registers(
        "x0",
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::move_immediate(
        "x1",
        "Integer",
        &APP_POOL_BYTES_PER_CELL.to_string(),
    ));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::store_u64("x0", abi::LOCAL[1], TV_POOL_OFFSET));

    // cursor (0,0; calloc already zeroed); cursor visible; current fg = white
    // (bg/bold/underline default to 0 from calloc).
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_VISIBLE_OFFSET,
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        TERM_DEFAULT_FG_PACKED,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CUR_FG_OFFSET,
    ));

    // objc_setAssociatedObject(termView, &TVSTATE_KEY, state, ASSIGN)
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.push(abi::move_register("x2", abi::LOCAL[1]));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
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
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));

    // state = objc_getAssociatedObject(termView, &TVSTATE_KEY)  (x0 = termView)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], "x0")); // state struct ptr
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq("clr_done")); // no state attached yet

    // bzero(cells, rows*cols*CELL_SIZE) when a grid is allocated.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        TV_CELLS_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("clr_cursor"));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[0],
        TV_ROWS_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::LOCAL[0],
        TV_COLS_OFFSET,
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[2],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        4,
    )); // * CELL_SIZE (16)
    asm.push(abi::move_register("x0", abi::SCRATCH[0]));
    asm.push(abi::move_register("x1", abi::SCRATCH[1]));
    asm.call_external("_bzero", LIB_SYSTEM);

    asm.push(abi::label("clr_cursor"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        TV_CURSOR_COL_OFFSET,
    ));

    asm.push(abi::label("clr_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
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

/// IMP for `TermView mfbDrawLine:` (`void mfbDrawLine:(id self, SEL _cmd, id)`):
/// stamp the box-drawing glyph the worker resolved (`TV_DRAW_GLYPH`) across a run
/// of cells with the current attributes, honouring the parked parameters
/// (`TV_DRAW_FIXED`/`LO`/`HI`/`HORIZ`). Main-thread only (invoked via
/// performSelectorOnMainThread), so grid mutation is serialised with the other
/// surface ops. Mirrors the console `emit_draw_line`: the fixed coordinate off the
/// grid, or a span with no on-grid cell, draws nothing; endpoints may be given in
/// either order. The draw does **not** request a redraw — the surface repaints on
/// the next present (`term::sync`/`io::flush`), mandatory-present (plan-35-D §3).
pub(super) fn emit_term_draw_line_helper() -> CodeFunction {
    let mut asm = Asm::new(MFB_DRAW_LINE_SYMBOL);
    // Frame: lr@0, then callee-saved loop-invariants — state@8, cells@16, cols@24,
    // glyph@32, fixed@40, horiz@48, hi@56, pos@64. Transients live in SCRATCH
    // (no calls follow the single objc_getAssociatedObject, so they persist).
    let frame = 80;
    let state = abi::LOCAL[0];
    let cells = abi::LOCAL[1];
    let cols = abi::LOCAL[2];
    let glyph = abi::LOCAL[3];
    let fixed = abi::LOCAL[4];
    let horiz = abi::LOCAL[5];
    let hi = abi::LOCAL[6];
    let pos = abi::LOCAL[7];
    let rows = abi::SCRATCH[0];
    let lo = abi::SCRATCH[1];
    let fixed_bound = abi::SCRATCH[2];
    let span_bound = abi::SCRATCH[3];
    let tmp = abi::SCRATCH[4];
    let idx = abi::SCRATCH[0]; // reused after the pre-loop clamps (rows is dead)
    let cell = abi::SCRATCH[1];
    let attr = abi::SCRATCH[2];

    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in [
        (state, 8),
        (cells, 16),
        (cols, 24),
        (glyph, 32),
        (fixed, 40),
        (horiz, 48),
        (hi, 56),
        (pos, 64),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)  (x0 = self)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(state, "x0"));
    asm.push(abi::compare_immediate(state, "0"));
    asm.push(abi::branch_eq("dl_done"));
    asm.push(abi::load_u64(cells, state, TV_CELLS_OFFSET));
    asm.push(abi::compare_immediate(cells, "0"));
    asm.push(abi::branch_eq("dl_done"));
    asm.push(abi::load_u64(rows, state, TV_ROWS_OFFSET));
    asm.push(abi::load_u64(cols, state, TV_COLS_OFFSET));
    asm.push(abi::load_u64(glyph, state, TV_DRAW_GLYPH_OFFSET));
    asm.push(abi::load_u64(fixed, state, TV_DRAW_FIXED_OFFSET));
    asm.push(abi::load_u64(horiz, state, TV_DRAW_HORIZ_OFFSET));
    asm.push(abi::load_u64(lo, state, TV_DRAW_LO_OFFSET));
    asm.push(abi::load_u64(hi, state, TV_DRAW_HI_OFFSET));

    // fixed_bound / span_bound depend on the direction: horizontal spans columns
    // on a fixed row, vertical spans rows on a fixed column.
    asm.push(abi::compare_immediate(horiz, "0"));
    asm.push(abi::branch_eq("dl_vert"));
    asm.push(abi::move_register(fixed_bound, rows));
    asm.push(abi::move_register(span_bound, cols));
    asm.push(abi::branch("dl_bounds_done"));
    asm.push(abi::label("dl_vert"));
    asm.push(abi::move_register(fixed_bound, cols));
    asm.push(abi::move_register(span_bound, rows));
    asm.push(abi::label("dl_bounds_done"));

    // Fixed coordinate must be on the grid: [0, fixed_bound-1], else nothing.
    asm.push(abi::compare_immediate(fixed, "0"));
    asm.push(abi::branch_lt("dl_done"));
    asm.push(abi::compare_registers(fixed, fixed_bound));
    asm.push(abi::branch_ge("dl_done"));

    // Normalise the span so lo <= hi.
    asm.push(abi::compare_registers(lo, hi));
    asm.push(abi::branch_le("dl_span_ok"));
    asm.push(abi::move_register(tmp, lo));
    asm.push(abi::move_register(lo, hi));
    asm.push(abi::move_register(hi, tmp));
    asm.push(abi::label("dl_span_ok"));

    // Clamp lo up to 0 and hi down to span_bound-1; empty span → nothing.
    asm.push(abi::compare_immediate(lo, "0"));
    asm.push(abi::branch_ge("dl_lo_ok"));
    asm.push(abi::move_immediate(lo, "Integer", "0"));
    asm.push(abi::label("dl_lo_ok"));
    asm.push(abi::subtract_immediate(tmp, span_bound, 1));
    asm.push(abi::compare_registers(hi, tmp));
    asm.push(abi::branch_le("dl_hi_ok"));
    asm.push(abi::move_register(hi, tmp));
    asm.push(abi::label("dl_hi_ok"));
    asm.push(abi::compare_registers(lo, hi));
    asm.push(abi::branch_gt("dl_done"));

    // pos = lo..=hi, stamping the glyph + current attributes into each cell.
    asm.push(abi::move_register(pos, lo));
    asm.push(abi::label("dl_loop"));
    asm.push(abi::compare_registers(pos, hi));
    asm.push(abi::branch_gt("dl_done"));
    // idx = horiz ? fixed*cols + pos : pos*cols + fixed
    asm.push(abi::compare_immediate(horiz, "0"));
    asm.push(abi::branch_eq("dl_v_idx"));
    asm.push(abi::multiply_registers(idx, fixed, cols));
    asm.push(abi::add_registers(idx, idx, pos));
    asm.push(abi::branch("dl_idx_done"));
    asm.push(abi::label("dl_v_idx"));
    asm.push(abi::multiply_registers(idx, pos, cols));
    asm.push(abi::add_registers(idx, idx, fixed));
    asm.push(abi::label("dl_idx_done"));
    asm.push(abi::shift_left_immediate(idx, idx, 4)); // * CELL_SIZE (16)
    asm.push(abi::add_registers(cell, cells, idx));
    asm.push(abi::store_u32(glyph, cell, CELL_GLYPH_OFFSET));
    asm.push(abi::load_u64(attr, state, TV_CUR_FG_OFFSET));
    asm.push(abi::store_u32(attr, cell, CELL_FG_OFFSET));
    asm.push(abi::load_u64(attr, state, TV_CUR_BG_OFFSET));
    asm.push(abi::store_u32(attr, cell, CELL_BG_OFFSET));
    asm.push(abi::load_u64(attr, state, TV_CUR_BOLD_OFFSET));
    asm.push(abi::store_u8(attr, cell, CELL_BOLD_OFFSET));
    asm.push(abi::load_u64(attr, state, TV_CUR_UNDERLINE_OFFSET));
    asm.push(abi::store_u8(attr, cell, CELL_UNDERLINE_OFFSET));
    asm.push(abi::add_immediate(pos, pos, 1));
    asm.push(abi::branch("dl_loop"));

    asm.push(abi::label("dl_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        (state, 8),
        (cells, 16),
        (cols, 24),
        (glyph, 32),
        (fixed, 40),
        (horiz, 48),
        (hi, 56),
        (pos, 64),
    ] {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.drawLine".to_string(),
        symbol: MFB_DRAW_LINE_SYMBOL.to_string(),
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

/// Grid context + scratch registers for the `mfbDrawBox:` stampers. `state`
/// (callee-saved) is read for the current attributes per cell; `cells`/`rows`/
/// `cols` are the grid; the rest are caller-saved scratch (`SCRATCH[1..8]` →
/// `x10..x17`) the stampers clobber. No objc calls run inside the stampers, so the
/// scratch is stable across the whole draw.
struct AppStampCtx {
    state: &'static str,
    cells: &'static str,
    rows: &'static str,
    cols: &'static str,
    lo: &'static str,
    hi: &'static str,
    pos: &'static str,
    idx: &'static str,
    cell: &'static str,
    tmp: &'static str,
    attr: &'static str,
}

/// Emit A's charwidth lookup: `out` = display width (1 or 2) of scalar `cp`, via the
/// two-stage property trie (`(flags@16 >> 4) & 3`, width 0 folded to 1 since a lone
/// zero-width scalar still takes a cell). Uses `s1`/`s2`/`s3` as scratch; `tag` makes
/// the two internal labels unique. The `_mfb_unicode_*` relocations these
/// `local_address` loads emit are exactly what make the shared build embed the ~1.5 MB
/// property table (mod.rs `references_unicode_table`) — call ONLY under `uses_term`.
fn app_emit_charwidth(asm: &mut Asm, cp: &str, out: &str, s1: &str, s2: &str, s3: &str, tag: &str) {
    let lookup = format!("{tag}_lookup");
    let done = format!("{tag}_done");
    asm.push(abi::move_immediate(out, "Integer", "1114112")); // 0x110000
    asm.push(abi::compare_registers(cp, out));
    asm.push(abi::branch_lt(&lookup));
    asm.push(abi::move_immediate(out, "Integer", "1"));
    asm.push(abi::branch(&done));
    asm.push(abi::label(&lookup));
    asm.push(abi::shift_right_immediate(s1, cp, 8));
    asm.push(abi::shift_left_immediate(s1, s1, 1));
    asm.local_address(s2, crate::target::shared::code::UNICODE_STAGE1_SYMBOL);
    asm.push(abi::add_registers(s2, s2, s1));
    asm.push(abi::load_u16(s1, s2, 0));
    asm.push(abi::move_immediate(s3, "Integer", "255"));
    asm.push(abi::and_registers(s3, cp, s3));
    asm.push(abi::add_registers(s1, s1, s3));
    asm.push(abi::shift_left_immediate(s1, s1, 1));
    asm.local_address(s2, crate::target::shared::code::UNICODE_STAGE2_SYMBOL);
    asm.push(abi::add_registers(s2, s2, s1));
    asm.push(abi::load_u16(s1, s2, 0));
    asm.push(abi::move_immediate(s3, "Integer", "24")); // property record size
    asm.push(abi::multiply_registers(s1, s1, s3));
    asm.local_address(s2, crate::target::shared::code::UNICODE_PROPERTIES_SYMBOL);
    asm.push(abi::add_registers(s2, s2, s1));
    // width = (flags @ offset 16 >> 4) & 0b11
    asm.push(abi::load_u16(out, s2, 16));
    asm.push(abi::shift_right_immediate(out, out, 4));
    asm.push(abi::move_immediate(s3, "Integer", "3"));
    asm.push(abi::and_registers(out, out, s3));
    asm.push(abi::compare_immediate(out, "0"));
    asm.push(abi::branch_ne(&done));
    asm.push(abi::move_immediate(out, "Integer", "1"));
    asm.push(abi::label(&done));
}

/// Store `glyph` + the current attributes into `cell` (already the cell address),
/// stamping a display width of 1. plan-70-D Phase 2: mirrors the console C fix — the
/// width byte MUST be written on every stamp so a narrow glyph stamped over a cell
/// that had been a wide primary (stored width 2) resets it, or the presenter/drawRect
/// would still treat the cell as 2 columns. Wide (width-2) stamps overwrite the byte
/// afterwards via `app_stamp_cell_wide`.
fn app_stamp_attrs(asm: &mut Asm, ctx: &AppStampCtx, glyph: &str) {
    asm.push(abi::store_u32(glyph, ctx.cell, CELL_GLYPH_OFFSET));
    asm.push(abi::load_u64(ctx.attr, ctx.state, TV_CUR_FG_OFFSET));
    asm.push(abi::store_u32(ctx.attr, ctx.cell, CELL_FG_OFFSET));
    asm.push(abi::load_u64(ctx.attr, ctx.state, TV_CUR_BG_OFFSET));
    asm.push(abi::store_u32(ctx.attr, ctx.cell, CELL_BG_OFFSET));
    asm.push(abi::load_u64(ctx.attr, ctx.state, TV_CUR_BOLD_OFFSET));
    asm.push(abi::store_u8(ctx.attr, ctx.cell, CELL_BOLD_OFFSET));
    asm.push(abi::load_u64(ctx.attr, ctx.state, TV_CUR_UNDERLINE_OFFSET));
    asm.push(abi::store_u8(ctx.attr, ctx.cell, CELL_UNDERLINE_OFFSET));
    asm.push(abi::move_immediate(ctx.attr, "Integer", "1"));
    asm.push(abi::store_u8(ctx.attr, ctx.cell, CELL_WIDTH_OFFSET));
}

/// Stamp `glyph` across a clamped run (the `mfbDrawBox:` edge stamper — the app
/// twin of the console `emit_stamp_run`). `fixed`/`ea`/`eb` are registers; the
/// span is normalised + clamped to the grid; a `fixed` off the grid or an empty
/// span branches to `skip` (placed by the caller). Does not clobber
/// `fixed`/`ea`/`eb`.
#[allow(clippy::too_many_arguments)]
fn app_stamp_run(
    asm: &mut Asm,
    ctx: &AppStampCtx,
    is_horizontal: bool,
    fixed: &str,
    ea: &str,
    eb: &str,
    glyph: &str,
    tag: &str,
    skip: &str,
) {
    let (fixed_limit, span_limit) = if is_horizontal {
        (ctx.rows, ctx.cols)
    } else {
        (ctx.cols, ctx.rows)
    };
    let span_ok = format!("{tag}_span_ok");
    let lo_ok = format!("{tag}_lo_ok");
    let hi_ok = format!("{tag}_hi_ok");
    let loop_top = format!("{tag}_loop");
    let loop_done = format!("{tag}_loop_done");
    asm.push(abi::compare_immediate(fixed, "0"));
    asm.push(abi::branch_lt(skip));
    asm.push(abi::compare_registers(fixed, fixed_limit));
    asm.push(abi::branch_ge(skip));
    asm.push(abi::move_register(ctx.lo, ea));
    asm.push(abi::move_register(ctx.hi, eb));
    asm.push(abi::compare_registers(ctx.lo, ctx.hi));
    asm.push(abi::branch_le(&span_ok));
    asm.push(abi::move_register(ctx.tmp, ctx.lo));
    asm.push(abi::move_register(ctx.lo, ctx.hi));
    asm.push(abi::move_register(ctx.hi, ctx.tmp));
    asm.push(abi::label(&span_ok));
    asm.push(abi::compare_immediate(ctx.lo, "0"));
    asm.push(abi::branch_ge(&lo_ok));
    asm.push(abi::move_immediate(ctx.lo, "Integer", "0"));
    asm.push(abi::label(&lo_ok));
    asm.push(abi::subtract_immediate(ctx.tmp, span_limit, 1));
    asm.push(abi::compare_registers(ctx.hi, ctx.tmp));
    asm.push(abi::branch_le(&hi_ok));
    asm.push(abi::move_register(ctx.hi, ctx.tmp));
    asm.push(abi::label(&hi_ok));
    asm.push(abi::compare_registers(ctx.lo, ctx.hi));
    asm.push(abi::branch_gt(skip));
    asm.push(abi::move_register(ctx.pos, ctx.lo));
    asm.push(abi::label(&loop_top));
    asm.push(abi::compare_registers(ctx.pos, ctx.hi));
    asm.push(abi::branch_gt(&loop_done));
    // plan-70-D Phase 2: an edge/fill cell over half a wide glyph clears the orphan.
    if is_horizontal {
        app_clear_wide_pair(asm, ctx, fixed, ctx.pos, &format!("{tag}_pc"));
        asm.push(abi::multiply_registers(ctx.idx, fixed, ctx.cols));
        asm.push(abi::add_registers(ctx.idx, ctx.idx, ctx.pos));
    } else {
        app_clear_wide_pair(asm, ctx, ctx.pos, fixed, &format!("{tag}_pc"));
        asm.push(abi::multiply_registers(ctx.idx, ctx.pos, ctx.cols));
        asm.push(abi::add_registers(ctx.idx, ctx.idx, fixed));
    }
    asm.push(abi::shift_left_immediate(ctx.idx, ctx.idx, 4));
    asm.push(abi::add_registers(ctx.cell, ctx.cells, ctx.idx));
    app_stamp_attrs(asm, ctx, glyph);
    asm.push(abi::add_immediate(ctx.pos, ctx.pos, 1));
    asm.push(abi::branch(&loop_top));
    asm.push(abi::label(&loop_done));
}

/// Stamp a single cell `(row, col)` with `glyph` when on the grid, else branch to
/// `skip` (placed by the caller). The `mfbDrawBox:` corner stamper.
fn app_stamp_cell(
    asm: &mut Asm,
    ctx: &AppStampCtx,
    row: &str,
    col: &str,
    glyph: &str,
    tag: &str,
    skip: &str,
) {
    asm.push(abi::compare_immediate(row, "0"));
    asm.push(abi::branch_lt(skip));
    asm.push(abi::compare_registers(row, ctx.rows));
    asm.push(abi::branch_ge(skip));
    asm.push(abi::compare_immediate(col, "0"));
    asm.push(abi::branch_lt(skip));
    asm.push(abi::compare_registers(col, ctx.cols));
    asm.push(abi::branch_ge(skip));
    // plan-70-D Phase 2: a box corner over half a wide glyph clears the orphan.
    app_clear_wide_pair(asm, ctx, row, col, &format!("{tag}_pc"));
    asm.push(abi::multiply_registers(ctx.idx, row, ctx.cols));
    asm.push(abi::add_registers(ctx.idx, ctx.idx, col));
    asm.push(abi::shift_left_immediate(ctx.idx, ctx.idx, 4));
    asm.push(abi::add_registers(ctx.cell, ctx.cells, ctx.idx));
    app_stamp_attrs(asm, ctx, glyph);
}

/// plan-70-D Phase 2: blank the orphaned half of any wide glyph a stamp at
/// `(row,col)` overwrites — a `WIDE_TRAIL` clears the primary to its left; a wide
/// primary clears the trail to its right (space glyph, width 1, attributes kept).
/// `(row,col)` must already be on the grid. Clobbers `ctx.idx`/`ctx.cell`/`ctx.tmp`.
/// Mirrors the console C `emit_clear_wide_pair`.
fn app_clear_wide_pair(asm: &mut Asm, ctx: &AppStampCtx, row: &str, col: &str, tag: &str) {
    let not_trail = format!("{tag}_not_trail");
    let done = format!("{tag}_done");
    asm.push(abi::multiply_registers(ctx.idx, row, ctx.cols));
    asm.push(abi::add_registers(ctx.idx, ctx.idx, col));
    asm.push(abi::shift_left_immediate(ctx.idx, ctx.idx, 4));
    asm.push(abi::add_registers(ctx.cell, ctx.cells, ctx.idx));
    asm.push(abi::load_u32(ctx.tmp, ctx.cell, CELL_GLYPH_OFFSET));
    // A WIDE_TRAIL here orphans the primary to its left.
    asm.push(abi::move_immediate(ctx.idx, "Integer", APP_WIDE_TRAIL));
    asm.push(abi::compare_registers(ctx.tmp, ctx.idx));
    asm.push(abi::branch_ne(&not_trail));
    asm.push(abi::compare_immediate(col, "0"));
    asm.push(abi::branch_le(&done)); // no left neighbour (defensive)
    asm.push(abi::subtract_immediate(ctx.cell, ctx.cell, CELL_SIZE)); // primary at col-1
    asm.push(abi::move_immediate(ctx.tmp, "Integer", "32")); // space
    asm.push(abi::store_u32(ctx.tmp, ctx.cell, CELL_GLYPH_OFFSET));
    asm.push(abi::move_immediate(ctx.tmp, "Integer", "1"));
    asm.push(abi::store_u8(ctx.tmp, ctx.cell, CELL_WIDTH_OFFSET));
    asm.push(abi::branch(&done));
    asm.push(abi::label(&not_trail));
    // A wide primary here (width 2) orphans the trail to its right.
    asm.push(abi::load_u8(ctx.tmp, ctx.cell, CELL_WIDTH_OFFSET));
    asm.push(abi::compare_immediate(ctx.tmp, "2"));
    asm.push(abi::branch_ne(&done));
    asm.push(abi::add_immediate(ctx.tmp, col, 1));
    asm.push(abi::compare_registers(ctx.tmp, ctx.cols));
    asm.push(abi::branch_ge(&done)); // no right neighbour (defensive)
    asm.push(abi::add_immediate(ctx.cell, ctx.cell, CELL_SIZE)); // trail at col+1
    asm.push(abi::move_immediate(ctx.tmp, "Integer", "32"));
    asm.push(abi::store_u32(ctx.tmp, ctx.cell, CELL_GLYPH_OFFSET));
    asm.push(abi::move_immediate(ctx.tmp, "Integer", "1"));
    asm.push(abi::store_u8(ctx.tmp, ctx.cell, CELL_WIDTH_OFFSET));
    asm.push(abi::label(&done));
}

/// plan-70-D Phase 2: width-aware single-cluster stamp for the positioned draw
/// helpers (`mfbDrawGlyph:`/`mfbDrawText:`). `w` is the already-computed display width
/// (1 or 2) — the caller looks it up (from the base scalar, so a pooled-tag `glyph`
/// still gets the right width). Clears any wide glyph it overwrites, stamps the
/// primary at `(row,col)` with `w`, and for a width-2 cluster stamps a `WIDE_TRAIL` in
/// the next cell (dropping the whole cluster if it would straddle the right edge).
/// Branches to `skip` when off-grid or clipped. Mirrors the console C
/// `emit_draw_glyph`/`emit_draw_text` contract.
#[allow(clippy::too_many_arguments)]
fn app_stamp_cluster(
    asm: &mut Asm,
    ctx: &AppStampCtx,
    row: &str,
    col: &str,
    glyph: &str,
    w: &str,
    tag: &str,
    skip: &str,
) {
    let done = format!("{tag}_done");
    asm.push(abi::compare_immediate(row, "0"));
    asm.push(abi::branch_lt(skip));
    asm.push(abi::compare_registers(row, ctx.rows));
    asm.push(abi::branch_ge(skip));
    asm.push(abi::compare_immediate(col, "0"));
    asm.push(abi::branch_lt(skip));
    asm.push(abi::compare_registers(col, ctx.cols));
    asm.push(abi::branch_ge(skip));
    // A wide cluster needs col+1 on the grid, else it is dropped (never split).
    asm.push(abi::compare_immediate(w, "2"));
    asm.push(abi::branch_ne(&format!("{tag}_narrow")));
    asm.push(abi::add_immediate(ctx.tmp, col, 1));
    asm.push(abi::compare_registers(ctx.tmp, ctx.cols));
    asm.push(abi::branch_ge(skip));
    asm.push(abi::label(&format!("{tag}_narrow")));
    // Clear a wide glyph already sitting on the primary cell, then stamp.
    app_clear_wide_pair(asm, ctx, row, col, &format!("{tag}_pc0"));
    asm.push(abi::multiply_registers(ctx.idx, row, ctx.cols));
    asm.push(abi::add_registers(ctx.idx, ctx.idx, col));
    asm.push(abi::shift_left_immediate(ctx.idx, ctx.idx, 4));
    asm.push(abi::add_registers(ctx.cell, ctx.cells, ctx.idx));
    app_stamp_attrs(asm, ctx, glyph); // stores width 1
    asm.push(abi::store_u8(w, ctx.cell, CELL_WIDTH_OFFSET)); // real width
    asm.push(abi::compare_immediate(w, "2"));
    asm.push(abi::branch_ne(&done));
    // Wide: clear + stamp the trailing sentinel in the next cell.
    asm.push(abi::add_immediate(ctx.pos, col, 1));
    app_clear_wide_pair(asm, ctx, row, ctx.pos, &format!("{tag}_pc1"));
    asm.push(abi::add_immediate(ctx.pos, col, 1));
    asm.push(abi::multiply_registers(ctx.idx, row, ctx.cols));
    asm.push(abi::add_registers(ctx.idx, ctx.idx, ctx.pos));
    asm.push(abi::shift_left_immediate(ctx.idx, ctx.idx, 4));
    asm.push(abi::add_registers(ctx.cell, ctx.cells, ctx.idx));
    asm.push(abi::move_immediate(ctx.tmp, "Integer", APP_WIDE_TRAIL));
    app_stamp_attrs(asm, ctx, ctx.tmp); // stores width 1
    asm.push(abi::move_immediate(ctx.tmp, "Integer", "0"));
    asm.push(abi::store_u8(ctx.tmp, ctx.cell, CELL_WIDTH_OFFSET)); // trail width 0
    asm.push(abi::label(&done));
}

/// IMP for `TermView mfbDrawBox:` (`void mfbDrawBox:(id self, SEL, id)`): draw the
/// rectangle `term::drawBox` requested. The worker resolved the six box glyphs
/// (edges + corners) to unichars and parked them plus the two raw corner points in
/// the TermView state; this reads them, normalises the points, stamps the four
/// clamped edges (this style's H/V glyph), then overwrites the four corner cells.
/// Main-thread only (marshaled), so grid mutation is serialised; present-driven,
/// so it requests no redraw (plan-35-D §3).
pub(super) fn emit_term_draw_box_helper() -> CodeFunction {
    let mut asm = Asm::new(MFB_DRAW_BOX_SYMBOL);
    // Frame: lr@0, then callee-saved state@8, cells@16, rows@24, cols@32,
    // xlo@40, xhi@48, ylo@56, yhi@64.
    let frame = 80;
    let state = abi::LOCAL[0];
    let cells = abi::LOCAL[1];
    let rows = abi::LOCAL[2];
    let cols = abi::LOCAL[3];
    let xlo = abi::LOCAL[4];
    let xhi = abi::LOCAL[5];
    let ylo = abi::LOCAL[6];
    let yhi = abi::LOCAL[7];
    let glyph = abi::SCRATCH[0];
    let a = abi::SCRATCH[1]; // reused as a normalise temp before the run loop
    let b = abi::SCRATCH[2];
    let ctx = AppStampCtx {
        state,
        cells,
        rows,
        cols,
        lo: abi::SCRATCH[1],
        hi: abi::SCRATCH[2],
        pos: abi::SCRATCH[3],
        idx: abi::SCRATCH[4],
        cell: abi::SCRATCH[5],
        tmp: abi::SCRATCH[6],
        attr: abi::SCRATCH[7],
    };
    let saved: [(&str, usize); 8] = [
        (state, 8),
        (cells, 16),
        (rows, 24),
        (cols, 32),
        (xlo, 40),
        (xhi, 48),
        (ylo, 56),
        (yhi, 64),
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

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)  (x0 = self)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(state, "x0"));
    asm.push(abi::compare_immediate(state, "0"));
    asm.push(abi::branch_eq("db_done"));
    asm.push(abi::load_u64(cells, state, TV_CELLS_OFFSET));
    asm.push(abi::compare_immediate(cells, "0"));
    asm.push(abi::branch_eq("db_done"));
    asm.push(abi::load_u64(rows, state, TV_ROWS_OFFSET));
    asm.push(abi::load_u64(cols, state, TV_COLS_OFFSET));

    // Normalise the two corner points: xlo/xhi over columns, ylo/yhi over rows.
    asm.push(abi::load_u64(a, state, TV_BOX_X1_OFFSET));
    asm.push(abi::load_u64(b, state, TV_BOX_X2_OFFSET));
    asm.push(abi::move_register(xlo, a));
    asm.push(abi::move_register(xhi, b));
    asm.push(abi::compare_registers(a, b));
    asm.push(abi::branch_le("db_x_ok"));
    asm.push(abi::move_register(xlo, b));
    asm.push(abi::move_register(xhi, a));
    asm.push(abi::label("db_x_ok"));
    asm.push(abi::load_u64(a, state, TV_BOX_Y1_OFFSET));
    asm.push(abi::load_u64(b, state, TV_BOX_Y2_OFFSET));
    asm.push(abi::move_register(ylo, a));
    asm.push(abi::move_register(yhi, b));
    asm.push(abi::compare_registers(a, b));
    asm.push(abi::branch_le("db_y_ok"));
    asm.push(abi::move_register(ylo, b));
    asm.push(abi::move_register(yhi, a));
    asm.push(abi::label("db_y_ok"));

    // Four edges: top/bottom use the H glyph, left/right the V glyph. Each edge's
    // glyph is loaded into `glyph` (SCRATCH[0]) just before it is stamped.
    let edges: [(bool, &str, &str, &str, usize, &str); 4] = [
        (true, ylo, xlo, xhi, TV_BOX_HG_OFFSET, "db_e0"),
        (true, yhi, xlo, xhi, TV_BOX_HG_OFFSET, "db_e1"),
        (false, xlo, ylo, yhi, TV_BOX_VG_OFFSET, "db_e2"),
        (false, xhi, ylo, yhi, TV_BOX_VG_OFFSET, "db_e3"),
    ];
    for (is_h, fixed, ea, eb, glyph_off, tag) in edges {
        let skip = format!("{tag}_skip");
        asm.push(abi::load_u64(glyph, state, glyph_off));
        app_stamp_run(&mut asm, &ctx, is_h, fixed, ea, eb, glyph, tag, &skip);
        asm.push(abi::label(&skip));
    }
    // Four corners on top of the edges.
    let corners: [(&str, &str, usize, &str); 4] = [
        (ylo, xlo, TV_BOX_CTL_OFFSET, "db_ctl"),
        (ylo, xhi, TV_BOX_CTR_OFFSET, "db_ctr"),
        (yhi, xlo, TV_BOX_CBL_OFFSET, "db_cbl"),
        (yhi, xhi, TV_BOX_CBR_OFFSET, "db_cbr"),
    ];
    for (row, col, glyph_off, tag) in corners {
        let skip = format!("{tag}_skip");
        asm.push(abi::load_u64(glyph, state, glyph_off));
        app_stamp_cell(&mut asm, &ctx, row, col, glyph, tag, &skip);
        asm.push(abi::label(&skip));
    }

    asm.push(abi::label("db_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.drawBox".to_string(),
        symbol: MFB_DRAW_BOX_SYMBOL.to_string(),
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

/// IMP for `TermView mfbFillRect:` (`void mfbFillRect:(id self, SEL, id)`): fill
/// the rectangle `term::fillRect` requested. The worker resolved the `FillStyle`
/// ordinal to a unichar and parked it plus the two raw corner points in the state;
/// this normalises the points, clamps the row range to the grid, and stamps one
/// clamped horizontal run per row with the fill glyph. Main-thread only;
/// present-driven (no redraw request).
pub(super) fn emit_term_fill_rect_helper() -> CodeFunction {
    let mut asm = Asm::new(MFB_FILL_RECT_SYMBOL);
    // Frame: lr@0, then callee-saved state@8, cells@16, rows@24, cols@32,
    // xlo@40, xhi@48, ylo@56, yhi@64.
    let frame = 80;
    let state = abi::LOCAL[0];
    let cells = abi::LOCAL[1];
    let rows = abi::LOCAL[2];
    let cols = abi::LOCAL[3];
    let xlo = abi::LOCAL[4];
    let xhi = abi::LOCAL[5];
    let ylo = abi::LOCAL[6];
    let yhi = abi::LOCAL[7];
    let glyph = abi::SCRATCH[0];
    let row = abi::SCRATCH[8];
    let ctx = AppStampCtx {
        state,
        cells,
        rows,
        cols,
        lo: abi::SCRATCH[1],
        hi: abi::SCRATCH[2],
        pos: abi::SCRATCH[3],
        idx: abi::SCRATCH[4],
        cell: abi::SCRATCH[5],
        tmp: abi::SCRATCH[6],
        attr: abi::SCRATCH[7],
    };
    let saved: [(&str, usize); 8] = [
        (state, 8),
        (cells, 16),
        (rows, 24),
        (cols, 32),
        (xlo, 40),
        (xhi, 48),
        (ylo, 56),
        (yhi, 64),
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

    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(state, "x0"));
    asm.push(abi::compare_immediate(state, "0"));
    asm.push(abi::branch_eq("fr_done"));
    asm.push(abi::load_u64(cells, state, TV_CELLS_OFFSET));
    asm.push(abi::compare_immediate(cells, "0"));
    asm.push(abi::branch_eq("fr_done"));
    asm.push(abi::load_u64(rows, state, TV_ROWS_OFFSET));
    asm.push(abi::load_u64(cols, state, TV_COLS_OFFSET));
    asm.push(abi::load_u64(glyph, state, TV_FILL_GLYPH_OFFSET));

    // Normalise the two corner points (ctx.lo/ctx.hi as scratch temps here).
    asm.push(abi::load_u64(ctx.lo, state, TV_FILL_X1_OFFSET));
    asm.push(abi::load_u64(ctx.hi, state, TV_FILL_X2_OFFSET));
    asm.push(abi::move_register(xlo, ctx.lo));
    asm.push(abi::move_register(xhi, ctx.hi));
    asm.push(abi::compare_registers(ctx.lo, ctx.hi));
    asm.push(abi::branch_le("fr_x_ok"));
    asm.push(abi::move_register(xlo, ctx.hi));
    asm.push(abi::move_register(xhi, ctx.lo));
    asm.push(abi::label("fr_x_ok"));
    asm.push(abi::load_u64(ctx.lo, state, TV_FILL_Y1_OFFSET));
    asm.push(abi::load_u64(ctx.hi, state, TV_FILL_Y2_OFFSET));
    asm.push(abi::move_register(ylo, ctx.lo));
    asm.push(abi::move_register(yhi, ctx.hi));
    asm.push(abi::compare_registers(ctx.lo, ctx.hi));
    asm.push(abi::branch_le("fr_y_ok"));
    asm.push(abi::move_register(ylo, ctx.hi));
    asm.push(abi::move_register(yhi, ctx.lo));
    asm.push(abi::label("fr_y_ok"));
    // Clamp the row range: ylo up to 0, yhi down to rows-1; empty → done.
    asm.push(abi::compare_immediate(ylo, "0"));
    asm.push(abi::branch_ge("fr_ylo_ok"));
    asm.push(abi::move_immediate(ylo, "Integer", "0"));
    asm.push(abi::label("fr_ylo_ok"));
    asm.push(abi::subtract_immediate(ctx.tmp, rows, 1));
    asm.push(abi::compare_registers(yhi, ctx.tmp));
    asm.push(abi::branch_le("fr_yhi_ok"));
    asm.push(abi::move_register(yhi, ctx.tmp));
    asm.push(abi::label("fr_yhi_ok"));
    asm.push(abi::compare_registers(ylo, yhi));
    asm.push(abi::branch_gt("fr_done"));

    // One horizontal run per row over ylo..=yhi.
    asm.push(abi::move_register(row, ylo));
    asm.push(abi::label("fr_row"));
    asm.push(abi::compare_registers(row, yhi));
    asm.push(abi::branch_gt("fr_done"));
    app_stamp_run(
        &mut asm, &ctx, true, row, xlo, xhi, glyph, "fr_run", "fr_next",
    );
    asm.push(abi::label("fr_next"));
    asm.push(abi::add_immediate(row, row, 1));
    asm.push(abi::branch("fr_row"));

    asm.push(abi::label("fr_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.fillRect".to_string(),
        symbol: MFB_FILL_RECT_SYMBOL.to_string(),
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

/// IMP for `TermView mfbDrawGlyph:` (`void mfbDrawGlyph:(id self, SEL, id)`):
/// stamp the single unichar the worker parked (`TV_GLYPH_G`) at (`TV_GLYPH_X`,
/// `TV_GLYPH_Y`) with the current attributes. A no-op if the cell is off the grid.
/// Main-thread only; present-driven.
pub(super) fn emit_term_draw_glyph_helper(uses_term: bool) -> CodeFunction {
    let mut asm = Asm::new(MFB_DRAW_GLYPH_SYMBOL);
    let frame = 64; // lr@0, state@8, cells@16, rows@24, cols@32, x@40, y@48
    let state = abi::LOCAL[0];
    let cells = abi::LOCAL[1];
    let rows = abi::LOCAL[2];
    let cols = abi::LOCAL[3];
    let x = abi::LOCAL[4];
    let y = abi::LOCAL[5];
    let glyph = abi::SCRATCH[0];
    let width = abi::SCRATCH[8]; // plan-70-D Phase 2 display-width scratch (free here)
    let ctx = AppStampCtx {
        state,
        cells,
        rows,
        cols,
        lo: abi::SCRATCH[1],
        hi: abi::SCRATCH[2],
        pos: abi::SCRATCH[3],
        idx: abi::SCRATCH[4],
        cell: abi::SCRATCH[5],
        tmp: abi::SCRATCH[6],
        attr: abi::SCRATCH[7],
    };
    let saved: [(&str, usize); 6] = [
        (state, 8),
        (cells, 16),
        (rows, 24),
        (cols, 32),
        (x, 40),
        (y, 48),
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
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(state, "x0"));
    asm.push(abi::compare_immediate(state, "0"));
    asm.push(abi::branch_eq("dgl_done"));
    asm.push(abi::load_u64(cells, state, TV_CELLS_OFFSET));
    asm.push(abi::compare_immediate(cells, "0"));
    asm.push(abi::branch_eq("dgl_done"));
    asm.push(abi::load_u64(rows, state, TV_ROWS_OFFSET));
    asm.push(abi::load_u64(cols, state, TV_COLS_OFFSET));
    asm.push(abi::load_u64(glyph, state, TV_GLYPH_G_OFFSET));
    asm.push(abi::load_u64(x, state, TV_GLYPH_X_OFFSET));
    asm.push(abi::load_u64(y, state, TV_GLYPH_Y_OFFSET));
    // plan-70-D Phase 2: the glyph's display width (gated on uses_term so a non-term
    // app never references the table); a wide glyph reserves a WIDE_TRAIL neighbour.
    if uses_term {
        app_emit_charwidth(&mut asm, glyph, width, ctx.idx, ctx.cell, ctx.tmp, "dgl_w");
    } else {
        asm.push(abi::move_immediate(width, "Integer", "1"));
    }
    app_stamp_cluster(&mut asm, &ctx, y, x, glyph, width, "dgl_s", "dgl_done");
    asm.push(abi::label("dgl_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.term.drawGlyph".to_string(),
        symbol: MFB_DRAW_GLYPH_SYMBOL.to_string(),
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

/// IMP for `TermView mfbDrawText:` (`void mfbDrawText:(id self, SEL, NSString*)`):
/// stamp the string on row `TV_TEXT_Y` starting at column `TV_TEXT_X`, one cell per
/// UTF-16 unit, with the current attributes. Does not move the cursor, wrap, or
/// scroll; clips at the right edge and skips control characters. Main-thread only
/// (invoked via performSelectorOnMainThread); present-driven.
pub(super) fn emit_term_draw_text_helper(uses_term: bool) -> CodeFunction {
    let mut asm = Asm::new(MFB_DRAW_TEXT_SYMBOL);
    // plan-70-D Phase 2: width/cluster/astral-aware, mirroring mfbWriteString: but
    // positioned (no cursor/wrap/scroll). Loop state is callee-saved (the [str …]
    // calls clobber scratch). Frame: lr@0, self@8, str@16, state@24, cells@32, i@40,
    // n@48, cols@56, rows@64, col@72, L@80, baseUnits@88, clampedL@96, width@104.
    // `selfv` is dead once state is resolved, so it doubles as the decoded codepoint
    // `cp` (callee-saved across the inner characterAtIndex:).
    let frame = 112;
    let selfv = abi::LOCAL[0];
    let cp = abi::LOCAL[0]; // reuses selfv's register after state is resolved
    let strv = abi::LOCAL[1];
    let state = abi::LOCAL[2];
    let cells = abi::LOCAL[3];
    let i = abi::LOCAL[4];
    let n = abi::LOCAL[5];
    let cols = abi::LOCAL[6];
    let rows = abi::LOCAL[7];
    let col = abi::LOCAL[8]; // running column, advances by display width
    let ctx = AppStampCtx {
        state,
        cells,
        rows,
        cols,
        lo: abi::SCRATCH[4],
        hi: abi::SCRATCH[5],
        pos: abi::SCRATCH[6],
        idx: abi::SCRATCH[1],
        cell: abi::SCRATCH[2],
        tmp: abi::SCRATCH[3],
        attr: abi::SCRATCH[0],
    };
    let width = abi::SCRATCH[7]; // display-width scratch for the stamp / column advance
    let y = abi::SCRATCH[8]; // row, reloaded from state right before each stamp
    let (off_l, off_base, off_clamped, off_w) = (80usize, 88usize, 96usize, 104usize);
    let saved: [(&str, usize); 9] = [
        (selfv, 8),
        (strv, 16),
        (state, 24),
        (cells, 32),
        (i, 40),
        (n, 48),
        (cols, 56),
        (rows, 64),
        (col, 72),
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
    asm.push(abi::move_register(selfv, "x0"));
    asm.push(abi::move_register(strv, "x2"));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.push(abi::move_register("x0", selfv));
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(state, "x0"));
    asm.push(abi::compare_immediate(state, "0"));
    asm.push(abi::branch_eq("dt_done"));
    asm.push(abi::load_u64(cells, state, TV_CELLS_OFFSET));
    asm.push(abi::compare_immediate(cells, "0"));
    asm.push(abi::branch_eq("dt_done"));
    asm.push(abi::load_u64(cols, state, TV_COLS_OFFSET));
    asm.push(abi::load_u64(rows, state, TV_ROWS_OFFSET));
    asm.push(abi::load_u64(col, state, TV_TEXT_X_OFFSET)); // running column
                                                           // Row must be on the grid.
    asm.push(abi::load_u64(y, state, TV_TEXT_Y_OFFSET));
    asm.push(abi::compare_immediate(y, "0"));
    asm.push(abi::branch_lt("dt_done"));
    asm.push(abi::compare_registers(y, rows));
    asm.push(abi::branch_ge("dt_done"));
    // n = [str length]; i = 0
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", strv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(n, "x0"));
    asm.push(abi::move_immediate(i, "Integer", "0"));
    asm.push(abi::label("dt_loop"));
    asm.push(abi::compare_registers(i, n));
    asm.push(abi::branch_ge("dt_done"));
    // Cluster length L (UTF-16 units) — the amount to advance i by.
    if uses_term {
        asm.load_selector(SEL_RANGE_COMPOSED.0);
        asm.push(abi::move_register("x2", i));
        asm.push(abi::move_register("x0", strv));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::store_u64("x1", abi::stack_pointer(), off_l));
    } else {
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), off_l));
    }
    // c = [str characterAtIndex:i]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_register("x2", i));
    asm.push(abi::move_register("x0", strv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(cp, "x0"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_base,
    ));
    // Astral decode from a UTF-16 surrogate pair (mirrors mfbWriteString:).
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "55296")); // 0xD800
    asm.push(abi::compare_registers(cp, abi::SCRATCH[0]));
    asm.push(abi::branch_lt("dt_not_surr"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "56320")); // 0xDC00
    asm.push(abi::compare_registers(cp, abi::SCRATCH[0]));
    asm.push(abi::branch_ge("dt_not_surr"));
    asm.push(abi::add_immediate(abi::SCRATCH[0], i, 1));
    asm.push(abi::compare_registers(abi::SCRATCH[0], n));
    asm.push(abi::branch_ge("dt_not_surr"));
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::add_immediate("x2", i, 1));
    asm.push(abi::move_register("x0", strv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::SCRATCH[0], "x0")); // lo
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "56320"));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_lt("dt_not_surr"));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "57344"));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_ge("dt_not_surr"));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "55296"));
    asm.push(abi::subtract_registers(
        abi::SCRATCH[2],
        cp,
        abi::SCRATCH[1],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        10,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "56320"));
    asm.push(abi::subtract_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[3],
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "65536"));
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    ));
    asm.push(abi::move_register(cp, abi::SCRATCH[2])); // full codepoint
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "2")); // base scalar = 2 units
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_base,
    ));
    asm.push(abi::label("dt_not_surr"));
    // Control char (< 0x20): advance i by L, leave the column unchanged.
    asm.push(abi::compare_immediate(cp, "32"));
    asm.push(abi::branch_lt("dt_advance_i"));
    // Display width from the base scalar (gated); spilled across the pool msgSend.
    if uses_term {
        app_emit_charwidth(
            &mut asm,
            cp,
            width,
            abi::SCRATCH[1],
            abi::SCRATCH[2],
            abi::SCRATCH[3],
            "dt_w",
        );
    } else {
        asm.push(abi::move_immediate(width, "Integer", "1"));
    }
    asm.push(abi::store_u64(width, abi::stack_pointer(), off_w));
    // Pool a multi-scalar cluster (L > base units) into the (y,col) cell's slot.
    if uses_term {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_l));
        asm.push(abi::load_u64(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_base,
        ));
        asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
        asm.push(abi::branch_eq("dt_pool_done"));
        // col must be on the grid to own a pool slot; else the stamp clips it anyway.
        asm.push(abi::compare_immediate(col, "0"));
        asm.push(abi::branch_lt("dt_pool_done"));
        asm.push(abi::compare_registers(col, cols));
        asm.push(abi::branch_ge("dt_pool_done"));
        // clampedL = min(L, POOL/2)
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "32"));
        asm.push(abi::branch_le("dt_pool_len_ok"));
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "32"));
        asm.push(abi::label("dt_pool_len_ok"));
        asm.push(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_clamped,
        ));
        // buffer = pool_base + (y*cols + col)*POOL
        asm.push(abi::load_u64(abi::SCRATCH[2], state, TV_POOL_OFFSET));
        asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
        asm.push(abi::branch_eq("dt_pool_done"));
        // Resolve the selector into x1 BEFORE building the buffer pointer: load_selector
        // calls sel_registerName, which clobbers every caller-saved scratch register.
        // Resolving it after the arithmetic left the buffer pointer (x2) holding garbage.
        asm.load_selector(SEL_GET_CHARACTERS.0);
        asm.push(abi::load_u64(abi::SCRATCH[0], state, TV_TEXT_Y_OFFSET));
        asm.push(abi::multiply_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            cols,
        ));
        asm.push(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], col));
        asm.push(abi::shift_left_immediate(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            6,
        ));
        // Reload pool_base: the load_selector call above clobbered SCRATCH[2], so the
        // pool pointer read for the guard is stale by now.
        asm.push(abi::load_u64(abi::SCRATCH[2], state, TV_POOL_OFFSET));
        asm.push(abi::add_registers(
            abi::SCRATCH[2],
            abi::SCRATCH[2],
            abi::SCRATCH[0],
        ));
        // [str getCharacters:buffer range:{i, clampedL}] — selector already resolved
        // into x1 above (before the buffer arithmetic that lives in caller-saved scratch).
        asm.push(abi::move_register("x2", abi::SCRATCH[2]));
        asm.push(abi::move_register("x3", i));
        asm.push(abi::load_u64("x4", abi::stack_pointer(), off_clamped));
        asm.push(abi::move_register("x0", strv));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        // cp = APP_GLYPH_POOLED_TAG | clampedL
        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_clamped,
        ));
        asm.push(abi::move_immediate(
            abi::SCRATCH[1],
            "Integer",
            APP_GLYPH_POOLED_TAG,
        ));
        asm.push(abi::or_registers(cp, abi::SCRATCH[1], abi::SCRATCH[0]));
        asm.push(abi::label("dt_pool_done"));
    }
    // Right-edge clip: the column only grows, so stop once at/past it.
    asm.push(abi::compare_registers(col, cols));
    asm.push(abi::branch_ge("dt_done"));
    // Left of the grid: skip the stamp but keep advancing.
    asm.push(abi::compare_immediate(col, "0"));
    asm.push(abi::branch_lt("dt_after_stamp"));
    asm.push(abi::load_u64(width, abi::stack_pointer(), off_w)); // reload (pool clobbered it)
    asm.push(abi::load_u64(y, state, TV_TEXT_Y_OFFSET));
    app_stamp_cluster(&mut asm, &ctx, y, col, cp, width, "dt_s", "dt_done");
    asm.push(abi::label("dt_after_stamp"));
    // Advance the column by the display width.
    asm.push(abi::load_u64(width, abi::stack_pointer(), off_w));
    asm.push(abi::add_registers(col, col, width));
    asm.push(abi::label("dt_advance_i"));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_l));
    asm.push(abi::add_registers(i, i, abi::SCRATCH[0]));
    asm.push(abi::branch("dt_loop"));
    asm.push(abi::label("dt_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.term.drawText".to_string(),
        symbol: MFB_DRAW_TEXT_SYMBOL.to_string(),
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
    // Frame: lr@0, x19(rowBytes)@8, x20(cells)@16, x21(rows)@24, x22(poolBase)@32,
    // x23(poolRowBytes)@40. plan-70-D Phase 2: the EGC pool is TermCell-parallel, so
    // a scroll must shift its per-cell slots in lockstep or a scrolled pooled cluster
    // would read another cell's units.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
    asm.push(abi::store_u64(abi::LOCAL[4], abi::stack_pointer(), 40));

    asm.push(abi::load_u64(abi::LOCAL[1], "x0", TV_CELLS_OFFSET)); // cells
    asm.push(abi::load_u64(abi::LOCAL[2], "x0", TV_ROWS_OFFSET)); // rows
    asm.push(abi::load_u64(abi::SCRATCH[0], "x0", TV_COLS_OFFSET)); // cols
    asm.push(abi::shift_left_immediate(abi::LOCAL[0], abi::SCRATCH[0], 4)); // rowBytes = cols*CELL_SIZE
    asm.push(abi::load_u64(abi::LOCAL[3], "x0", TV_POOL_OFFSET)); // pool base (may be 0)
    asm.push(abi::shift_left_immediate(abi::LOCAL[4], abi::SCRATCH[0], 6)); // poolRowBytes = cols*POOL(64)

    // memmove(cells, cells + rowBytes, (rows-1)*rowBytes)
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::LOCAL[2], 1));
    asm.push(abi::multiply_registers(
        "x2",
        abi::SCRATCH[0],
        abi::LOCAL[0],
    )); // len
    asm.push(abi::move_register("x0", abi::LOCAL[1])); // dst
    asm.push(abi::add_registers("x1", abi::LOCAL[1], abi::LOCAL[0])); // src
    asm.call_external("_memmove", LIB_SYSTEM);

    // bzero(cells + (rows-1)*rowBytes, rowBytes) — clear the new bottom row.
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::LOCAL[2], 1));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::LOCAL[0],
    ));
    asm.push(abi::add_registers("x0", abi::LOCAL[1], abi::SCRATCH[0]));
    asm.push(abi::move_register("x1", abi::LOCAL[0]));
    asm.call_external("_bzero", LIB_SYSTEM);

    // Same shift over the EGC pool (guarded — a pool-less state skips it).
    asm.push(abi::compare_immediate(abi::LOCAL[3], "0"));
    asm.push(abi::branch_eq("scroll_pool_done"));
    // memmove(pool, pool + poolRowBytes, (rows-1)*poolRowBytes)
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::LOCAL[2], 1));
    asm.push(abi::multiply_registers(
        "x2",
        abi::SCRATCH[0],
        abi::LOCAL[4],
    ));
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.push(abi::add_registers("x1", abi::LOCAL[3], abi::LOCAL[4]));
    asm.call_external("_memmove", LIB_SYSTEM);
    // bzero(pool + (rows-1)*poolRowBytes, poolRowBytes)
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::LOCAL[2], 1));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::LOCAL[4],
    ));
    asm.push(abi::add_registers("x0", abi::LOCAL[3], abi::SCRATCH[0]));
    asm.push(abi::move_register("x1", abi::LOCAL[4]));
    asm.call_external("_bzero", LIB_SYSTEM);
    asm.push(abi::label("scroll_pool_done"));

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
    asm.push(abi::load_u64(abi::LOCAL[4], abi::stack_pointer(), 40));
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
pub(super) fn emit_term_write_string_helper(uses_term: bool) -> CodeFunction {
    // plan-70-D: the width path looks up A's charwidth table, whose `_mfb_unicode_*`
    // relocations make the shared build embed the ~1.5 MB unicode table. Emit it
    // ONLY when the app actually uses `term::` — otherwise a non-term app would
    // carry the table for a helper it never calls (the bootstrap always wires the
    // TermView class, so this function is always emitted, but its body is dead code
    // for a non-term app). The astral surrogate decode below is table-free and
    // stays unconditional.
    let mut asm = Asm::new(MFB_WRITE_STRING_SYMBOL);
    // Frame: lr@0, x19(self)@8, x20(str)@16, x21(state)@24, x22(cells)@32,
    // x23(i)@40, x24(n)@48, x25(cols)@56, x26(rows)@64, x27(char)@72.
    // plan-70-D: sp+80 = display width, sp+88 = cluster unit length L, sp+96 =
    // base-scalar unit count (1 BMP / 2 astral) for the inline-vs-pooled decision.
    let frame = 112;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in [
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
        (abi::LOCAL[6], 56),
        (abi::LOCAL[7], 64),
        (abi::LOCAL[8], 72),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register(abi::LOCAL[0], "x0")); // self
    asm.push(abi::move_register(abi::LOCAL[1], "x2")); // str

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0"));
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq("w_done"));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::LOCAL[2], TV_CELLS_OFFSET)); // cells
    asm.push(abi::compare_immediate(abi::LOCAL[3], "0"));
    asm.push(abi::branch_eq("w_done"));
    asm.push(abi::load_u64(abi::LOCAL[6], abi::LOCAL[2], TV_COLS_OFFSET));
    asm.push(abi::load_u64(abi::LOCAL[7], abi::LOCAL[2], TV_ROWS_OFFSET));

    // n = [str length]; i = 0
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[5], "x0"));
    asm.push(abi::move_immediate(abi::LOCAL[4], "Integer", "0"));

    asm.push(abi::label("w_loop"));
    asm.push(abi::compare_registers(abi::LOCAL[4], abi::LOCAL[5]));
    asm.push(abi::branch_ge("w_done"));
    // plan-70-D Phase 2: the extended grapheme cluster at i — its length in UTF-16
    // units (AppKit groups base+combining marks and emoji ZWJ sequences). The
    // NSRange result is {location=x0, length=x1}; keep the length (spilled to sp+88)
    // as the amount to advance and, if >1 scalar, the pooled unit count.
    if uses_term {
        asm.load_selector(SEL_RANGE_COMPOSED.0);
        asm.push(abi::move_register("x2", abi::LOCAL[4])); // index i
        asm.push(abi::move_register("x0", abi::LOCAL[1])); // str
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::store_u64("x1", abi::stack_pointer(), 88)); // cluster length L
    } else {
        // Non-term app: the writer is dead code; a single unit per step is enough
        // and avoids the extra msgSend + table.
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 88));
    }
    // c = [str characterAtIndex:i]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_register("x2", abi::LOCAL[4]));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[8], "x0")); // char code
                                                       // base-scalar unit count: 1 unless the astral decode below combines a pair.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 96));

    // plan-70-D: decode an astral scalar from a UTF-16 surrogate pair so it lands
    // in ONE cell holding the full codepoint (fixes the surrogate-splitting tofu).
    // A high surrogate is U+D800..U+DBFF; if a low surrogate U+DC00..U+DFFF
    // follows, combine them and consume both units. Constants go through a register
    // because aarch64 `cmp`/`sub` immediates are 12-bit. `LOCAL[8]` (char, callee-
    // saved) survives the inner msgSend; `SCRATCH` (caller-saved) does not.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "55296")); // 0xD800
    asm.push(abi::compare_registers(abi::LOCAL[8], abi::SCRATCH[0]));
    asm.push(abi::branch_lt("w_not_surrogate"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "56320")); // 0xDC00
    asm.push(abi::compare_registers(abi::LOCAL[8], abi::SCRATCH[0]));
    asm.push(abi::branch_ge("w_not_surrogate")); // >= 0xDC00 → not a HIGH surrogate
    asm.push(abi::add_immediate(abi::SCRATCH[0], abi::LOCAL[4], 1)); // i+1
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::LOCAL[5]));
    asm.push(abi::branch_ge("w_not_surrogate")); // no next unit
                                                 // lo = [str characterAtIndex:(i+1)]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::add_immediate("x2", abi::LOCAL[4], 1)); // i+1 (SCRATCH may be clobbered by load_selector)
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::SCRATCH[0], "x0")); // lo
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "56320")); // 0xDC00
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_lt("w_not_surrogate"));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "57344")); // 0xE000
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_ge("w_not_surrogate")); // lo >= 0xE000 → not a low surrogate
                                                 // codepoint = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "55296"));
    asm.push(abi::subtract_registers(
        abi::SCRATCH[2],
        abi::LOCAL[8],
        abi::SCRATCH[1],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        10,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "56320"));
    asm.push(abi::subtract_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[3],
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "65536"));
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    ));
    asm.push(abi::move_register(abi::LOCAL[8], abi::SCRATCH[2])); // full codepoint
                                                                  // (the cluster length L spilled at sp+88 advances i past both surrogate units)
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "2")); // base scalar = 2 units
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 96));
    asm.push(abi::label("w_not_surrogate"));

    asm.push(abi::compare_immediate(abi::LOCAL[8], "10")); // \n
    asm.push(abi::branch_eq("w_newline"));
    asm.push(abi::compare_immediate(abi::LOCAL[8], "13")); // \r
    asm.push(abi::branch_eq("w_cr"));
    asm.push(abi::compare_immediate(abi::LOCAL[8], "9")); // \t
    asm.push(abi::branch_eq("w_tab"));

    if uses_term {
        // plan-70-D: display width (1/2) of this scalar via A's two-stage property
        // trie. Spilled to sp+80 (a free frame slot) because the scroll call below
        // clobbers caller-saved registers.
        app_emit_charwidth(
            &mut asm,
            abi::LOCAL[8],
            abi::SCRATCH[0],
            abi::SCRATCH[1],
            abi::SCRATCH[2],
            abi::SCRATCH[3],
            "w_width",
        );
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 80)); // spill width
    }

    // printable: wrap if cursor_col >= cols (wide-at-edge handled after w_col_ok)
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::LOCAL[6]));
    asm.push(abi::branch_lt("w_col_ok"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::label("w_col_ok"));
    if uses_term {
        // plan-70-D: a width-2 glyph at the last column wraps rather than straddling
        // the right edge.
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 80)); // width
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "2"));
        asm.push(abi::branch_ne("w_wide_ok"));
        asm.push(abi::load_u64(
            abi::SCRATCH[1],
            abi::LOCAL[2],
            TV_CURSOR_COL_OFFSET,
        ));
        asm.push(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1)); // col+1
        asm.push(abi::compare_registers(abi::SCRATCH[1], abi::LOCAL[6]));
        asm.push(abi::branch_lt("w_wide_ok"));
        asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
        asm.push(abi::store_u64(
            abi::SCRATCH[1],
            abi::LOCAL[2],
            TV_CURSOR_COL_OFFSET,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[1],
            abi::LOCAL[2],
            TV_CURSOR_ROW_OFFSET,
        ));
        asm.push(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
        asm.push(abi::store_u64(
            abi::SCRATCH[1],
            abi::LOCAL[2],
            TV_CURSOR_ROW_OFFSET,
        ));
        asm.push(abi::label("w_wide_ok"));
    }
    // scroll if cursor_row >= rows
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[1], abi::LOCAL[7]));
    asm.push(abi::branch_lt("w_row_ok"));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate(abi::SCRATCH[1], abi::LOCAL[7], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::label("w_row_ok"));
    if uses_term {
        // plan-70-D Phase 2: a multi-scalar grapheme cluster (L UTF-16 units beyond
        // the base scalar — combining marks, ZWJ emoji) can't fit the inline glyph
        // field. Copy its units into this cell's EGC pool slot and tag the glyph as
        // pooled so drawRect rebuilds the whole cluster. Inline iff L == the base
        // scalar's unit count (1 BMP / 2 astral). The msgSend clobbers scratch but
        // not the callee-saved locals, and the cell ptr is recomputed just below.
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 88)); // L
        asm.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), 96)); // base units
        asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
        asm.push(abi::branch_eq("w_pool_done"));
        // clampedL = min(L, POOL/2) so the copy can't overrun the 64-byte slot.
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "32"));
        asm.push(abi::branch_le("w_pool_len_ok"));
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "32"));
        asm.push(abi::label("w_pool_len_ok"));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 104)); // clampedL
                                                                              // buffer = pool_base + (row*cols + col) * APP_POOL_BYTES_PER_CELL
        asm.push(abi::load_u64(
            abi::SCRATCH[2],
            abi::LOCAL[2],
            TV_POOL_OFFSET,
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
        asm.push(abi::branch_eq("w_pool_done")); // no pool -> inline base scalar
                                                 // Resolve the selector into x1 BEFORE building the buffer pointer (mirrors
                                                 // drawText): load_selector calls sel_registerName, which clobbers every
                                                 // caller-saved scratch register — including the SCRATCH[2] the buffer lives in.
        asm.load_selector(SEL_GET_CHARACTERS.0);
        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::LOCAL[2],
            TV_CURSOR_ROW_OFFSET,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[1],
            abi::LOCAL[2],
            TV_CURSOR_COL_OFFSET,
        ));
        asm.push(abi::multiply_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            abi::LOCAL[6],
        )); // row*cols
        asm.push(abi::add_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            abi::SCRATCH[1],
        )); // +col
        asm.push(abi::shift_left_immediate(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            6,
        )); // *POOL(64)
            // Reload pool_base: the load_selector call above clobbered SCRATCH[2], so the
            // pool pointer read for the guard is stale by now.
        asm.push(abi::load_u64(
            abi::SCRATCH[2],
            abi::LOCAL[2],
            TV_POOL_OFFSET,
        ));
        asm.push(abi::add_registers(
            abi::SCRATCH[2],
            abi::SCRATCH[2],
            abi::SCRATCH[0],
        )); // buffer ptr
            // [str getCharacters:buffer range:{i, clampedL}]  (NSRange in x3/x4)
        asm.push(abi::move_register("x2", abi::SCRATCH[2])); // buffer
        asm.push(abi::move_register("x3", abi::LOCAL[4])); // range.location = i
        asm.push(abi::load_u64("x4", abi::stack_pointer(), 104)); // range.length = clampedL
        asm.push(abi::move_register("x0", abi::LOCAL[1])); // str
        asm.call_external("_objc_msgSend", LIB_OBJC);
        // glyph = APP_GLYPH_POOLED_TAG | clampedL
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 104));
        asm.push(abi::move_immediate(
            abi::SCRATCH[1],
            "Integer",
            APP_GLYPH_POOLED_TAG,
        ));
        asm.push(abi::or_registers(
            abi::LOCAL[8],
            abi::SCRATCH[1],
            abi::SCRATCH[0],
        ));
        asm.push(abi::label("w_pool_done"));
    }
    // cell = cells + (row*cols + col)*CELL_SIZE
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[1],
        abi::LOCAL[6],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        abi::SCRATCH[2],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        4,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::LOCAL[3],
        abi::SCRATCH[3],
    )); // cell ptr
    asm.push(abi::store_u32(
        abi::LOCAL[8],
        abi::SCRATCH[3],
        CELL_GLYPH_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[4],
        abi::LOCAL[2],
        TV_CUR_FG_OFFSET,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[4],
        abi::SCRATCH[3],
        CELL_FG_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[4],
        abi::LOCAL[2],
        TV_CUR_BG_OFFSET,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[4],
        abi::SCRATCH[3],
        CELL_BG_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[4],
        abi::LOCAL[2],
        TV_CUR_BOLD_OFFSET,
    ));
    asm.push(abi::store_u8(
        abi::SCRATCH[4],
        abi::SCRATCH[3],
        CELL_BOLD_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[4],
        abi::LOCAL[2],
        TV_CUR_UNDERLINE_OFFSET,
    ));
    asm.push(abi::store_u8(
        abi::SCRATCH[4],
        abi::SCRATCH[3],
        CELL_UNDERLINE_OFFSET,
    ));
    if uses_term {
        // plan-70-D: store the display width. A wide glyph (width 2) reserves the
        // next cell as a wide-trailing sentinel (same row: the wide-at-edge wrap
        // guaranteed col <= cols-2) and advances the cursor two columns; drawRect
        // skips the trail.
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 80)); // width
        asm.push(abi::store_u8(
            abi::SCRATCH[0],
            abi::SCRATCH[3],
            CELL_WIDTH_OFFSET,
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "2"));
        asm.push(abi::branch_ne("w_advance_one"));
        asm.push(abi::add_immediate(
            abi::SCRATCH[1],
            abi::SCRATCH[3],
            CELL_SIZE,
        )); // trail cell
        asm.push(abi::move_immediate(
            abi::SCRATCH[2],
            "Integer",
            APP_WIDE_TRAIL,
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[2],
            abi::SCRATCH[1],
            CELL_GLYPH_OFFSET,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[4],
            abi::LOCAL[2],
            TV_CUR_FG_OFFSET,
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[4],
            abi::SCRATCH[1],
            CELL_FG_OFFSET,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[4],
            abi::LOCAL[2],
            TV_CUR_BG_OFFSET,
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[4],
            abi::SCRATCH[1],
            CELL_BG_OFFSET,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[4],
            abi::LOCAL[2],
            TV_CUR_BOLD_OFFSET,
        ));
        asm.push(abi::store_u8(
            abi::SCRATCH[4],
            abi::SCRATCH[1],
            CELL_BOLD_OFFSET,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[4],
            abi::LOCAL[2],
            TV_CUR_UNDERLINE_OFFSET,
        ));
        asm.push(abi::store_u8(
            abi::SCRATCH[4],
            abi::SCRATCH[1],
            CELL_UNDERLINE_OFFSET,
        ));
        asm.push(abi::move_immediate(abi::SCRATCH[4], "Integer", "0"));
        asm.push(abi::store_u8(
            abi::SCRATCH[4],
            abi::SCRATCH[1],
            CELL_WIDTH_OFFSET,
        ));
        // cursor_col += 2
        asm.push(abi::load_u64(
            abi::SCRATCH[2],
            abi::LOCAL[2],
            TV_CURSOR_COL_OFFSET,
        ));
        asm.push(abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 2));
        asm.push(abi::store_u64(
            abi::SCRATCH[2],
            abi::LOCAL[2],
            TV_CURSOR_COL_OFFSET,
        ));
        asm.push(abi::branch("w_next"));
        asm.push(abi::label("w_advance_one"));
    }
    // cursor_col++ (the sole advance for a non-term app; the width==1 arm otherwise)
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[2],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::branch("w_next"));

    // \n: col = 0, row++ (scroll if needed)
    asm.push(abi::label("w_newline"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[1], abi::LOCAL[7]));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate(abi::SCRATCH[1], abi::LOCAL[7], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::branch("w_next"));

    // \r: col = 0
    asm.push(abi::label("w_cr"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::branch("w_next"));

    // \t: col = (col & ~3) + 4, wrapping to a new line if it runs off the edge
    asm.push(abi::label("w_tab"));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::shift_right_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        2,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        2,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 4));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::LOCAL[6]));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[1], abi::LOCAL[7]));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate(abi::SCRATCH[1], abi::LOCAL[7], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::LOCAL[2],
        TV_CURSOR_ROW_OFFSET,
    ));

    asm.push(abi::label("w_next"));
    // plan-70-D Phase 2: advance i by the cluster's UTF-16 unit length (sp+88).
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 88));
    asm.push(abi::add_registers(
        abi::LOCAL[4],
        abi::LOCAL[4],
        abi::SCRATCH[0],
    ));
    asm.push(abi::branch("w_loop"));

    // Grid mutation is complete. Redraw is present-driven (plan-35-D §3): the
    // surface repaints only on the next `term::sync`/`io::flush`, never per write,
    // so a program that draws without a following present shows nothing new
    // (mandatory present, plan-35 D1).
    asm.push(abi::label("w_done"));

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
        (abi::LOCAL[6], 56),
        (abi::LOCAL[7], 64),
        (abi::LOCAL[8], 72),
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
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        // x22 carries `tkd_commit`'s remaining-byte count across `_write`
        // (bug-241). Unlike the `kd_*` sibling this helper had no other use for
        // it, so it must be saved here before being clobbered.
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
        (abi::LOCAL[6], 56),
        (abi::LOCAL[7], 64),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register(abi::LOCAL[0], "x0")); // self (TermView)
    asm.push(abi::move_register(abi::LOCAL[4], "x2")); // event

    // chars = [event characters]; if [chars length] == 0 -> done
    asm.load_selector(SEL_CHARACTERS.0);
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // chars
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("tkd_done"));
    // c = [chars characterAtIndex:0]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[5], "x0")); // char code

    // app, input line buffer, input mode.
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // app
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", INPUT_LINE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[6], "x0")); // input line buffer
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", INPUT_MODE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[7], "x0")); // input mode

    // Dispatch on the key.
    asm.push(abi::compare_immediate(
        abi::LOCAL[7],
        INPUT_MODE_RAW_NO_ECHO,
    ));
    asm.push(abi::branch_eq("tkd_raw"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "13")); // CR
    asm.push(abi::branch_eq("tkd_commit"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "10")); // LF
    asm.push(abi::branch_eq("tkd_commit"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "3")); // Enter
    asm.push(abi::branch_eq("tkd_commit"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "127")); // Delete
    asm.push(abi::branch_eq("tkd_backspace"));
    asm.push(abi::compare_immediate(abi::LOCAL[5], "8")); // Backspace
    asm.push(abi::branch_eq("tkd_backspace"));

    // Default: [inputLine appendString:chars]; echo to the surface for io.input.
    asm.load_selector(SEL_APPEND_STRING.0);
    asm.push(abi::move_register("x2", abi::LOCAL[2]));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::LOCAL[7], INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("tkd_done"));
    // [self mfbWriteString:chars]
    asm.load_selector(SEL_MFB_WRITE_STRING.0);
    asm.push(abi::move_register("x2", abi::LOCAL[2]));
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("tkd_done"));

    // Commit: deliver the buffered line + newline to the pipe; echo a newline.
    asm.push(abi::label("tkd_commit"));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[4], "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // UTF-8 bytes of the line
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // bytes still to deliver
                                                       // Deliver the whole line, resuming after a partial write (bug-241) — see
                                                       // `kd_commit`, which this mirrors.
    asm.push(abi::label("tkd_commit_write"));
    asm.push(abi::compare_immediate(abi::LOCAL[3], "0"));
    asm.push(abi::branch_eq("tkd_commit_newline"));
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.push(abi::move_register("x1", abi::LOCAL[2]));
    asm.push(abi::move_register("x2", abi::LOCAL[3]));
    asm.call_external("_write", LIB_SYSTEM);
    // O_NONBLOCK write end (bug-114): on -1/EAGAIN (pipe full, worker not
    // reading) give up on the line rather than block the UI thread; skip the
    // trailing newline write so a partial line is never terminated as a whole
    // one, and fall through to echo + clear. `<= 0` also makes the loop provably
    // terminate — each pass delivers at least one byte or leaves.
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_le("tkd_commit_echo"));
    asm.push(abi::add_registers(abi::LOCAL[2], abi::LOCAL[2], "x0"));
    asm.push(abi::subtract_registers(abi::LOCAL[3], abi::LOCAL[3], "x0"));
    asm.push(abi::branch("tkd_commit_write"));
    asm.push(abi::label("tkd_commit_newline"));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "10"));
    asm.push(abi::store_u8(abi::SCRATCH[0], abi::stack_pointer(), 72));
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 72));
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::label("tkd_commit_echo"));
    asm.push(abi::compare_immediate(abi::LOCAL[7], INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("tkd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, abi::LOCAL[2], STR_NEWLINE.0);
    asm.push(abi::move_register(abi::LOCAL[5], "x0")); // "\n" (callee-saved across load_selector)
    asm.load_selector(SEL_MFB_WRITE_STRING.0);
    asm.push(abi::move_register("x2", abi::LOCAL[5]));
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("tkd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, abi::LOCAL[2], STR_EMPTY.0);
    asm.push(abi::move_register(abi::LOCAL[5], "x0")); // empty string
    asm.load_selector(SEL_SET_STRING.0);
    asm.push(abi::move_register("x2", abi::LOCAL[5]));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("tkd_done"));

    // Backspace: drop the last character from the buffer.
    asm.push(abi::label("tkd_backspace"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("tkd_done"));
    asm.push(abi::move_register(abi::LOCAL[4], "x0")); // buffer length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", abi::LOCAL[4], 1));
    asm.push(abi::move_immediate("x3", "Integer", "1"));
    asm.push(abi::move_register("x0", abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("tkd_done"));

    // Raw read mode: write this key's UTF-8 to the input pipe; no echo/buffering.
    asm.push(abi::label("tkd_raw"));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[4], "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // UTF-8 bytes for chars
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("tkd_done"));
    asm.push(abi::move_register("x2", "x0"));
    asm.push(abi::move_register("x0", abi::LOCAL[4]));
    asm.push(abi::move_register("x1", abi::LOCAL[2]));
    asm.call_external("_write", LIB_SYSTEM);

    asm.push(abi::label("tkd_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
        (abi::LOCAL[6], 56),
        (abi::LOCAL[7], 64),
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
    // super_class@104}, minRows@112, minCols@120, oldPool@128, newPool@136.
    // plan-70-D Phase 2: the TermCell-parallel EGC pool is reallocated + overlap-
    // copied in lockstep with the cell grid (a resized pooled cluster must keep its
    // units in its new cell's slot).
    let frame = 144;
    let (off_w, off_h) = (80, 88);
    let (off_super_recv, off_super_cls) = (96, 104);
    let (off_min_rows, off_min_cols) = (112, 120);
    let (off_old_pool, off_new_pool) = (128, 136);
    let saved: [(&str, usize); 9] = [
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
        (abi::LOCAL[6], 56),
        (abi::LOCAL[7], 64),
        (abi::LOCAL[8], 72),
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
    asm.push(abi::move_register(abi::LOCAL[0], "x0")); // self
                                                       // Spill the NSSize args (d0 = width, d1 = height); the super call clobbers them.
    asm.push(abi::float_move_x_from_d(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[0],
    ));
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), off_w));
    asm.push(abi::float_move_x_from_d(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), off_h));

    // [super setFrameSize:newSize] — actually resize the NSView. Build the
    // objc_super { receiver = self; super_class = NSView } record on the stack.
    asm.push(abi::store_u64(
        abi::LOCAL[0],
        abi::stack_pointer(),
        off_super_recv,
    ));
    asm.external_data(abi::SCRATCH[0], CLASS_NS_VIEW, LIB_APPKIT);
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_super_cls,
    ));
    asm.load_selector(SEL_SET_FRAME_SIZE.0); // sel -> x1 (clobbers x0)
    asm.push(abi::add_immediate(
        "x0",
        abi::stack_pointer(),
        off_super_recv,
    )); // &super
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_w));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_h));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.call_external("_objc_msgSendSuper", LIB_OBJC);

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY); nil -> no grid yet.
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0"));
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq("sfs_done"));

    // newCols = floor(width / cellW); newRows = floor(height / cellH); each >= 1.
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_w));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CELL_W_OFFSET,
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::float_floor_to_signed_x(
        abi::LOCAL[6],
        abi::FP_SCRATCH[0],
    )); // newCols
    asm.push(abi::compare_immediate(abi::LOCAL[6], "1"));
    asm.push(abi::branch_ge("sfs_cols_ok"));
    asm.push(abi::move_immediate(abi::LOCAL[6], "Integer", "1"));
    asm.push(abi::label("sfs_cols_ok"));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), off_h));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CELL_H_OFFSET,
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::float_floor_to_signed_x(
        abi::LOCAL[5],
        abi::FP_SCRATCH[0],
    )); // newRows
    asm.push(abi::compare_immediate(abi::LOCAL[5], "1"));
    asm.push(abi::branch_ge("sfs_rows_ok"));
    asm.push(abi::move_immediate(abi::LOCAL[5], "Integer", "1"));
    asm.push(abi::label("sfs_rows_ok"));

    // old geometry.
    asm.push(abi::load_u64(abi::LOCAL[2], abi::LOCAL[1], TV_CELLS_OFFSET)); // oldCells
    asm.push(abi::load_u64(abi::LOCAL[3], abi::LOCAL[1], TV_ROWS_OFFSET)); // oldRows
    asm.push(abi::load_u64(abi::LOCAL[4], abi::LOCAL[1], TV_COLS_OFFSET)); // oldCols

    // Unchanged geometry -> nothing to do (AppKit already marks the resize dirty).
    asm.push(abi::compare_registers(abi::LOCAL[5], abi::LOCAL[3]));
    asm.push(abi::branch_ne("sfs_resize"));
    asm.push(abi::compare_registers(abi::LOCAL[6], abi::LOCAL[4]));
    asm.push(abi::branch_eq("sfs_done"));
    asm.push(abi::label("sfs_resize"));

    // newCells = calloc(newRows*newCols, CELL_SIZE); leave the grid intact on OOM.
    asm.push(abi::multiply_registers("x0", abi::LOCAL[5], abi::LOCAL[6]));
    asm.push(abi::move_immediate("x1", "Integer", &CELL_SIZE.to_string()));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::move_register(abi::LOCAL[7], "x0")); // newCells
    asm.push(abi::compare_immediate(abi::LOCAL[7], "0"));
    asm.push(abi::branch_eq("sfs_done"));

    // newPool = calloc(newRows*newCols, POOL); on OOM free newCells and bail so the
    // old grid + pool stay consistent (both old-geometry). Cache old/new pool ptrs.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_POOL_OFFSET,
    )); // oldPool
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_old_pool,
    ));
    asm.push(abi::multiply_registers("x0", abi::LOCAL[5], abi::LOCAL[6]));
    asm.push(abi::move_immediate(
        "x1",
        "Integer",
        &APP_POOL_BYTES_PER_CELL.to_string(),
    ));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::store_u64("x0", abi::stack_pointer(), off_new_pool)); // newPool
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_ne("sfs_pool_ok"));
    asm.push(abi::move_register("x0", abi::LOCAL[7])); // free the orphaned newCells
    asm.call_external("_free", LIB_SYSTEM);
    asm.push(abi::branch("sfs_done"));
    asm.push(abi::label("sfs_pool_ok"));

    // Preserve the top-left overlap: for r in 0..min(oldRows,newRows) copy
    // min(oldCols,newCols) cells (row strides differ, so copy row by row).
    asm.push(abi::move_register(abi::SCRATCH[0], abi::LOCAL[3])); // minRows = min(oldRows, newRows)
    asm.push(abi::compare_registers(abi::LOCAL[3], abi::LOCAL[5]));
    asm.push(abi::branch_le("sfs_minrows"));
    asm.push(abi::move_register(abi::SCRATCH[0], abi::LOCAL[5]));
    asm.push(abi::label("sfs_minrows"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_min_rows,
    ));
    asm.push(abi::move_register(abi::SCRATCH[0], abi::LOCAL[4])); // minCols = min(oldCols, newCols)
    asm.push(abi::compare_registers(abi::LOCAL[4], abi::LOCAL[6]));
    asm.push(abi::branch_le("sfs_mincols"));
    asm.push(abi::move_register(abi::SCRATCH[0], abi::LOCAL[6]));
    asm.push(abi::label("sfs_mincols"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_min_cols,
    ));
    // No old grid -> nothing to copy.
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq("sfs_copy_done"));

    asm.push(abi::move_immediate(abi::LOCAL[8], "Integer", "0")); // r = 0
    asm.push(abi::label("sfs_copy_loop"));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_min_rows,
    ));
    asm.push(abi::compare_registers(abi::LOCAL[8], abi::SCRATCH[0]));
    asm.push(abi::branch_ge("sfs_copy_done"));
    // dst = newCells + (r*newCols)*CELL_SIZE
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::LOCAL[8],
        abi::LOCAL[6],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        4,
    ));
    asm.push(abi::add_registers("x0", abi::LOCAL[7], abi::SCRATCH[0]));
    // src = oldCells + (r*oldCols)*CELL_SIZE
    asm.push(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::LOCAL[8],
        abi::LOCAL[4],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        4,
    ));
    asm.push(abi::add_registers("x1", abi::LOCAL[2], abi::SCRATCH[1]));
    // len = minCols * CELL_SIZE
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_min_cols,
    ));
    asm.push(abi::shift_left_immediate("x2", abi::SCRATCH[0], 4));
    asm.call_external("_memcpy", LIB_SYSTEM);
    asm.push(abi::add_immediate(abi::LOCAL[8], abi::LOCAL[8], 1));
    asm.push(abi::branch("sfs_copy_loop"));
    asm.push(abi::label("sfs_copy_done"));

    // Copy the EGC pool overlap (same min extents, POOL(64)-byte stride). newPool is
    // guaranteed non-null here; guard only the old pool.
    asm.push(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        off_old_pool,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[3], "0"));
    asm.push(abi::branch_eq("sfs_pool_copy_done"));
    asm.push(abi::move_immediate(abi::LOCAL[8], "Integer", "0")); // r = 0
    asm.push(abi::label("sfs_pool_copy_loop"));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_min_rows,
    ));
    asm.push(abi::compare_registers(abi::LOCAL[8], abi::SCRATCH[0]));
    asm.push(abi::branch_ge("sfs_pool_copy_done"));
    // dst = newPool + (r*newCols)*POOL
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        off_new_pool,
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::LOCAL[8],
        abi::LOCAL[6],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        6,
    ));
    asm.push(abi::add_registers("x0", abi::SCRATCH[2], abi::SCRATCH[0]));
    // src = oldPool + (r*oldCols)*POOL
    asm.push(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        off_old_pool,
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::LOCAL[8],
        abi::LOCAL[4],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        6,
    ));
    asm.push(abi::add_registers("x1", abi::SCRATCH[3], abi::SCRATCH[1]));
    // len = minCols * POOL
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_min_cols,
    ));
    asm.push(abi::shift_left_immediate("x2", abi::SCRATCH[0], 6));
    asm.call_external("_memcpy", LIB_SYSTEM);
    asm.push(abi::add_immediate(abi::LOCAL[8], abi::LOCAL[8], 1));
    asm.push(abi::branch("sfs_pool_copy_loop"));
    asm.push(abi::label("sfs_pool_copy_done"));

    // Publish the new grid + geometry, then free the old buffer.
    asm.push(abi::store_u64(
        abi::LOCAL[7],
        abi::LOCAL[1],
        TV_CELLS_OFFSET,
    ));
    asm.push(abi::store_u64(abi::LOCAL[5], abi::LOCAL[1], TV_ROWS_OFFSET));
    asm.push(abi::store_u64(abi::LOCAL[6], abi::LOCAL[1], TV_COLS_OFFSET));
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq("sfs_freed"));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_free", LIB_SYSTEM);
    asm.push(abi::label("sfs_freed"));

    // Publish + free the EGC pool in lockstep with the cell grid.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_new_pool,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_POOL_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_old_pool,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("sfs_pool_freed"));
    asm.push(abi::move_register("x0", abi::SCRATCH[0]));
    asm.call_external("_free", LIB_SYSTEM);
    asm.push(abi::label("sfs_pool_freed"));

    // Clamp the cursor into the new extent.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::LOCAL[5]));
    asm.push(abi::branch_lt("sfs_cur_row_ok"));
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::LOCAL[5], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_ROW_OFFSET,
    ));
    asm.push(abi::label("sfs_cur_row_ok"));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::LOCAL[6]));
    asm.push(abi::branch_lt("sfs_cur_col_ok"));
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::LOCAL[6], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::label("sfs_cur_col_ok"));

    // Full redraw of the resized surface. setFrameSize: runs on the main thread,
    // so message the view directly (no marshaling needed).
    asm.load_selector(SEL_SET_NEEDS_DISPLAY.0);
    asm.push(abi::move_immediate("x2", "Integer", "1")); // YES
    asm.push(abi::move_register("x0", abi::LOCAL[0]));
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
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(name))
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
            last.get("target").as_deref(),
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
        assert_eq!(last.get("target").as_deref(), Some("tkd_done"));
    }
}
