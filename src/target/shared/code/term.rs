//! Native code generation for the built-in `term::` console backend
//! (plan-01-term.md §6.1, Phase 2). Each helper emits a self-contained AArch64
//! runtime function that updates the term-state global (writable slots in the
//! program-entry frame, reached off the pinned arena-state register `x19` at
//! `term_state_offset`) and writes ANSI escape sequences to stdout.
//!
//! The §4.2.1 gate lives here: every helper except `term::on`/`term::isOn`
//! begins by loading the `active` flag and short-circuiting to a no-op (or, for
//! getters, the inert default) while TUI mode is off.

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

// Frame layout. `LOCALS_SIZE` is the size of this locals region, which
// `finalize_vreg_body_with_locals` rounds to 16 and reserves; the vreg frame
// owns saving the link register, not a slot named here. The
// Darwin variadic `ioctl` spill is handled by the macOS `emit_terminal_size`
// hook, which brackets its own `sub_sp`/`str x2, [sp]`/`add_sp` around the call
// rather than commandeering a fixed slot here.
const LOCALS_SIZE: usize = 64;
const ARG0_OFFSET: usize = 8;
const ARG1_OFFSET: usize = 16;
/// Scratch buffer for runtime decimal formatting and the `winsize` struct.
const SCRATCH_OFFSET: usize = 32;
const SCRATCH_END: usize = 56;

const DARWIN_TIOCGWINSZ: &str = "1074295912";
const LINUX_TIOCGWINSZ: &str = "21523";

// Fixed ANSI escape-sequence byte strings (ESC = 0x1b). `term::on` resets state
// to defaults and switches to the alternate screen; `term::off` restores it.
//
// These two are the whole set. plan-35-C moved every other sequence — clear,
// bold, underline, cursor show/hide, the SGR colour prefixes, and the cursor
// -addressing pieces — into `term_grid.rs`'s `append_const`, which composes them
// into the shadow-grid diff. The thirteen leftover data objects here were still
// emitted into every binary that used `term::` while no emitted code referenced
// them (bug-326-A21).
const ESC_ON: &[u8] =
    b"\x1b[?1049h\x1b[0m\x1b[38;2;255;255;255m\x1b[48;2;0;0;0m\x1b[2J\x1b[H\x1b[?25h";
const ESC_OFF: &[u8] = b"\x1b[?25h\x1b[?1049l\x1b[0m";

const ESC_ON_SYMBOL: &str = "_mfb_term_esc_on";
const ESC_OFF_SYMBOL: &str = "_mfb_term_esc_off";

const TERM_COLOR_RECORD_SIZE: usize = 24;
const TERM_SIZE_RECORD_SIZE: usize = 16;
/// Default foreground while inactive (white, packed `r | g<<8 | b<<16`).
const DEFAULT_FOREGROUND_PACKED: &str = "16777215";

fn esc_entries() -> &'static [(&'static str, &'static [u8])] {
    &[(ESC_ON_SYMBOL, ESC_ON), (ESC_OFF_SYMBOL, ESC_OFF)]
}

/// Read-only data objects for the fixed escape-sequence byte strings.
pub(super) fn console_data_objects() -> Vec<CodeDataObject> {
    esc_entries()
        .iter()
        .map(|(symbol, bytes)| CodeDataObject {
            symbol: (*symbol).to_string(),
            kind: "raw".to_string(),
            layout: "ANSI escape sequence (raw bytes)".to_string(),
            align: 1,
            size: bytes.len(),
            value: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        })
        .collect()
}

fn data_reloc(from: &str, symbol: &str, kind: RelocIntent) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind,
        binding: "data".to_string(),
        library: None,
    }
}

/// Materialize the address of a data symbol into `dst` (adrp + add page-off).
fn load_data_address(
    from: &str,
    symbol: &str,
    dst: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::load_page_address(dst, symbol));
    relocations.push(data_reloc(from, symbol, RelocIntent::DataAddrHi));
    instructions.push(abi::add_page_offset(dst, dst, symbol));
    relocations.push(data_reloc(from, symbol, RelocIntent::DataAddrLo));
}

