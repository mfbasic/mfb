//! Linux GTK4 app-mode IO ops: `emit_app_term_*` and `emit_app_io_*` emitters
//! (terminal-size/set-color/attr/cursor/clear/move/write/flush/input) (plan-11 split).

use super::*;
use crate::codegen::engine::util::Vregs;

/// App-mode `term::*` dispatcher. Returns the helper body for the calls the GTK
/// surface implements; the rest fall back to the console backend (no-op while the
/// arena term-state stays inactive).
pub(crate) fn emit_app_term_helper(
    call: &str,
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Option<Result<(), String>> {
    match call {
        "term.on" => emit_app_term_on(symbol, tso, instructions, relocations),
        "term.off" => emit_app_term_off(symbol, tso, instructions, relocations),
        "term.isOn" => emit_app_term_is_on(symbol, instructions, relocations),
        "term.didResize" => emit_app_term_did_resize(symbol, instructions, relocations),
        "term.clear" => emit_app_term_clear(symbol, instructions, relocations),
        "term.sync" => emit_app_term_sync(symbol, instructions, relocations),
        "term.moveTo" => emit_app_term_move_to(symbol, instructions, relocations),
        "term.setForeground" => emit_app_term_set_color(
            symbol,
            ST_TERM_CUR_FG,
            tso,
            crate::codegen::error::constants::TERM_STATE_FG_OFFSET,
            instructions,
            relocations,
        ),
        "term.setBackground" => emit_app_term_set_color(
            symbol,
            ST_TERM_CUR_BG,
            tso,
            crate::codegen::error::constants::TERM_STATE_BG_OFFSET,
            instructions,
            relocations,
        ),
        "term.setBold" => emit_app_term_set_attr(
            symbol,
            ST_TERM_CUR_BOLD,
            tso,
            crate::codegen::error::constants::TERM_STATE_BOLD_OFFSET,
            instructions,
            relocations,
        ),
        "term.setUnderline" => emit_app_term_set_attr(
            symbol,
            ST_TERM_CUR_UNDERLINE,
            tso,
            crate::codegen::error::constants::TERM_STATE_UNDERLINE_OFFSET,
            instructions,
            relocations,
        ),
        "term.terminalSize" => emit_app_term_terminal_size(symbol, instructions, relocations),
        "term.showCursor" => emit_app_term_set_cursor(symbol, "1", instructions, relocations),
        "term.hideCursor" => emit_app_term_set_cursor(symbol, "0", instructions, relocations),
        _ => return None,
    }
    Some(Ok(()))
}

/// `term::sync()` app arm (plan-35-E): the single coalesced present. Schedules ONE
/// main-loop present via the redraw-idle, which marshals a consistent snapshot of the
/// live grid then `queue_draw`s (see [`term_draw::emit_term_redraw_idle_helper`]), so
/// the draw never observes a torn frame. A clean no-op while TUI mode is off (the
/// §4.2.1 gate), matching the console `term::sync` no-op.
fn emit_app_term_sync(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the `abi_function` vreg finalizer
    // builds it and saves lr across `g_idle_add`. No value is held across the call,
    // so no vregs are needed; the gate/g_idle_add staging uses role tokens only.
    let mut asm = Asm::new(symbol);
    emit_gtk_term_active_gate(&mut asm, "sync_inactive"); // no-op present while off
    asm.local_address(abi::c_arg(0), TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::label("sync_inactive"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// The plan-01-term §4.2.1 no-op gate: branch to `inactive` when TUI mode is off
/// (app-state `ST_TERM_ACTIVE == 0`), so every GTK term setter is a no-op while
/// inactive, matching macOS app-mode and the console backend (bug-111). Reads via
/// `load_state` (clobbers only the first scratch register, realized `x9`), so
/// argument registers are preserved for the active path. Spelled with the neutral
/// scratch token — not raw `x9` — because `io::flush`'s app body appends this gate
/// into a vreg-finalized `abi_function` body (plan-101), where a physical register
/// would trip the plan-34-D guard; it realizes to the same `x9` in the standalone
/// `term::` bodies, keeping their native goldens byte-identical.
fn emit_gtk_term_active_gate(asm: &mut Asm, inactive: &str) {
    asm.load_state(abi::SCRATCH[0], ST_TERM_ACTIVE);
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(inactive));
}

/// `term::terminalSize()`: OK(record) where the arena-allocated 16-byte record is
/// `{ columns@0, rows@8 }` = the fixed grid size. On allocation failure, propagate
/// the allocator's error result. Result ABI: x0 = tag, x1 = record/err code.
fn emit_app_term_terminal_size(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): the `abi_function` finalizer builds the frame and
    // saves lr across arena_alloc. No value is held across that call (columns/rows
    // are re-read from the state global afterwards, the record pointer is the call
    // result), so no vregs are needed — role tokens only.
    let mut asm = Asm::new(symbol);
    // While TUI mode is inactive, terminalSize is unsupported (matches macOS and
    // plan-01-term §4.2.1) rather than reporting the grid size (bug-111).
    emit_gtk_term_active_gate(&mut asm, "ts_unsupported");
    // record = arena_alloc(16, 8) -> x0=tag, x1=ptr (clobbers caller-saved).
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "16"));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    asm.call_internal(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_ne("ts_err")); // non-OK tag -> propagate x0/x1/x2
    asm.load_state(abi::SCRATCH[0], ST_TERM_COLS);
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::c_arg(1), 0)); // columns
    asm.load_state(abi::SCRATCH[0], ST_TERM_ROWS);
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::c_arg(1), 8)); // rows
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // OK; x1 = record
    asm.push(abi::branch("ts_err"));
    // plan-88-C: code + message symbol from `ERRORCODE_CONSTANTS` (this app helper
    // returns via x0/x1 and the custom `asm.local_address`, so the shared
    // `raise_error_into` does not fit; table-sourced, byte-identical).
    let (unsupported_code, unsupported_symbol) =
        crate::codegen::registry::runtime_error_emission("ErrUnsupported")
            .expect("ErrUnsupported is an errorCode constant");
    asm.push(abi::label("ts_unsupported"));
    asm.push(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        crate::codegen::error::constants::RESULT_ERR_TAG,
    ));
    asm.push(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        unsupported_code,
    ));
    asm.local_address(
        crate::codegen::error::constants::RESULT_ERROR_MESSAGE_REGISTER,
        unsupported_symbol,
    );
    asm.push(abi::label("ts_err"));
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
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
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no calls, so no vregs and no own frame; the pinned
    // arena base is the arch-neutral `abi::ARENA` (NOT raw `x19`, which is wrong on
    // x86-64) and scratch is the neutral SCRATCH pool.
    let mut asm = Asm::new(symbol);
    emit_gtk_term_active_gate(&mut asm, "sc_inactive"); // §4.2.1 no-op gate (bug-111)
    asm.push(abi::shift_left_immediate(abi::SCRATCH[1], abi::c_arg(1), 8)); // g<<8
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::c_arg(2),
        16,
    )); // b<<16
    asm.push(abi::or_registers(
        abi::SCRATCH[1],
        abi::c_arg(0),
        abi::SCRATCH[1],
    )); // r | g<<8
    asm.push(abi::or_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[2],
    )); // | b<<16 -> packed (pure)
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::ARENA,
        tso + arena_field,
    )); // arena (no flags)
    asm.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        &COLOR_SET.to_string(),
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[1],
        abi::SCRATCH[2],
    )); // packed | COLOR_SET
    asm.store_state(abi::SCRATCH[2], field); // app current color (SCRATCH[0] = store_state scratch)
    asm.push(abi::label("sc_inactive"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::setBold`/`setUnderline(enabled /*x0*/)`: store the flag to the app field
/// and the arena term-state (so the console getter returns it).
fn emit_app_term_set_attr(
    symbol: &str,
    field: usize,
    tso: usize,
    arena_field: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no calls, no vregs, no own frame; arena base via the
    // arch-neutral `abi::ARENA`.
    let mut asm = Asm::new(symbol);
    emit_gtk_term_active_gate(&mut asm, "sa_inactive"); // §4.2.1 no-op gate (bug-111)
    asm.push(abi::store_u64(abi::c_arg(0), abi::ARENA, tso + arena_field)); // arena
    asm.store_state(abi::c_arg(0), field); // app field (store_state uses SCRATCH[0], c_arg(0) safe)
    asm.push(abi::label("sa_inactive"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::showCursor`/`hideCursor`: store the cursor-visible flag and redraw.
fn emit_app_term_set_cursor(
    symbol: &str,
    visible: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): the finalizer builds the frame and saves lr across
    // g_idle_add. The visible flag is stored before the call, so no vreg is needed;
    // scratch is the neutral SCRATCH pool.
    let mut asm = Asm::new(symbol);
    emit_gtk_term_active_gate(&mut asm, "cur_inactive"); // §4.2.1 no-op gate (bug-111)
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", visible));
    asm.store_state(abi::SCRATCH[1], ST_TERM_CURSOR_VISIBLE);
    asm.local_address(abi::c_arg(0), TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::label("cur_inactive"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::on`: reset the attributes to defaults (app + arena term-state), mark
/// active, and schedule the view swap on the main thread (plan-01-term.md §6.3).
fn emit_app_term_on(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): the finalizer builds the frame and saves lr across
    // g_idle_add. Every state store happens before the call, so no vreg is needed;
    // arena base via the arch-neutral `abi::ARENA`, scratch via the SCRATCH pool.
    let mut asm = Asm::new(symbol);
    // App current attributes -> defaults (fg/bg/bold/underline cleared, cursor on).
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    for field in [
        ST_TERM_CUR_FG,
        ST_TERM_CUR_BG,
        ST_TERM_CUR_BOLD,
        ST_TERM_CUR_UNDERLINE,
        // planning/term.md #11: entering TUI mode starts with no pending resize.
        ST_TERM_DID_RESIZE,
    ] {
        asm.store_state(abi::SCRATCH[1], field);
    }
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "1"));
    asm.store_state(abi::SCRATCH[1], ST_TERM_CURSOR_VISIBLE);
    asm.store_state(abi::SCRATCH[1], ST_TERM_ACTIVE);
    // Arena term-state defaults so the console getters report them (plan §4.2.1).
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::ARENA,
        tso + crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        TERM_DEFAULT_FG,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::ARENA,
        tso + crate::codegen::error::constants::TERM_STATE_FG_OFFSET,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    for field in [
        crate::codegen::error::constants::TERM_STATE_BG_OFFSET,
        crate::codegen::error::constants::TERM_STATE_BOLD_OFFSET,
        crate::codegen::error::constants::TERM_STATE_UNDERLINE_OFFSET,
    ] {
        asm.push(abi::store_u64(abi::SCRATCH[1], abi::ARENA, tso + field));
    }
    // bug-150: entering TUI mode flips the transcript into immediate single-key
    // delivery (MODE_RAW) once, so the key-press handler routes each keystroke
    // straight to the input pipe from the moment `term::on` runs instead of
    // buffering until Return. `io::input`/`io::readLine` still switch to
    // MODE_LINE_ECHO for their own read (emit_app_io_input).
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", MODE_RAW));
    asm.store_state(abi::SCRATCH[1], ST_INPUT_MODE);
    asm.local_address(abi::c_arg(0), TERM_SHOW_IDLE_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::off`: clear the active flag (app + arena) and restore the transcript.
fn emit_app_term_off(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): the finalizer builds the frame and saves lr across
    // the two g_idle_add calls; all state stores precede them, so no vreg is needed.
    let mut asm = Asm::new(symbol);
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    asm.store_state(abi::SCRATCH[1], ST_TERM_ACTIVE);
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::ARENA,
        tso + crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
    ));
    // bug-150: leaving TUI mode returns the transcript to line input so
    // subsequent reads commit on Return again (symmetric with the console
    // `term::off` cooked-mode restore).
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        MODE_LINE_ECHO,
    ));
    asm.store_state(abi::SCRATCH[1], ST_INPUT_MODE);
    // plan-35-E: schedule a final present (snapshot + queue_draw) BEFORE the hide
    // idle, so the last drawn frame is marshaled before the surface is swapped back
    // to the transcript. Idle sources drain FIFO, so the present runs first.
    asm.local_address(abi::c_arg(0), TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.local_address(abi::c_arg(0), TERM_HIDE_IDLE_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::isOn`: OK(Boolean) = the active flag. Result ABI x0=tag, x1=value.
fn emit_app_term_is_on(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): leaf (no calls), so no vregs and no own frame.
    let mut asm = Asm::new(symbol);
    asm.load_state(abi::c_arg(1), ST_TERM_ACTIVE); // value
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // tag = OK
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::didResize` (planning/term.md #11): OK(Boolean) = the cached resize flag,
/// read-and-cleared so it latches from a genuine window resize until observed. The
/// flag lives in the address-based GTK state global (set by `_mfb_gtkapp_term_resize`
/// on the main loop), so this worker-side read needs no arena register. Result ABI
/// x0=tag, x1=value; leaf helper (state access is adrp/add + ldr/str, no call).
fn emit_app_term_did_resize(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): leaf (no calls), so no vregs and no own frame.
    let mut asm = Asm::new(symbol);
    asm.load_state(abi::c_arg(1), ST_TERM_DID_RESIZE); // value (uses SCRATCH[0] for the address)
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.store_state(abi::c_arg(2), ST_TERM_DID_RESIZE); // clear (uses SCRATCH[0] for the address)
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // tag = OK
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::clear`: blank chars to spaces, reset fg/bg cells to default (0), home the
/// cursor, schedule a redraw.
fn emit_app_term_clear(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): the finalizer builds the frame and saves lr across
    // the three memset calls and g_idle_add; nothing is held across any of them, so
    // no vreg is needed.
    let mut asm = Asm::new(symbol);
    emit_gtk_term_active_gate(&mut asm, "clr_inactive"); // §4.2.1 no-op gate (bug-111)
                                                         // Blank the whole backing store (chars/fg/bg = 0). chars clears to 0 rather
                                                         // than ' ': cells are u32 since bug-203, and `memset` writes whole bytes, so
                                                         // ' ' would pack four spaces per cell. The draw renders 0 as blank.
    asm.state_array(abi::c_arg(0), ST_TERM_CHARS);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memset");
    asm.state_array(abi::c_arg(0), ST_TERM_FG);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memset");
    asm.state_array(abi::c_arg(0), ST_TERM_BG);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memset");
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    asm.store_state(abi::SCRATCH[1], ST_TERM_ROW);
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    asm.store_state(abi::SCRATCH[1], ST_TERM_COL);
    asm.local_address(abi::c_arg(0), TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::label("clr_inactive"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::moveTo(row /*x0*/, col /*x1*/)`: clamp to the grid and set the cursor.
fn emit_app_term_move_to(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): leaf clamp (state access is adrp/ldr/str, no call),
    // so no vregs and no own frame; the incoming row/col in c_arg(0)/c_arg(1) survive
    // and scratch is the neutral SCRATCH pool.
    let mut asm = Asm::new(symbol);
    emit_gtk_term_active_gate(&mut asm, "mt_inactive"); // §4.2.1 no-op gate (bug-111)
                                                        // row = clamp(x0, 0, rows-1)
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_ge("mt_row_lo"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    asm.push(abi::label("mt_row_lo"));
    asm.load_state(abi::SCRATCH[0], ST_TERM_ROWS);
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1)); // rows-1
    asm.push(abi::compare_registers(abi::c_arg(0), abi::SCRATCH[0]));
    asm.push(abi::branch_le("mt_row_hi"));
    asm.push(abi::move_register(abi::c_arg(0), abi::SCRATCH[0]));
    asm.push(abi::label("mt_row_hi"));
    asm.store_state(abi::c_arg(0), ST_TERM_ROW);
    // col = clamp(x1, 0, cols-1)
    asm.push(abi::compare_immediate(abi::c_arg(1), "0"));
    asm.push(abi::branch_ge("mt_col_lo"));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.push(abi::label("mt_col_lo"));
    asm.load_state(abi::SCRATCH[0], ST_TERM_COLS);
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1)); // cols-1
    asm.push(abi::compare_registers(abi::c_arg(1), abi::SCRATCH[0]));
    asm.push(abi::branch_le("mt_col_hi"));
    asm.push(abi::move_register(abi::c_arg(1), abi::SCRATCH[0]));
    asm.push(abi::label("mt_col_hi"));
    asm.store_state(abi::c_arg(1), ST_TERM_COL);
    asm.push(abi::label("mt_inactive"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

// --- io::* app-mode helper bodies ------------------------------------------

/// App-mode append for `io.print`/`io.write`/`io.printError`/`io.writeError`
/// (plan-101 append shape). The MFB string object is in the first arg register
/// (`[arg0]` = length, `arg0+8` = UTF-8 bytes). When a transcript buffer is
/// attached, append to it; otherwise fall back to the stdout/stderr file
/// descriptor (the only path verified in headless runs). Returns `OK`
/// (`mfb_return(0)` = 0).
///
/// Appends its vreg stream into the caller's `abi_function` body; the wrapper
/// builds the frame and saves the callee-saved vregs held across the C calls (the
/// old standalone helper managed its own frame + raw x19-x21 spills, which cannot
/// appear in a vreg-finalized body, plan-34-D). The fd-fallback newline byte lives
/// in the member-reserved local scratch at `sp+0` (see `lower_write_family`).
pub(crate) fn emit_app_io_write(
    symbol: &str,
    stderr: bool,
    newline: bool,
    _term_state_offset: Option<usize>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let fd = if stderr { "2" } else { "1" };
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let v_str = vregs.next(); // string object (the arg), live to the end
    let v_len = vregs.next(); // text length
    let v_chunk = vregs.next(); // heap chunk marshaled to the main loop
    asm.push(abi::move_register(&v_str, abi::c_arg(0))); // preserve string object

    // term:: active -> render into the TUI grid instead of the transcript.
    asm.load_state(abi::SCRATCH[1], ST_TERM_ACTIVE);
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
    asm.push(abi::branch_eq("not_term"));
    asm.push(abi::move_register(abi::c_arg(0), &v_str)); // string obj
    asm.push(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        if newline { "1" } else { "0" },
    ));
    asm.call_internal(TERM_WRITE_SYMBOL);
    asm.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));
    asm.push(abi::label("not_term"));

    // buffer = state.text_buffer; nil => fd fallback (headless / pre-window).
    asm.load_state(abi::SCRATCH[1], ST_TEXT_BUFFER);
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
    asm.push(abi::branch_eq("fd_path"));

    // --- transcript path: marshal to the GTK main thread (plan-05 §6.4) ---
    // GTK is not thread-safe, so the worker copies the bytes into a heap chunk and
    // schedules an idle source; the main loop drains it via _mfb_gtkapp_append_idle.
    // Chunk layout: [0]=len (u64), [16..]=bytes. stderr runs are prefixed with
    // "[stderr] " (matching macOS) so error output is visually distinguished.
    let prefix_len = if stderr { STR_STDERR_PREFIX.1.len() } else { 0 };
    let extra = prefix_len + if newline { 1 } else { 0 };
    asm.push(abi::load_u64(&v_len, &v_str, 0)); // text len
    asm.push(abi::add_immediate(abi::c_arg(0), &v_len, prefix_len + 17)); // 16 hdr + prefix + text + nl
    asm.call_external("malloc");
    asm.push(abi::move_register(&v_chunk, abi::c_return(0))); // heap chunk
                                                              // On allocation failure the memcpy below would fault on the worker thread
                                                              // (bug-240). Degrade to the fd path instead: it needs no allocation, so the
                                                              // output still reaches the user rather than killing the program.
    asm.push(abi::compare_immediate(&v_chunk, "0"));
    asm.push(abi::branch_eq("fd_path"));
    if stderr {
        asm.push(abi::add_immediate(abi::c_arg(0), &v_chunk, 16)); // memcpy(chunk+16, "[stderr] ", 9)
        asm.local_address(abi::c_arg(1), STR_STDERR_PREFIX.0);
        asm.push(abi::move_immediate(
            abi::c_arg(2),
            "Integer",
            &prefix_len.to_string(),
        ));
        asm.call_external("memcpy");
    }
    asm.push(abi::add_immediate(abi::c_arg(0), &v_chunk, 16 + prefix_len)); // memcpy(dst=chunk+16+prefix,
    asm.push(abi::add_immediate(abi::c_arg(1), &v_str, 8)); //                     src=text bytes,
    asm.push(abi::move_register(abi::c_arg(2), &v_len)); //                       n=text len)
    asm.call_external("memcpy");
    if newline {
        asm.push(abi::add_immediate(
            abi::SCRATCH[1],
            &v_chunk,
            16 + prefix_len,
        ));
        asm.push(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[1], &v_len)); // &chunk[16+prefix+len]
        asm.push(abi::move_immediate(abi::SCRATCH[2], "Integer", "10"));
        asm.push(abi::store_u8(abi::SCRATCH[2], abi::SCRATCH[1], 0)); // '\n'
    }
    asm.push(abi::add_immediate(abi::SCRATCH[1], &v_len, extra)); // chunk len = text + prefix + nl
    asm.push(abi::store_u64(abi::SCRATCH[1], &v_chunk, 0));
    asm.local_address(abi::c_arg(0), APPEND_IDLE_SYMBOL);
    asm.push(abi::move_register(abi::c_arg(1), &v_chunk)); // user_data = chunk
    asm.call_external("g_idle_add");
    asm.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));

    // --- fd fallback path ---
    asm.push(abi::label("fd_path"));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", fd));
    asm.push(abi::add_immediate(abi::c_arg(1), &v_str, 8));
    asm.push(abi::load_u64(abi::c_arg(2), &v_str, 0));
    asm.call_external("write");
    if newline {
        // '\n' lives in the member-reserved local scratch (sp+0); the fd path needs
        // no writable data object.
        asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "10"));
        asm.push(abi::store_u8(abi::SCRATCH[1], abi::stack_pointer(), 0));
        asm.push(abi::move_immediate(abi::c_arg(0), "Integer", fd));
        asm.push(abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), 0));
        asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "1"));
        asm.call_external("write");
    }
    asm.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // RESULT_OK_TAG

    asm.push(abi::label("done"));
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// App-mode `io.flush` (plan-35-E): while TUI mode is active, flushing drives the
/// same coalesced present as `term::sync` — one main-loop present via the redraw-idle
/// (snapshot the live grid, then `queue_draw`). While TUI is off it is a no-op
/// (transcript writes are already marshaled synchronously), returning `OK`.
pub(crate) fn emit_app_io_flush(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the `abi_function` vreg finalizer
    // builds it. `g_idle_add` args in `c_arg`; the OK tag in the result register
    // `mfb_return(0)` (not `c_arg(0)`).
    let mut asm = Asm::new(symbol);
    emit_gtk_term_active_gate(&mut asm, "flush_inactive"); // present only while TUI on
    asm.local_address(abi::c_arg(0), TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("g_idle_add");
    asm.push(abi::label("flush_inactive"));
    asm.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// App-mode append for `io.input` (plan-05 §5.4, plan-101 append shape): switch
/// the transcript to echo mode (so the user sees what they type, like the macOS
/// `io::input` path), render the prompt via the `io.write` helper, then read a
/// committed line via the `io.readLine` helper (which reads fd 0 — the window-input
/// pipe). The prompt string is already in the first arg register on entry, held in
/// a callee-saved vreg across the set-input-mode sequence; the `abi_function`
/// wrapper builds the frame and saves lr.
pub(crate) fn emit_app_io_input(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let v_prompt = vregs.next();
    asm.push(abi::move_register(&v_prompt, abi::mfb_arg(0))); // preserve prompt
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        MODE_LINE_ECHO,
    ));
    asm.store_state(abi::SCRATCH[1], ST_INPUT_MODE);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_prompt)); // prompt
    asm.call_internal(IO_WRITE_SYMBOL); // arg0 = prompt; result ignored
    asm.call_internal(IO_READ_LINE_SYMBOL); // result in the return registers
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// App-mode `io.isInputTerminal`/`io.isOutputTerminal`/`io.isErrorTerminal`
/// (plan-05 §5.4): the window is the interactive console, so all three return
/// `OK(TRUE)`. Result ABI: x0 = tag (0 = ok), x1 = value.
pub(crate) fn emit_app_io_is_terminal(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) {
    let _ = symbol;
    // Result registers via the abstract ABI tokens (`mfb_return(1)` = value,
    // `mfb_return(0)` = tag), so the `abi_function` vreg finalizer accepts the
    // appended stream (plan-101).
    instructions.push(abi::move_immediate(abi::mfb_return(1), "Boolean", "1")); // value = TRUE
    instructions.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // tag = OK
    instructions.push(abi::return_());
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
