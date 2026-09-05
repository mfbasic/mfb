//! Native code generation for the built-in `term::` console backend
//! (plan-01-term.md §6.1, Phase 2). Each helper emits a self-contained AArch64
//! runtime function that updates the term-state global (writable slots in the
//! program-entry frame, reached off the pinned arena-state register `x19` at
//! `term_state_offset`) and writes ANSI escape sequences to stdout.
//!
//! The §4.2.1 gate lives here: every helper except `term::on`/`term::isOn`
//! begins by loading the `active` flag and short-circuiting to a no-op (or, for
//! getters, the inert default) while TUI mode is off.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::term::grid as term_grid;
use crate::target::shared::abi;
use std::collections::HashMap;

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

/// Bytes allocated for the `color::Color` record `term::getForeground`/`getBackground`
/// return: four `Byte` fields, one 8-byte slot each (plan-122-F widened it from the
/// retired 3-field `TermColor`).
const COLOR_RECORD_SIZE: usize = 32;
const TERM_SIZE_RECORD_SIZE: usize = 16;
/// Default foreground while inactive (white, packed `r | g<<8 | b<<16`).
const DEFAULT_FOREGROUND_PACKED: &str = "16777215";

fn esc_entries() -> &'static [(&'static str, &'static [u8])] {
    &[(ESC_ON_SYMBOL, ESC_ON), (ESC_OFF_SYMBOL, ESC_OFF)]
}

/// Read-only data objects for the fixed escape-sequence byte strings.
pub(crate) fn console_data_objects() -> Vec<CodeDataObject> {
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
    dst: impl Into<Operand>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let dst = dst.into();
    instructions.push(abi::load_page_address(&dst, symbol));
    relocations.push(data_reloc(from, symbol, RelocIntent::DataAddrHi));
    instructions.push(abi::add_page_offset(&dst, &dst, symbol));
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
    raise_error_into(symbol, "ErrUnsupported", instructions, relocations);
}