/// Emit a write of a fixed escape-sequence byte string to stdout (fd 1). The
/// write result is intentionally ignored: a failed escape write is not a program
/// error (term setters are best-effort, plan §4.2.1 / §9.4).
fn emit_write_const(ctx: &mut EmitCtx, data_symbol: &str, len: usize) -> Result<(), String> {
    // This helper's original parameters were (from, symbol): `from` is the
    // emitting function (each relocation's source) and `symbol` is the constant
    // being addressed. They are NOT the usual order, so name them explicitly —
    // conflating them emits relocations sourced from the data symbol, which the
    // linker rejects with "relocation source does not match function".
    let from = ctx.symbol;
    let symbol = data_symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    load_data_address(
        from,
        symbol,
        abi::string_data_register(),
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::move_immediate(
        abi::string_length_register(),
        "Integer",
        &len.to_string(),
    ));
    ctx.instructions
        .push(abi::move_immediate(abi::return_register(), "Integer", "1"));
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)
}

/// Load `active` and branch to `target` when TUI mode is off (the §4.2.1 gate).
fn emit_gate_inactive(
    term_state_offset: usize,
    target: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.push(abi::load_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    instructions.push(abi::compare_immediate("%v9", "0"));
    instructions.push(abi::branch_eq(target));
}

/// Emit a `Result.Err(ERR_UNSUPPORTED_OPERATION)` into the result registers.
fn emit_unsupported(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::move_immediate(
        RESULT_VALUE_REGISTER,
        "Integer",
        ERR_UNSUPPORTED_CODE,
    ));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_ERR_TAG,
    ));
    push_error_message_address(symbol, ERR_UNSUPPORTED_SYMBOL, instructions, relocations);
}

pub(super) fn lower_term_helper(
    call: &str,
    symbol: &str,
    term_state_offset: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2): the decimal/record-build scratch buffers
    // are an explicit sp-relative local region; x9-x15 scratch becomes vregs.
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    match call {
        "term.on" => emit_on(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            &done,
        )?,
        "term.off" => emit_off(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            &done,
        )?,
        "term.isOn" => emit_is_on(term_state_offset, &mut instructions),
        "term.setForeground" => emit_set_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_FG_OFFSET,
            &mut instructions,
        ),
        "term.setBackground" => emit_set_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BG_OFFSET,
            &mut instructions,
        ),
        "term.setBold" => emit_set_attr(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BOLD_OFFSET,
            &mut instructions,
        ),
        "term.setUnderline" => emit_set_attr(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_UNDERLINE_OFFSET,
            &mut instructions,
        ),
        "term.showCursor" => {
            emit_set_cursor_visible(symbol, term_state_offset, "1", &mut instructions)
        }
        "term.hideCursor" => {
            emit_set_cursor_visible(symbol, term_state_offset, "0", &mut instructions)
        }
        "term.clear" => emit_clear_grid(symbol, term_state_offset, &mut instructions),
        "term.sync" => {
            // plan-35-C: present the frame — diff the back buffer against the
            // last-presented front buffer and emit only the changed cells as one
            // batched write. A no-op while TUI mode is off (grid pointer null).
            let request = match platform.family() {
                PlatformFamily::MacOS => DARWIN_TIOCGWINSZ,
                PlatformFamily::Linux => LINUX_TIOCGWINSZ,
                // Windows ignores the ioctl request value; emit_terminal_size
                // uses GetConsoleScreenBufferInfo. A placeholder keeps the match total.
                PlatformFamily::Windows => "0",
            };
            term_grid::emit_grid_present(
                symbol,
                term_state_offset,
                SCRATCH_END,
                request,
                platform,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.push(abi::move_immediate(
                RESULT_TAG_REGISTER,
                "Integer",
                RESULT_OK_TAG,
            ));
        }
        "term.moveTo" => emit_move_to(symbol, term_state_offset, &mut instructions),
        "term.drawHLine" => emit_draw_line(symbol, term_state_offset, true, &mut instructions),
        "term.drawVLine" => emit_draw_line(symbol, term_state_offset, false, &mut instructions),
        "term.getForeground" => emit_get_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_FG_OFFSET,
            DEFAULT_FOREGROUND_PACKED,
            &done,
            &mut instructions,
            &mut relocations,
        ),
        "term.getBackground" => emit_get_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BG_OFFSET,
            "0",
            &done,
            &mut instructions,
            &mut relocations,
        ),
        "term.getBold" => emit_get_attr(
            term_state_offset,
            term_state_offset + TERM_STATE_BOLD_OFFSET,
            &done,
            &mut instructions,
        ),
        "term.getUnderline" => emit_get_attr(
            term_state_offset,
            term_state_offset + TERM_STATE_UNDERLINE_OFFSET,
            &done,
            &mut instructions,
        ),
        "term.terminalSize" => emit_terminal_size(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            &done,
        )?,
        other => return Err(format!("unknown term runtime helper '{other}'")),
    }

    instructions.push(abi::label(&done));
    instructions.push(abi::return_());

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCALS_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

