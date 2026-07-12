//! Linux GTK4 app-mode IO ops: `emit_app_term_*` and `emit_app_io_*` emitters
//! (terminal-size/set-color/attr/cursor/clear/move/write/flush/input) (plan-11 split).

use super::*;

/// App-mode `term::*` dispatcher. Returns the helper body for the calls the GTK
/// surface implements; the rest fall back to the console backend (no-op while the
/// arena term-state stays inactive).
pub(crate) fn emit_app_term_helper(
    call: &str,
    symbol: &str,
    tso: usize,
) -> Option<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>)> {
    let helper = match call {
        "term.on" => emit_app_term_on(symbol, tso),
        "term.off" => emit_app_term_off(symbol, tso),
        "term.isOn" => emit_app_term_is_on(symbol),
        "term.clear" => emit_app_term_clear(symbol),
        "term.moveTo" => emit_app_term_move_to(symbol),
        "term.setForeground" => {
            emit_app_term_set_color(symbol, ST_TERM_CUR_FG, tso, code::TERM_STATE_FG_OFFSET)
        }
        "term.setBackground" => {
            emit_app_term_set_color(symbol, ST_TERM_CUR_BG, tso, code::TERM_STATE_BG_OFFSET)
        }
        "term.setBold" => {
            emit_app_term_set_attr(symbol, ST_TERM_CUR_BOLD, tso, code::TERM_STATE_BOLD_OFFSET)
        }
        "term.setUnderline" => emit_app_term_set_attr(
            symbol,
            ST_TERM_CUR_UNDERLINE,
            tso,
            code::TERM_STATE_UNDERLINE_OFFSET,
        ),
        "term.terminalSize" => emit_app_term_terminal_size(symbol),
        "term.showCursor" => emit_app_term_set_cursor(symbol, "1"),
        "term.hideCursor" => emit_app_term_set_cursor(symbol, "0"),
        _ => return None,
    };
    Some(helper)
}

/// The plan-01-term §4.2.1 no-op gate: branch to `inactive` when TUI mode is off
/// (app-state `ST_TERM_ACTIVE == 0`), so every GTK term setter is a no-op while
/// inactive, matching macOS app-mode and the console backend (bug-111). Reads via
/// `load_state` (clobbers only x9), so argument registers are preserved for the
/// active path.
fn emit_gtk_term_active_gate(asm: &mut Asm, inactive: &str) {
    asm.load_state("x9", ST_TERM_ACTIVE);
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq(inactive));
}