pub(crate) fn lower_term_helper(
    call: &str,
    symbol: &str,
    term_state_offset: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>, usize), String> {
    // Vreg-allocated (plan-00-G Phase 2): the decimal/record-build scratch buffers
    // are an explicit sp-relative local region; x9-x15 scratch becomes vregs. The
    // `abi_function` wrapper (`term::gen_shared per-member body`) seeds the
    // entry label and finalizes; this body returns the pre-finalize
    // `(instructions, relocations, stack_size)` it consumes.
    let done = format!("{symbol}_done");
    let mut instructions: Vec<CodeInstruction> = Vec::new();
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
        "term.drawBox" => emit_draw_box(symbol, term_state_offset, &mut instructions),
        "term.fillRect" => emit_fill_rect(symbol, term_state_offset, &mut instructions),
        "term.drawText" => emit_draw_text(
            symbol,
            term_state_offset,
            &mut instructions,
            &mut relocations,
        ),
        "term.drawGlyph" => emit_draw_glyph(
            symbol,
            term_state_offset,
            &mut instructions,
            &mut relocations,
        ),
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
        "term.didResize" => emit_did_resize(term_state_offset, &mut instructions),
        other => return Err(format!("unknown term runtime helper '{other}'")),
    }

    instructions.push(abi::label(&done));
    instructions.push(abi::return_());

    Ok((instructions, relocations, LOCALS_SIZE))
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
        (TERM_STATE_DID_RESIZE_OFFSET, "0"),
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
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
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
        .push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    ctx.instructions.push(abi::add_immediate(
        abi::c_arg(2),
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

/// `term::didResize()` (planning/term.md #11): read the cached resize flag and
/// clear it in the same helper, so the flag latches `true` from the moment a
/// terminal/window size change is detected until the program observes it, then
/// resets. Like `term::isOn` this getter is not gated on `active` — an inactive
/// terminal never sets the flag, so it simply reads 0/false. The flag is set by
/// the shared CLI reflow (`term_grid::emit_grid_resize`) and, in `--app` mode, by
/// each app backend's resize hook mirroring into this same term-state slot, so the
/// shared getter is correct in both modes.
fn emit_did_resize(term_state_offset: usize, instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_DID_RESIZE_OFFSET,
    ));
    instructions.push(abi::move_immediate("%v9", "Integer", "0"));
    instructions.push(abi::store_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_DID_RESIZE_OFFSET,
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
    // plan-122-F: the single MFB argument is a `color::Color` RECORD POINTER, in
    // `c_arg(0)` (established by reading this file's own pre-change unpacking, which
    // took its three `Byte` channels from `c_arg(0..2)` — NOT `return_register()`,
    // which `emit_get_color` twenty lines below uses for the arena allocator's first
    // argument).
    //
    // The pointer is moved into a callee-usable temporary FIRST and every field is
    // then loaded off that temporary. Loading straight out of `c_arg(0)` while also
    // writing `%v9`/`%v10` would be safe here today, but staging is the rule for
    // this file: an emitter that writes an argument slot before every incoming
    // argument is read destroys one, and the symptom is a SIGSEGV at a tiny address
    // rather than a wrong colour.
    //
    // Field offsets are the record's declaration order — red 0, green 8, blue 16,
    // alpha 24 — matching what `emit_get_color` stores. ALPHA IS DELIBERATELY NOT
    // READ: a terminal cell has no alpha channel, and the state slot it packs into
    // is only 0xBBGGRR.
    instructions.extend([
        abi::move_register("%v12", abi::c_arg(0)),
        abi::load_u64("%v9", "%v12", 0),
        abi::load_u64("%v10", "%v12", 8),
        abi::load_u64("%v11", "%v12", 16),
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
    instructions.push(abi::move_register("%v9", abi::c_arg(0)));
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
        abi::move_register("%v12", abi::c_arg(0)),
        abi::compare_immediate("%v12", "0"),
        abi::branch_ge(&row_lo),
        abi::move_immediate("%v12", "Integer", "0"),
        abi::label(&row_lo),
        abi::compare_registers("%v12", "%v10"),
        abi::branch_lt(&row_hi),
        abi::subtract_immediate("%v12", "%v10", 1),
        abi::label(&row_hi),
        // col = clamp(ARG[1], 0, cols-1)
        abi::move_register("%v13", abi::c_arg(1)),
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

/// Emit `dst = packed_glyph(table[ord])`, a select-by-ordinal chain over a 7-entry
/// `LineStyle` code-point table. Ordinal 0 is the fall-through default, so an
/// out-of-range ordinal can never strand `dst`. `tag` uniquifies the labels.
fn emit_select_glyph(
    ord: &str,
    dst: &str,
    table: &[u32],
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let done = format!("{tag}_done");
    instructions.push(abi::move_immediate(
        dst,
        "Integer",
        &packed_glyph(table[0]).to_string(),
    ));
    for (ordinal, codepoint) in table.iter().enumerate().skip(1) {
        let next = format!("{tag}_{ordinal}");
        instructions.extend([
            abi::compare_immediate(ord, &ordinal.to_string()),
            abi::branch_ne(&next),
            abi::move_immediate(dst, "Integer", &packed_glyph(*codepoint).to_string()),
            abi::branch(&done),
            abi::label(&next),
        ]);
    }
    instructions.push(abi::label(&done));
}

/// Grid context + throwaway scratch registers shared by the run/cell stampers.
/// `rows`/`cols` are the grid dims, `back` the back-cell base, and the four
/// attribute registers the current fg/bg/bold/underline; the remaining fields are
/// scratch the stampers clobber, which the caller must keep disjoint from any
/// value that must stay live across a stamp (e.g. the normalised box extents).
struct StampCtx<'a> {
    rows: &'a str,
    cols: &'a str,
    back: &'a str,
    fg: &'a str,
    bg: &'a str,
    bold: &'a str,
    un: &'a str,
    lo: &'a str,
    hi: &'a str,
    idx: &'a str,
    cell: &'a str,
    pos: &'a str,
    tmp: &'a str,
}

/// Stamp `glyph` (with the ctx attributes) across a clamped run. `fixed` is the
/// line coordinate and `ea`/`eb` the two span endpoints (either order). When
/// `is_horizontal`, `fixed` is the row and the run spans columns; otherwise
/// `fixed` is the column and the run spans rows. The span is normalised (lo<=hi)
/// and clamped to the grid; a `fixed` off the grid, or a span with no on-grid
/// cell, branches to `skip` (which the caller places after the call). Does not
/// clobber `fixed`/`ea`/`eb` or the ctx attributes — only ctx scratch — so a
/// caller can reuse the extents for further runs and corners.
#[allow(clippy::too_many_arguments)]
fn emit_stamp_run(
    ctx: &StampCtx,
    is_horizontal: bool,
    fixed: &str,
    ea: &str,
    eb: &str,
    glyph: &str,
    tag: &str,
    skip: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let fixed_limit = if is_horizontal { ctx.rows } else { ctx.cols };
    let span_limit = if is_horizontal { ctx.cols } else { ctx.rows };
    let span_ok = format!("{tag}_span_ok");
    let lo_ok = format!("{tag}_lo_ok");
    let hi_ok = format!("{tag}_hi_ok");
    let loop_top = format!("{tag}_loop");
    let loop_done = format!("{tag}_loop_done");
    instructions.extend([
        // The fixed coordinate must be on the grid: [0, fixed_limit-1].
        abi::compare_immediate(fixed, "0"),
        abi::branch_lt(skip),
        abi::compare_registers(fixed, fixed_limit),
        abi::branch_ge(skip),
        // lo = min(ea, eb), hi = max(ea, eb).
        abi::move_register(ctx.lo, ea),
        abi::move_register(ctx.hi, eb),
        abi::compare_registers(ctx.lo, ctx.hi),
        abi::branch_le(&span_ok),
        abi::move_register(ctx.tmp, ctx.lo),
        abi::move_register(ctx.lo, ctx.hi),
        abi::move_register(ctx.hi, ctx.tmp),
        abi::label(&span_ok),
        // Clamp lo up to 0 and hi down to span_limit-1; empty span → skip.
        abi::compare_immediate(ctx.lo, "0"),
        abi::branch_ge(&lo_ok),
        abi::move_immediate(ctx.lo, "Integer", "0"),
        abi::label(&lo_ok),
        abi::subtract_immediate(ctx.tmp, span_limit, 1),
        abi::compare_registers(ctx.hi, ctx.tmp),
        abi::branch_le(&hi_ok),
        abi::move_register(ctx.hi, ctx.tmp),
        abi::label(&hi_ok),
        abi::compare_registers(ctx.lo, ctx.hi),
        abi::branch_gt(skip),
        abi::move_register(ctx.pos, ctx.lo),
        abi::label(&loop_top),
        abi::compare_registers(ctx.pos, ctx.hi),
        abi::branch_gt(&loop_done),
    ]);
    // plan-70-C: line/box/fill glyphs are all single-width; stamp width 1 into the
    // cell so the presenter advances one column even over a stale wide-cell width
    // byte. `tmp` is reused as the constant-1 source across the run.
    instructions.push(abi::move_immediate(ctx.tmp, "Integer", "1"));
    // Cell index: H → fixed*cols + pos ; V → pos*cols + fixed (`cols` is stride).
    if is_horizontal {
        instructions.extend([
            abi::multiply_registers(ctx.idx, fixed, ctx.cols),
            abi::add_registers(ctx.idx, ctx.idx, ctx.pos),
        ]);
    } else {
        instructions.extend([
            abi::multiply_registers(ctx.idx, ctx.pos, ctx.cols),
            abi::add_registers(ctx.idx, ctx.idx, fixed),
        ]);
    }
    instructions.extend([
        abi::shift_left_immediate(ctx.idx, ctx.idx, 4), // * CELL_SIZE (16)
        abi::add_registers(ctx.cell, ctx.back, ctx.idx),
    ]);
    // plan-70-C Phase 2: clear the paired half of any wide glyph this run cell
    // overwrites. The cell's column is `pos` for a horizontal run, `fixed` for a
    // vertical one.
    let run_col = if is_horizontal { ctx.pos } else { fixed };
    emit_clear_wide_pair(ctx, ctx.cell, run_col, &format!("{tag}_run"), instructions);
    instructions.extend([
        abi::store_u32(glyph, ctx.cell, term_grid::C_GLYPH),
        abi::store_u32(ctx.fg, ctx.cell, term_grid::C_FG),
        abi::store_u32(ctx.bg, ctx.cell, term_grid::C_BG),
        abi::store_u8(ctx.bold, ctx.cell, term_grid::C_BOLD),
        abi::store_u8(ctx.un, ctx.cell, term_grid::C_UN),
        abi::store_u8(ctx.tmp, ctx.cell, term_grid::C_WIDTH),
        abi::add_immediate(ctx.pos, ctx.pos, 1),
        abi::branch(&loop_top),
        abi::label(&loop_done),
    ]);
}

/// plan-70-C: before overwriting the on-grid cell at address `cell` / column
/// `col`, clear the OTHER half of any wide glyph this cell is part of, so no
/// orphaned half-glyph remains. If the cell is a `WIDE_TRAIL`, blank the primary
/// to its left (col-1); if it is a wide primary (stored width 2), blank the trail
/// to its right (col+1). Blanking = glyph 0 (a space) + width 1, attributes kept.
/// Uses `ctx.idx` and `ctx.lo` as scratch — both dead at every call site (the
/// cell address is already in `ctx.cell`; `lo` is a spent span bound).
fn emit_clear_wide_pair(
    ctx: &StampCtx,
    cell: &str,
    col: &str,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let after = format!("{tag}_cwp_after");
    let not_trail = format!("{tag}_cwp_nt");
    instructions.extend([
        abi::load_u32(ctx.lo, cell, term_grid::C_GLYPH),
        abi::move_immediate(ctx.idx, "Integer", term_grid::WIDE_TRAIL),
        abi::compare_registers(ctx.lo, ctx.idx),
        abi::branch_ne(&not_trail),
        // Trailing sentinel: clear the primary to the left (if any).
        abi::compare_immediate(col, "0"),
        abi::branch_le(&after),
        abi::subtract_immediate(ctx.idx, cell, term_grid::CELL_SIZE),
        abi::move_immediate(ctx.lo, "Integer", "0"),
        abi::store_u32(ctx.lo, ctx.idx, term_grid::C_GLYPH),
        abi::move_immediate(ctx.lo, "Integer", "1"),
        abi::store_u8(ctx.lo, ctx.idx, term_grid::C_WIDTH),
        abi::branch(&after),
        abi::label(&not_trail),
        // Wide primary (width 2): clear the trailing sentinel to the right.
        abi::load_u8(ctx.lo, cell, term_grid::C_WIDTH),
        abi::compare_immediate(ctx.lo, "2"),
        abi::branch_ne(&after),
        abi::add_immediate(ctx.idx, col, 1),
        abi::compare_registers(ctx.idx, ctx.cols),
        abi::branch_ge(&after),
        abi::add_immediate(ctx.idx, cell, term_grid::CELL_SIZE),
        abi::move_immediate(ctx.lo, "Integer", "0"),
        abi::store_u32(ctx.lo, ctx.idx, term_grid::C_GLYPH),
        abi::move_immediate(ctx.lo, "Integer", "1"),
        abi::store_u8(ctx.lo, ctx.idx, term_grid::C_WIDTH),
        abi::label(&after),
    ]);
}

/// Stamp a single cell `(row, col)` with `glyph` + the ctx attributes when it is
/// on the grid (`0<=row<rows`, `0<=col<cols`); otherwise branch to `skip` (placed
/// by the caller after the call). Used for `drawBox` corners.
fn emit_stamp_cell(
    ctx: &StampCtx,
    row: &str,
    col: &str,
    glyph: &str,
    width: &str,
    tag: &str,
    skip: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::compare_immediate(row, "0"),
        abi::branch_lt(skip),
        abi::compare_registers(row, ctx.rows),
        abi::branch_ge(skip),
        abi::compare_immediate(col, "0"),
        abi::branch_lt(skip),
        abi::compare_registers(col, ctx.cols),
        abi::branch_ge(skip),
        abi::multiply_registers(ctx.idx, row, ctx.cols),
        abi::add_registers(ctx.idx, ctx.idx, col),
        abi::shift_left_immediate(ctx.idx, ctx.idx, 4),
        abi::add_registers(ctx.cell, ctx.back, ctx.idx),
    ]);
    // plan-70-C Phase 2: clear the paired half of any wide glyph this cell was
    // part of, before overwriting it, so no orphaned half-glyph remains.
    emit_clear_wide_pair(ctx, ctx.cell, col, tag, instructions);
    instructions.extend([
        abi::store_u32(glyph, ctx.cell, term_grid::C_GLYPH),
        abi::store_u32(ctx.fg, ctx.cell, term_grid::C_FG),
        abi::store_u32(ctx.bg, ctx.cell, term_grid::C_BG),
        abi::store_u8(ctx.bold, ctx.cell, term_grid::C_BOLD),
        abi::store_u8(ctx.un, ctx.cell, term_grid::C_UN),
        // plan-70-C: record the cluster width so the presenter advances correctly
        // (and never inherits a stale width byte from a prior wide occupant).
        abi::store_u8(width, ctx.cell, term_grid::C_WIDTH),
    ]);
}

/// Load the grid pointer, dims, back base, and current attributes into the given
/// registers; branch to `inactive` when the grid is unallocated. Shared prologue
/// for the drawing helpers.
#[allow(clippy::too_many_arguments)]
fn emit_load_grid(
    term_state_offset: usize,
    gp: &str,
    rows: &str,
    cols: &str,
    back: &str,
    fg: &str,
    bg: &str,
    bold: &str,
    un: &str,
    inactive: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(inactive),
        abi::load_u64(rows, gp, 0),
        abi::load_u64(cols, gp, 8),
        abi::add_immediate(back, gp, term_grid::HDR_SIZE),
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
}

/// `term::drawHLine`/`drawVLine` (console): stamp a fixed box-drawing glyph across
/// a run of back-buffer cells with the current colours/attributes; the run is
/// shown on the next `term::sync`. A no-op while TUI mode is off or the grid is
/// unallocated — the same gate every writer honours (§4.2.1).
///
/// Arguments arrive in registers as `ARG[0]` = the `LineStyle` ordinal, then the
/// coordinates in row-before-column order:
///   drawHLine(line, row, columnA, columnB) — fixed `row`, span over columns.
///   drawVLine(line, rowA, column, rowB) — fixed `column`, span over rows.
/// Both name a start point `(row, column)` and then the far end of the run, so the
/// fixed coordinate is `ARG[1]` for the horizontal form and `ARG[2]` for the
/// vertical one.
/// `is_horizontal` selects, at emit time, the glyph table; the span endpoints may
/// be given in either order and are clamped to the grid; a fixed coordinate off
/// the grid, or a span with no on-grid cell, draws nothing.
fn emit_draw_line(
    symbol: &str,
    term_state_offset: usize,
    is_horizontal: bool,
    instructions: &mut Vec<CodeInstruction>,
) {
    let inactive = format!("{symbol}_inactive");
    let ord = "%v9";
    let fixed = "%v10";
    let ea = "%v11";
    let eb = "%v12";
    let gp = "%v13";
    let rows = "%v14";
    let cols = "%v15";
    let back = "%v16";
    let glyph = "%v17";
    let fg = "%v18";
    let bg = "%v19";
    let bold = "%v20";
    let un = "%v21";
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: "%v22",
        hi: "%v23",
        idx: "%v24",
        cell: "%v25",
        pos: "%v26",
        tmp: "%v27",
    };

    emit_gate_inactive(term_state_offset, &inactive, instructions);
    // Row-before-column argument order (both members name a start point `(row,
    // column)` and then the far end of the run), so the fixed coordinate is the
    // FIRST argument for `drawHLine(line, row, columnA, columnB)` and the SECOND
    // for `drawVLine(line, rowA, column, rowB)`.
    let (fixed_arg, ea_arg, eb_arg) = if is_horizontal { (1, 2, 3) } else { (2, 1, 3) };
    instructions.extend([
        abi::move_register(ord, abi::c_arg(0)),
        abi::move_register(fixed, abi::c_arg(fixed_arg)),
        abi::move_register(ea, abi::c_arg(ea_arg)),
        abi::move_register(eb, abi::c_arg(eb_arg)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    let table = if is_horizontal {
        &TERM_HLINE_CODEPOINTS
    } else {
        &TERM_VLINE_CODEPOINTS
    };
    emit_select_glyph(
        ord,
        glyph,
        table,
        &format!("{symbol}_dl_glyph"),
        instructions,
    );
    emit_stamp_run(
        &ctx,
        is_horizontal,
        fixed,
        ea,
        eb,
        glyph,
        &format!("{symbol}_dl"),
        &inactive,
        instructions,
    );
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::drawBox(line, rowA, columnA, rowB, columnB)` (console): draw a rectangle
/// in the given `LineStyle`. The two points are opposite corners, each written row
/// before column, and either corner may be given first. Draws the four edges — top/bottom horizontal runs and left/right
/// vertical runs, each using this style's own line glyph (so dashed/dotted styles
/// get dashed/dotted edges) — then overwrites the four corner cells with the
/// matching corner glyph (`*Dash`/`*Dot` reuse the Light or Heavy corners). Each
/// edge and corner is clamped independently, so a box partly off the grid draws
/// the visible part; a fully off-grid box draws nothing. Shown on the next
/// `term::sync`; a no-op while TUI mode is off.
fn emit_draw_box(symbol: &str, term_state_offset: usize, instructions: &mut Vec<CodeInstruction>) {
    let inactive = format!("{symbol}_inactive");
    let ord = "%v9";
    let ax1 = "%v10";
    let ay1 = "%v11";
    let ax2 = "%v12";
    let ay2 = "%v13";
    let gp = "%v14";
    let rows = "%v15";
    let cols = "%v16";
    let back = "%v17";
    let fg = "%v18";
    let bg = "%v19";
    let bold = "%v20";
    let un = "%v21";
    let xlo = "%v22";
    let xhi = "%v23";
    let ylo = "%v24";
    let yhi = "%v25";
    let hglyph = "%v26";
    let vglyph = "%v27";
    let ctl = "%v28";
    let ctr = "%v29";
    let cbl = "%v30";
    let cbr = "%v31";
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: "%v32",
        hi: "%v33",
        idx: "%v34",
        cell: "%v35",
        pos: "%v36",
        tmp: "%v37",
    };

    emit_gate_inactive(term_state_offset, &inactive, instructions);
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — every `term::` point is
    // written row before column — so the column registers are args 2 and 4 and the
    // row registers args 1 and 3.
    instructions.extend([
        abi::move_register(ord, abi::c_arg(0)),
        abi::move_register(ay1, abi::c_arg(1)),
        abi::move_register(ax1, abi::c_arg(2)),
        abi::move_register(ay2, abi::c_arg(3)),
        abi::move_register(ax2, abi::c_arg(4)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    // Normalise the two corners: xlo/xhi over columns, ylo/yhi over rows. Edges
    // and corners are placed from these, so left is always the smaller column etc.
    let x_ok = format!("{symbol}_box_x_ok");
    let y_ok = format!("{symbol}_box_y_ok");
    instructions.extend([
        abi::move_register(xlo, ax1),
        abi::move_register(xhi, ax2),
        abi::compare_registers(ax1, ax2),
        abi::branch_le(&x_ok),
        abi::move_register(xlo, ax2),
        abi::move_register(xhi, ax1),
        abi::label(&x_ok),
        abi::move_register(ylo, ay1),
        abi::move_register(yhi, ay2),
        abi::compare_registers(ay1, ay2),
        abi::branch_le(&y_ok),
        abi::move_register(ylo, ay2),
        abi::move_register(yhi, ay1),
        abi::label(&y_ok),
    ]);
    // Resolve the edge glyphs (this style's H/V forms) and the four corner glyphs.
    emit_select_glyph(
        ord,
        hglyph,
        &TERM_HLINE_CODEPOINTS,
        &format!("{symbol}_box_h"),
        instructions,
    );
    emit_select_glyph(
        ord,
        vglyph,
        &TERM_VLINE_CODEPOINTS,
        &format!("{symbol}_box_v"),
        instructions,
    );
    emit_select_glyph(
        ord,
        ctl,
        &TERM_CORNER_TL_CODEPOINTS,
        &format!("{symbol}_box_tl"),
        instructions,
    );
    emit_select_glyph(
        ord,
        ctr,
        &TERM_CORNER_TR_CODEPOINTS,
        &format!("{symbol}_box_tr"),
        instructions,
    );
    emit_select_glyph(
        ord,
        cbl,
        &TERM_CORNER_BL_CODEPOINTS,
        &format!("{symbol}_box_bl"),
        instructions,
    );
    emit_select_glyph(
        ord,
        cbr,
        &TERM_CORNER_BR_CODEPOINTS,
        &format!("{symbol}_box_br"),
        instructions,
    );
    // Four edges (each clamped/skipped independently), then four corners on top.
    // top: row ylo, cols xlo..xhi ; bottom: row yhi.
    let edges: &[(bool, &str, &str, &str, &str, &str)] = &[
        (true, ylo, xlo, xhi, hglyph, "e0"),
        (true, yhi, xlo, xhi, hglyph, "e1"),
        (false, xlo, ylo, yhi, vglyph, "e2"),
        (false, xhi, ylo, yhi, vglyph, "e3"),
    ];
    for (is_h, fixed, ea, eb, glyph, tag) in edges {
        let skip = format!("{symbol}_box_{tag}_skip");
        emit_stamp_run(
            &ctx,
            *is_h,
            fixed,
            ea,
            eb,
            glyph,
            &format!("{symbol}_box_{tag}"),
            &skip,
            instructions,
        );
        instructions.push(abi::label(&skip));
    }
    let corners: &[(&str, &str, &str, &str)] = &[
        (ylo, xlo, ctl, "cTL"),
        (ylo, xhi, ctr, "cTR"),
        (yhi, xlo, cbl, "cBL"),
        (yhi, xhi, cbr, "cBR"),
    ];
    // Box corners are single-width box-drawing glyphs (plan-70-C: pass width 1).
    instructions.push(abi::move_immediate(ctx.tmp, "Integer", "1"));
    for (row, col, glyph, tag) in corners {
        let skip = format!("{symbol}_box_{tag}_skip");
        emit_stamp_cell(&ctx, row, col, glyph, ctx.tmp, &skip, &skip, instructions);
        instructions.push(abi::label(&skip));
    }
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::fillRect(fill, rowA, columnA, rowB, columnB)` (console): fill the rectangle
/// between two opposite corners (each written row before column, either order first)
/// with the `FillStyle` block
/// or shade glyph, using the current colours/attributes. Implemented as one clamped
/// horizontal run per row (reusing the line stamper), so the region clamps to the
/// grid the same way; a fully off-grid rectangle fills nothing. Shown on the next
/// `term::sync`; a no-op while TUI mode is off.
fn emit_fill_rect(symbol: &str, term_state_offset: usize, instructions: &mut Vec<CodeInstruction>) {
    let inactive = format!("{symbol}_inactive");
    let ord = "%v9";
    let ax1 = "%v10";
    let ay1 = "%v11";
    let ax2 = "%v12";
    let ay2 = "%v13";
    let gp = "%v14";
    let rows = "%v15";
    let cols = "%v16";
    let back = "%v17";
    let fg = "%v18";
    let bg = "%v19";
    let bold = "%v20";
    let un = "%v21";
    let xlo = "%v22";
    let xhi = "%v23";
    let ylo = "%v24";
    let yhi = "%v25";
    let glyph = "%v26";
    let row = "%v27";
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: "%v28",
        hi: "%v29",
        idx: "%v30",
        cell: "%v31",
        pos: "%v32",
        tmp: "%v33",
    };

    emit_gate_inactive(term_state_offset, &inactive, instructions);
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — every `term::` point is
    // written row before column — so the column registers are args 2 and 4 and the
    // row registers args 1 and 3.
    instructions.extend([
        abi::move_register(ord, abi::c_arg(0)),
        abi::move_register(ay1, abi::c_arg(1)),
        abi::move_register(ax1, abi::c_arg(2)),
        abi::move_register(ay2, abi::c_arg(3)),
        abi::move_register(ax2, abi::c_arg(4)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    // Normalise the corners (xlo/xhi over columns, ylo/yhi over rows), then clamp
    // the row range to the grid (the per-row run clamps the columns).
    let x_ok = format!("{symbol}_fr_x_ok");
    let y_ok = format!("{symbol}_fr_y_ok");
    let ylo_ok = format!("{symbol}_fr_ylo_ok");
    let yhi_ok = format!("{symbol}_fr_yhi_ok");
    instructions.extend([
        abi::move_register(xlo, ax1),
        abi::move_register(xhi, ax2),
        abi::compare_registers(ax1, ax2),
        abi::branch_le(&x_ok),
        abi::move_register(xlo, ax2),
        abi::move_register(xhi, ax1),
        abi::label(&x_ok),
        abi::move_register(ylo, ay1),
        abi::move_register(yhi, ay2),
        abi::compare_registers(ay1, ay2),
        abi::branch_le(&y_ok),
        abi::move_register(ylo, ay2),
        abi::move_register(yhi, ay1),
        abi::label(&y_ok),
        abi::compare_immediate(ylo, "0"),
        abi::branch_ge(&ylo_ok),
        abi::move_immediate(ylo, "Integer", "0"),
        abi::label(&ylo_ok),
        abi::subtract_immediate(ctx.tmp, rows, 1),
        abi::compare_registers(yhi, ctx.tmp),
        abi::branch_le(&yhi_ok),
        abi::move_register(yhi, ctx.tmp),
        abi::label(&yhi_ok),
        abi::compare_registers(ylo, yhi),
        abi::branch_gt(&inactive),
    ]);
    emit_select_glyph(
        ord,
        glyph,
        &TERM_FILL_CODEPOINTS,
        &format!("{symbol}_fr_glyph"),
        instructions,
    );
    // One horizontal run per row over ylo..=yhi.
    let loop_row = format!("{symbol}_fr_row");
    let row_next = format!("{symbol}_fr_next");
    instructions.extend([
        abi::move_register(row, ylo),
        abi::label(&loop_row),
        abi::compare_registers(row, yhi),
        abi::branch_gt(&inactive),
    ]);
    emit_stamp_run(
        &ctx,
        true,
        row,
        xlo,
        xhi,
        glyph,
        &format!("{symbol}_fr"),
        &row_next,
        instructions,
    );
    instructions.extend([
        abi::label(&row_next),
        abi::add_immediate(row, row, 1),
        abi::branch(&loop_row),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// Runtime UTF-8 encode: `glyph = utf8_pack(cp)`, the grid glyph encoding (bytes
/// little-endian in a u32). `cp` is a runtime scalar in a register; `b`/`sh`/`mask`
/// are scratch. Branches on the four length ranges. Assumes `cp` is a valid scalar
/// (the caller guards control code points); an out-of-range value still produces a
/// well-formed 4-byte pack rather than corrupting anything.
fn emit_encode_utf8(
    cp: &str,
    glyph: &str,
    b: &str,
    sh: &str,
    mask: &str,
    tag: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let e1 = format!("{tag}_1");
    let e2 = format!("{tag}_2");
    let e3 = format!("{tag}_3");
    let done = format!("{tag}_done");
    // Emit `dst = 0x80 | ((cp >> shift) & 0x3F)` shifted into byte position `at`,
    // OR'd into `glyph`. `first` sets `glyph` directly (lead byte).
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.extend([
        abi::compare_immediate(cp, "128"),
        abi::branch_lt(&e1),
        abi::compare_immediate(cp, "2048"),
        abi::branch_lt(&e2),
        abi::compare_immediate(cp, "65536"),
        abi::branch_lt(&e3),
        // 4-byte: F0|cp>>18, then continuation bytes at 6/12/18-bit groups.
        abi::shift_right_immediate(sh, cp, 18),
        abi::move_immediate(b, "Integer", "240"),
        abi::or_registers(glyph, b, sh),
        abi::shift_right_immediate(sh, cp, 12),
        abi::and_registers(sh, sh, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 8),
        abi::or_registers(glyph, glyph, sh),
        abi::shift_right_immediate(sh, cp, 6),
        abi::and_registers(sh, sh, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 16),
        abi::or_registers(glyph, glyph, sh),
        abi::and_registers(sh, cp, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 24),
        abi::or_registers(glyph, glyph, sh),
        abi::branch(&done),
        // 3-byte: E0|cp>>12, then 6/12-bit continuation.
        abi::label(&e3),
        abi::shift_right_immediate(sh, cp, 12),
        abi::move_immediate(b, "Integer", "224"),
        abi::or_registers(glyph, b, sh),
        abi::shift_right_immediate(sh, cp, 6),
        abi::and_registers(sh, sh, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 8),
        abi::or_registers(glyph, glyph, sh),
        abi::and_registers(sh, cp, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 16),
        abi::or_registers(glyph, glyph, sh),
        abi::branch(&done),
        // 2-byte: C0|cp>>6, then 6-bit continuation.
        abi::label(&e2),
        abi::shift_right_immediate(sh, cp, 6),
        abi::move_immediate(b, "Integer", "192"),
        abi::or_registers(glyph, b, sh),
        abi::and_registers(sh, cp, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 8),
        abi::or_registers(glyph, glyph, sh),
        abi::branch(&done),
        // 1-byte: ASCII, glyph = cp.
        abi::label(&e1),
        abi::move_register(glyph, cp),
        abi::label(&done),
    ]);
}

/// `term::drawGlyph(row, column, codepoint)` (console): stamp a single Unicode scalar
/// at `row`/`column` with the current attributes; a no-op if the cell is off the
/// grid or `codepoint` is a control character (< 0x20, which would corrupt the
/// presented frame). Shown on the next `term::sync`.
fn emit_draw_glyph(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let inactive = format!("{symbol}_inactive");
    let x = "%v9";
    let y = "%v10";
    let cp = "%v11";
    let gp = "%v12";
    let rows = "%v13";
    let cols = "%v14";
    let back = "%v15";
    let fg = "%v16";
    let bg = "%v17";
    let bold = "%v18";
    let un = "%v19";
    let glyph = "%v20";
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: "%v21",
        hi: "%v22",
        idx: "%v23",
        cell: "%v24",
        pos: "%v25",
        tmp: "%v26",
    };
    // plan-70-C width path.
    let width = "%v30";
    let prop = "%v31";
    let wa = "%v32";
    let wb = "%v33";
    let wc = "%v34";
    let trailc = "%v35";
    let trailglyph = "%v36";
    let after_trail = format!("{symbol}_dg_aftertrail");
    let w_have = format!("{symbol}_dg_whave");

    emit_gate_inactive(term_state_offset, &inactive, instructions);
    // `drawGlyph(row, column, codepoint)` — the point is row-first.
    instructions.extend([
        abi::move_register(y, abi::c_arg(0)),
        abi::move_register(x, abi::c_arg(1)),
        abi::move_register(cp, abi::c_arg(2)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    // Skip control code points (would corrupt the presented escape stream).
    instructions.push(abi::compare_immediate(cp, "32"));
    instructions.push(abi::branch_lt(&inactive));
    emit_encode_utf8(
        cp,
        glyph,
        "%v27",
        "%v28",
        "%v29",
        &format!("{symbol}_dg_enc"),
        instructions,
    );
    // plan-70-C: a single glyph is one scalar (one cluster), so just its width. A
    // width-2 glyph reserves a WIDE_TRAIL neighbor at x+1; a zero-width glyph
    // (a lone combining mark) falls back to width 1.
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_dg_pp"),
        cp,
        prop,
        wa,
        instructions,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        prop,
        wa,
        wb,
        width,
        wc,
        instructions,
    );
    instructions.extend([
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&w_have),
        abi::move_immediate(width, "Integer", "1"),
        abi::label(&w_have),
    ]);
    emit_stamp_cell(
        &ctx,
        y,
        x,
        glyph,
        width,
        &format!("{symbol}_dgp"),
        &inactive,
        instructions,
    );
    instructions.extend([
        // Wide glyph: stamp the wide-trailing sentinel at x+1 (clipped by
        // emit_stamp_cell if off the right edge).
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&after_trail),
        abi::add_immediate(trailc, x, 1),
        abi::move_immediate(trailglyph, "Integer", term_grid::WIDE_TRAIL),
        abi::move_immediate(wa, "Integer", "0"),
    ]);
    emit_stamp_cell(
        &ctx,
        y,
        trailc,
        trailglyph,
        wa,
        &format!("{symbol}_dgt"),
        &after_trail,
        instructions,
    );
    instructions.push(abi::label(&after_trail));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::drawText(row, column, text)` (console): stamp `text` on `row` starting at
/// `column`, one grid cell per Unicode scalar, with the current attributes. It
/// does not move the shadow cursor, does not wrap or scroll, and clips at the right
/// edge (columns before 0 are skipped; the run stops at the last column). Control
/// characters are skipped (not stamped) but still advance a column. A no-op if the
/// row is off the grid or TUI mode is off. Shown on the next `term::sync`.
fn emit_draw_text(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let inactive = format!("{symbol}_inactive");
    let x = "%v9";
    let y = "%v10";
    let strobj = "%v11";
    let gp = "%v12";
    let rows = "%v13";
    let cols = "%v14";
    let back = "%v15";
    let fg = "%v16";
    let bg = "%v17";
    let bold = "%v18";
    let un = "%v19";
    let ptr = "%v20";
    let rem = "%v21";
    let col = "%v22";
    let b0 = "%v23";
    let len = "%v24";
    let glyph = "%v25";
    let t = "%v26";
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: "%v27",
        hi: "%v28",
        idx: "%v29",
        cell: "%v30",
        pos: "%v31",
        tmp: "%v32",
    };
    // plan-70-C cluster walk + EGC pool + wide handling (mirrors emit_grid_write).
    let cp = "%v33";
    let width = "%v34";
    let prop = "%v35";
    let sbc = "%v36";
    let sicb = "%v37";
    let clen = "%v38";
    let pptr = "%v39";
    let pb0 = "%v40";
    let plen = "%v41";
    let pcp = "%v42";
    let pprop = "%v43";
    let pbc = "%v44";
    let picb = "%v45";
    let pwidth = "%v46";
    let sa = "%v47";
    let sb = "%v48";
    let sc = "%v49";
    let sd = "%v50";
    let poolbase = "%v51";
    let poolslot = "%v52";
    let ncells = "%v53";
    let trailc = "%v54";
    let trailg = "%v55";
    let loop_top = format!("{symbol}_dt_loop");
    let advance = format!("{symbol}_dt_adv");
    let skip_ctrl = format!("{symbol}_dt_ctrl");
    let l2 = format!("{symbol}_dt_l2");
    let l3 = format!("{symbol}_dt_l3");
    let clamp = format!("{symbol}_dt_clamp");
    let pack = format!("{symbol}_dt_pack");
    let cellw = format!("{symbol}_dt_cellw");
    let peek_top = format!("{symbol}_dt_peek");
    let peek_done = format!("{symbol}_dt_peekdone");
    let peek_nb = format!("{symbol}_dt_peeknb");
    let peek_l2 = format!("{symbol}_dt_pl2");
    let peek_l3 = format!("{symbol}_dt_pl3");
    let peek_clamp = format!("{symbol}_dt_pclamp");
    let peek_hlen = format!("{symbol}_dt_phlen");
    let peek_wok = format!("{symbol}_dt_pwok");
    let w_have = format!("{symbol}_dt_whave");
    let store_inline = format!("{symbol}_dt_inline");
    let pool_copy_loop = format!("{symbol}_dt_pcopy");
    let pool_copy_done = format!("{symbol}_dt_pcopydone");

    emit_gate_inactive(term_state_offset, &inactive, instructions);
    // `drawText(row, column, text)` — the point is row-first.
    instructions.extend([
        abi::move_register(y, abi::c_arg(0)),
        abi::move_register(x, abi::c_arg(1)),
        abi::move_register(strobj, abi::c_arg(2)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    instructions.extend([
        // Row must be on the grid.
        abi::compare_immediate(y, "0"),
        abi::branch_lt(&inactive),
        abi::compare_registers(y, rows),
        abi::branch_ge(&inactive),
        // ptr = strobj + 8 (past the length word); rem = length; col = x.
        abi::add_immediate(ptr, strobj, 8),
        abi::load_u64(rem, strobj, 0),
        abi::move_register(col, x),
        abi::label(&loop_top),
        abi::compare_immediate(rem, "0"),
        abi::branch_eq(&inactive),
        // Clip at the right edge: once col reaches cols, nothing more is visible.
        abi::compare_registers(col, cols),
        abi::branch_ge(&inactive),
        abi::load_u8(b0, ptr, 0),
        // Control characters (< 0x20): skip stamping, advance one column + byte.
        abi::compare_immediate(b0, "32"),
        abi::branch_lt(&skip_ctrl),
        // UTF-8 length from the lead byte.
        abi::move_immediate(len, "Integer", "1"),
        abi::compare_immediate(b0, "128"),
        abi::branch_lo(&pack),
        abi::compare_immediate(b0, "224"),
        abi::branch_lo(&l2),
        abi::compare_immediate(b0, "240"),
        abi::branch_lo(&l3),
        abi::move_immediate(len, "Integer", "4"),
        abi::branch(&clamp),
        abi::label(&l2),
        abi::move_immediate(len, "Integer", "2"),
        abi::branch(&clamp),
        abi::label(&l3),
        abi::move_immediate(len, "Integer", "3"),
        abi::label(&clamp),
        // A truncated trailing sequence is treated as one raw byte.
        abi::compare_registers(len, rem),
        abi::branch_ls(&pack),
        abi::move_immediate(len, "Integer", "1"),
        abi::label(&pack),
        abi::move_register(glyph, b0),
        abi::compare_immediate(len, "2"),
        abi::branch_lo(&cellw),
        abi::load_u8(t, ptr, 1),
        abi::shift_left_immediate(t, t, 8),
        abi::or_registers(glyph, glyph, t),
        abi::compare_immediate(len, "3"),
        abi::branch_lo(&cellw),
        abi::load_u8(t, ptr, 2),
        abi::shift_left_immediate(t, t, 16),
        abi::or_registers(glyph, glyph, t),
        abi::compare_immediate(len, "4"),
        abi::branch_lo(&cellw),
        abi::load_u8(t, ptr, 3),
        abi::shift_left_immediate(t, t, 24),
        abi::or_registers(glyph, glyph, t),
        abi::label(&cellw),
    ]);
    // Base scalar → codepoint → boundclass/icb/charwidth; then fold following
    // non-breaking scalars into the cluster (plan-70-C, mirrors emit_grid_write).
    crate::codegen::string::unicode_props::emit_utf8_codepoint_by_len(
        &format!("{symbol}_dtd"),
        ptr,
        len,
        b0,
        cp,
        sa,
        sb,
        instructions,
    );
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_dtb"),
        cp,
        prop,
        sa,
        instructions,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        prop,
        sbc,
        sicb,
        width,
        sa,
        instructions,
    );
    instructions.push(abi::move_register(clen, len));
    instructions.push(abi::label(&peek_top));
    instructions.extend([
        abi::subtract_registers(sc, rem, clen),
        abi::compare_immediate(sc, "0"),
        abi::branch_eq(&peek_done),
        abi::add_registers(pptr, ptr, clen),
        abi::load_u8(pb0, pptr, 0),
        abi::compare_immediate(pb0, "32"),
        abi::branch_lt(&peek_done),
        abi::move_immediate(plen, "Integer", "1"),
        abi::compare_immediate(pb0, "128"),
        abi::branch_lo(&peek_hlen),
        abi::compare_immediate(pb0, "224"),
        abi::branch_lo(&peek_l2),
        abi::compare_immediate(pb0, "240"),
        abi::branch_lo(&peek_l3),
        abi::move_immediate(plen, "Integer", "4"),
        abi::branch(&peek_clamp),
        abi::label(&peek_l2),
        abi::move_immediate(plen, "Integer", "2"),
        abi::branch(&peek_clamp),
        abi::label(&peek_l3),
        abi::move_immediate(plen, "Integer", "3"),
        abi::label(&peek_clamp),
        abi::compare_registers(plen, sc),
        abi::branch_ls(&peek_hlen),
        abi::move_immediate(plen, "Integer", "1"),
        abi::label(&peek_hlen),
        abi::add_registers(sc, clen, plen),
        abi::move_immediate(sd, "Integer", &term_grid::POOL_BYTES_PER_CELL.to_string()),
        abi::compare_registers(sc, sd),
        abi::branch_hi(&peek_done),
    ]);
    crate::codegen::string::unicode_props::emit_utf8_codepoint_by_len(
        &format!("{symbol}_dtpd"),
        pptr,
        plen,
        pb0,
        pcp,
        sa,
        sb,
        instructions,
    );
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_dtpl"),
        pcp,
        pprop,
        sa,
        instructions,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        pprop,
        pbc,
        picb,
        pwidth,
        sa,
        instructions,
    );
    crate::codegen::string::unicode_props::emit_grapheme_break_branch_free(
        &format!("{symbol}_dtpb"),
        sbc,
        sicb,
        pbc,
        picb,
        &peek_done,
        &peek_nb,
        instructions,
    );
    instructions.push(abi::label(&peek_nb));
    instructions.extend([
        abi::add_registers(clen, clen, plen),
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&peek_wok),
        abi::move_register(width, pwidth),
        abi::label(&peek_wok),
    ]);
    crate::codegen::string::unicode_props::emit_grapheme_state_update_free(
        &format!("{symbol}_dtps"),
        sbc,
        sicb,
        pbc,
        picb,
        instructions,
    );
    instructions.push(abi::branch(&peek_top));
    instructions.push(abi::label(&peek_done));
    instructions.extend([
        // A lone zero-width cluster still takes one cell / one column.
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&w_have),
        abi::move_immediate(width, "Integer", "1"),
        abi::label(&w_have),
        // Clip a wide cluster that would exceed the right edge: drop it and stop
        // the run (never split a wide glyph across the edge).
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&format!("{symbol}_dt_afterclip")),
        abi::add_immediate(sa, col, 1),
        abi::compare_registers(sa, cols),
        abi::branch_lo(&format!("{symbol}_dt_afterclip")),
        abi::branch(&inactive),
        abi::label(&format!("{symbol}_dt_afterclip")),
        // Glyph: inline single scalar, else pooled into the on-grid cell's slot.
        abi::compare_registers(clen, len),
        abi::branch_eq(&store_inline),
        abi::move_immediate(glyph, "Integer", &term_grid::GLYPH_POOLED_TAG.to_string()),
        abi::or_registers(glyph, glyph, clen),
        abi::compare_immediate(col, "0"),
        abi::branch_lt(&store_inline),
        // pool_base = gp + HDR + ncells*(2*CELL+OUTBUF); slot = pool_base + (y*cols+col)*POOL
        abi::multiply_registers(ncells, rows, cols),
        abi::move_immediate(
            sa,
            "Integer",
            &(2 * term_grid::CELL_SIZE + term_grid::OUTBUF_PER_CELL).to_string(),
        ),
        abi::multiply_registers(sa, ncells, sa),
        abi::add_immediate(sa, sa, term_grid::HDR_SIZE),
        abi::add_registers(poolbase, gp, sa),
        abi::multiply_registers(sa, y, cols),
        abi::add_registers(sa, sa, col),
        abi::move_immediate(sc, "Integer", &term_grid::POOL_BYTES_PER_CELL.to_string()),
        abi::multiply_registers(sc, sa, sc),
        abi::add_registers(poolslot, poolbase, sc),
        abi::move_immediate(sc, "Integer", "0"),
        abi::label(&pool_copy_loop),
        abi::compare_registers(sc, clen),
        abi::branch_ge(&pool_copy_done),
        abi::add_registers(sd, ptr, sc),
        abi::load_u8(sa, sd, 0),
        abi::add_registers(sd, poolslot, sc),
        abi::store_u8(sa, sd, 0),
        abi::add_immediate(sc, sc, 1),
        abi::branch(&pool_copy_loop),
        abi::label(&pool_copy_done),
        abi::label(&store_inline),
    ]);
    // Stamp the primary at (y, col) — off-grid (col<0 or off-row) skips to advance.
    emit_stamp_cell(
        &ctx,
        y,
        col,
        glyph,
        width,
        &format!("{symbol}_dtp"),
        &advance,
        instructions,
    );
    instructions.extend([
        // Wide glyph: stamp the wide-trailing sentinel at col+1.
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&advance),
        abi::add_immediate(trailc, col, 1),
        abi::move_immediate(trailg, "Integer", term_grid::WIDE_TRAIL),
        abi::move_immediate(sa, "Integer", "0"),
    ]);
    emit_stamp_cell(
        &ctx,
        y,
        trailc,
        trailg,
        sa,
        &format!("{symbol}_dtt"),
        &advance,
        instructions,
    );
    instructions.extend([
        abi::label(&advance),
        abi::add_registers(col, col, width),
        abi::add_registers(ptr, ptr, clen),
        abi::subtract_registers(rem, rem, clen),
        abi::branch(&loop_top),
        // Control character: advance one column and one byte without stamping.
        abi::label(&skip_ctrl),
        abi::add_immediate(col, col, 1),
        abi::add_immediate(ptr, ptr, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_top),
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
    // Allocate the 4-field `color::Color` record (plan-122-F widened it from the
    // retired 3-field `TermColor`).
    instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &COLOR_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
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
        // alpha: a terminal cell has no alpha channel, so the fourth field is
        // always fully opaque. Stored as an immediate rather than unpacked from the
        // state slot, which only carries 0xBBGGRR (plan-122-F).
        abi::move_immediate("%v13", "Integer", "255"),
        abi::store_u64("%v13", "%v9", 24),
        abi::move_register(RESULT_VALUE_REGISTER, "%v9"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&alloc_error),
    ]);
    raise_error_into(symbol, "ErrOutOfMemory", instructions, relocations);
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
        abi::move_immediate(abi::c_arg(1), "Integer", request),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), SCRATCH_OFFSET),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
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
        abi::store_u64("%v11", abi::mfb_return(1), 0),
        abi::store_u64("%v10", abi::mfb_return(1), 8),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&unsupported),
    ]);
    emit_unsupported(symbol, ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(done), abi::label(&alloc_error)]);
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
    Ok(())
}