fn emit_on(ctx: &mut EmitCtx, term_state_offset: usize, done: &str) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // plan-35-B: allocate the console shadow-grid header block sized to the
    // terminal *before* marking TUI mode active, so a program never sees
    // `active == 1` with a null grid. On allocation failure surface
    // `ERR_OUT_OF_MEMORY` and leave the terminal untouched.
    let alloc_fail = format!("{symbol}_grid_alloc_fail");
    let request = match platform.family() {
        PlatformFamily::MacOS => DARWIN_TIOCGWINSZ,
        PlatformFamily::Linux => LINUX_TIOCGWINSZ,
        // Windows ignores the ioctl request value (emit_terminal_size uses
        // GetConsoleScreenBufferInfo); a placeholder keeps the match total.
        PlatformFamily::Windows => "0",
    };
    term_grid::emit_grid_alloc(
        symbol,
        term_state_offset,
        request,
        SCRATCH_OFFSET,
        ARG0_OFFSET,
        ARG1_OFFSET,
        &alloc_fail,
        platform,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    // Reset all state to defaults (plan §4.2). Foreground white, background
    // black, bold/underline off, cursor visible, active on.
    let writes: &[(usize, &str)] = &[
        (TERM_STATE_ACTIVE_OFFSET, "1"),
        (TERM_STATE_FG_OFFSET, DEFAULT_FOREGROUND_PACKED),
        (TERM_STATE_BG_OFFSET, "0"),
        (TERM_STATE_BOLD_OFFSET, "0"),
        (TERM_STATE_UNDERLINE_OFFSET, "0"),
        (TERM_STATE_CURSOR_VISIBLE_OFFSET, "1"),
    ];
    for (offset, value) in writes {
        ctx.instructions
            .push(abi::move_immediate("%v9", "Integer", value));
        ctx.instructions.push(abi::store_u64(
            "%v9",
            ARENA_STATE_REGISTER,
            term_state_offset + offset,
        ));
    }
    // Enable ANSI/VT output interpretation before the first escape write. No-op on
    // POSIX terminals; on Windows this sets ENABLE_VIRTUAL_TERMINAL_PROCESSING on
    // the stdout console handle so the ESC_ON sequence (and every later styling
    // write) renders instead of printing raw (plan-66-D).
    platform.emit_enable_vt_output(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    emit_write_const(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ESC_ON_SYMBOL,
        ESC_ON.len(),
    )?;
    // bug-149: entering interactive TUI mode also puts the console tty into
    // single-key (cbreak) mode once — `~ICANON`/`~ECHO`/`VMIN=1`/`VTIME=0` — so a
    // `pollInput` + `readChar` loop registers bare keypresses without waiting for
    // Return. The saved cooked discipline is parked in the term-state region (off
    // `ARENA_STATE_REGISTER`), from which `term::off` and `io::input`/
    // `io::readLine` restore it. When stdin is not a tty (piped input,
    // acceptance harness) `emit_configure_stdin_terminal` leaves the raw-active
    // flag at 0 and this is inert. A `tcgetattr`/`tcsetattr` failure branches to
    // `raw_failed`, which clears the flag — a terminal-setup failure must not make
    // the reads think raw mode is live (term setters are best-effort, §4.2.1).
    let raw_failed = format!("{symbol}_raw_failed");
    let raw_done = format!("{symbol}_raw_done");
    let raw_slots = TerminalModeSlots {
        active: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        saved_tag: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        saved_value: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        saved_message: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        original: term_state_offset + TERM_STATE_COOKED_TERMIOS_OFFSET,
        modified: term_state_offset + TERM_STATE_RAW_TERMIOS_OFFSET,
    };
    emit_configure_stdin_terminal(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        &raw_slots,
        ARENA_STATE_REGISTER,
        true,
        true,
        &raw_failed,
    )?;
    ctx.instructions.push(abi::branch(&raw_done));
    ctx.instructions.push(abi::label(&raw_failed));
    ctx.instructions
        .push(abi::move_immediate("%v9", "Integer", "0"));
    ctx.instructions.push(abi::store_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
    ));
    ctx.instructions.push(abi::label(&raw_done));
    ctx.instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ctx.instructions.push(abi::branch(done));
    // Grid allocation failed: active was never set, so the terminal is untouched.
    ctx.instructions.push(abi::label(&alloc_fail));
    ctx.instructions.push(abi::move_immediate(
        RESULT_VALUE_REGISTER,
        "Integer",
        ERR_OUT_OF_MEMORY_CODE,
    ));
    ctx.instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_ERR_TAG,
    ));
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        ctx.instructions,
        ctx.relocations,
    );
    Ok(())
}