/// `term::terminalSize()`: OK(record) where the arena-allocated 16-byte record is
/// `{ columns@0, rows@8 }` = the fixed grid size. On allocation failure, propagate
/// the allocator's error result. Result ABI: x0 = tag, x1 = record/err code.
fn emit_app_term_terminal_size(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    // While TUI mode is inactive, terminalSize is unsupported (matches macOS and
    // plan-01-term §4.2.1) rather than reporting the grid size (bug-111).
    emit_gtk_term_active_gate(&mut asm, "ts_unsupported");
    // record = arena_alloc(16, 8) -> x0=tag, x1=ptr (clobbers caller-saved).
    asm.push(abi::move_immediate("x0", "Integer", "16"));
    asm.push(abi::move_immediate("x1", "Integer", "8"));
    asm.call_internal(code::ARENA_ALLOC_SYMBOL);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_ne("ts_err")); // non-OK tag -> propagate x0/x1/x2
    asm.load_state("x9", ST_TERM_COLS);
    asm.push(abi::store_u64("x9", "x1", 0)); // columns
    asm.load_state("x9", ST_TERM_ROWS);
    asm.push(abi::store_u64("x9", "x1", 8)); // rows
    asm.push(abi::move_immediate("x0", "Integer", "0")); // OK; x1 = record
    asm.push(abi::branch("ts_err"));
    asm.push(abi::label("ts_unsupported"));
    asm.push(abi::move_immediate("x0", "Integer", code::RESULT_ERR_TAG));
    asm.push(abi::move_immediate("x1", "Integer", code::ERR_UNSUPPORTED_CODE));
    asm.local_address(code::RESULT_ERROR_MESSAGE_REGISTER, code::ERR_UNSUPPORTED_SYMBOL);
    asm.push(abi::label("ts_err"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::setForeground`/`setBackground(r /*x0*/, g /*x1*/, b /*x2*/)`: pack
/// `r|g<<8|b<<16` and store it to the arena term-state (so the console-backed
/// getters return it) and to the app current-color field (with COLOR_SET, so the
/// grid cells tag with it and explicit black stays distinct).
fn emit_app_term_set_color(
    symbol: &str,
    field: usize,
    tso: usize,
    arena_field: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    emit_gtk_term_active_gate(&mut asm, "sc_inactive"); // §4.2.1 no-op gate (bug-111)
    asm.push(abi::shift_left_immediate("x10", "x1", 8)); // g<<8
    asm.push(abi::shift_left_immediate("x11", "x2", 16)); // b<<16
    asm.push(abi::or_registers("x10", "x0", "x10")); // r | g<<8
    asm.push(abi::or_registers("x10", "x10", "x11")); // | b<<16 -> packed (pure)
    asm.push(abi::store_u64("x10", ARENA_REG, tso + arena_field)); // arena (no flags)
    asm.push(abi::move_immediate(
        "x11",
        "Integer",
        &COLOR_SET.to_string(),
    ));
    asm.push(abi::or_registers("x11", "x10", "x11")); // packed | COLOR_SET
    asm.store_state("x11", field); // app current color (x9 = store_state scratch)
    asm.push(abi::label("sc_inactive"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::setBold`/`setUnderline(enabled /*x0*/)`: store the flag to the app field
/// and the arena term-state (so the console getter returns it).
fn emit_app_term_set_attr(
    symbol: &str,
    field: usize,
    tso: usize,
    arena_field: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    emit_gtk_term_active_gate(&mut asm, "sa_inactive"); // §4.2.1 no-op gate (bug-111)
    asm.push(abi::store_u64("x0", ARENA_REG, tso + arena_field)); // arena
    asm.store_state("x0", field); // app field (store_state uses x9, x0 safe)
    asm.push(abi::label("sa_inactive"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::showCursor`/`hideCursor`: store the cursor-visible flag and redraw.
fn emit_app_term_set_cursor(
    symbol: &str,
    visible: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    emit_gtk_term_active_gate(&mut asm, "cur_inactive"); // §4.2.1 no-op gate (bug-111)
    asm.push(abi::move_immediate("x10", "Integer", visible));
    asm.store_state("x10", ST_TERM_CURSOR_VISIBLE);
    asm.local_address("x0", TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::label("cur_inactive"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::on`: reset the attributes to defaults (app + arena term-state), mark
/// active, and schedule the view swap on the main thread (plan-01-term.md §6.3).
fn emit_app_term_on(
    symbol: &str,
    tso: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    // App current attributes -> defaults (fg/bg/bold/underline cleared, cursor on).
    asm.push(abi::move_immediate("x10", "Integer", "0"));
    for field in [
        ST_TERM_CUR_FG,
        ST_TERM_CUR_BG,
        ST_TERM_CUR_BOLD,
        ST_TERM_CUR_UNDERLINE,
    ] {
        asm.store_state("x10", field);
    }
    asm.push(abi::move_immediate("x10", "Integer", "1"));
    asm.store_state("x10", ST_TERM_CURSOR_VISIBLE);
    asm.store_state("x10", ST_TERM_ACTIVE);
    // Arena term-state defaults so the console getters report them (plan §4.2.1).
    asm.push(abi::move_immediate("x10", "Integer", "1"));
    asm.push(abi::store_u64(
        "x10",
        ARENA_REG,
        tso + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::move_immediate("x10", "Integer", TERM_DEFAULT_FG));
    asm.push(abi::store_u64(
        "x10",
        ARENA_REG,
        tso + code::TERM_STATE_FG_OFFSET,
    ));
    asm.push(abi::move_immediate("x10", "Integer", "0"));
    for field in [
        code::TERM_STATE_BG_OFFSET,
        code::TERM_STATE_BOLD_OFFSET,
        code::TERM_STATE_UNDERLINE_OFFSET,
    ] {
        asm.push(abi::store_u64("x10", ARENA_REG, tso + field));
    }
    // bug-150: entering TUI mode flips the transcript into immediate single-key
    // delivery (MODE_RAW) once, so the key-press handler routes each keystroke
    // straight to the input pipe from the moment `term::on` runs instead of
    // buffering until Return. `io::input`/`io::readLine` still switch to
    // MODE_LINE_ECHO for their own read (emit_app_io_input_helper).
    asm.push(abi::move_immediate("x10", "Integer", MODE_RAW));
    asm.store_state("x10", ST_INPUT_MODE);
    asm.local_address("x0", TERM_SHOW_IDLE_SYMBOL);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::off`: clear the active flag (app + arena) and restore the transcript.
fn emit_app_term_off(
    symbol: &str,
    tso: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::move_immediate("x10", "Integer", "0"));
    asm.store_state("x10", ST_TERM_ACTIVE);
    asm.push(abi::store_u64(
        "x10",
        ARENA_REG,
        tso + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    // bug-150: leaving TUI mode returns the transcript to line input so
    // subsequent reads commit on Return again (symmetric with the console
    // `term::off` cooked-mode restore).
    asm.push(abi::move_immediate("x10", "Integer", MODE_LINE_ECHO));
    asm.store_state("x10", ST_INPUT_MODE);
    asm.local_address("x0", TERM_HIDE_IDLE_SYMBOL);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::isOn`: OK(Boolean) = the active flag. Result ABI x0=tag, x1=value.
fn emit_app_term_is_on(symbol: &str) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.load_state("x1", ST_TERM_ACTIVE); // value
    asm.push(abi::move_immediate("x0", "Integer", "0")); // tag = OK
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::clear`: blank chars to spaces, reset fg/bg cells to default (0), home the
/// cursor, schedule a redraw.
fn emit_app_term_clear(symbol: &str) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    emit_gtk_term_active_gate(&mut asm, "clr_inactive"); // §4.2.1 no-op gate (bug-111)
    // Blank the whole backing store (chars=' ', fg/bg=0).
    asm.state_array("x0", ST_TERM_CHARS);
    asm.push(abi::move_immediate("x1", "Integer", "32"));
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS).to_string(),
    ));
    asm.call_external("memset");
    asm.state_array("x0", ST_TERM_FG);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memset");
    asm.state_array("x0", ST_TERM_BG);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memset");
    asm.push(abi::move_immediate("x10", "Integer", "0"));
    asm.store_state("x10", ST_TERM_ROW);
    asm.push(abi::move_immediate("x10", "Integer", "0"));
    asm.store_state("x10", ST_TERM_COL);
    asm.local_address("x0", TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::label("clr_inactive"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

/// `term::moveTo(row /*x0*/, col /*x1*/)`: clamp to the grid and set the cursor.
fn emit_app_term_move_to(symbol: &str) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    emit_gtk_term_active_gate(&mut asm, "mt_inactive"); // §4.2.1 no-op gate (bug-111)
    // row = clamp(x0, 0, rows-1)
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_ge("mt_row_lo"));
    asm.push(abi::move_immediate("x0", "Integer", "0"));
    asm.push(abi::label("mt_row_lo"));
    asm.load_state("x9", ST_TERM_ROWS);
    asm.push(abi::subtract_immediate("x9", "x9", 1)); // rows-1
    asm.push(abi::compare_registers("x0", "x9"));
    asm.push(abi::branch_le("mt_row_hi"));
    asm.push(abi::move_register("x0", "x9"));
    asm.push(abi::label("mt_row_hi"));
    asm.store_state("x0", ST_TERM_ROW);
    // col = clamp(x1, 0, cols-1)
    asm.push(abi::compare_immediate("x1", "0"));
    asm.push(abi::branch_ge("mt_col_lo"));
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.push(abi::label("mt_col_lo"));
    asm.load_state("x9", ST_TERM_COLS);
    asm.push(abi::subtract_immediate("x9", "x9", 1)); // cols-1
    asm.push(abi::compare_registers("x1", "x9"));
    asm.push(abi::branch_le("mt_col_hi"));
    asm.push(abi::move_register("x1", "x9"));
    asm.push(abi::label("mt_col_hi"));
    asm.store_state("x1", ST_TERM_COL);
    asm.push(abi::label("mt_inactive"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    (term_frame(), asm.ins, asm.rel)
}

fn term_frame() -> CodeFrame {
    CodeFrame {
        stack_size: 0,
        callee_saved: Vec::new(),
    }
}

// --- io::* app-mode helper bodies ------------------------------------------

/// App-mode `io.print`/`io.write`/`io.printError`/`io.writeError`. The MFB string
/// object is in `x0` (`[x0]` = length, `x0+8` = UTF-8 bytes). When a transcript
/// buffer is attached, append to it; otherwise fall back to the stdout/stderr file
/// descriptor (the only path verified in headless runs). Returns `OK` (x0 = 0).
pub(crate) fn emit_app_io_write_helper(
    symbol: &str,
    stderr: bool,
    newline: bool,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let fd = if stderr { "2" } else { "1" };
    let mut asm = Asm::new(symbol);
    // lr@0, x19(string)@8, x20(len)@16, x21(heap chunk)@24, newline byte@32.
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
    asm.push(abi::move_register("x19", "x0")); // preserve string object

    // term:: active -> render into the TUI grid instead of the transcript.
    asm.load_state("x9", ST_TERM_ACTIVE);
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("not_term"));
    asm.push(abi::move_register("x0", "x19")); // string obj
    asm.push(abi::move_immediate(
        "x1",
        "Integer",
        if newline { "1" } else { "0" },
    ));
    asm.call_internal(TERM_WRITE_SYMBOL);
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));
    asm.push(abi::label("not_term"));

    // buffer = state.text_buffer; nil => fd fallback (headless / pre-window).
    asm.load_state("x10", ST_TEXT_BUFFER);
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_eq("fd_path"));

    // --- transcript path: marshal to the GTK main thread (plan-05 §6.4) ---
    // GTK is not thread-safe, so the worker copies the bytes into a heap chunk and
    // schedules an idle source; the main loop drains it via _mfb_gtkapp_append_idle.
    // Chunk layout: [0]=len (u64), [16..]=bytes. stderr runs are prefixed with
    // "[stderr] " (matching macOS) so error output is visually distinguished.
    let prefix_len = if stderr { STR_STDERR_PREFIX.1.len() } else { 0 };
    let extra = prefix_len + if newline { 1 } else { 0 };
    asm.push(abi::load_u64("x20", "x19", 0)); // text len
    asm.push(abi::add_immediate("x0", "x20", prefix_len + 17)); // 16 hdr + prefix + text + nl
    asm.call_external("malloc");
    asm.push(abi::move_register("x21", "x0")); // heap chunk
    if stderr {
        asm.push(abi::add_immediate("x0", "x21", 16)); // memcpy(chunk+16, "[stderr] ", 9)
        asm.local_address("x1", STR_STDERR_PREFIX.0);
        asm.push(abi::move_immediate(
            "x2",
            "Integer",
            &prefix_len.to_string(),
        ));
        asm.call_external("memcpy");
    }
    asm.push(abi::add_immediate("x0", "x21", 16 + prefix_len)); // memcpy(dst=chunk+16+prefix,
    asm.push(abi::add_immediate("x1", "x19", 8)); //                     src=text bytes,
    asm.push(abi::move_register("x2", "x20")); //                       n=text len)
    asm.call_external("memcpy");
    if newline {
        asm.push(abi::add_immediate("x9", "x21", 16 + prefix_len));
        asm.push(abi::add_registers("x9", "x9", "x20")); // &chunk[16+prefix+len]
        asm.push(abi::move_immediate("x10", "Integer", "10"));
        asm.push(abi::store_u8("x10", "x9", 0)); // '\n'
    }
    asm.push(abi::add_immediate("x9", "x20", extra)); // chunk len = text + prefix + nl
    asm.push(abi::store_u64("x9", "x21", 0));
    asm.local_address("x0", APPEND_IDLE_SYMBOL);
    asm.push(abi::move_register("x1", "x21")); // user_data = chunk
    asm.call_external("g_idle_add");
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));

    // --- fd fallback path ---
    asm.push(abi::label("fd_path"));
    asm.push(abi::move_immediate("x0", "Integer", fd));
    asm.push(abi::add_immediate("x1", "x19", 8));
    asm.push(abi::load_u64("x2", "x19", 0));
    asm.call_external("write");
    if newline {
        asm.push(abi::move_immediate("x9", "Integer", "10"));
        asm.push(abi::store_u8("x9", abi::stack_pointer(), 32));
        asm.push(abi::move_immediate("x0", "Integer", fd));
        asm.push(abi::add_immediate("x1", abi::stack_pointer(), 32));
        asm.push(abi::move_immediate("x2", "Integer", "1"));
        asm.call_external("write");
    }
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG

    asm.push(abi::label("done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode `io.flush`: returns `OK` immediately. SCAFFOLD: real flush must
/// drain the pending main-thread transcript update (§5.4) once
/// marshaling lands.
pub(crate) fn emit_app_io_flush_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode `io.input` (plan-05 §5.4): switch the transcript to echo mode (so the
/// user sees what they type, like the macOS `io::input` path), render the prompt
/// via the `io.write` helper, then read a committed line via the `io.readLine`
/// helper (which reads fd 0 — the window-input pipe). Prompt string is in `x0`.
pub(crate) fn emit_app_io_input_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 8)); // preserve prompt
    asm.push(abi::move_immediate("x10", "Integer", MODE_LINE_ECHO));
    asm.store_state("x10", ST_INPUT_MODE);
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8)); // prompt
    asm.call_internal(IO_WRITE_SYMBOL); // x0 = prompt; result ignored
    asm.call_internal(IO_READ_LINE_SYMBOL); // result in x0/x1/x2
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode `io.isInputTerminal`/`io.isOutputTerminal`/`io.isErrorTerminal`
/// (plan-05 §5.4): the window is the interactive console, so all three return
/// `OK(TRUE)`. Result ABI: x0 = tag (0 = ok), x1 = value.
pub(crate) fn emit_app_io_is_terminal_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x1", "Boolean", "1")); // value = TRUE
    asm.push(abi::move_immediate("x0", "Integer", "0")); // tag = OK
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode raw key input (plan-05 §5.4): set the transcript to RAW mode so each
/// keystroke's bytes go straight to the input pipe. Appended inline at the start of
/// the `io.readChar`/`io.readByte` helpers (the GTK analog of macOS
/// `emit_set_raw_input_mode`).
pub(crate) fn emit_set_raw_input_mode(
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    from: &str,
) {
    // Injected into shared helper bodies (`io_helpers::lower_io_read_char_helper`),
    // so the scratch is spelled through the neutral token pool (plan-34-D);
    // realized to the same x10 at the selection seam.
    let mut asm = Asm::new(from);
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", MODE_RAW));
    asm.store_state(abi::SCRATCH[1], ST_INPUT_MODE);
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}
