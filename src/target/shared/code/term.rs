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
use crate::arch::aarch64::abi;

// Frame layout. `sp+0` is reserved for the Darwin variadic `ioctl` spill that
// `emit_terminal_size` performs (it stores `x2` to `sp+0`), so the saved link
// register lives at the top of the frame, well clear of it.
const FRAME_SIZE: usize = 80;
const LR_OFFSET: usize = 64;
const ARG0_OFFSET: usize = 8;
const ARG1_OFFSET: usize = 16;
const ARG2_OFFSET: usize = 24;
/// Scratch buffer for runtime decimal formatting and the `winsize` struct.
const SCRATCH_OFFSET: usize = 32;
const SCRATCH_END: usize = 56;

const DARWIN_TIOCGWINSZ: &str = "1074295912";
const LINUX_TIOCGWINSZ: &str = "21523";

// Fixed ANSI escape-sequence byte strings (ESC = 0x1b). `term::on` resets state
// to defaults and switches to the alternate screen; `term::off` restores it.
const ESC_ON: &[u8] =
    b"\x1b[?1049h\x1b[0m\x1b[38;2;255;255;255m\x1b[48;2;0;0;0m\x1b[2J\x1b[H\x1b[?25h";
const ESC_OFF: &[u8] = b"\x1b[?25h\x1b[?1049l\x1b[0m";
const ESC_CLEAR: &[u8] = b"\x1b[2J\x1b[H";
const ESC_BOLD_ON: &[u8] = b"\x1b[1m";
const ESC_BOLD_OFF: &[u8] = b"\x1b[22m";
const ESC_UNDERLINE_ON: &[u8] = b"\x1b[4m";
const ESC_UNDERLINE_OFF: &[u8] = b"\x1b[24m";
const ESC_SHOW_CURSOR: &[u8] = b"\x1b[?25h";
const ESC_HIDE_CURSOR: &[u8] = b"\x1b[?25l";
const ESC_FG_PREFIX: &[u8] = b"\x1b[38;2;";
const ESC_BG_PREFIX: &[u8] = b"\x1b[48;2;";
const ESC_SEMICOLON: &[u8] = b";";
const ESC_LETTER_M: &[u8] = b"m";
const ESC_BRACKET: &[u8] = b"\x1b[";
const ESC_LETTER_H: &[u8] = b"H";

const ESC_ON_SYMBOL: &str = "_mfb_term_esc_on";
const ESC_OFF_SYMBOL: &str = "_mfb_term_esc_off";
const ESC_CLEAR_SYMBOL: &str = "_mfb_term_esc_clear";
const ESC_BOLD_ON_SYMBOL: &str = "_mfb_term_esc_bold_on";
const ESC_BOLD_OFF_SYMBOL: &str = "_mfb_term_esc_bold_off";
const ESC_UNDERLINE_ON_SYMBOL: &str = "_mfb_term_esc_underline_on";
const ESC_UNDERLINE_OFF_SYMBOL: &str = "_mfb_term_esc_underline_off";
const ESC_SHOW_CURSOR_SYMBOL: &str = "_mfb_term_esc_show_cursor";
const ESC_HIDE_CURSOR_SYMBOL: &str = "_mfb_term_esc_hide_cursor";
const ESC_FG_PREFIX_SYMBOL: &str = "_mfb_term_esc_fg_prefix";
const ESC_BG_PREFIX_SYMBOL: &str = "_mfb_term_esc_bg_prefix";
const ESC_SEMICOLON_SYMBOL: &str = "_mfb_term_esc_semicolon";
const ESC_LETTER_M_SYMBOL: &str = "_mfb_term_esc_m";
const ESC_BRACKET_SYMBOL: &str = "_mfb_term_esc_bracket";
const ESC_LETTER_H_SYMBOL: &str = "_mfb_term_esc_h";

const TERM_COLOR_RECORD_SIZE: usize = 24;
const TERM_SIZE_RECORD_SIZE: usize = 16;
/// Default foreground while inactive (white, packed `r | g<<8 | b<<16`).
const DEFAULT_FOREGROUND_PACKED: &str = "16777215";