fn emit_off(ctx: &mut EmitCtx, term_state_offset: usize, done: &str) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, ctx.instructions);
    // plan-35-C: present the final frame before restoring the user's screen, so
    // the last frame the program drew is shown. Reuse the `term::sync` helper as
    // the present routine (force-emitted whenever `term::` is used).
    ctx.instructions
        .push(abi::branch_link("_mfb_rt_term_term_sync"));
    ctx.relocations
        .push(internal_branch(symbol, "_mfb_rt_term_term_sync"));
    // bug-149: leaving TUI mode restores the saved cooked line discipline that
    // `term::on` captured, so the terminal returns to canonical/echoing input.
    // A no-op when the raw-active flag is 0 (stdin was never put into raw mode).
    let raw_restore_skip = format!("{symbol}_raw_restore_skip");
    ctx.instructions.push(abi::load_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
    ));
    ctx.instructions.push(abi::compare_immediate("%v9", "0"));
    ctx.instructions.push(abi::branch_eq(&raw_restore_skip));
    ctx.instructions
        .push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    ctx.instructions
        .push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ctx.instructions.push(abi::add_immediate(
        abi::ARG[2],
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_COOKED_TERMIOS_OFFSET,
    ));
    platform.emit_terminal_control_call(
        TerminalControlCall::SetAttrs,
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions
        .push(abi::move_immediate("%v9", "Integer", "0"));
    ctx.instructions.push(abi::store_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
    ));
    ctx.instructions.push(abi::label(&raw_restore_skip));
    emit_write_const(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ESC_OFF_SYMBOL,
        ESC_OFF.len(),
    )?;
    ctx.instructions
        .push(abi::move_immediate("%v9", "Integer", "0"));
    ctx.instructions.push(abi::store_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    // plan-35-B: free the shadow-grid block and zero its slot (no-op if null).
    term_grid::emit_grid_free(symbol, term_state_offset, ctx.instructions, ctx.relocations);
    ctx.instructions.push(abi::label(&inactive));
    ctx.instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    let _ = done;
    Ok(())
}

fn emit_is_on(term_state_offset: usize, instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::setForeground`/`setBackground` (plan-35-B): pack `r|g<<8|b<<16` into the
/// term-state colour slot — the "current attribute" the grid writer stamps into
/// cells. Emits no ANSI; the colour is applied when `term::sync` presents.
fn emit_set_color(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
) {
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    instructions.extend([
        abi::move_register("%v9", abi::ARG[0]),
        abi::move_register("%v10", abi::ARG[1]),
        abi::move_register("%v11", abi::ARG[2]),
        abi::shift_left_immediate("%v10", "%v10", 8),
        abi::shift_left_immediate("%v11", "%v11", 16),
        abi::or_registers("%v9", "%v9", "%v10"),
        abi::or_registers("%v9", "%v9", "%v11"),
        abi::store_u64("%v9", ARENA_STATE_REGISTER, state_offset),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::setBold`/`setUnderline` (plan-35-B): store the flag into its term-state
/// slot (the current attribute). Emits no ANSI.
fn emit_set_attr(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
) {
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    instructions.push(abi::move_register("%v9", abi::ARG[0]));
    instructions.push(abi::store_u64("%v9", ARENA_STATE_REGISTER, state_offset));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::showCursor`/`hideCursor` (plan-35-B): store the cursor-visible flag; the
/// present applies it. Emits no ANSI.
fn emit_set_cursor_visible(
    symbol: &str,
    term_state_offset: usize,
    value: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    instructions.push(abi::move_immediate("%v9", "Integer", value));
    instructions.push(abi::store_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::clear` (plan-35-B): blank the back buffer (every cell zero-filled — a
/// blank glyph on the default/black background, not the caller's current
/// background) and home the shadow cursor. Emits no ANSI; the cleared state is
/// shown when `term::sync` presents. (bug-175 H: doc corrected to match the
/// zero-fill.)
fn emit_clear_grid(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
) {
    let inactive = format!("{symbol}_inactive");
    let clr = format!("{symbol}_clr_loop");
    let clr_done = format!("{symbol}_clr_done");
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    instructions.extend([
        abi::load_u64(
            "%v9",
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&inactive),
        // words = rows*cols*CELL_SIZE/8 = rows*cols*2 ; back = gp + HDR_SIZE
        abi::load_u64("%v10", "%v9", 0),
        abi::load_u64("%v11", "%v9", 8),
        abi::multiply_registers("%v10", "%v10", "%v11"),
        abi::shift_left_immediate("%v10", "%v10", 1),
        abi::add_immediate("%v12", "%v9", 40),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&clr),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&clr_done),
        abi::store_u64("%v13", "%v12", 0),
        abi::add_immediate("%v12", "%v12", 8),
        abi::subtract_immediate("%v10", "%v10", 1),
        abi::branch(&clr),
        abi::label(&clr_done),
        // Home the shadow cursor (cursorRow @ 16, cursorCol @ 24).
        abi::store_u64("%v13", "%v9", 16),
        abi::store_u64("%v13", "%v9", 24),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::moveTo(row, column)` (plan-35-B): set the shadow cursor in the grid
/// header, clamping negatives to 0 and high values to the last valid cell. Emits
/// no ANSI; the cursor is honoured by the next glyph write and by the present.
fn emit_move_to(symbol: &str, term_state_offset: usize, instructions: &mut Vec<CodeInstruction>) {
    let inactive = format!("{symbol}_inactive");
    let row_lo = format!("{symbol}_row_lo");
    let col_lo = format!("{symbol}_col_lo");
    let row_hi = format!("{symbol}_row_hi");
    let col_hi = format!("{symbol}_col_hi");
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    instructions.extend([
        abi::load_u64(
            "%v9",
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&inactive),
        abi::load_u64("%v10", "%v9", 0), // rows
        abi::load_u64("%v11", "%v9", 8), // cols
        // row = clamp(ARG[0], 0, rows-1)
        abi::move_register("%v12", abi::ARG[0]),
        abi::compare_immediate("%v12", "0"),
        abi::branch_ge(&row_lo),
        abi::move_immediate("%v12", "Integer", "0"),
        abi::label(&row_lo),
        abi::compare_registers("%v12", "%v10"),
        abi::branch_lt(&row_hi),
        abi::subtract_immediate("%v12", "%v10", 1),
        abi::label(&row_hi),
        // col = clamp(ARG[1], 0, cols-1)
        abi::move_register("%v13", abi::ARG[1]),
        abi::compare_immediate("%v13", "0"),
        abi::branch_ge(&col_lo),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&col_lo),
        abi::compare_registers("%v13", "%v11"),
        abi::branch_lt(&col_hi),
        abi::subtract_immediate("%v13", "%v11", 1),
        abi::label(&col_hi),
        abi::store_u64("%v12", "%v9", 16), // cursorRow
        abi::store_u64("%v13", "%v9", 24), // cursorCol
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// Pack a code point's UTF-8 bytes little-endian into the u32 grid `glyph`
/// encoding (`byte0 | byte1<<8 | byte2<<16`, the same layout `term_grid` writes
/// and `term::sync` reads back). Every box-drawing glyph here is a 3-byte run.
fn packed_glyph(codepoint: u32) -> u32 {
    let ch = char::from_u32(codepoint).expect("box-drawing code point is valid");
    let mut buf = [0u8; 4];
    let bytes = ch.encode_utf8(&mut buf).as_bytes();
    let mut packed = 0u32;
    for (index, byte) in bytes.iter().enumerate() {
        packed |= (*byte as u32) << (8 * index as u32);
    }
    packed
}

/// `term::drawHLine`/`drawVLine` (console): stamp a fixed box-drawing glyph across
/// a run of back-buffer cells with the current colours/attributes; the run is
/// shown on the next `term::sync`. A no-op while TUI mode is off or the grid is
/// unallocated — the same gate every writer honours (§4.2.1).
///
/// Arguments arrive in registers as `ARG[0]` = the `LineStyle` ordinal, then the
/// fixed line and the two span endpoints:
///   drawHLine(line, row, colA, colB) — fixed `row`, span over columns.
///   drawVLine(line, col, rowA, rowB) — fixed `col`, span over rows.
/// `is_horizontal` selects, at emit time, which grid dimension bounds the fixed
/// coordinate, which bounds the span, how a cell index is formed, and which glyph
/// table is used. Endpoints may be given in either order (normalised to lo<=hi)
/// and are clamped to the grid; a fixed coordinate off the grid, or a span with no
/// on-grid cell, draws nothing (rather than clamping onto an edge line).
fn emit_draw_line(
    symbol: &str,
    term_state_offset: usize,
    is_horizontal: bool,
    instructions: &mut Vec<CodeInstruction>,
) {
    let inactive = format!("{symbol}_inactive");
    let ord = "%v9";
    let fixed = "%v10";
    let lo = "%v11";
    let hi = "%v12";
    let gp = "%v13";
    let rows = "%v14";
    let cols = "%v15";
    let back = "%v16";
    let glyph = "%v17";
    let fg = "%v18";
    let bg = "%v19";
    let bold = "%v20";
    let un = "%v21";
    let idx = "%v22";
    let cell = "%v23";
    let pos = "%v24";
    let tmp = "%v25";

    emit_gate_inactive(term_state_offset, &inactive, instructions);
    // Snapshot the arguments; `lo`/`hi` provisionally hold (endpointA, endpointB)
    // and are swapped below when given high-to-low.
    instructions.extend([
        abi::move_register(ord, abi::ARG[0]),
        abi::move_register(fixed, abi::ARG[1]),
        abi::move_register(lo, abi::ARG[2]),
        abi::move_register(hi, abi::ARG[3]),
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&inactive),
        abi::load_u64(rows, gp, 0),
        abi::load_u64(cols, gp, 8),
        abi::add_immediate(back, gp, term_grid::HDR_SIZE),
    ]);
    // The grid dimension bounding the fixed coordinate vs. the span endpoints.
    let fixed_limit = if is_horizontal { rows } else { cols };
    let span_limit = if is_horizontal { cols } else { rows };
    // The fixed coordinate must be on the grid: [0, fixed_limit-1], else nothing.
    instructions.extend([
        abi::compare_immediate(fixed, "0"),
        abi::branch_lt(&inactive),
        abi::compare_registers(fixed, fixed_limit),
        abi::branch_ge(&inactive),
    ]);
    // Normalise the span so lo <= hi.
    let span_ok = format!("{symbol}_dl_span_ok");
    instructions.extend([
        abi::compare_registers(lo, hi),
        abi::branch_le(&span_ok),
        abi::move_register(tmp, lo),
        abi::move_register(lo, hi),
        abi::move_register(hi, tmp),
        abi::label(&span_ok),
    ]);
    // Clamp lo up to 0 and hi down to span_limit-1.
    let lo_ok = format!("{symbol}_dl_lo_ok");
    let hi_ok = format!("{symbol}_dl_hi_ok");
    instructions.extend([
        abi::compare_immediate(lo, "0"),
        abi::branch_ge(&lo_ok),
        abi::move_immediate(lo, "Integer", "0"),
        abi::label(&lo_ok),
        abi::subtract_immediate(tmp, span_limit, 1),
        abi::compare_registers(hi, tmp),
        abi::branch_le(&hi_ok),
        abi::move_register(hi, tmp),
        abi::label(&hi_ok),
        // Whole line clamped off the grid (lo > hi) → nothing.
        abi::compare_registers(lo, hi),
        abi::branch_gt(&inactive),
        // Current attributes stamped into each cell (mirrors `emit_grid_write`).
        abi::load_u64(
            fg,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_FG_OFFSET,
        ),
        abi::load_u64(
            bg,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_BG_OFFSET,
        ),
        abi::load_u64(
            bold,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_BOLD_OFFSET,
        ),
        abi::load_u64(
            un,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_UNDERLINE_OFFSET,
        ),
    ]);
    // Select the glyph from the ordinal (0..=6); ordinal 0 (`Light`) is the
    // fall-through default so an out-of-range value can never strand `glyph`.
    let table = if is_horizontal {
        &TERM_HLINE_CODEPOINTS
    } else {
        &TERM_VLINE_CODEPOINTS
    };
    let glyph_done = format!("{symbol}_dl_glyph_done");
    instructions.push(abi::move_immediate(
        glyph,
        "Integer",
        &packed_glyph(table[0]).to_string(),
    ));
    for (ordinal, codepoint) in table.iter().enumerate().skip(1) {
        let next = format!("{symbol}_dl_g{ordinal}");
        instructions.extend([
            abi::compare_immediate(ord, &ordinal.to_string()),
            abi::branch_ne(&next),
            abi::move_immediate(glyph, "Integer", &packed_glyph(*codepoint).to_string()),
            abi::branch(&glyph_done),
            abi::label(&next),
        ]);
    }
    instructions.push(abi::label(&glyph_done));
    // pos = lo..=hi, stamping the glyph into each cell. Cell index:
    //   H → fixed*cols + pos ; V → pos*cols + fixed  (`cols` is the row stride).
    let loop_top = format!("{symbol}_dl_loop");
    let loop_done = format!("{symbol}_dl_done");
    instructions.extend([
        abi::move_register(pos, lo),
        abi::label(&loop_top),
        abi::compare_registers(pos, hi),
        abi::branch_gt(&loop_done),
    ]);
    if is_horizontal {
        instructions.extend([
            abi::multiply_registers(idx, fixed, cols),
            abi::add_registers(idx, idx, pos),
        ]);
    } else {
        instructions.extend([
            abi::multiply_registers(idx, pos, cols),
            abi::add_registers(idx, idx, fixed),
        ]);
    }
    instructions.extend([
        abi::shift_left_immediate(idx, idx, 4), // * CELL_SIZE (16)
        abi::add_registers(cell, back, idx),
        abi::store_u32(glyph, cell, term_grid::C_GLYPH),
        abi::store_u32(fg, cell, term_grid::C_FG),
        abi::store_u32(bg, cell, term_grid::C_BG),
        abi::store_u8(bold, cell, term_grid::C_BOLD),
        abi::store_u8(un, cell, term_grid::C_UN),
        abi::add_immediate(pos, pos, 1),
        abi::branch(&loop_top),
        abi::label(&loop_done),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

fn emit_get_color(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    inert_packed: &str,
    done: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let inert = format!("{symbol}_inert");
    let have_src = format!("{symbol}_have_src");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    // Determine the source packed value: state when active, the inert default
    // otherwise (§4.2.1). Park it in the scratch arg slot.
    emit_gate_inactive(term_state_offset, &inert, instructions);
    instructions.push(abi::load_u64("%v10", ARENA_STATE_REGISTER, state_offset));
    instructions.push(abi::branch(&have_src));
    instructions.push(abi::label(&inert));
    instructions.push(abi::move_immediate("%v10", "Integer", inert_packed));
    instructions.push(abi::label(&have_src));
    instructions.push(abi::store_u64("%v10", abi::stack_pointer(), ARG0_OFFSET));
    // Allocate the 3-field TermColor record.
    instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &TERM_COLOR_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register("%v9", RESULT_VALUE_REGISTER),
        abi::load_u64("%v10", abi::stack_pointer(), ARG0_OFFSET),
        abi::move_immediate("%v12", "Integer", "255"),
        abi::and_registers("%v13", "%v10", "%v12"),
        abi::store_u64("%v13", "%v9", 0),
        abi::shift_right_immediate("%v14", "%v10", 8),
        abi::and_registers("%v13", "%v14", "%v12"),
        abi::store_u64("%v13", "%v9", 8),
        abi::shift_right_immediate("%v14", "%v10", 16),
        abi::and_registers("%v13", "%v14", "%v12"),
        abi::store_u64("%v13", "%v9", 16),
        abi::move_register(RESULT_VALUE_REGISTER, "%v9"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, instructions, relocations);
}

fn emit_get_attr(
    term_state_offset: usize,
    state_offset: usize,
    done: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let inert = format!("term_get_attr_inert_{state_offset}");
    emit_gate_inactive(term_state_offset, &inert, instructions);
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        state_offset,
    ));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::branch(done));
    instructions.push(abi::label(&inert));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

fn emit_terminal_size(
    ctx: &mut EmitCtx,
    term_state_offset: usize,
    done: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let unsupported = format!("{symbol}_unsupported");
    let active = format!("{symbol}_active");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let request = match platform.family() {
        PlatformFamily::MacOS => DARWIN_TIOCGWINSZ,
        PlatformFamily::Linux => LINUX_TIOCGWINSZ,
        // Windows ignores the ioctl request value (emit_terminal_size uses
        // GetConsoleScreenBufferInfo); a placeholder keeps the match total.
        PlatformFamily::Windows => "0",
    };
    // Gate: terminalSize is the one read with no inert value; while inactive it
    // returns ERR_UNSUPPORTED_OPERATION (§4.7).
    ctx.instructions.push(abi::load_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    ctx.instructions.push(abi::compare_immediate("%v9", "0"));
    ctx.instructions.push(abi::branch_ne(&active));
    emit_unsupported(symbol, ctx.instructions, ctx.relocations);
    ctx.instructions.push(abi::branch(done));
    ctx.instructions.push(abi::label(&active));
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_immediate(abi::ARG[1], "Integer", request),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), SCRATCH_OFFSET),
    ]);
    platform.emit_terminal_size(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&unsupported),
        abi::load_u16("%v10", abi::stack_pointer(), SCRATCH_OFFSET),
        abi::load_u16("%v11", abi::stack_pointer(), SCRATCH_OFFSET + 2),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&unsupported),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&unsupported),
        abi::store_u64("%v10", abi::stack_pointer(), ARG0_OFFSET),
        abi::store_u64("%v11", abi::stack_pointer(), ARG1_OFFSET),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &TERM_SIZE_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::load_u64("%v10", abi::stack_pointer(), ARG0_OFFSET),
        abi::load_u64("%v11", abi::stack_pointer(), ARG1_OFFSET),
        abi::store_u64("%v11", abi::RET[1], 0),
        abi::store_u64("%v10", abi::RET[1], 8),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&unsupported),
    ]);
    emit_unsupported(symbol, ctx.instructions, ctx.relocations);
    ctx.instructions.extend([
        abi::branch(done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        ctx.instructions,
        ctx.relocations,
    );
    Ok(())
}