fn esc_entries() -> &'static [(&'static str, &'static [u8])] {
    &[
        (ESC_ON_SYMBOL, ESC_ON),
        (ESC_OFF_SYMBOL, ESC_OFF),
        (ESC_CLEAR_SYMBOL, ESC_CLEAR),
        (ESC_BOLD_ON_SYMBOL, ESC_BOLD_ON),
        (ESC_BOLD_OFF_SYMBOL, ESC_BOLD_OFF),
        (ESC_UNDERLINE_ON_SYMBOL, ESC_UNDERLINE_ON),
        (ESC_UNDERLINE_OFF_SYMBOL, ESC_UNDERLINE_OFF),
        (ESC_SHOW_CURSOR_SYMBOL, ESC_SHOW_CURSOR),
        (ESC_HIDE_CURSOR_SYMBOL, ESC_HIDE_CURSOR),
        (ESC_FG_PREFIX_SYMBOL, ESC_FG_PREFIX),
        (ESC_BG_PREFIX_SYMBOL, ESC_BG_PREFIX),
        (ESC_SEMICOLON_SYMBOL, ESC_SEMICOLON),
        (ESC_LETTER_M_SYMBOL, ESC_LETTER_M),
        (ESC_BRACKET_SYMBOL, ESC_BRACKET),
        (ESC_LETTER_H_SYMBOL, ESC_LETTER_H),
    ]
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
fn emit_write_const(
    from: &str,
    symbol: &str,
    len: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    load_data_address(
        from,
        symbol,
        abi::string_data_register(),
        instructions,
        relocations,
    );
    instructions.push(abi::move_immediate(
        abi::string_length_register(),
        "Integer",
        &len.to_string(),
    ));
    instructions.push(abi::move_immediate(abi::return_register(), "Integer", "1"));
    platform.emit_write(from, platform_imports, instructions, relocations)
}

/// Format the unsigned value in `value_reg` as ASCII decimal into the scratch
/// buffer and write it to stdout. Uses x9..x15 as scratch.
fn emit_write_decimal(
    from: &str,
    value_reg: &str,
    tag: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let loop_label = format!("{from}_dec_{tag}");
    instructions.extend([
        abi::move_register("%v10", value_reg),
        abi::move_immediate("%v11", "Integer", "10"),
        abi::add_immediate("%v12", abi::stack_pointer(), SCRATCH_END),
        abi::label(&loop_label),
        abi::unsigned_divide_registers("%v13", "%v10", "%v11"),
        abi::multiply_subtract_registers("%v14", "%v13", "%v11", "%v10"),
        abi::add_immediate("%v14", "%v14", 48),
        abi::subtract_immediate("%v12", "%v12", 1),
        abi::store_u8("%v14", "%v12", 0),
        abi::move_register("%v10", "%v13"),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&loop_label),
        abi::add_immediate("%v9", abi::stack_pointer(), SCRATCH_END),
        abi::subtract_registers(abi::string_length_register(), "%v9", "%v12"),
        abi::move_register(abi::string_data_register(), "%v12"),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
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
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2): the decimal/record-build scratch buffers
    // are an explicit sp-relative local region; x9-x15 scratch becomes vregs.
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    match call {
        "term.on" => emit_on(
            symbol,
            term_state_offset,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.off" => emit_off(
            symbol,
            term_state_offset,
            &done,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.isOn" => emit_is_on(term_state_offset, &mut instructions),
        "term.setForeground" => emit_set_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_FG_OFFSET,
            ESC_FG_PREFIX_SYMBOL,
            ESC_FG_PREFIX.len(),
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.setBackground" => emit_set_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BG_OFFSET,
            ESC_BG_PREFIX_SYMBOL,
            ESC_BG_PREFIX.len(),
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.setBold" => emit_set_attr(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BOLD_OFFSET,
            ESC_BOLD_ON_SYMBOL,
            ESC_BOLD_ON.len(),
            ESC_BOLD_OFF_SYMBOL,
            ESC_BOLD_OFF.len(),
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.setUnderline" => emit_set_attr(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_UNDERLINE_OFFSET,
            ESC_UNDERLINE_ON_SYMBOL,
            ESC_UNDERLINE_ON.len(),
            ESC_UNDERLINE_OFF_SYMBOL,
            ESC_UNDERLINE_OFF.len(),
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.showCursor" => emit_surface(
            symbol,
            term_state_offset,
            Some((term_state_offset + TERM_STATE_CURSOR_VISIBLE_OFFSET, 1)),
            ESC_SHOW_CURSOR_SYMBOL,
            ESC_SHOW_CURSOR.len(),
            &done,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.hideCursor" => emit_surface(
            symbol,
            term_state_offset,
            Some((term_state_offset + TERM_STATE_CURSOR_VISIBLE_OFFSET, 0)),
            ESC_HIDE_CURSOR_SYMBOL,
            ESC_HIDE_CURSOR.len(),
            &done,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.clear" => emit_surface(
            symbol,
            term_state_offset,
            None,
            ESC_CLEAR_SYMBOL,
            ESC_CLEAR.len(),
            &done,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        "term.moveTo" => emit_move_to(
            symbol,
            term_state_offset,
            &done,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
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
            symbol,
            term_state_offset,
            &done,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?,
        other => return Err(format!("unknown term runtime helper '{other}'")),
    }

    instructions.push(abi::label(&done));
    instructions.push(abi::return_());

    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], LR_OFFSET);
    Ok((frame, instructions, relocations, stack_slots))
}

#[allow(clippy::too_many_arguments)]
fn emit_on(
    symbol: &str,
    term_state_offset: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
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
        instructions.push(abi::move_immediate("%v9", "Integer", value));
        instructions.push(abi::store_u64(
            "%v9",
            ARENA_STATE_REGISTER,
            term_state_offset + offset,
        ));
    }
    emit_write_const(
        symbol,
        ESC_ON_SYMBOL,
        ESC_ON.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_off(
    symbol: &str,
    term_state_offset: usize,
    done: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    emit_write_const(
        symbol,
        ESC_OFF_SYMBOL,
        ESC_OFF.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::move_immediate("%v9", "Integer", "0"));
    instructions.push(abi::store_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
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

#[allow(clippy::too_many_arguments)]
fn emit_set_color(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    prefix_symbol: &str,
    prefix_len: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let inactive = format!("{symbol}_inactive");
    // Save r/g/b before any write clobbers x0/x1/x2.
    instructions.push(abi::store_u64("x0", abi::stack_pointer(), ARG0_OFFSET));
    instructions.push(abi::store_u64("x1", abi::stack_pointer(), ARG1_OFFSET));
    instructions.push(abi::store_u64("x2", abi::stack_pointer(), ARG2_OFFSET));
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    // Pack r | g<<8 | b<<16 and store to the state attribute.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), ARG0_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), ARG1_OFFSET),
        abi::load_u64("%v11", abi::stack_pointer(), ARG2_OFFSET),
        abi::shift_left_immediate("%v10", "%v10", 8),
        abi::shift_left_immediate("%v11", "%v11", 16),
        abi::or_registers("%v9", "%v9", "%v10"),
        abi::or_registers("%v9", "%v9", "%v11"),
        abi::store_u64("%v9", ARENA_STATE_REGISTER, state_offset),
    ]);
    emit_write_const(
        symbol,
        prefix_symbol,
        prefix_len,
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::load_u64("%v15", abi::stack_pointer(), ARG0_OFFSET));
    emit_write_decimal(
        symbol,
        "%v15",
        "r",
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    emit_write_const(
        symbol,
        ESC_SEMICOLON_SYMBOL,
        ESC_SEMICOLON.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::load_u64("%v15", abi::stack_pointer(), ARG1_OFFSET));
    emit_write_decimal(
        symbol,
        "%v15",
        "g",
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    emit_write_const(
        symbol,
        ESC_SEMICOLON_SYMBOL,
        ESC_SEMICOLON.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::load_u64("%v15", abi::stack_pointer(), ARG2_OFFSET));
    emit_write_decimal(
        symbol,
        "%v15",
        "b",
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    emit_write_const(
        symbol,
        ESC_LETTER_M_SYMBOL,
        ESC_LETTER_M.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_set_attr(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    on_symbol: &str,
    on_len: usize,
    off_symbol: &str,
    off_len: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let inactive = format!("{symbol}_inactive");
    let off_label = format!("{symbol}_attr_off");
    let written = format!("{symbol}_attr_written");
    instructions.push(abi::store_u64("x0", abi::stack_pointer(), ARG0_OFFSET));
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    instructions.push(abi::load_u64("%v9", abi::stack_pointer(), ARG0_OFFSET));
    instructions.push(abi::store_u64("%v9", ARENA_STATE_REGISTER, state_offset));
    instructions.push(abi::load_u64("%v9", abi::stack_pointer(), ARG0_OFFSET));
    instructions.push(abi::compare_immediate("%v9", "0"));
    instructions.push(abi::branch_eq(&off_label));
    emit_write_const(
        symbol,
        on_symbol,
        on_len,
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::branch(&written));
    instructions.push(abi::label(&off_label));
    emit_write_const(
        symbol,
        off_symbol,
        off_len,
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&written));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_surface(
    symbol: &str,
    term_state_offset: usize,
    cursor_update: Option<(usize, u64)>,
    esc_symbol: &str,
    esc_len: usize,
    done: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    if let Some((offset, value)) = cursor_update {
        instructions.push(abi::move_immediate("%v9", "Integer", &value.to_string()));
        instructions.push(abi::store_u64("%v9", ARENA_STATE_REGISTER, offset));
    }
    emit_write_const(
        symbol,
        esc_symbol,
        esc_len,
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    let _ = done;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_move_to(
    symbol: &str,
    term_state_offset: usize,
    done: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let inactive = format!("{symbol}_inactive");
    let row_clamp = format!("{symbol}_row_ok");
    let col_clamp = format!("{symbol}_col_ok");
    instructions.push(abi::store_u64("x0", abi::stack_pointer(), ARG0_OFFSET));
    instructions.push(abi::store_u64("x1", abi::stack_pointer(), ARG1_OFFSET));
    emit_gate_inactive(term_state_offset, &inactive, instructions);
    emit_write_const(
        symbol,
        ESC_BRACKET_SYMBOL,
        ESC_BRACKET.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    // row (0-based) clamped to >= 0, then +1 for 1-based ANSI.
    instructions.extend([
        abi::load_u64("%v15", abi::stack_pointer(), ARG0_OFFSET),
        abi::compare_immediate("%v15", "0"),
        abi::branch_ge(&row_clamp),
        abi::move_immediate("%v15", "Integer", "0"),
        abi::label(&row_clamp),
        abi::add_immediate("%v15", "%v15", 1),
    ]);
    emit_write_decimal(
        symbol,
        "%v15",
        "row",
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    emit_write_const(
        symbol,
        ESC_SEMICOLON_SYMBOL,
        ESC_SEMICOLON.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v15", abi::stack_pointer(), ARG1_OFFSET),
        abi::compare_immediate("%v15", "0"),
        abi::branch_ge(&col_clamp),
        abi::move_immediate("%v15", "Integer", "0"),
        abi::label(&col_clamp),
        abi::add_immediate("%v15", "%v15", 1),
    ]);
    emit_write_decimal(
        symbol,
        "%v15",
        "col",
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    emit_write_const(
        symbol,
        ESC_LETTER_H_SYMBOL,
        ESC_LETTER_H.len(),
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    let _ = done;
    Ok(())
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
        abi::move_immediate("x1", "Integer", "8"),
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

#[allow(clippy::too_many_arguments)]
fn emit_terminal_size(
    symbol: &str,
    term_state_offset: usize,
    done: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let unsupported = format!("{symbol}_unsupported");
    let active = format!("{symbol}_active");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let request = if platform.target() == "macos-aarch64" {
        DARWIN_TIOCGWINSZ
    } else {
        LINUX_TIOCGWINSZ
    };
    // Gate: terminalSize is the one read with no inert value; while inactive it
    // returns ERR_UNSUPPORTED_OPERATION (§4.7).
    instructions.push(abi::load_u64(
        "%v9",
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    instructions.push(abi::compare_immediate("%v9", "0"));
    instructions.push(abi::branch_ne(&active));
    emit_unsupported(symbol, instructions, relocations);
    instructions.push(abi::branch(done));
    instructions.push(abi::label(&active));
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_immediate("x1", "Integer", request),
        abi::add_immediate("x2", abi::stack_pointer(), SCRATCH_OFFSET),
    ]);
    platform.emit_terminal_size(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
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
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::load_u64("%v10", abi::stack_pointer(), ARG0_OFFSET),
        abi::load_u64("%v11", abi::stack_pointer(), ARG1_OFFSET),
        abi::store_u64("%v11", "x1", 0),
        abi::store_u64("%v10", "x1", 8),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&unsupported),
    ]);
    emit_unsupported(symbol, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, instructions, relocations);
    Ok(())
}
